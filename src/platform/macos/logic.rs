//! The macOS collector's decisions, kept free of FFI so they compile and are tested on Linux
//! too (`platform/mod.rs` includes this file in Linux test builds).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::io_detail::OpenFile;
use crate::model::{CoreKind, Kind, Oom, Policy, Process, Queue};
use crate::platform::{
    AccessMode, DeletedFile, Files, Network, PipeEnd, PipeId, Remote, Source, Sources,
};

/// Executables that run virtual machines or containers. A trailing `*` matches any suffix.
const VIRTUALIZATION: [&str; 8] = [
    "com.apple.Virtualization.VirtualMachine",
    "qemu-system-*",
    "limactl",
    "vfkit",
    "krunkit",
    "OrbStack Helper",
    "com.docker.*",
    "Docker Desktop",
];

/// Directories that hold software shipped with macOS. `/usr/local/` is excluded: it is where
/// users install their own software.
const SYSTEM_PATHS: [&str; 5] = ["/System/", "/usr/", "/bin/", "/sbin/", "/Library/Apple/"];

/// Users below this id are macOS service accounts; people start at 501.
const FIRST_PERSON_UID: u32 = 500;

/// What a process is, from its owner and executable.
pub fn classify(pid: u32, uid: u32, my_uid: u32, path: &str, name: &str) -> Kind {
    let file = file_name(path);
    let virtualization = |candidate: &str| {
        !candidate.is_empty()
            && VIRTUALIZATION
                .iter()
                .any(|pattern| match pattern.strip_suffix('*') {
                    Some(prefix) => candidate.starts_with(prefix),
                    None => candidate == *pattern,
                })
    };
    if pid == 0 {
        Kind::Kernel
    } else if virtualization(file) || virtualization(name) {
        Kind::Container
    } else if uid < FIRST_PERSON_UID
        || (SYSTEM_PATHS.iter().any(|prefix| path.starts_with(prefix))
            && !path.starts_with("/usr/local/"))
    {
        Kind::System
    } else if uid == my_uid {
        Kind::Session
    } else {
        Kind::System
    }
}

/// The application a process belongs to: the outermost `.app` bundle in its path, so helpers
/// inside `Safari.app/Contents/...` group with Safari, or else its executable's file name.
pub fn group(path: &str, name: &str) -> String {
    if let Some(bundle) = path
        .split('/')
        .find(|part| part.len() > ".app".len() && part.ends_with(".app"))
    {
        return bundle.to_owned();
    }
    match file_name(path) {
        "" => name.to_owned(),
        file => file.to_owned(),
    }
}

/// A stable flock key standing in for a cgroup, such as `/session/Safari.app`.
pub fn cgroup_path(kind: Kind, group: &str) -> String {
    let kind = match kind {
        Kind::Kernel => "kernel",
        Kind::System => "system",
        Kind::Session => "session",
        Kind::Container => "container",
    };
    format!("/{kind}/{group}")
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or_default()
}

/// Maps a Mach task priority onto Linux's scale, where lower runs first. Mach runs higher
/// numbers first and gives applications 31 by default, which reads as Linux's 20 (nice 0).
/// Background work at 4 reads as 47; kernel and real-time threads above 51 read as negative,
/// like Linux's real-time priorities.
pub fn priority_from_mach(priority: i32) -> i32 {
    51 - priority
}

/// The scheduling policy of a Mach policy number (`pti_policy`, from osfmk/mach/policy.h):
/// 1 is timeshare, 2 round robin and 4 FIFO. Other numbers read as unknown.
pub fn policy_from_mach(policy: i32) -> Policy {
    match policy {
        1 => Policy::Normal,
        2 => Policy::RoundRobin,
        4 => Policy::Fifo,
        _ => Policy::Unknown,
    }
}

/// When a process started, in seconds since the Unix epoch, from `pbi_start_tvsec` and
/// `pbi_start_tvusec`.
pub fn start_seconds(seconds: u64, microseconds: u64) -> f64 {
    seconds as f64 + microseconds as f64 / 1e6
}

/// The executable path and argv from a `KERN_PROCARGS2` buffer: a native-endian `int argc`,
/// the NUL-terminated path, NUL padding, then `argc` NUL-terminated arguments and the
/// environment. A truncated buffer yields the arguments it holds, the last one cut short.
pub fn parse_procargs(bytes: &[u8]) -> Option<(String, Vec<String>)> {
    let argc = i32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok()?;
    let rest = &bytes[4..];
    let path_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    let path = String::from_utf8_lossy(&rest[..path_end]).into_owned();
    let mut rest = &rest[path_end..];
    while let [0, tail @ ..] = rest {
        rest = tail;
    }
    let mut argv = Vec::new();
    while argv.len() < argc && !rest.is_empty() {
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        argv.push(String::from_utf8_lossy(&rest[..end]).into_owned());
        rest = rest.get(end + 1..).unwrap_or_default();
    }
    Some((path, argv))
}

/// Mach absolute time in nanoseconds. Apple Silicon counts at 24 MHz (125/3 ns per tick) and
/// Intel in nanoseconds (1/1); the product is taken in 128 bits so it cannot overflow.
pub fn nanoseconds(ticks: u64, numer: u32, denom: u32) -> u64 {
    let nanoseconds = u128::from(ticks) * u128::from(numer) / u128::from(denom.max(1));
    u64::try_from(nanoseconds).unwrap_or(u64::MAX)
}

/// Whether `pbi_status` is SZOMB (5 in xnu bsd/sys/proc.h): the process has exited and has no
/// task left to report on.
pub fn zombie(status: u32) -> bool {
    status == 5
}

/// Whether a description read when the process had `before` (pid version, name) still holds
/// `now`. exec keeps the pid and start time, which key the cache, and can keep the name (a
/// shell that runs `exec zsh`, a self-restart), but the kernel gives every exec a new pid
/// version.
pub fn same_program(before: (i32, &str), now: (i32, &str)) -> bool {
    before == now
}

/// The Linux-style state letter from `pbi_status` and the task's running thread count. macOS
/// has no uninterruptible sleep, so never 'D'.
pub fn state(status: u32, running: i32) -> char {
    // SSTOP from xnu bsd/sys/proc.h.
    const STOPPED: u32 = 4;
    match status {
        _ if zombie(status) => 'Z',
        STOPPED => 'T',
        _ if running > 0 => 'R',
        _ => 'S',
    }
}

/// Share of the interval a CPU was busy, from two readings of its `[user, system, idle, nice]`
/// scheduler ticks. The counters are 32-bit and wrap, so differences are taken modulo 2^32.
/// None when no ticks passed between the readings.
pub fn busy(previous: [u32; 4], current: [u32; 4]) -> Option<f32> {
    let [user, system, idle, nice] =
        [0, 1, 2, 3].map(|i| u64::from(current[i].wrapping_sub(previous[i])));
    let total = user + system + idle + nice;
    if total == 0 {
        return None;
    }
    Some(((user + system + nice) as f32 / total as f32).clamp(0.0, 1.0))
}

/// A stand-in for the memory stall percentage from `kern.memorystatus_vm_pressure_level`
/// (1 normal, 2 warning, 4 critical). macOS has no stall accounting, so this is a level.
pub fn pressure_from_level(level: i32) -> f32 {
    match level {
        2 => 25.0,
        4 => 75.0,
        _ => 0.0,
    }
}

/// The pressure readings from the memory pressure level, None when it could not be read, and
/// the pressures missing: macOS has no CPU or I/O stall accounting at all.
pub fn pressure(level: Option<i32>) -> ([f32; 3], Sources) {
    let mut missing = Sources::from([Source::CpuPressure, Source::IoPressure]);
    let memory = level.map_or_else(
        || {
            missing.insert(Source::MemoryPressure);
            0.0
        },
        pressure_from_level,
    );
    ([0.0, memory, 0.0], missing)
}

/// What macOS never measures: the pressures it has no stall accounting for, the last CPU, the
/// run queue, cgroups, socket traffic, the lock table, GPU memory (Apple GPUs share memory
/// with the CPU and report none per process), syscall I/O (no per-process counters of the
/// bytes and calls of read and write), memory by kind (no smaps_rollup) and the kernel counters
/// (no interrupt or softirq table, no IRQ affinity, and no readable count of OOM kills). The clock is missing too unless it is measured
/// per cluster (`clocked`), and the core kinds when the IORegistry names none (`kinds_known`).
pub fn missing(clocked: bool, kinds_known: bool) -> Sources {
    let mut missing = Sources::from([
        Source::CpuPressure,
        Source::IoPressure,
        Source::LastCpu,
        Source::RunQueue,
        Source::Cgroups,
        Source::SocketTraffic,
        Source::FileLocks,
        Source::GpuMemory,
        Source::SyscallIo,
        Source::MemoryDetail,
        Source::Interrupts,
        Source::Softirqs,
        Source::IrqAffinity,
        Source::OomKills,
    ]);
    if !clocked {
        missing.insert(Source::CpuClock);
    }
    if !kinds_known {
        missing.insert(Source::CoreKinds);
    }
    missing
}

