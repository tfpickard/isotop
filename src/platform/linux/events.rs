//! Fork, exec and exit events from the proc connector, and exit records from taskstats, read on
//! the `isotop-events` thread while `Source::LifeEvents` is demanded.
//!
//! Both are netlink sockets, and both work only in the initial PID and user namespaces. The proc
//! connector needs no privilege from Linux 6.6 on (CAP_NET_ADMIN before); taskstats always needs
//! CAP_NET_ADMIN. Messages are in the kernel's byte order, as `include/uapi/linux/cn_proc.h`,
//! `connector.h`, `genetlink.h` and `taskstats.h` lay them out.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex};

use super::net::{attributes, u16_at, u32_at, u64_at};
use super::{parse_boot_time, parse_stat};
use crate::model::Identity;
use crate::platform::{
    Cause, ExitRecord, LifeQueue, LifeSource, Observation, Observed, Source, Sources,
};

/// How long the thread waits for a message before it checks whether it is still wanted, and how
/// long opening waits for the kernel's replies, in milliseconds.
const TIMEOUT_MS: i32 = 200;

/// Bytes of `struct nlmsghdr`, `struct cn_msg` and `struct genlmsghdr`.
const NLMSG_HEADER: usize = 16;
const CN_MSG_HEADER: usize = 20;
const GENL_HEADER: usize = 4;

// connector.h and cn_proc.h.
const CN_IDX_PROC: u32 = 1;
const CN_VAL_PROC: u32 = 1;
const PROC_CN_MCAST_LISTEN: u32 = 1;
const PROC_EVENT_NONE: u32 = 0;
const PROC_EVENT_FORK: u32 = 1;
const PROC_EVENT_EXEC: u32 = 2;
const PROC_EVENT_EXIT: u32 = 0x8000_0000;

// genetlink.h and taskstats.h.
const GENL_ID_CTRL: u16 = 16;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const TASKSTATS_CMD_GET: u8 = 1;
const TASKSTATS_CMD_NEW: u8 = 2;
const TASKSTATS_TYPE_PID: u16 = 1;
const TASKSTATS_TYPE_STATS: u16 = 3;
const TASKSTATS_TYPE_AGGR_PID: u16 = 4;
const TASKSTATS_TYPE_AGGR_TGID: u16 = 5;
const TASKSTATS_CMD_ATTR_REGISTER_CPUMASK: u16 = 3;
/// The first `struct taskstats` version with `ac_tgid` and the AGROUP flag (the "v12 begin"
/// comment in taskstats.h).
const TASKSTATS_TGID_VERSION: u16 = 12;
/// The first version with `ac_btime64` ("v10" in taskstats.h).
const TASKSTATS_BTIME64_VERSION: u16 = 10;

// acct.h: `ac_flag` bits.
const AFORK: u8 = 0x01;
const AGROUP: u8 = 0x20;

/// Byte offsets of the `struct taskstats` fields isotop reads, from taskstats.h as of Linux 6.8
/// (TASKSTATS_VERSION 14) and checked there with `offsetof`. New versions only append fields.
mod field {
    pub const VERSION: usize = 0;
    pub const EXITCODE: usize = 4;
    pub const FLAG: usize = 8;
    pub const COMM: usize = 80;
    pub const COMM_LENGTH: usize = 32;
    pub const PID: usize = 128;
    pub const BTIME: usize = 136;
    pub const ETIME: usize = 144;
    pub const UTIME: usize = 152;
    pub const STIME: usize = 160;
    pub const HIWATER_RSS: usize = 200;
    pub const READ_BYTES: usize = 248;
    pub const WRITE_BYTES: usize = 256;
    pub const NVCSW: usize = 272;
    pub const NIVCSW: usize = 280;
    pub const BTIME64: usize = 344;
    pub const TGID: usize = 368;
}

/// More threads or unfinished processes than this means records were lost; the tables start over.
const MOST_TRACKED: usize = 1 << 16;

/// CLOCK_MONOTONIC in nanoseconds.
pub fn monotonic_ns() -> u64 {
    clock_ns(libc::CLOCK_MONOTONIC)
}

fn clock_ns(clock: libc::clockid_t) -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is a valid, writable timespec for the duration of the call.
    if unsafe { libc::clock_gettime(clock, &mut time) } != 0 {
        return 0;
    }
    (time.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.tv_nsec as u64)
}

