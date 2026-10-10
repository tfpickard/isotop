//! The macOS collector's decisions, kept free of FFI so they compile and are tested on Linux
//! too (`platform/mod.rs` includes this file in Linux test builds).

// On Linux only the tests below use these items.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::model::{CoreKind, Kind};
use crate::platform::{Network, Remote};

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
pub fn busy(previous: [u32; 4], current: [u32; 4]) -> f32 {
    let [user, system, idle, nice] =
        [0, 1, 2, 3].map(|i| u64::from(current[i].wrapping_sub(previous[i])));
    let total = user + system + idle + nice;
    if total == 0 {
        return 0.0;
    }
    ((user + system + nice) as f32 / total as f32).clamp(0.0, 1.0)
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
}

/// The pids at both ends of each connected Unix socket pair, once per pair. The pids may be
/// equal. A socket shared by several processes counts for the first one listed.
pub fn pair_unix(sockets: &[UnixSocket]) -> Vec<(u32, u32)> {
    let mut owners: HashMap<u64, (u32, u64)> = HashMap::new();
    for socket in sockets.iter().filter(|socket| socket.pcb != 0) {
        owners
            .entry(socket.pcb)
            .or_insert((socket.pid, socket.peer));
    }
    let mut pairs: Vec<(u64, (u32, u32))> = owners
        .iter()
        .filter(|&(&pcb, &(_, peer))| peer != 0 && pcb < peer)
        .filter_map(|(&pcb, &(pid, peer))| {
            let &(other, back) = owners.get(&peer)?;
            (back == pcb).then_some((pcb, (pid, other)))
        })
        .collect();
    pairs.sort_unstable();
    pairs.into_iter().map(|(_, pair)| pair).collect()
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

/// Links, outside connections and remotes from every socket the scan could read. macOS reports
/// no per-socket byte counters, so `loopback` stays empty.
pub fn network(unix: &[UnixSocket], tcp: &[TcpSocket], own: &HashSet<IpAddr>) -> Network {
    let mut network = Network::default();
    let (tcp_pairs, remotes) = pair_tcp(tcp, own);
    for (a, b) in pair_unix(unix).into_iter().chain(tcp_pairs) {
        if a != b {
            *network.links.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    for remote in &remotes {
        *network.outside.entry(remote.pid).or_default() += 1;
    }
    network.remotes = remotes;
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(busy([0; 4], [0; 4]), 0.0);
        assert_eq!(busy([10, 10, 10, 10], [40, 20, 70, 10]), 0.4);
        let before = [u32::MAX - 9, 0, u32::MAX - 29, 0];
        let after = [20, 0, 10, 0];
        assert!((busy(before, after) - 30.0 / 70.0).abs() < 1e-6);
        assert_eq!(busy([0, 0, 0, 0], [5, 0, 0, 5]), 1.0);
    }

    #[test]
    fn pressure_levels_and_available_memory() {
        assert_eq!(pressure_from_level(1), 0.0);
        assert_eq!(pressure_from_level(2), 25.0);
        assert_eq!(pressure_from_level(4), 75.0);
        assert_eq!(pressure_from_level(0), 0.0);
        assert_eq!(available(10, 5, 16384), 15 * 16384);
        assert_eq!(available(u32::MAX, u32::MAX, u64::MAX), u64::MAX);
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
            },
            UnixSocket {
                pid: 20,
                pcb: 0xb,
                peer: 0xa,
            },
            // The same socket inherited by a child counts once.
            UnixSocket {
                pid: 30,
                pcb: 0xb,
                peer: 0xa,
            },
            // A pair inside one process.
            UnixSocket {
                pid: 40,
                pcb: 0xc,
                peer: 0xd,
            },
            UnixSocket {
                pid: 40,
                pcb: 0xd,
                peer: 0xc,
            },
            // A listener and a socket whose peer belongs to an unreadable process.
            UnixSocket {
                pid: 50,
                pcb: 0xe,
                peer: 0,
            },
            UnixSocket {
                pid: 60,
                pcb: 0xf,
                peer: 0x99,
            },
        ];
        assert_eq!(pair_unix(&sockets), vec![(10, 20), (40, 40)]);
        let network = network(&sockets, &[], &HashSet::new());
        assert_eq!(network.links, HashMap::from([((10, 20), 1)]));
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
}