/// Bytes in one `memorystatus_priority_entry` (`ffi.rs` asserts the layout): `pid` at 0 and
/// `priority` at 4, both little-endian `i32`.
pub const JETSAM_ENTRY_SIZE: usize = 24;

/// The jetsam priority band of each pid in the bytes `memorystatus_control` returned for
/// `MEMORYSTATUS_CMD_GET_PRIORITY_LIST`. None when the bytes cannot be that list: empty, or an
/// entry whose pid is negative or whose band is far outside the 0 to 210 the kernel uses
/// (a wrong layout reads as such). A trailing partial entry is ignored.
pub fn jetsam_priorities(bytes: &[u8]) -> Option<HashMap<u32, i32>> {
    let word = |entry: &[u8], offset: usize| {
        i32::from_le_bytes([
            entry[offset],
            entry[offset + 1],
            entry[offset + 2],
            entry[offset + 3],
        ])
    };
    let mut priorities = HashMap::new();
    for entry in bytes.as_chunks::<JETSAM_ENTRY_SIZE>().0 {
        let (pid, priority) = (word(entry, 0), word(entry, 4));
        if pid < 0 || !(-2..=1000).contains(&priority) {
            return None;
        }
        priorities.insert(pid as u32, priority);
    }
    (!priorities.is_empty()).then_some(priorities)
}

/// The ranking a jetsam band gives. Jetsam kills the lowest bands first, so the band negated is
/// the badness: higher is killed sooner. There is no adjustment setting and no exemption to
/// report.
pub fn jetsam_oom(priority: i32) -> Oom {
    Oom {
        badness: -priority,
        adjust: None,
        exempt: false,
    }
}

/// Bytes the kernel can hand out without paging out application memory. `free` from
/// `HOST_VM_INFO64` already includes the speculative pages (xnu `host_statistics64` sets it to
/// `vm_page_free_count + vm_page_speculative_count`), so they are not added again.
pub fn available(free: u32, inactive: u32, page_size: u64) -> u64 {
    (u64::from(free) + u64::from(inactive)).saturating_mul(page_size)
}

/// The core kind from an IORegistry `cluster-type` byte. `M` (the newest chips' middle tier)
/// counts as performance.
pub fn core_kind(cluster_type: u8) -> Option<CoreKind> {
    match cluster_type {
        b'E' => Some(CoreKind::Efficiency),
        b'P' | b'M' => Some(CoreKind::Performance),
        _ => None,
    }
}

/// A little-endian integer of one to eight bytes, the form device-tree numbers take.
pub fn little_endian(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || bytes.len() > 8 {
        return None;
    }
    let mut value = [0; 8];
    value[..bytes.len()].copy_from_slice(bytes);
    Some(u64::from_le_bytes(value))
}

/// One process's cumulative CPU counters from `proc_pid_rusage` (`RUSAGE_INFO_V6`): CPU time in
/// nanoseconds and cycles, in total and on performance cores only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub time: u64,
    pub performance_time: u64,
    pub cycles: u64,
    pub performance_cycles: u64,
}

/// CPU seconds a cluster must run in an interval before its clock is reported: below that the
/// few cycles counted say little about the clock.
pub const CLOCK_FLOOR: f64 = 0.05;

/// The average clock of the performance and efficiency clusters in MHz over one interval, as
/// `[performance, efficiency]`, from each process's counters at the start and end of it: the
/// cycles each cluster ran divided by the CPU time it ran them in, summed over the processes.
/// A cluster with less than `CLOCK_FLOOR` seconds of CPU time, or whose processes counted no
/// cycles (virtual machines), has no reading (None). A process whose counters went backwards
/// (a reused pid) or disagree with each other is skipped.
pub fn cluster_clocks(
    intervals: impl IntoIterator<Item = (Counters, Counters)>,
) -> [Option<f32>; 2] {
    // Cycles and nanoseconds per cluster, as [performance, efficiency].
    let mut cycles = [0_u64; 2];
    let mut time = [0_u64; 2];
    for (before, after) in intervals {
        let delta = |field: fn(&Counters) -> u64| field(&after).checked_sub(field(&before));
        let (Some(all_time), Some(performance_time), Some(all_cycles), Some(performance_cycles)) = (
            delta(|c| c.time),
            delta(|c| c.performance_time),
            delta(|c| c.cycles),
            delta(|c| c.performance_cycles),
        ) else {
            continue;
        };
        let (Some(efficiency_time), Some(efficiency_cycles)) = (
            all_time.checked_sub(performance_time),
            all_cycles.checked_sub(performance_cycles),
        ) else {
            continue;
        };
        for (cluster, (spent, counted)) in [
            (performance_time, performance_cycles),
            (efficiency_time, efficiency_cycles),
        ]
        .into_iter()
        .enumerate()
        {
            time[cluster] = time[cluster].saturating_add(spent);
            cycles[cluster] = cycles[cluster].saturating_add(counted);
        }
    }
    [0, 1].map(|cluster| {
        let seconds = time[cluster] as f64 / 1e9;
        (seconds >= CLOCK_FLOOR && cycles[cluster] > 0)
            .then(|| (cycles[cluster] as f64 / seconds / 1e6) as f32)
    })
}

/// Who a process descends from, as macOS reports it.
#[derive(Clone, Copy, Debug)]
pub struct Lineage {
    pub pid: u32,
    pub ppid: u32,
    /// The pid responsible for it (the app that launched it through launchd or XPC), when the
    /// responsibility SPI answered.
    pub responsible: Option<u32>,
}

/// The parent of each process, in input order. A process goes under its responsible app, so
/// helpers that launchd started sit under their app rather than under launchd. It keeps its
/// own parent when that parent already descends from the responsible app, so a command under a
/// shell in a terminal stays under the shell instead of moving up beside it. Any choice that
/// would close a loop falls back to the parent pid.
pub fn parents(lineages: &[Lineage]) -> Vec<u32> {
    let index: HashMap<u32, usize> = lineages
        .iter()
        .enumerate()
        .map(|(i, lineage)| (lineage.pid, i))
        .collect();
    let mut parent: Vec<u32> = lineages.iter().map(|lineage| lineage.ppid).collect();
    let steps = lineages.len();
    // Whether `ancestor` is reached by walking up from `start` (inclusive) through `parent`.
    let reaches = |parent: &[u32], start: u32, ancestor: u32| {
        let mut pid = start;
        for _ in 0..=steps {
            if pid == ancestor {
                return true;
            }
            match index.get(&pid) {
                Some(&i) if parent[i] != pid => pid = parent[i],
                _ => return false,
            }
        }
        false
    };
    // Shallow processes first, so a shell is settled before the commands under it.
    let depth = |i: usize| {
        let mut pid = lineages[i].pid;
        let mut depth = 0;
        while let Some(&j) = index.get(&pid) {
            if depth > steps || lineages[j].ppid == pid {
                break;
            }
            pid = lineages[j].ppid;
            depth += 1;
        }
        depth
    };
    let mut order: Vec<usize> = (0..lineages.len()).collect();
    order.sort_by_cached_key(|&i| (depth(i), lineages[i].pid));
    for i in order {
        let Lineage {
            pid,
            ppid,
            responsible,
        } = lineages[i];
        let Some(app) = responsible.filter(|&app| app != pid && app != ppid) else {
            continue;
        };
        if !index.contains_key(&app)
            || (index.contains_key(&ppid) && reaches(&parent, ppid, app))
            || reaches(&parent, app, pid)
        {
            continue;
        }
        parent[i] = app;
    }
    parent
}

/// The TCP state number for an established connection (`TSI_S_ESTABLISHED`).
pub const ESTABLISHED: i32 = 4;

/// One TCP socket as `proc_pidfdinfo` reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TcpSocket {
    pub pid: u32,
    /// `soi_so`: identifies the socket while it is open.
    pub id: u64,
    pub local: SocketAddr,
    pub foreign: SocketAddr,
    pub state: i32,
}

/// One Unix-domain socket as `proc_pidfdinfo` reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnixSocket {
    pub pid: u32,
    /// `soi_pcb`, its protocol control block, and `unsi_conn_pcb`, the peer's; 0 when not
    /// connected.
    pub pcb: u64,
    pub peer: u64,
    /// What waits unread in its receive buffer (`unix_queue`); None for a listener.
    pub queue: Option<Queue>,
}

/// `SOCK_DGRAM` (bsd/sys/socket.h): a datagram socket, whose receive buffer holds packets.
const SOCK_DGRAM: i32 = 2;

/// `SO_ACCEPTCONN` in `soi_options`: the socket is listening.
const SO_ACCEPTCONN: i16 = 0x0002;

/// What waits unread in a Unix socket's receive buffer, from `soi_type`, `soi_options` and
/// `soi_rcv.sbi_cc`, the bytes in the buffer. For a stream socket they are the bytes to read, and
/// for a datagram socket they are the packets queued, which count as "a datagram is waiting" and
/// not as bytes, as on Linux, where the kernel gives the size of the first packet only. A
/// listener's buffer holds no data, so it has no queue.
pub fn unix_queue(kind: i32, options: i16, buffered: u32) -> Option<Queue> {
    if options & SO_ACCEPTCONN != 0 {
        return None;
    }
    Some(if kind == SOCK_DGRAM {
        Queue {
            unread: 0,
            datagram_waiting: buffered > 0,
        }
    } else {
        Queue {
            unread: u64::from(buffered),
            datagram_waiting: false,
        }
    })
}