/// Opens the proc connector and taskstats, each once, and starts the `isotop-events` thread that
/// reads them into `queue`; returns the sources that opened, or None, with no thread, when
/// neither did. Linux needs no list of processes to watch.
pub fn open_events(
    queue: Arc<LifeQueue>,
    _identities: Arc<Mutex<HashMap<u32, Identity>>>,
) -> Option<Sources> {
    if !initial_namespaces() {
        return None;
    }
    let clock = Clock {
        suspended_ns: i128::from(clock_ns(libc::CLOCK_BOOTTIME))
            - i128::from(clock_ns(libc::CLOCK_MONOTONIC)),
        // SAFETY: sysconf takes no pointers and has no preconditions.
        hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64,
        boot_time: fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|text| parse_boot_time(&text)),
    };
    let mut decoder = Decoder::new(clock, read_start);
    let mut early = Vec::new();
    let taskstats = open_taskstats(&mut decoder, &mut early);
    let connector = open_connector(&mut decoder, &mut early);
    let mut opened = Sources::default();
    if connector.is_some() {
        opened.insert(Source::ProcConnector);
    }
    if taskstats.is_some() {
        opened.insert(Source::Taskstats);
    }
    if connector.is_none() && taskstats.is_none() {
        return None;
    }
    for observed in early {
        queue.push(observed);
    }
    std::thread::Builder::new()
        .name("isotop-events".into())
        .spawn(move || run(&queue, connector, taskstats, decoder))
        .ok()?;
    Some(opened)
}

/// Whether this process is in the initial PID and user namespaces, whose inode numbers are fixed
/// (`PROC_PID_INIT_INO` and `PROC_USER_INIT_INO` in `include/linux/proc_ns.h`). True when a link
/// cannot be read, so that the sockets decide.
fn initial_namespaces() -> bool {
    [("pid", "pid:[4026531836]"), ("user", "user:[4026531837]")]
        .iter()
        .all(|(name, initial)| {
            fs::read_link(format!("/proc/self/ns/{name}"))
                .map_or(true, |link| link.as_os_str() == *initial)
        })
}

/// The start of process `pid` in ticks since boot, from /proc/<pid>/stat.
fn read_start(pid: u32) -> Option<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat(pid, &text, 1).map(|(process, _)| process.id.start)
}