/// Each socket's control block with the first socket listed for it: a socket shared by several
/// processes counts for the first one.
fn owned(sockets: &[UnixSocket]) -> HashMap<u64, &UnixSocket> {
    let mut owners = HashMap::new();
    for socket in sockets.iter().filter(|socket| socket.pcb != 0) {
        owners.entry(socket.pcb).or_insert(socket);
    }
    owners
}

/// The pids at both ends of each connected Unix socket pair, once per pair. The pids may be
/// equal. A socket shared by several processes counts for the first one listed.
pub fn pair_unix(sockets: &[UnixSocket]) -> Vec<(u32, u32)> {
    let owners = owned(sockets);
    let mut pairs: Vec<(u64, (u32, u32))> = owners
        .iter()
        .filter(|&(&pcb, socket)| socket.peer != 0 && pcb < socket.peer)
        .filter_map(|(&pcb, socket)| {
            let other = owners.get(&socket.peer)?;
            (other.peer == pcb).then_some((pcb, (socket.pid, other.pid)))
        })
        .collect();
    pairs.sort_unstable();
    pairs.into_iter().map(|(_, pair)| pair).collect()
}

/// What waits unread between processes, by (reader pid, writer pid): the reader owns a socket,
/// the writer its connected peer, and the sockets of one pair add up. A pair inside one process
/// and a socket with no queue, or whose peer is not connected back or not readable, add nothing.
pub fn unix_queues(sockets: &[UnixSocket]) -> HashMap<(u32, u32), Queue> {
    let owners = owned(sockets);
    let mut queues: HashMap<(u32, u32), Queue> = HashMap::new();
    for (&pcb, socket) in &owners {
        if let Some(queue) = socket.queue
            && let Some(other) = owners.get(&socket.peer)
            && other.peer == pcb
            && socket.pid != other.pid
        {
            queues
                .entry((socket.pid, other.pid))
                .or_default()
                .add(queue);
        }
    }
    queues
}

/// TCP connections between two local sockets, once per connection with the pids at both ends
/// (possibly equal), and the established connections that leave the machine. `own` holds the
/// host's addresses. Both ends of a local connection are established and each one's local
/// address is the other's foreign address.
pub fn pair_tcp(sockets: &[TcpSocket], own: &HashSet<IpAddr>) -> (Vec<(u32, u32)>, Vec<Remote>) {
    let mut seen = HashSet::new();
    let established: Vec<&TcpSocket> = sockets
        .iter()
        .filter(|socket| socket.state == ESTABLISHED && seen.insert(socket.id))
        .collect();
    let ends: HashMap<(SocketAddr, SocketAddr), u32> = established
        .iter()
        .map(|socket| ((socket.local, socket.foreign), socket.pid))
        .collect();
    let mut pairs = Vec::new();
    let mut remotes = Vec::new();
    for socket in established {
        if let Some(&peer) = ends.get(&(socket.foreign, socket.local)) {
            if socket.local < socket.foreign {
                pairs.push((socket.pid, peer));
            }
            continue;
        }
        let address = socket.foreign.ip();
        if !address.is_loopback() && !address.is_unspecified() && !own.contains(&address) {
            remotes.push(Remote {
                pid: socket.pid,
                inode: socket.id,
                address,
                port: socket.foreign.port(),
                rtt: 0,
                sent: 0,
                received: 0,
            });
        }
    }
    (pairs, remotes)
}

/// Links, outside connections and remotes from every socket the scan could read, with the
/// links that run over at least one Unix socket in `unix`. macOS reports no per-socket byte
/// counters, so `loopback` stays empty.
pub fn network(unix: &[UnixSocket], tcp: &[TcpSocket], own: &HashSet<IpAddr>) -> Network {
    let mut network = Network::default();
    let (tcp_pairs, remotes) = pair_tcp(tcp, own);
    for (a, b) in pair_unix(unix) {
        if a != b {
            let pair = (a.min(b), a.max(b));
            *network.links.entry(pair).or_default() += 1;
            network.unix.insert(pair);
        }
    }
    for (a, b) in tcp_pairs {
        if a != b {
            *network.links.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    for remote in &remotes {
        *network.outside.entry(remote.pid).or_default() += 1;
    }
    network.remotes = remotes;
    network.unix_queues = unix_queues(unix);
    network
}

/// An address from `in_sockinfo`: IPv4 sits in the last four bytes when `INI_IPV4` (1) is set
/// in `insi_vflag`; IPv4-mapped IPv6 addresses become IPv4.
pub fn address(vflag: u8, bytes: [u8; 16]) -> IpAddr {
    const INI_IPV4: u8 = 0x1;
    if vflag & INI_IPV4 != 0 {
        IpAddr::V4(Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]))
    } else {
        Ipv6Addr::from(bytes).to_canonical()
    }
}

/// A port from `in_sockinfo`: a network-order `u_short` widened to `int`.
pub fn port(raw: i32) -> u16 {
    u16::from_be(raw as u16)
}

/// The file type bits of a mode and the regular-file type (bsd/sys/stat.h).
const S_IFMT: u16 = 0o170000;
const S_IFREG: u16 = 0o100000;

/// The fields of a vnode's `vinfo_stat` that decide what a descriptor holds.
#[derive(Clone, Copy, Debug)]
pub struct VnodeStat {
    pub device: u32,
    pub mode: u16,
    pub links: u16,
    pub inode: u64,
    pub size: i64,
}

/// What one descriptor refers to, as far as open files are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Descriptor {
    /// A regular file with at least one name, by device and inode.
    File(u64, u64),
    /// A regular file with no name left.
    Deleted(DeletedFile),
    /// Anything else: a directory, a device, a FIFO, a vnode that could not be read, or a
    /// socket, pipe, kqueue or shared memory object, which are not vnodes at all.
    Other,
}

/// Classifies a vnode descriptor by its attributes. A regular file whose link count is 0 was
/// deleted while open: the kernel keeps it until the last descriptor closes. Unlike Linux,
/// macOS gives no path suffix to confirm it, but none is needed, since the count comes from
/// the open file itself.
pub fn descriptor(stat: VnodeStat) -> Descriptor {
    if stat.mode & S_IFMT != S_IFREG {
        return Descriptor::Other;
    }
    let device = u64::from(stat.device);
    if stat.links == 0 {
        Descriptor::Deleted(DeletedFile {
            device,
            inode: stat.inode,
            size: u64::try_from(stat.size).unwrap_or(0),
        })
    } else {
        Descriptor::File(device, stat.inode)
    }
}

/// Counts examined descriptors, each file once however many descriptors refer to it.
pub fn tally(descriptors: impl IntoIterator<Item = Descriptor>) -> Files {
    let mut open = HashSet::new();
    let mut deleted = BTreeSet::new();
    for descriptor in descriptors {
        match descriptor {
            Descriptor::File(device, inode) => {
                open.insert((device, inode));
            }
            Descriptor::Deleted(file) => {
                open.insert((file.device, file.inode));
                deleted.insert(file);
            }
            Descriptor::Other => {}
        }
    }
    Files {
        open: u32::try_from(open.len()).unwrap_or(u32::MAX),
        deleted: deleted.into_iter().collect(),
        partial: false,
    }
}

/// The open-file count of a table with `examined + beyond` vnode descriptors of which
/// `examined` were looked at and held `files` distinct regular files: the rest are assumed to
/// hold files in the same proportion. Deleted files are never extrapolated.
pub fn estimate(files: u32, examined: usize, beyond: usize) -> u32 {
    if examined == 0 {
        return 0;
    }
    let extra = beyond as u64 * files as u64 / examined as u64;
    files.saturating_add(u32::try_from(extra).unwrap_or(u32::MAX))
}

/// The open files of one table: `examined` holds what each examined vnode descriptor refers to,
/// or None for one that could not be read, and `beyond` counts vnodes past the examined bound.
/// A vnode that could not be read is treated as one that was not examined: the open count is
/// estimated over it and the table is partial, rather than it reading as holding no file.
pub fn table(
    examined: impl IntoIterator<Item = Option<Descriptor>>,
    beyond: usize,
    truncated: bool,
) -> Files {
    let mut read = Vec::new();
    let mut unread = 0;
    for descriptor in examined {
        match descriptor {
            Some(descriptor) => read.push(descriptor),
            None => unread += 1,
        }
    }
    let count = read.len();
    let beyond = beyond + unread;
    let mut files = tally(read);
    files.open = estimate(files.open, count, beyond);
    files.partial = truncated || beyond > 0;
    files
}

/// The Linux state letter of a Mach thread run state (`pth_run_state`, TH_STATE_* in
/// mach/thread_info.h): running R, waiting S, uninterruptible D, stopped T and halted Z, the
/// nearest of the five letters threads show on Linux. `'?'` for a value the header does not
/// list.
pub fn thread_state(run_state: i32) -> char {
    match run_state {
        1 => 'R',
        2 => 'T',
        3 => 'S',
        4 => 'D',
        5 => 'Z',
        _ => '?',
    }
}