/// Reads both sockets until nothing but the thread holds `queue`.
fn run(
    queue: &Arc<LifeQueue>,
    connector: Option<OwnedFd>,
    taskstats: Option<OwnedFd>,
    mut decoder: Decoder,
) {
    let connector_fd = connector.as_ref().map(AsRawFd::as_raw_fd);
    let mut polled: Vec<libc::pollfd> = [&connector, &taskstats]
        .into_iter()
        .flatten()
        .map(|socket| libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let mut buffer = vec![0_u8; 1 << 16];
    let mut observed = Vec::new();
    while Arc::strong_count(queue) > 1 {
        // SAFETY: polled is a valid array of pollfd with the length passed.
        let ready = unsafe {
            libc::poll(
                polled.as_mut_ptr(),
                polled.len() as libc::nfds_t,
                TIMEOUT_MS,
            )
        };
        if ready <= 0 {
            continue;
        }
        for entry in &polled {
            if entry.revents == 0 {
                continue;
            }
            loop {
                match receive(entry.fd, &mut buffer, libc::MSG_DONTWAIT) {
                    Ok(size) if Some(entry.fd) == connector_fd => {
                        decoder.connector(&buffer[..size], &mut observed);
                    }
                    Ok(size) => decoder.taskstats(&buffer[..size], monotonic_ns(), &mut observed),
                    Err(error) if error.raw_os_error() == Some(libc::ENOBUFS) => queue.overrun(),
                    Err(_) => break,
                }
            }
        }
        for one in observed.drain(..) {
            queue.push(one);
        }
    }
}

/// Subscribes to the proc connector. The kernel handles the request inside `send`, and its
/// acknowledgement is multicast to the listeners before `send` returns; outside the initial
/// namespaces the request is ignored and no acknowledgement comes.
fn open_connector(decoder: &mut Decoder, early: &mut Vec<Observed>) -> Option<OwnedFd> {
    let socket = netlink(libc::NETLINK_CONNECTOR, CN_IDX_PROC).ok()?;
    let nonce = std::process::id();
    let mut message = Vec::with_capacity(NLMSG_HEADER + CN_MSG_HEADER + 4);
    push_header(&mut message, libc::NLMSG_DONE as u16, 0, 0);
    for value in [CN_IDX_PROC, CN_VAL_PROC, 0, nonce] {
        message.extend_from_slice(&value.to_ne_bytes());
    }
    message.extend_from_slice(&4_u16.to_ne_bytes());
    message.extend_from_slice(&0_u16.to_ne_bytes());
    message.extend_from_slice(&PROC_CN_MCAST_LISTEN.to_ne_bytes());
    finish(&mut message);
    send(socket.as_raw_fd(), &message).ok()?;
    let mut buffer = vec![0_u8; 1 << 16];
    for _ in 0..256 {
        let size = receive(socket.as_raw_fd(), &mut buffer, 0).ok()?;
        if decoder.connector(&buffer[..size], early) == Some(nonce.wrapping_add(1)) {
            return Some(socket);
        }
    }
    None
}

/// Looks up the taskstats family and registers for the exit records of every CPU.
fn open_taskstats(decoder: &mut Decoder, early: &mut Vec<Observed>) -> Option<OwnedFd> {
    let socket = netlink(libc::NETLINK_GENERIC, 0).ok()?;
    let mut request = Vec::new();
    push_header(&mut request, GENL_ID_CTRL, libc::NLM_F_REQUEST as u16, 1);
    request.extend_from_slice(&[CTRL_CMD_GETFAMILY, 1, 0, 0]);
    push_attribute(&mut request, CTRL_ATTR_FAMILY_NAME, b"TASKSTATS\0");
    finish(&mut request);
    send(socket.as_raw_fd(), &request).ok()?;
    let mut buffer = vec![0_u8; 1 << 16];
    let size = receive(socket.as_raw_fd(), &mut buffer, 0).ok()?;
    let family = family_id(&buffer[..size])?;
    let cpus = fs::read_to_string("/sys/devices/system/cpu/possible")
        .map(|text| text.trim().to_owned())
        .unwrap_or_else(|_| {
            let count = std::thread::available_parallelism().map_or(1, usize::from);
            format!("0-{}", count - 1)
        });
    let mut register = Vec::new();
    let flags = (libc::NLM_F_REQUEST | libc::NLM_F_ACK) as u16;
    push_header(&mut register, family, flags, 2);
    register.extend_from_slice(&[TASKSTATS_CMD_GET, 1, 0, 0]);
    push_attribute(
        &mut register,
        TASKSTATS_CMD_ATTR_REGISTER_CPUMASK,
        format!("{cpus}\0").as_bytes(),
    );
    finish(&mut register);
    send(socket.as_raw_fd(), &register).ok()?;
    decoder.family = Some(family);
    // Records of other CPUs can come before the acknowledgement.
    for _ in 0..256 {
        let size = receive(socket.as_raw_fd(), &mut buffer, 0).ok()?;
        match acknowledgement(&buffer[..size], 2) {
            Some(0) => return Some(socket),
            Some(_) => break,
            None => decoder.taskstats(&buffer[..size], monotonic_ns(), early),
        }
    }
    decoder.family = None;
    None
}

/// The family id in a CTRL_CMD_GETFAMILY reply.
fn family_id(datagram: &[u8]) -> Option<u16> {
    if u16_at(datagram, 4)? != GENL_ID_CTRL {
        return None;
    }
    let length = (u32_at(datagram, 0)? as usize).min(datagram.len());
    attributes(&datagram[..length], NLMSG_HEADER + GENL_HEADER)
        .find(|&(kind, _)| kind == CTRL_ATTR_FAMILY_ID)
        .and_then(|(_, payload)| u16_at(payload, 0))
}

/// The error code of the NLMSG_ERROR reply to request `sequence` (0 for success), or None when
/// the datagram is something else.
fn acknowledgement(datagram: &[u8], sequence: u32) -> Option<i32> {
    if i32::from(u16_at(datagram, 4)?) != libc::NLMSG_ERROR || u32_at(datagram, 8)? != sequence {
        return None;
    }
    Some(u32_at(datagram, NLMSG_HEADER)? as i32)
}

fn push_header(message: &mut Vec<u8>, kind: u16, flags: u16, sequence: u32) {
    message.extend_from_slice(&0_u32.to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(&flags.to_ne_bytes());
    message.extend_from_slice(&sequence.to_ne_bytes());
    message.extend_from_slice(&0_u32.to_ne_bytes());
}

fn push_attribute(message: &mut Vec<u8>, kind: u16, payload: &[u8]) {
    message.extend_from_slice(&((4 + payload.len()) as u16).to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(payload);
    message.resize(message.len().next_multiple_of(4), 0);
}

/// Writes the message's length into its header.
fn finish(message: &mut [u8]) {
    let length = message.len() as u32;
    message[0..4].copy_from_slice(&length.to_ne_bytes());
}

/// A bound netlink socket of `protocol` that has joined the multicast `groups`, with receive
/// timeout `TIMEOUT_MS`.
fn netlink(protocol: libc::c_int, groups: u32) -> io::Result<OwnedFd> {
    // SAFETY: socket() has no memory-safety preconditions; the result is checked below.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            protocol,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a freshly created descriptor that nothing else owns.
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: an all-zero sockaddr_nl is valid; the family and groups are set before use.
    let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as u16;
    address.nl_groups = groups;
    // SAFETY: address outlives the call and its length is passed exactly.
    let bound = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_nl).cast(),
            std::mem::size_of::<libc::sockaddr_nl>() as u32,
        )
    };
    if bound != 0 {
        return Err(io::Error::last_os_error());
    }
    let timeout = libc::timeval {
        tv_sec: 0,
        tv_usec: libc::suseconds_t::from(TIMEOUT_MS) * 1000,
    };
    set_option(socket.as_raw_fd(), libc::SO_RCVTIMEO, &timeout);
    // A burst of forks outruns the default buffer; the kernel caps this at rmem_max.
    set_option(socket.as_raw_fd(), libc::SO_RCVBUF, &(1_i32 << 22));
    Ok(socket)
}

/// Sets a socket option, best effort: a socket keeps working with its defaults.
fn set_option<T>(fd: RawFd, name: libc::c_int, value: &T) {
    // SAFETY: value is valid for reads of size_of::<T>() bytes for the duration of the call.
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            name,
            (value as *const T).cast(),
            std::mem::size_of::<T>() as u32,
        );
    }
}