/// The unique ids of a process's threads in the order a list shows them: oldest first. The
/// kernel numbers threads as it creates them and never reuses a number within a boot, so the
/// smallest id is the oldest thread, which macOS has as the main thread.
pub fn oldest_first(ids: &mut [u64]) {
    ids.sort_unstable();
}

/// The thread id a unique 64-bit thread id stands for: its low 32 bits.
pub fn thread_tid(unique: u64) -> u32 {
    unique as u32
}

/// What a thread is called: its own name, or the process's when it has none.
pub fn thread_name(own: &str, process: &str) -> String {
    if own.is_empty() { process } else { own }.to_owned()
}

/// `FREAD` and `FWRITE` (bsd/sys/fcntl.h): the kernel's open flags, which `fi_openflags` holds.
const FREAD: u32 = 0x0001;
const FWRITE: u32 = 0x0002;

/// The file type of a FIFO (bsd/sys/stat.h).
const S_IFIFO: u16 = 0o010000;

/// What a descriptor may do, from its `fi_openflags`: None when it may do neither.
pub fn access(open_flags: u32) -> Option<AccessMode> {
    match (open_flags & FREAD != 0, open_flags & FWRITE != 0) {
        (true, true) => Some(AccessMode::Both),
        (true, false) => Some(AccessMode::Read),
        (false, true) => Some(AccessMode::Write),
        (false, false) => None,
    }
}

/// One descriptor of an anonymous pipe, from its `pipe_fdinfo`: the kernel addresses of its end
/// of the pipe (`pipe_handle`) and of the other end (`pipe_peerhandle`), its open flags, and
/// `pipe_stat.vst_size`, the bytes waiting, which either end reports (checked on macOS 27.2: a
/// pipe holding 5 bytes says 5 at both). The two descriptors of a pipe swap handle and peer, so
/// the lower of the two names the pipe for both.
pub fn pipe_end(handle: u64, peer: u64, open_flags: u32, size: i64) -> Option<PipeEnd> {
    Some(PipeEnd {
        // A pipe whose other end was closed has no peer to share a name with.
        id: PipeId::Anonymous(if peer == 0 { handle } else { handle.min(peer) }),
        access: access(open_flags)?,
        buffered: u64::try_from(size).ok(),
    })
}

/// One descriptor of a named pipe, from the attributes of its vnode: None when the vnode is
/// another kind of file. Named by device and inode; macOS counts nothing buffered in a FIFO.
pub fn fifo_end(stat: VnodeStat, open_flags: u32) -> Option<PipeEnd> {
    (stat.mode & S_IFMT == S_IFIFO).then_some(())?;
    Some(PipeEnd {
        id: PipeId::Fifo {
            device: u64::from(stat.device),
            inode: stat.inode,
        },
        access: access(open_flags)?,
        buffered: None,
    })
}

/// `PROX_FDTYPE_VNODE`, `_SOCKET` and `_PIPE` (bsd/sys/proc_info.h): the `proc_fdtype` of the
/// descriptors that are files, sockets and pipes. Every other type is a kernel object with no
/// file behind it: a kqueue, shared memory, a semaphore, the filesystem events device, a network
/// policy or channel.
const PROX_FDTYPE_VNODE: u32 = 1;
const PROX_FDTYPE_SOCKET: u32 = 2;
const PROX_FDTYPE_PIPE: u32 = 6;

/// What a descriptor is, from the `proc_fdtype` the listing gives, which costs no call per
/// descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FdKind {
    /// A vnode: a file, a directory, a device or a named pipe. Only a call per descriptor tells.
    Vnode,
    Pipe,
    Socket,
    /// A kqueue, shared memory or the like: the nearest thing macOS has to Linux's `anon_inode`.
    Anonymous,
}

pub fn fd_kind(proc_fdtype: u32) -> FdKind {
    match proc_fdtype {
        PROX_FDTYPE_VNODE => FdKind::Vnode,
        PROX_FDTYPE_SOCKET => FdKind::Socket,
        PROX_FDTYPE_PIPE => FdKind::Pipe,
        _ => FdKind::Anonymous,
    }
}

/// The file type of a character device (bsd/sys/stat.h).
const S_IFCHR: u16 = 0o020000;

/// `FAPPEND`, the kernel's `O_APPEND` (bsd/sys/fcntl.h), as `fi_openflags` holds it.
const FAPPEND: u32 = 0x0008;

/// One vnode descriptor as `PROC_PIDFDVNODEPATHINFO` describes it.
#[derive(Clone, Copy, Debug)]
pub struct PathedVnode<'a> {
    pub stat: VnodeStat,
    /// The path of the vnode, raw (lossily decoded); empty for a file the kernel can no longer
    /// name.
    pub path: &'a str,
    /// `fi_openflags`.
    pub open_flags: u32,
    /// `fi_offset`, the position of the open file.
    pub offset: i64,
}

/// What a vnode descriptor is for the per-file I/O detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Vnode {
    /// A regular file opened for reading, writing or both.
    File(OpenFile),
    /// A terminal: a character device under `/dev/tty*` or `/dev/console`.
    Tty,
    /// A directory, any other device, a named pipe, a file opened for neither reading nor writing
    /// (a descriptor for events or paths only), and a file whose position or size the kernel gave
    /// as negative, which are no numbers of bytes.
    Other,
}

/// Whether the character device at `path` is a terminal: macOS names them `/dev/ttys000`, with
/// `/dev/tty` for the controlling terminal and `/dev/console`.
fn terminal(path: &str) -> bool {
    path.starts_with("/dev/tty") || path.starts_with("/dev/console")
}

/// Classifies a vnode descriptor `fd`. A regular file whose link count is 0 was deleted while
/// open, as for `descriptor`. `fi_offset` is its position (`pos` of Linux's fdinfo), and
/// `fi_openflags` gives its access mode and whether it appends.
pub fn vnode(fd: u32, vnode: PathedVnode) -> Vnode {
    let PathedVnode {
        stat,
        path,
        open_flags,
        offset,
    } = vnode;
    match stat.mode & S_IFMT {
        S_IFCHR if terminal(path) => Vnode::Tty,
        S_IFREG => match (
            access(open_flags),
            u64::try_from(offset),
            u64::try_from(stat.size),
        ) {
            (Some(access), Ok(position), Ok(size)) => Vnode::File(OpenFile {
                fd,
                device: u64::from(stat.device),
                inode: stat.inode,
                size,
                path: path.to_owned(),
                deleted: stat.links == 0,
                access,
                appending: open_flags & FAPPEND != 0,
                position,
            }),
            _ => Vnode::Other,
        },
        _ => Vnode::Other,
    }
}