fn send(fd: RawFd, message: &[u8]) -> io::Result<()> {
    // SAFETY: an all-zero sockaddr_nl is valid and addresses the kernel.
    let mut kernel: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    kernel.nl_family = libc::AF_NETLINK as u16;
    // SAFETY: message and kernel outlive the call and their lengths are passed exactly.
    let sent = unsafe {
        libc::sendto(
            fd,
            message.as_ptr().cast(),
            message.len(),
            0,
            (&kernel as *const libc::sockaddr_nl).cast(),
            std::mem::size_of::<libc::sockaddr_nl>() as u32,
        )
    };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn receive(fd: RawFd, buffer: &mut [u8], flags: libc::c_int) -> io::Result<usize> {
    // SAFETY: the buffer is valid for writes of its full length.
    let received = unsafe { libc::recv(fd, buffer.as_mut_ptr().cast(), buffer.len(), flags) };
    if received < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(received as usize)
}

/// Converts the connector's CLOCK_MONOTONIC timestamps and taskstats's epoch seconds into process
/// starts as /proc/<pid>/stat gives them: ticks since boot, counting time suspended.
#[derive(Clone, Copy, Debug)]
struct Clock {
    /// CLOCK_BOOTTIME minus CLOCK_MONOTONIC when the thread started: the time spent suspended.
    suspended_ns: i128,
    /// USER_HZ, the unit of a start.
    hz: f64,
    /// When the machine booted, in seconds since the Unix epoch (`btime` in /proc/stat).
    boot_time: Option<f64>,
}

impl Clock {
    /// The start of a process forked at monotonic `timestamp_ns`.
    fn start_ticks(&self, timestamp_ns: u64) -> u64 {
        let boot_ns = (i128::from(timestamp_ns) + self.suspended_ns).max(0);
        (boot_ns as f64 * self.hz / 1e9) as u64
    }

    /// The start of a process that began at epoch second `seconds`; None without the boot time.
    fn ticks_of_epoch(&self, seconds: u64) -> Option<u64> {
        let boot = self.boot_time?;
        Some(((seconds as f64 - boot).max(0.0) * self.hz) as u64)
    }
}

/// One thread's exit record, as far as isotop reads it.
#[derive(Debug)]
struct ThreadRecord {
    status: u32,
    flag: u8,
    comm: String,
    tid: u32,
    /// `ac_tgid`, in records whose version has it.
    tgid: Option<u32>,
    btime: u64,
    etime: u64,
    cpu: u64,
    hiwater_kib: u64,
    read_bytes: u64,
    write_bytes: u64,
    nvcsw: u64,
    nivcsw: u64,
}

fn parse_record(stats: &[u8]) -> Option<ThreadRecord> {
    let version = u16_at(stats, field::VERSION)?;
    let comm = stats.get(field::COMM..field::COMM + field::COMM_LENGTH)?;
    let comm = &comm[..comm
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(comm.len())];
    let btime = if version >= TASKSTATS_BTIME64_VERSION {
        u64_at(stats, field::BTIME64)?
    } else {
        u64::from(u32_at(stats, field::BTIME)?)
    };
    Some(ThreadRecord {
        status: u32_at(stats, field::EXITCODE)?,
        flag: *stats.get(field::FLAG)?,
        comm: String::from_utf8_lossy(comm).into_owned(),
        tid: u32_at(stats, field::PID)?,
        tgid: if version >= TASKSTATS_TGID_VERSION {
            Some(u32_at(stats, field::TGID)?)
        } else {
            None
        },
        btime,
        etime: u64_at(stats, field::ETIME)?,
        cpu: u64_at(stats, field::UTIME)?.saturating_add(u64_at(stats, field::STIME)?),
        hiwater_kib: u64_at(stats, field::HIWATER_RSS)?,
        read_bytes: u64_at(stats, field::READ_BYTES)?,
        write_bytes: u64_at(stats, field::WRITE_BYTES)?,
        nvcsw: u64_at(stats, field::NVCSW)?,
        nivcsw: u64_at(stats, field::NIVCSW)?,
    })
}

/// Turns connector and taskstats datagrams into observations. Between messages it keeps which
/// thread belongs to which process, and the records of a process's threads until its last.
struct Decoder {
    clock: Clock,
    /// The taskstats family id once taskstats is registered.
    family: Option<u16>,
    /// Thread id to the process (tgid) it belongs to, from the connector's thread forks.
    threads: HashMap<u32, u32>,
    /// The records of each process's exited threads, summed, until its last thread's.
    sums: HashMap<u32, ExitRecord>,
    /// Reads the start of a process; tests put a table in place of /proc.
    start: fn(u32) -> Option<u64>,
}

impl Decoder {
    fn new(clock: Clock, start: fn(u32) -> Option<u64>) -> Self {
        Decoder {
            clock,
            family: None,
            threads: HashMap::new(),
            sums: HashMap::new(),
            start,
        }
    }

    /// Decodes one connector datagram into `out`. Returns the `ack` number of an
    /// acknowledgement that reports success, which is how `open_connector` knows it listens.
    fn connector(&mut self, datagram: &[u8], out: &mut Vec<Observed>) -> Option<u32> {
        let message = datagram.get(NLMSG_HEADER..)?;
        if u32_at(message, 0)? != CN_IDX_PROC || u32_at(message, 4)? != CN_VAL_PROC {
            return None;
        }
        let ack = u32_at(message, 12)?;
        let event = message.get(CN_MSG_HEADER..)?;
        let at = u64_at(event, 8)?;
        let data = |index: usize| u32_at(event, 16 + 4 * index);
        match u32_at(event, 0)? {
            PROC_EVENT_NONE => return (data(0)? == 0).then_some(ack),
            PROC_EVENT_FORK => {
                let (parent, child, child_tgid) = (data(1)?, data(2)?, data(3)?);
                if child != child_tgid {
                    self.thread_started(child, child_tgid);
                    return None;
                }
                out.push(Observed {
                    at,
                    source: LifeSource::Connector,
                    what: Observation::Fork {
                        parent,
                        child,
                        parent_start: (self.start)(parent),
                        child_start: (self.start)(child),
                        estimated_start: self.clock.start_ticks(at),
                    },
                });
            }
            PROC_EVENT_EXEC => {
                let (pid, tgid) = (data(0)?, data(1)?);
                if pid == tgid {
                    out.push(Observed {
                        at,
                        source: LifeSource::Connector,
                        what: Observation::Exec { pid, start: None },
                    });
                }
            }
            PROC_EVENT_EXIT => {
                let (pid, tgid, status) = (data(0)?, data(1)?, data(2)?);
                if pid != tgid {
                    // With taskstats the thread's record, which may come later, forgets it.
                    if self.family.is_none() {
                        self.threads.remove(&pid);
                    }
                    return None;
                }
                out.push(Observed {
                    at,
                    source: LifeSource::Connector,
                    what: Observation::Exit {
                        pid,
                        start: None,
                        cause: Cause::of_status(status),
                    },
                });
            }
            _ => {}
        }
        None
    }

    fn thread_started(&mut self, tid: u32, tgid: u32) {
        if self.threads.len() >= MOST_TRACKED {
            self.threads.clear();
        }
        self.threads.insert(tid, tgid);
    }

    /// Decodes one taskstats datagram, received at monotonic `received`, into `out`: a process's
    /// record once its last thread's has come.
    fn taskstats(&mut self, datagram: &[u8], received: u64, out: &mut Vec<Observed>) {
        let Some(family) = self.family else {
            return;
        };
        let mut offset = 0;
        while let Some(length) = u32_at(datagram, offset).map(|length| length as usize) {
            let Some(message) = datagram
                .get(offset..offset + length)
                .filter(|_| length >= 16)
            else {
                return;
            };
            if u16_at(message, 4) == Some(family)
                && message.get(NLMSG_HEADER) == Some(&TASKSTATS_CMD_NEW)
            {
                self.record_message(message, received, out);
            }
            offset += length.next_multiple_of(4);
        }
    }

    fn record_message(&mut self, message: &[u8], received: u64, out: &mut Vec<Observed>) {
        let mut thread = None;
        let mut group_ended = false;
        for (kind, payload) in attributes(message, NLMSG_HEADER + GENL_HEADER) {
            match kind {
                TASKSTATS_TYPE_AGGR_PID => {
                    let nested: Vec<(u16, &[u8])> = attributes(payload, 0).collect();
                    let tid = nested
                        .iter()
                        .find(|(kind, _)| *kind == TASKSTATS_TYPE_PID)
                        .and_then(|(_, pid)| u32_at(pid, 0));
                    thread = nested
                        .iter()
                        .find(|(kind, _)| *kind == TASKSTATS_TYPE_STATS)
                        .and_then(|(_, stats)| parse_record(stats))
                        .map(|record| ThreadRecord {
                            tid: tid.unwrap_or(record.tid),
                            ..record
                        });
                }
                // Sent with the last thread's record of a process that had more than one; its
                // figures are not used.
                TASKSTATS_TYPE_AGGR_TGID => group_ended = true,
                _ => {}
            }
        }
        let Some(thread) = thread else {
            return;
        };
        let tgid = thread
            .tgid
            .or_else(|| self.threads.get(&thread.tid).copied())
            .unwrap_or(thread.tid);
        self.threads.remove(&thread.tid);
        let last = if thread.tgid.is_some() {
            thread.flag & AGROUP != 0
        } else {
            // Before AGROUP: the group's end is the aggregate's arrival, or the leader's record
            // when no thread of it is known to live on.
            group_ended || (thread.tid == tgid && !self.threads.values().any(|&of| of == tgid))
        };
        if self.sums.len() >= MOST_TRACKED {
            self.sums.clear();
        }
        let sum = self.sums.entry(tgid).or_default();
        let first = sum.threads == 0;
        sum.threads += 1;
        sum.cpu = sum.cpu.saturating_add(thread.cpu);
        sum.read_bytes = sum.read_bytes.saturating_add(thread.read_bytes);
        sum.write_bytes = sum.write_bytes.saturating_add(thread.write_bytes);
        sum.nvcsw = sum.nvcsw.saturating_add(thread.nvcsw);
        sum.nivcsw = sum.nivcsw.saturating_add(thread.nivcsw);
        sum.hiwater_rss = sum.hiwater_rss.max(thread.hiwater_kib.saturating_mul(1024));
        sum.btime = if first {
            thread.btime
        } else {
            sum.btime.min(thread.btime)
        };
        sum.etime = sum.etime.max(thread.etime);
        // The leader's command and AFORK speak for the process; another thread's command only
        // until the leader's record comes.
        if thread.tid == tgid {
            sum.comm = thread.comm;
            sum.forked_never_execd = thread.flag & AFORK != 0;
        } else if sum.comm.is_empty() {
            sum.comm = thread.comm;
        }
        if !last {
            return;
        }
        let Some(record) = self.sums.remove(&tgid) else {
            return;
        };
        out.push(Observed {
            at: received,
            source: LifeSource::Taskstats,
            what: Observation::Record {
                tgid,
                start: self.clock.ticks_of_epoch(record.btime),
                record,
                cause: Cause::of_status(thread.status),
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOCK: Clock = Clock {
        suspended_ns: 2_000_000_000,
        hz: 100.0,
        boot_time: Some(1_700_000_000.0),
    };

    /// Starts in ticks: pid 300 started 50 s after boot, 301 61 s after; every other pid is gone.
    fn table(pid: u32) -> Option<u64> {
        match pid {
            300 => Some(5000),
            301 => Some(6100),
            _ => None,
        }
    }

    fn decoder() -> Decoder {
        Decoder::new(CLOCK, table)
    }

    // A proc connector datagram as linux/cn_proc.h and linux/connector.h lay it out (Linux 6.8):
    // nlmsghdr (16 bytes, type NLMSG_DONE), cn_msg (20: idx 1, val 1, seq, ack, len 40, flags),
    // proc_event (40: what, cpu, timestamp_ns, event_data). Little-endian, as on every target
    // isotop builds for.

    /// PROC_EVENT_FORK at 61.5 s: parent pid 300 tgid 300, child pid 301 tgid 301.
    const FORK: [u8; 76] = [
        76, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // nlmsghdr
        1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, // cn_msg
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0x87, 0xaf, 0x51, 0x0e, 0, 0, 0, // what, cpu, 61.5e9 ns
        44, 1, 0, 0, 44, 1, 0, 0, 45, 1, 0, 0, 45, 1, 0, 0, // 300, 300, 301, 301
        0, 0, 0, 0, 0, 0, 0, 0, // the rest of the union
    ];

    /// PROC_EVENT_EXEC at 62 s: pid 301 tgid 301.
    const EXEC: [u8; 76] = [
        76, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // nlmsghdr
        1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, // cn_msg
        2, 0, 0, 0, 0, 0, 0, 0, 0, 0xec, 0x7c, 0x6f, 0x0e, 0, 0, 0, // what, cpu, 62e9 ns
        45, 1, 0, 0, 45, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 301, 301
        0, 0, 0, 0, 0, 0, 0, 0,
    ];

    /// PROC_EVENT_EXIT at 63 s: pid 301 tgid 301, exit_code 0x0300 (status 3), exit_signal 17.
    const EXIT: [u8; 76] = [
        76, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // nlmsghdr
        1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 40, 0, 0, 0, // cn_msg
        0, 0, 0, 0x80, 0, 0, 0, 0, 0, 0xb6, 0x17, 0xab, 0x0e, 0, 0, 0, // what, cpu, 63e9 ns
        45, 1, 0, 0, 45, 1, 0, 0, 0, 3, 0, 0, 17, 0, 0, 0, // 301, 301, 0x300, 17
        44, 1, 0, 0, 44, 1, 0, 0, // parent 300, 300
    ];

    /// A connector datagram with `what` and the event data words, built like the arrays above.
    fn event(what: u32, at: u64, data: [u32; 6]) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_header(&mut bytes, libc::NLMSG_DONE as u16, 0, 0);
        for value in [CN_IDX_PROC, CN_VAL_PROC, 0, 0] {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        bytes.extend_from_slice(&40_u16.to_ne_bytes());
        bytes.extend_from_slice(&0_u16.to_ne_bytes());
        bytes.extend_from_slice(&what.to_ne_bytes());
        bytes.extend_from_slice(&0_u32.to_ne_bytes());
        bytes.extend_from_slice(&at.to_ne_bytes());
        for value in data {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        finish(&mut bytes);
        bytes
    }

    fn decode(decoder: &mut Decoder, datagrams: &[&[u8]]) -> Vec<Observed> {
        let mut out = Vec::new();
        for datagram in datagrams {
            decoder.connector(datagram, &mut out);
        }
        out
    }

    /// A `struct taskstats` of `version` as linux/taskstats.h lays it out (Linux 6.8,
    /// TASKSTATS_VERSION 14; the offsets checked there with offsetof), for thread `tid`:
    /// `ac_tgid` is written only when the version has it.
    fn stats(version: u16, tid: u32, tgid: u32, flag: u8, comm: &str, cpu: u64) -> [u8; 432] {
        let mut stats = [0_u8; 432];
        let mut put = |at: usize, bytes: &[u8]| stats[at..at + bytes.len()].copy_from_slice(bytes);
        put(0, &version.to_ne_bytes()); // version
        put(4, &0x0100_u32.to_ne_bytes()); // ac_exitcode: status 1
        put(8, &[flag]); // ac_flag
        put(80, comm.as_bytes()); // ac_comm[32]
        put(128, &tid.to_ne_bytes()); // ac_pid
        put(136, &1_700_000_061_u32.to_ne_bytes()); // ac_btime
        put(144, &(cpu * 3).to_ne_bytes()); // ac_etime, microseconds
        put(152, &(cpu / 2).to_ne_bytes()); // ac_utime
        put(160, &(cpu - cpu / 2).to_ne_bytes()); // ac_stime
        put(200, &2048_u64.to_ne_bytes()); // hiwater_rss, KiB
        put(248, &4096_u64.to_ne_bytes()); // read_bytes
        put(256, &512_u64.to_ne_bytes()); // write_bytes
        put(272, &7_u64.to_ne_bytes()); // nvcsw
        put(280, &3_u64.to_ne_bytes()); // nivcsw
        if version >= 10 {
            put(344, &1_700_000_061_u64.to_ne_bytes()); // ac_btime64
        }
        if version >= 12 {
            put(368, &tgid.to_ne_bytes()); // ac_tgid
        }
        stats
    }

    const FAMILY: u16 = 27;

    /// A TASKSTATS_CMD_NEW datagram: nlmsghdr, genlmsghdr, then TASKSTATS_TYPE_AGGR_PID holding
    /// TASKSTATS_TYPE_PID and TASKSTATS_TYPE_STATS, and TASKSTATS_TYPE_AGGR_TGID with `aggregate`.
    fn taskstats_message(tid: u32, stats: &[u8; 432], aggregate: Option<u32>) -> Vec<u8> {
        let mut message = Vec::new();
        push_header(&mut message, FAMILY, 0, 0);
        message.extend_from_slice(&[TASKSTATS_CMD_NEW, 1, 0, 0]);
        let mut nested = Vec::new();
        push_attribute(&mut nested, TASKSTATS_TYPE_PID, &tid.to_ne_bytes());
        push_attribute(&mut nested, TASKSTATS_TYPE_STATS, stats);
        push_attribute(&mut message, TASKSTATS_TYPE_AGGR_PID, &nested);
        if let Some(tgid) = aggregate {
            let mut nested = Vec::new();
            push_attribute(&mut nested, 2, &tgid.to_ne_bytes());
            push_attribute(&mut nested, TASKSTATS_TYPE_STATS, &[0; 432]);
            push_attribute(&mut message, TASKSTATS_TYPE_AGGR_TGID, &nested);
        }
        finish(&mut message);
        message
    }

    fn records(decoder: &mut Decoder, messages: &[Vec<u8>]) -> Vec<Observed> {
        decoder.family = Some(FAMILY);
        let mut out = Vec::new();
        for message in messages {
            decoder.taskstats(message, 99, &mut out);
        }
        out
    }

    #[test]
    fn proc_connector_messages_decode_fork_exec_and_exit() {
        let observed = decode(&mut decoder(), &[&FORK, &EXEC, &EXIT]);
        let what: Vec<&Observation> = observed.iter().map(|one| &one.what).collect();
        assert_eq!(
            what,
            [
                &Observation::Fork {
                    parent: 300,
                    child: 301,
                    parent_start: Some(5000),
                    child_start: Some(6100),
                    estimated_start: 6350,
                },
                &Observation::Exec {
                    pid: 301,
                    start: None
                },
                &Observation::Exit {
                    pid: 301,
                    start: None,
                    cause: Cause::Code(3)
                },
            ]
        );
        assert!(
            observed
                .iter()
                .all(|one| one.source == LifeSource::Connector)
        );
        assert_eq!(
            observed.iter().map(|one| one.at).collect::<Vec<_>>(),
            [61_500_000_000, 62_000_000_000, 63_000_000_000]
        );
    }

    #[test]
    fn connector_exit_status_decodes_code_signal_and_core() {
        let cause = |status: u32| {
            let exit = event(PROC_EVENT_EXIT, 1, [9, 9, status, 17, 1, 1]);
            match &decode(&mut decoder(), &[&exit])[..] {
                [
                    Observed {
                        what: Observation::Exit { cause, .. },
                        ..
                    },
                ] => *cause,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(cause(0), Cause::Code(0));
        assert_eq!(cause(1 << 8), Cause::Code(1));
        assert_eq!(cause(255 << 8), Cause::Code(255));
        assert_eq!(
            cause(15),
            Cause::Signal {
                number: 15,
                core: false
            }
        );
        assert_eq!(
            cause(9),
            Cause::Signal {
                number: 9,
                core: false
            }
        );
        assert_eq!(
            cause(0x80 | 11),
            Cause::Signal {
                number: 11,
                core: true
            }
        );
    }

    #[test]
    fn a_child_gone_before_its_stat_is_read_gets_its_start_from_the_fork_timestamp() {
        // A child that is gone before its stat can be read gets its start from the fork's
        // timestamp: 61.5 s monotonic plus 2 s suspended is 63.5 s since boot, 6350 ticks.
        let fork = event(PROC_EVENT_FORK, 61_500_000_000, [300, 300, 777, 777, 0, 0]);
        let observed = decode(&mut decoder(), &[&fork]);
        assert!(matches!(
            observed[0].what,
            Observation::Fork {
                child_start: None,
                estimated_start: 6350,
                ..
            }
        ));
        assert_eq!(
            CLOCK.start_ticks(0),
            200,
            "suspended time counts as boot time"
        );
        assert_eq!(CLOCK.ticks_of_epoch(1_700_000_061), Some(6100));
        assert_eq!(
            Clock {
                boot_time: None,
                ..CLOCK
            }
            .ticks_of_epoch(1_700_000_061),
            None
        );
        // The timestamp itself stays monotonic, as `Observed::at` is; the collector maps it onto
        // `Snapshot::elapsed`.
        assert_eq!(observed[0].at, 61_500_000_000);
    }

    #[test]
    fn thread_forks_and_exits_are_not_reported_as_processes() {
        let mut decoder = decoder();
        let thread_fork = event(PROC_EVENT_FORK, 1, [301, 301, 302, 301, 0, 0]);
        let thread_exec = event(PROC_EVENT_EXEC, 2, [302, 301, 0, 0, 0, 0]);
        let thread_exit = event(PROC_EVENT_EXIT, 3, [302, 301, 0, 17, 300, 300]);
        let acknowledged = event(PROC_EVENT_NONE, 4, [0; 6]);
        assert!(
            decode(
                &mut decoder,
                &[&thread_fork, &thread_exec, &thread_exit, &acknowledged]
            )
            .is_empty()
        );
        // Without taskstats nothing needs the thread once it has exited.
        assert!(decoder.threads.is_empty());
        decoder.family = Some(FAMILY);
        decode(&mut decoder, &[&thread_fork, &thread_exit]);
        assert_eq!(decoder.threads.get(&302), Some(&301), "kept for its record");
        let mut out = Vec::new();
        assert_eq!(decoder.connector(&acknowledged, &mut out), Some(0));
        let failed = event(PROC_EVENT_NONE, 4, [1, 0, 0, 0, 0, 0]);
        assert_eq!(decoder.connector(&failed, &mut out), None);
    }

    #[test]
    fn taskstats_thread_records_sum_into_one_process_record() {
        let mut decoder = decoder();
        // Version 14 records carry ac_tgid: two threads of process 301, then the leader with
        // AGROUP and AFORK, and the aggregate, which is not used.
        let observed = records(
            &mut decoder,
            &[
                taskstats_message(302, &stats(14, 302, 301, 0, "worker", 100), None),
                taskstats_message(303, &stats(14, 303, 301, 0, "worker", 200), None),
                taskstats_message(
                    301,
                    &stats(14, 301, 301, AGROUP | AFORK, "server", 300),
                    Some(301),
                ),
            ],
        );
        let [
            Observed {
                at: 99,
                source: LifeSource::Taskstats,
                what:
                    Observation::Record {
                        tgid: 301,
                        record,
                        cause: Cause::Code(1),
                        start: Some(6100),
                    },
            },
        ] = &observed[..]
        else {
            panic!("{observed:?}");
        };
        assert_eq!(
            *record,
            ExitRecord {
                comm: "server".into(),
                btime: 1_700_000_061,
                etime: 900,
                cpu: 600,
                hiwater_rss: 2048 * 1024,
                read_bytes: 3 * 4096,
                write_bytes: 3 * 512,
                nvcsw: 21,
                nivcsw: 9,
                forked_never_execd: true,
                threads: 3,
            }
        );
        assert!(decoder.sums.is_empty());
    }

    #[test]
    fn taskstats_records_without_ac_tgid_are_attributed_through_thread_forks() {
        let mut decoder = decoder();
        decoder.family = Some(FAMILY);
        // Version 11 records have no ac_tgid and no AGROUP: thread 302's process is known only
        // from its fork, and the aggregate marks the end.
        decode(
            &mut decoder,
            &[&event(PROC_EVENT_FORK, 1, [301, 301, 302, 301, 0, 0])],
        );
        let observed = records(
            &mut decoder,
            &[
                taskstats_message(302, &stats(11, 302, 0, 0, "worker", 100), None),
                taskstats_message(301, &stats(11, 301, 0, 0, "server", 300), Some(301)),
                // A process nobody saw fork a thread counts as its own and ends with its leader.
                taskstats_message(400, &stats(9, 400, 0, AFORK, "sh", 50), None),
            ],
        );
        let summary: Vec<(u32, u32, u64, &str)> = observed
            .iter()
            .map(|one| match &one.what {
                Observation::Record { tgid, record, .. } => {
                    (*tgid, record.threads, record.cpu, record.comm.as_str())
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(summary, [(301, 2, 400, "server"), (400, 1, 50, "sh")]);
        assert!(decoder.threads.is_empty() && decoder.sums.is_empty());
    }
}