/// How busy `process` is for choosing which get an `IoDetail`. macOS has no counters of the
/// calls to read and write, which Linux ranks by, so it ranks by the bytes that reached storage
/// per second: the growth of `ri_diskio_bytesread` plus `ri_diskio_byteswritten` between samples,
/// which `Process::io_rate` is. That leaves out I/O the page cache absorbed, so a process that
/// reads and writes files already in memory ranks low. None until a second reading exists.
pub fn by_disk_rate(process: &Process) -> Option<f64> {
    process.io_rate.map(f64::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Identity, Pipe, pair_pipes};

    #[test]
    fn the_oldest_thread_leads_on_macos() {
        let mut ids = [3_753_481, 3_753_475, 3_753_480];
        oldest_first(&mut ids);
        assert_eq!(ids, [3_753_475, 3_753_480, 3_753_481]);
        // The id is 64 bits wide, the thread id its low half.
        assert_eq!(thread_tid(0x1_0000_0007), 7);
        assert_eq!(thread_tid(3_753_475), 3_753_475);
        assert_eq!(thread_name("", "Safari"), "Safari");
        assert_eq!(
            thread_name("com.apple.main-thread", "Safari"),
            "com.apple.main-thread"
        );
    }

    #[test]
    fn macos_run_states_map_to_linux_letters() {
        let letters: Vec<char> = (0..=6).map(thread_state).collect();
        assert_eq!(letters, ['?', 'R', 'T', 'S', 'D', 'Z', '?']);
    }

    fn counters(
        time: u64,
        performance_time: u64,
        cycles: u64,
        performance_cycles: u64,
    ) -> Counters {
        Counters {
            time,
            performance_time,
            cycles,
            performance_cycles,
        }
    }

    #[test]
    fn cluster_clocks_divide_each_clusters_cycles_by_its_cpu_time() {
        const MS: u64 = 1_000_000;
        // One process ran 300 ms on P at 3.2 GHz and 100 ms on E at 2.0 GHz; another ran 100
        // ms on P at 4.0 GHz. P: (960 + 400) M cycles in 0.4 s = 3400 MHz; E: 200 M in 0.1 s.
        let first = (
            counters(1_000 * MS, 600 * MS, 5_000_000_000, 4_000_000_000),
            counters(1_400 * MS, 900 * MS, 6_160_000_000, 4_960_000_000),
        );
        let second = (
            counters(0, 0, 0, 0),
            counters(100 * MS, 100 * MS, 400_000_000, 400_000_000),
        );
        assert_eq!(
            cluster_clocks([first, second]),
            [Some(3400.0), Some(2000.0)]
        );
    }

    #[test]
    fn a_cluster_with_too_little_cpu_time_has_no_clock_reading() {
        const MS: u64 = 1_000_000;
        // E ran 40 ms, under the 50 ms floor; P ran exactly 50 ms.
        let interval = (
            counters(0, 0, 0, 0),
            counters(90 * MS, 50 * MS, 230_000_000, 150_000_000),
        );
        assert_eq!(cluster_clocks([interval]), [Some(3000.0), None]);
        assert_eq!(cluster_clocks([]), [None, None]);
    }

    #[test]
    fn cluster_clocks_have_no_reading_without_cycle_counters() {
        // Virtual machines count CPU time but no cycles.
        let interval = (
            counters(0, 0, 0, 0),
            counters(2_000_000_000, 1_000_000_000, 0, 0),
        );
        assert_eq!(cluster_clocks([interval]), [None, None]);
    }

    #[test]
    fn cluster_clocks_skip_processes_whose_counters_went_backwards_or_disagree() {
        const MS: u64 = 1_000_000;
        let good = (
            counters(0, 0, 0, 0),
            counters(200 * MS, 100 * MS, 500_000_000, 300_000_000),
        );
        let reused = (
            counters(900 * MS, 400 * MS, 9_000_000_000, 5_000_000_000),
            counters(100 * MS, 50 * MS, 100_000_000, 60_000_000),
        );
        let impossible = (
            counters(0, 0, 0, 0),
            counters(100 * MS, 200 * MS, 900_000_000, 800_000_000),
        );
        assert_eq!(
            cluster_clocks([good, reused, impossible]),
            cluster_clocks([good])
        );
        assert_eq!(cluster_clocks([good]), [Some(3000.0), Some(2000.0)]);
    }

    #[test]
    fn processes_are_classified_by_pid_helper_owner_and_path() {
        let me = 501;
        assert_eq!(classify(0, 0, me, "", "kernel_task"), Kind::Kernel);
        for (path, name) in [
            (
                "/System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine",
                "com.apple.Virtualization.VirtualMachine",
            ),
            (
                "/opt/homebrew/bin/qemu-system-aarch64",
                "qemu-system-aarch64",
            ),
            ("/opt/homebrew/bin/limactl", "limactl"),
            ("", "vfkit"),
            ("/opt/homebrew/bin/krunkit", "krunkit"),
            (
                "/Applications/OrbStack.app/Contents/Frameworks/OrbStack Helper.app/Contents/MacOS/OrbStack Helper",
                "OrbStack Helper",
            ),
            (
                "/Applications/Docker.app/Contents/MacOS/com.docker.backend",
                "com.docker.backend",
            ),
            (
                "/Applications/Docker.app/Contents/MacOS/Docker Desktop",
                "Docker Desktop",
            ),
        ] {
            assert_eq!(classify(900, me, me, path, name), Kind::Container, "{name}");
        }
        assert_eq!(
            classify(300, 0, me, "/sbin/launchd", "launchd"),
            Kind::System
        );
        assert_eq!(classify(310, 88, me, "", "windowserver"), Kind::System);
        assert_eq!(classify(320, me, me, "/bin/zsh", "zsh"), Kind::System);
        assert_eq!(
            classify(
                330,
                me,
                me,
                "/System/Applications/Mail.app/Contents/MacOS/Mail",
                "Mail"
            ),
            Kind::System
        );
        assert_eq!(
            classify(340, me, me, "/Library/Apple/usr/libexec/oah/oahd", "oahd"),
            Kind::System
        );
        assert_eq!(
            classify(
                350,
                me,
                me,
                "/Applications/Safari.app/Contents/MacOS/Safari",
                "Safari"
            ),
            Kind::Session
        );
        assert_eq!(
            classify(360, me, me, "/usr/local/bin/htop", "htop"),
            Kind::Session
        );
        assert_eq!(
            classify(370, 502, me, "/Users/other/bin/tool", "tool"),
            Kind::System
        );
        assert_eq!(classify(380, me, me, "", "unknown"), Kind::Session);
        assert_eq!(
            classify(390, me, me, "/opt/qemu-system-tools/bin/run", "run"),
            Kind::Session,
            "patterns match the file name, not directories"
        );
    }

    #[test]
    fn groups_are_the_outermost_app_bundle_or_the_file_name() {
        assert_eq!(
            group(
                "/Applications/Safari.app/Contents/XPCServices/com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent",
                "WebContent"
            ),
            "Safari.app"
        );
        assert_eq!(
            group(
                "/Applications/Slack.app/Contents/Frameworks/Slack Helper (Renderer).app/Contents/MacOS/Slack Helper (Renderer)",
                "Slack Helper"
            ),
            "Slack.app"
        );
        assert_eq!(group("/usr/sbin/cfprefsd", "cfprefsd"), "cfprefsd");
        assert_eq!(group("", "kernel_task"), "kernel_task");
        assert_eq!(group("/opt/.app/bin/tool", "tool"), "tool");
        assert_eq!(
            cgroup_path(Kind::Session, "Safari.app"),
            "/session/Safari.app"
        );
        assert_eq!(
            cgroup_path(Kind::Container, "limactl"),
            "/container/limactl"
        );
        assert_eq!(cgroup_path(Kind::System, "cfprefsd"), "/system/cfprefsd");
    }

    #[test]
    fn mach_priorities_map_to_the_linux_scale_where_lower_runs_first() {
        assert_eq!(priority_from_mach(31), 20);
        assert_eq!(priority_from_mach(4), 47);
        assert_eq!(priority_from_mach(47), 4);
        assert_eq!(priority_from_mach(97), -46);
        assert!(priority_from_mach(46) < priority_from_mach(31));
    }

    fn procargs(argc: i32, body: &[u8]) -> Vec<u8> {
        let mut bytes = argc.to_ne_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn procargs_skip_the_padding_after_the_path_and_stop_before_the_environment() {
        let bytes = procargs(
            3,
            b"/usr/bin/env\0\0\0\0\0\0env\0-i\0\0HOME=/Users/me\0PATH=/bin\0",
        );
        assert_eq!(
            parse_procargs(&bytes),
            Some((
                "/usr/bin/env".into(),
                vec!["env".into(), "-i".into(), String::new()]
            ))
        );
        let none = procargs(0, b"/sbin/launchd\0\0\0\0HOME=/\0");
        assert_eq!(
            parse_procargs(&none),
            Some(("/sbin/launchd".into(), vec![]))
        );
    }

    #[test]
    fn truncated_procargs_keep_what_they_hold() {
        let cut = procargs(3, &[b"/bin/sleep\0\0sleep\0".as_slice(), b"10"].concat());
        assert_eq!(
            parse_procargs(&cut),
            Some(("/bin/sleep".into(), vec!["sleep".into(), "10".into()]))
        );
        let path_only = procargs(2, b"/bin/sl");
        assert_eq!(parse_procargs(&path_only), Some(("/bin/sl".into(), vec![])));
        assert_eq!(parse_procargs(&[1, 0]), None);
        assert_eq!(parse_procargs(&procargs(-1, b"/x\0x\0")), None);
        assert_eq!(
            parse_procargs(&procargs(1, b"")),
            Some((String::new(), vec![]))
        );
        let invalid = procargs(1, b"/bin/\xff\0\xfe\0");
        assert_eq!(
            parse_procargs(&invalid),
            Some(("/bin/\u{fffd}".into(), vec!["\u{fffd}".into()]))
        );
    }

    #[test]
    fn mach_time_converts_to_nanoseconds_and_states_map_to_letters() {
        assert_eq!(nanoseconds(24_000_000, 125, 3), 1_000_000_000);
        assert_eq!(nanoseconds(5, 1, 1), 5);
        assert_eq!(nanoseconds(u64::MAX, 125, 3), u64::MAX);
        assert_eq!(nanoseconds(7, 1, 0), 7);
        assert_eq!(state(5, 0), 'Z');
        assert_eq!(state(4, 3), 'T');
        assert_eq!(state(2, 1), 'R');
        assert_eq!(state(3, 0), 'S');
        assert_eq!(state(2, 0), 'S');
    }

    #[test]
    fn a_zombie_needs_no_live_task_and_still_reads_as_z() {
        assert!(zombie(5));
        assert!(!zombie(2) && !zombie(4));
        // The sampler zeroes the task info of a zombie, so no thread is running.
        assert_eq!(state(5, 0), 'Z');
    }

    #[test]
    fn a_description_is_read_again_after_an_exec_that_keeps_the_name() {
        assert!(same_program((7, "zsh"), (7, "zsh")));
        assert!(!same_program((7, "zsh"), (8, "zsh")), "exec zsh from zsh");
        assert!(!same_program((7, "-zsh"), (7, "zsh")), "a renamed process");
    }

    #[test]
    fn busy_share_survives_a_wrapping_tick_counter() {
        assert_eq!(busy([0; 4], [0; 4]), None, "no ticks passed");
        assert_eq!(busy([10, 10, 10, 10], [40, 20, 70, 10]), Some(0.4));
        let before = [u32::MAX - 9, 0, u32::MAX - 29, 0];
        let after = [20, 0, 10, 0];
        assert!((busy(before, after).unwrap() - 30.0 / 70.0).abs() < 1e-6);
        assert_eq!(busy([0, 0, 0, 0], [5, 0, 0, 5]), Some(1.0));
    }

    #[test]
    fn pressure_levels_and_available_memory() {
        assert_eq!(pressure_from_level(1), 0.0);
        assert_eq!(pressure_from_level(2), 25.0);
        assert_eq!(pressure_from_level(4), 75.0);
        assert_eq!(pressure_from_level(0), 0.0);
        let unstalled = Sources::from([Source::CpuPressure, Source::IoPressure]);
        assert_eq!(pressure(Some(2)), ([0.0, 25.0, 0.0], unstalled));
        assert_eq!(
            pressure(None).1,
            unstalled.union(Source::MemoryPressure.into())
        );
        assert_eq!(available(10, 5, 16384), 15 * 16384);
        assert_eq!(available(u32::MAX, u32::MAX, u64::MAX), u64::MAX);
    }

    #[test]
    fn macos_reports_gpu_memory_as_missing() {
        for clocked in [false, true] {
            let gaps = missing(clocked, true);
            assert!(gaps.contains(Source::GpuMemory), "{gaps:?}");
            assert_eq!(gaps.contains(Source::CpuClock), !clocked, "{gaps:?}");
            assert!(!gaps.contains(Source::MemoryPressure) && !gaps.contains(Source::CoreKinds));
        }
        assert!(missing(true, false).contains(Source::CoreKinds));
    }

    #[test]
    fn macos_reports_memory_detail_as_missing_and_leaves_oom_to_root() {
        for clocked in [false, true] {
            let gaps = missing(clocked, true);
            assert!(gaps.contains(Source::MemoryDetail), "{gaps:?}");
            assert!(!gaps.contains(Source::Oom), "root can read it: {gaps:?}");
        }
        assert_eq!(Source::MemoryDetail.name(), "memory detail");
        assert_eq!(Source::Oom.name(), "oom score");
    }

    /// The list `memorystatus_control` returns for these `(pid, band)` pairs.
    fn jetsam_list(entries: &[(i32, i32)]) -> Vec<u8> {
        entries
            .iter()
            .flat_map(|&(pid, band)| {
                let mut entry = [0u8; JETSAM_ENTRY_SIZE];
                entry[0..4].copy_from_slice(&pid.to_le_bytes());
                entry[4..8].copy_from_slice(&band.to_le_bytes());
                // The memory limit and the state flags, which are not read.
                entry[16..20].copy_from_slice(&4096i32.to_le_bytes());
                entry[20..24].copy_from_slice(&0x41u32.to_le_bytes());
                entry
            })
            .collect()
    }

    #[test]
    fn jetsam_priorities_rank_lower_bands_as_killed_sooner() {
        let bytes = jetsam_list(&[(1, 100), (42, 10), (77, 0), (0, 0), (500, 18)]);
        let priorities = jetsam_priorities(&bytes).unwrap();
        assert_eq!(priorities.len(), 5);
        let badness = |pid: u32| jetsam_oom(priorities[&pid]).badness;
        assert!(badness(77) > badness(42), "idle goes before the foreground");
        assert!(badness(42) > badness(500));
        assert!(badness(500) > badness(1), "the system band goes last");
        assert_eq!(
            jetsam_oom(10),
            Oom {
                badness: -10,
                adjust: None,
                exempt: false
            }
        );
    }

    #[test]
    fn bytes_that_are_not_a_jetsam_list_are_refused() {
        assert_eq!(jetsam_priorities(&[]), None);
        assert_eq!(jetsam_priorities(&[0; 23]), None, "less than one entry");
        assert_eq!(jetsam_priorities(&jetsam_list(&[(-5, 10)])), None);
        assert_eq!(jetsam_priorities(&jetsam_list(&[(5, 70000)])), None);
        let mut cut = jetsam_list(&[(1, 10), (2, 20)]);
        cut.truncate(JETSAM_ENTRY_SIZE + 9);
        assert_eq!(jetsam_priorities(&cut).unwrap().len(), 1);
    }

    #[test]
    fn macos_reports_syscall_io_as_missing() {
        for clocked in [false, true] {
            for kinds_known in [false, true] {
                let gaps = missing(clocked, kinds_known);
                assert!(gaps.contains(Source::SyscallIo), "{gaps:?}");
            }
        }
        assert_eq!(Source::SyscallIo.name(), "syscall io");
    }

    #[test]
    fn macos_reports_the_kernel_counters_as_missing() {
        for clocked in [false, true] {
            let gaps = missing(clocked, true);
            for source in [
                Source::Interrupts,
                Source::Softirqs,
                Source::IrqAffinity,
                Source::OomKills,
            ] {
                assert!(gaps.contains(source), "{gaps:?}");
            }
        }
    }

    #[test]
    fn mach_policy_maps_to_policy() {
        assert_eq!(policy_from_mach(1), Policy::Normal);
        assert_eq!(policy_from_mach(2), Policy::RoundRobin);
        assert_eq!(policy_from_mach(4), Policy::Fifo);
        for other in [0, 3, 5, -1] {
            assert_eq!(policy_from_mach(other), Policy::Unknown, "policy {other}");
        }
    }

    #[test]
    fn start_seconds_join_the_seconds_and_microseconds() {
        assert_eq!(start_seconds(1_700_000_000, 250_000), 1_700_000_000.25);
        assert_eq!(start_seconds(0, 0), 0.0);
    }

    #[test]
    fn cluster_types_and_device_tree_numbers_decode() {
        assert_eq!(core_kind(b'E'), Some(CoreKind::Efficiency));
        assert_eq!(core_kind(b'P'), Some(CoreKind::Performance));
        assert_eq!(core_kind(b'M'), Some(CoreKind::Performance));
        assert_eq!(core_kind(0), None);
        assert_eq!(little_endian(&[7, 0, 0, 0]), Some(7));
        assert_eq!(little_endian(&[1, 2]), Some(0x0201));
        assert_eq!(little_endian(&[]), None);
        assert_eq!(little_endian(&[0; 9]), None);
    }

    fn lineage(pid: u32, ppid: u32, responsible: Option<u32>) -> Lineage {
        Lineage {
            pid,
            ppid,
            responsible,
        }
    }

    #[test]
    fn helpers_move_under_their_app_and_shell_commands_stay_under_the_shell() {
        let launchd = lineage(1, 0, Some(1));
        let ghostty = lineage(600, 1, Some(600));
        // login is root's and unreadable, so it is not listed.
        let shell = lineage(610, 605, Some(600));
        let command = lineage(620, 610, Some(600));
        let helper = lineage(700, 1, Some(600));
        let orphan = lineage(710, 1, Some(9999));
        let unanswered = lineage(720, 1, None);
        // Listed deepest first to show the order does not matter.
        let list = [command, shell, helper, orphan, unanswered, ghostty, launchd];
        assert_eq!(parents(&list), vec![610, 600, 600, 1, 1, 1, 0]);
    }

    #[test]
    fn responsible_pids_never_close_a_loop() {
        let a = lineage(10, 1, Some(20));
        let b = lineage(20, 1, Some(10));
        let child = lineage(30, 10, Some(30));
        let grandchild_app = lineage(10, 1, Some(40));
        let under = lineage(40, 10, Some(40));
        let first = parents(&[a, b, child]);
        assert!(
            first == vec![20, 1, 10] || first == vec![1, 10, 10],
            "{first:?}"
        );
        // The responsible app is the process's own descendant.
        assert_eq!(parents(&[grandchild_app, under]), vec![1, 10]);
        let looped = [lineage(5, 6, None), lineage(6, 5, Some(5))];
        assert_eq!(parents(&looped), vec![6, 5]);
    }

    fn v4(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    fn tcp(pid: u32, id: u64, local: &str, foreign: &str, state: i32) -> TcpSocket {
        TcpSocket {
            pid,
            id,
            local: v4(local),
            foreign: v4(foreign),
            state,
        }
    }

    #[test]
    fn unix_sockets_pair_through_their_control_blocks() {
        let sockets = [
            UnixSocket {
                pid: 10,
                pcb: 0xa,
                peer: 0xb,
                queue: None,
            },
            UnixSocket {
                pid: 20,
                pcb: 0xb,
                peer: 0xa,
                queue: None,
            },
            // The same socket inherited by a child counts once.
            UnixSocket {
                pid: 30,
                pcb: 0xb,
                peer: 0xa,
                queue: None,
            },
            // A pair inside one process.
            UnixSocket {
                pid: 40,
                pcb: 0xc,
                peer: 0xd,
                queue: None,
            },
            UnixSocket {
                pid: 40,
                pcb: 0xd,
                peer: 0xc,
                queue: None,
            },
            // A listener and a socket whose peer belongs to an unreadable process.
            UnixSocket {
                pid: 50,
                pcb: 0xe,
                peer: 0,
                queue: None,
            },
            UnixSocket {
                pid: 60,
                pcb: 0xf,
                peer: 0x99,
                queue: None,
            },
        ];
        assert_eq!(pair_unix(&sockets), vec![(10, 20), (40, 40)]);
        let network = network(&sockets, &[], &HashSet::new());
        assert_eq!(network.links, HashMap::from([((10, 20), 1)]));
    }

    #[test]
    fn unix_pairs_fill_the_unix_set_as_well_as_links() {
        let unix = [
            UnixSocket {
                pid: 20,
                pcb: 0xa,
                peer: 0xb,
                queue: None,
            },
            UnixSocket {
                pid: 10,
                pcb: 0xb,
                peer: 0xa,
                queue: None,
            },
            // A pair inside one process is no link.
            UnixSocket {
                pid: 40,
                pcb: 0xc,
                peer: 0xd,
                queue: None,
            },
            UnixSocket {
                pid: 40,
                pcb: 0xd,
                peer: 0xc,
                queue: None,
            },
        ];
        let tcp = [
            // The Unix pair also talks over TCP.
            tcp(10, 1, "127.0.0.1:50000", "127.0.0.1:8080", ESTABLISHED),
            tcp(20, 2, "127.0.0.1:8080", "127.0.0.1:50000", ESTABLISHED),
            // A pair over TCP only.
            tcp(30, 3, "127.0.0.1:50001", "127.0.0.1:9000", ESTABLISHED),
            tcp(50, 4, "127.0.0.1:9000", "127.0.0.1:50001", ESTABLISHED),
        ];
        let network = network(&unix, &tcp, &HashSet::new());
        assert_eq!(network.links, HashMap::from([((10, 20), 2), ((30, 50), 1)]));
        assert_eq!(network.unix, HashSet::from([(10, 20)]));
    }

    #[test]
    fn loopback_tcp_pairs_by_addresses_and_counts_each_connection_once() {
        let sockets = [
            tcp(10, 1, "127.0.0.1:50000", "127.0.0.1:8080", ESTABLISHED),
            tcp(20, 2, "127.0.0.1:8080", "127.0.0.1:50000", ESTABLISHED),
            tcp(20, 3, "127.0.0.1:8080", "0.0.0.0:0", 1),
            tcp(30, 4, "[::1]:50001", "[::1]:9000", ESTABLISHED),
            tcp(30, 5, "[::1]:9000", "[::1]:50001", ESTABLISHED),
            // Closing on one side: not established, so not paired.
            tcp(40, 6, "127.0.0.1:50002", "127.0.0.1:8080", 5),
            tcp(20, 7, "127.0.0.1:8080", "127.0.0.1:50002", ESTABLISHED),
        ];
        let (pairs, remotes) = pair_tcp(&sockets, &HashSet::new());
        assert_eq!(pairs, vec![(20, 10), (30, 30)]);
        assert!(remotes.is_empty(), "{remotes:?}");
        let network = network(&[], &sockets, &HashSet::new());
        assert_eq!(network.links, HashMap::from([((10, 20), 1)]));
        assert!(network.outside.is_empty());
        assert!(network.loopback.is_empty());
    }

    #[test]
    fn outside_connections_exclude_loopback_unspecified_and_own_addresses() {
        let own = HashSet::from(["192.168.1.5".parse().unwrap()]);
        let sockets = [
            tcp(10, 1, "192.168.1.5:50000", "93.184.216.34:443", ESTABLISHED),
            tcp(10, 2, "192.168.1.5:50001", "93.184.216.34:443", ESTABLISHED),
            // Shared by a child: one connection.
            tcp(11, 2, "192.168.1.5:50001", "93.184.216.34:443", ESTABLISHED),
            tcp(10, 3, "192.168.1.5:50002", "1.1.1.1:443", 2),
            tcp(20, 4, "192.168.1.5:50003", "192.168.1.5:22", ESTABLISHED),
            tcp(30, 5, "127.0.0.1:50004", "127.0.0.1:5432", ESTABLISHED),
            tcp(40, 6, "0.0.0.0:0", "0.0.0.0:0", ESTABLISHED),
            tcp(
                50,
                7,
                "[2001:db8::2]:50005",
                "[2606:4700::1111]:443",
                ESTABLISHED,
            ),
        ];
        let network = network(&[], &sockets, &own);
        assert_eq!(network.outside, HashMap::from([(10, 2), (50, 1)]));
        assert_eq!(
            network.remotes[0],
            Remote {
                pid: 10,
                inode: 1,
                address: "93.184.216.34".parse().unwrap(),
                port: 443,
                rtt: 0,
                sent: 0,
                received: 0,
            }
        );
        assert_eq!(network.remotes.len(), 3);
        assert!(network.links.is_empty());
    }

    #[test]
    fn socket_addresses_and_ports_decode_from_network_order() {
        let mut bytes = [0; 16];
        bytes[12..].copy_from_slice(&[127, 0, 0, 1]);
        assert_eq!(address(1, bytes), IpAddr::V4(Ipv4Addr::LOCALHOST));
        let mapped: Ipv6Addr = "::ffff:10.0.0.7".parse().unwrap();
        assert_eq!(
            address(2, mapped.octets()),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))
        );
        assert_eq!(
            address(2, Ipv6Addr::LOCALHOST.octets()),
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        );
        let raw = i32::from(u16::from_ne_bytes(8080_u16.to_be_bytes()));
        assert_eq!(port(raw), 8080);
    }

    fn stat(mode: u16, links: u16, inode: u64, size: i64) -> VnodeStat {
        VnodeStat {
            device: 0x0100_0004,
            mode,
            links,
            inode,
            size,
        }
    }

    #[test]
    fn a_regular_file_with_no_links_left_is_deleted_and_other_vnodes_are_not_files() {
        let regular = 0o100644;
        assert_eq!(
            descriptor(stat(regular, 1, 42, 10)),
            Descriptor::File(0x0100_0004, 42)
        );
        assert_eq!(
            descriptor(stat(regular, 0, 43, 5 << 30)),
            Descriptor::Deleted(DeletedFile {
                device: 0x0100_0004,
                inode: 43,
                size: 5 << 30,
            })
        );
        // Directories, character devices (/dev/null, ttys) and FIFOs are vnodes but not files,
        // even when their link count reads 0.
        for mode in [0o040755, 0o020666, 0o010600, 0o120755] {
            assert_eq!(descriptor(stat(mode, 0, 44, 0)), Descriptor::Other);
        }
        let Descriptor::Deleted(file) = descriptor(stat(regular, 0, 45, -1)) else {
            panic!("a deleted file");
        };
        assert_eq!(file.size, 0, "a negative off_t is no size");
    }

    #[test]
    fn a_file_held_through_several_descriptors_counts_once_by_device_and_inode() {
        let regular = 0o100600;
        let files = tally(
            [
                stat(regular, 1, 10, 1),
                stat(regular, 1, 10, 1),
                stat(regular, 2, 11, 1),
                stat(regular, 0, 12, 4096),
                stat(regular, 0, 12, 4096),
                stat(0o040755, 3, 13, 0),
            ]
            .map(descriptor)
            .into_iter()
            .chain([Descriptor::Other]),
        );
        assert_eq!(files.open, 3, "10, 11 and the deleted 12");
        assert_eq!(files.deleted.len(), 1);
        assert_eq!(files.deleted_bytes(), 4096);
        assert!(!files.partial);
        // The same inode on another device is another file.
        let other_device = Descriptor::File(0x0100_0005, 10);
        assert_eq!(
            tally([descriptor(stat(regular, 1, 10, 1)), other_device]).open,
            2
        );
    }

    #[test]
    fn unexamined_vnodes_are_estimated_in_proportion_and_deleted_files_are_not() {
        assert_eq!(estimate(1000, 4096, 4096), 2000);
        assert_eq!(estimate(7, 7, 0), 7);
        assert_eq!(estimate(0, 0, 100), 0);
        assert_eq!(estimate(u32::MAX, 1, 1), u32::MAX);
    }

    #[test]
    fn a_vnode_that_could_not_be_read_is_estimated_and_marks_the_table_partial() {
        let regular = 0o100600;
        let file = |inode| Some(descriptor(stat(regular, 1, inode, 1)));
        let complete = table([file(1), file(2), Some(Descriptor::Other)], 0, false);
        assert_eq!((complete.open, complete.partial), (2, false));
        // Two of four vnodes read and held files; the two unread are estimated alike.
        let unread = table([file(1), None, file(2), None], 0, false);
        assert_eq!((unread.open, unread.partial), (4, true));
        let deleted = Some(descriptor(stat(regular, 0, 3, 4096)));
        // One read vnode held a file; it stands for the unread one and the two past the bound.
        let past_the_bound = table([deleted, None], 2, false);
        assert_eq!(past_the_bound.open, 4);
        assert_eq!(
            past_the_bound.deleted_bytes(),
            4096,
            "deleted files are not estimated"
        );
        assert!(table([], 0, true).partial);
        assert_eq!(
            table([None], 0, false).open,
            0,
            "nothing read is no estimate"
        );
    }

    fn queue(unread: u64, datagram_waiting: bool) -> Queue {
        Queue {
            unread,
            datagram_waiting,
        }
    }

    #[test]
    fn unix_queues_are_bytes_for_streams_and_a_flag_for_datagrams() {
        let stream = 1;
        assert_eq!(unix_queue(stream, 0, 4096), Some(queue(4096, false)));
        assert_eq!(
            unix_queue(stream, 0, 0),
            Some(queue(0, false)),
            "idle, not unknown"
        );
        assert_eq!(unix_queue(SOCK_DGRAM, 0, 1500), Some(queue(0, true)));
        assert_eq!(unix_queue(SOCK_DGRAM, 0, 0), Some(queue(0, false)));
        assert_eq!(
            unix_queue(stream, SO_ACCEPTCONN, 3),
            None,
            "a listener holds no data"
        );
    }

    #[test]
    fn unix_queues_run_from_the_writer_to_the_reader_and_add_up_per_pair() {
        let socket = |pid, pcb, peer, queue| UnixSocket {
            pid,
            pcb,
            peer,
            queue,
        };
        let sockets = [
            socket(10, 0xa, 0xb, Some(queue(100, false))),
            socket(20, 0xb, 0xa, Some(queue(7, false))),
            // A second pair between the same processes.
            socket(10, 0xc, 0xd, Some(queue(0, true))),
            socket(20, 0xd, 0xc, Some(queue(50, false))),
            // Inside one process, a listener, and a peer nobody can read.
            socket(30, 0xe, 0xf, Some(queue(9, false))),
            socket(30, 0xf, 0xe, Some(queue(9, false))),
            socket(40, 0x10, 0, None),
            socket(50, 0x11, 0x99, Some(queue(5, false))),
        ];
        assert_eq!(
            unix_queues(&sockets),
            HashMap::from([((10, 20), queue(100, true)), ((20, 10), queue(57, false)),])
        );
        let network = network(&sockets, &[], &HashSet::new());
        assert_eq!(network.unix_queues, unix_queues(&sockets));
    }

    #[test]
    fn pipe_handles_pair_reader_and_writer() {
        // pipe(2) makes two descriptors that swap handle and peer; the reader's end holds the
        // bytes waiting.
        let reading = pipe_end(0x2000, 0x1000, FREAD, 512).unwrap();
        let writing = pipe_end(0x1000, 0x2000, FWRITE, 0).unwrap();
        assert_eq!(reading.id, writing.id);
        assert_eq!(reading.access, AccessMode::Read);
        assert_eq!(writing.access, AccessMode::Write);
        // Another pipe is another name, and a pipe whose peer closed still has one.
        assert_ne!(pipe_end(0x4000, 0x3000, FREAD, 0).unwrap().id, reading.id);
        assert_eq!(
            pipe_end(0x5000, 0, FWRITE, 0).unwrap().id,
            PipeId::Anonymous(0x5000)
        );
        assert_eq!(pipe_end(0x1, 0x2, 0, 0), None, "neither read nor write");
        let (producer, consumer) = (
            Identity { pid: 10, start: 1 },
            Identity { pid: 20, start: 2 },
        );
        let (pipes, truncated) =
            pair_pipes([(producer, &[writing][..]), (consumer, &[reading][..])]);
        assert!(!truncated);
        assert_eq!(
            pipes,
            [Pipe {
                reader: consumer,
                writer: producer,
                two_way: false,
                buffered: Some(512),
                fifo: false,
            }]
        );
    }

    #[test]
    fn a_fifo_is_a_vnode_of_type_fifo_named_by_device_and_inode() {
        let fifo = 0o010600;
        let end = fifo_end(stat(fifo, 1, 77, 0), FREAD | FWRITE).unwrap();
        assert_eq!(
            (end.id, end.access, end.buffered),
            (
                PipeId::Fifo {
                    device: 0x0100_0004,
                    inode: 77
                },
                AccessMode::Both,
                None
            )
        );
        assert_eq!(
            fifo_end(stat(0o100600, 1, 78, 5), FREAD),
            None,
            "a regular file"
        );
        assert_eq!(
            fifo_end(stat(fifo, 1, 79, 0), 0),
            None,
            "opened for neither"
        );
    }

    /// A vnode descriptor at `offset`, opened with `open_flags`, whose file has `mode`.
    fn vnode_at(mode: u16, links: u16, open_flags: u32, offset: i64, path: &str) -> Vnode {
        vnode(
            9,
            PathedVnode {
                stat: stat(mode, links, 4242, 1 << 20),
                path,
                open_flags,
                offset,
            },
        )
    }

    #[test]
    fn macos_open_flags_and_fd_types_map_to_access_and_target() {
        // The listing alone tells the kinds that are not vnodes; a kqueue, shared memory and the
        // like are the anonymous ones.
        for (proc_fdtype, kind) in [
            (1, FdKind::Vnode),
            (2, FdKind::Socket),
            (6, FdKind::Pipe),
            (0, FdKind::Anonymous),
            (3, FdKind::Anonymous),
            (4, FdKind::Anonymous),
            (5, FdKind::Anonymous),
            (7, FdKind::Anonymous),
            (9, FdKind::Anonymous),
            (10, FdKind::Anonymous),
            (11, FdKind::Anonymous),
            (99, FdKind::Anonymous),
        ] {
            assert_eq!(fd_kind(proc_fdtype), kind, "proc_fdtype {proc_fdtype}");
        }
        // The open flags are the kernel's: FREAD 1, FWRITE 2, FAPPEND 8.
        let regular = 0o100644;
        let file = |flags, offset| match vnode_at(regular, 1, flags, offset, "/Users/demo/log.txt")
        {
            Vnode::File(file) => file,
            other => panic!("a regular file, not {other:?}"),
        };
        let read = file(FREAD, 4096);
        assert_eq!(
            (read.access, read.position, read.appending, read.deleted),
            (AccessMode::Read, 4096, false, false)
        );
        assert_eq!(
            (read.fd, read.device, read.inode, read.size),
            (9, 0x0100_0004, 4242, 1 << 20)
        );
        assert_eq!(read.path, "/Users/demo/log.txt");
        assert_eq!(file(FWRITE, 0).access, AccessMode::Write);
        assert_eq!(file(FREAD | FWRITE, 0).access, AccessMode::Both);
        let append = file(FWRITE | 0x0008 | 0x0200, 100);
        assert!(append.appending, "FAPPEND is 8, whatever else is set");
        assert!(!file(FWRITE | 0x0200, 100).appending);
        // A file with no link left was deleted while open.
        let Vnode::File(gone) = vnode_at(regular, 0, FWRITE, 7, "") else {
            panic!("a deleted regular file");
        };
        assert!(gone.deleted);
        // Terminals are character devices under /dev/tty* and /dev/console; other devices, the
        // directories, named pipes and descriptors opened for neither reading nor writing are
        // none of the kinds, and a negative offset is no position.
        let device = 0o020620;
        for path in ["/dev/ttys003", "/dev/tty", "/dev/console"] {
            assert_eq!(vnode_at(device, 1, FREAD | FWRITE, 0, path), Vnode::Tty);
        }
        for (mode, flags, offset, path) in [
            (device, FWRITE, 0, "/dev/null"),
            (0o020666, FREAD, 0, "/dev/random"),
            (0o040755, FREAD, 0, "/Users/demo"),
            (0o010600, FREAD, 0, "/tmp/fifo"),
            (0o120755, FREAD, 0, "/tmp/link"),
            (regular, 0, 0, "/tmp/opened-for-events"),
            (regular, FREAD, -1, "/tmp/negative"),
        ] {
            assert_eq!(
                vnode_at(mode, 1, flags, offset, path),
                Vnode::Other,
                "{path}"
            );
        }
        // A size that is negative is no size either, and is not made up as zero.
        let unsized_file = PathedVnode {
            stat: stat(regular, 1, 4243, -1),
            path: "/tmp/unsized",
            open_flags: FREAD,
            offset: 0,
        };
        assert_eq!(vnode(9, unsized_file), Vnode::Other);
    }

    #[test]
    fn macos_selects_by_disk_io_rate() {
        use crate::io_detail::{IO_PROCESSES, select};
        use crate::model::{SyscallIo, testing};
        // Thirty processes whose disk rate is their pid; the busiest by syscalls are the lowest
        // pids, which would be chosen if macOS ranked by calls, and are not.
        let process = |pid: u32, disk: Option<f32>| Process {
            io_rate: disk,
            syscall_io: Some(SyscallIo {
                reads: 1000.0 - pid as f32,
                ..SyscallIo::default()
            }),
            ..testing::process(pid, 1)
        };
        let mut processes: Vec<Process> =
            (1..=30).map(|pid| process(pid, Some(pid as f32))).collect();
        processes.push(process(31, None));
        processes.push(process(32, Some(0.0)));
        let chosen: Vec<u32> = select(&processes, by_disk_rate)
            .iter()
            .map(|id| id.pid)
            .collect();
        let expected: Vec<u32> = (7..=30).rev().collect();
        assert_eq!(
            chosen, expected,
            "24 processes, the largest disk rates first"
        );
        assert_eq!(chosen.len(), IO_PROCESSES);
        assert_eq!(by_disk_rate(&processes[30]), None, "no second reading yet");
        assert_eq!(by_disk_rate(&processes[3]), Some(4.0));
    }
}
