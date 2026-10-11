//! Everything isotop reads from the operating system. The rest of the program sees only the
//! contract below, which each platform module provides under the same names:
//!
//! - `Sampler`, read on the frame loop once per sample:
//!   - `Sampler::new() -> Sampler`
//!   - `hz(&self) -> f32`: the unit of `RawProcess::ticks`, in ticks per second;
//!   - `processes(&mut self) -> io::Result<Vec<RawProcess>>`, in any order; an error means
//!     live mode cannot run at all;
//!   - `memory(&self) -> (Option<u64>, Option<u64>)`: total and available bytes, each None when
//!     it could not be read;
//!   - `pressure(&self) -> ([f32; 3], Sources)`: CPU, memory and I/O stall percentages, and
//!     the pressure sources among them that could not be read (their value is 0);
//!   - `cpus(&mut self, dt: f32) -> Vec<Cpu>`: per-CPU use over the last `dt` seconds, sorted
//!     by id. It is called after `processes` in every sample, so a platform may derive CPU
//!     readings from the processes it has just read. A reading it cannot take, the first
//!     reading of a rate included, is None in the `Cpu`, never 0;
//!   - `missing(&self) -> Sources`: what this platform does not measure, as of the last
//!     `processes` and `cpus` calls, added to `Snapshot::missing` every sample. `Source` lists
//!     what each source's absence means for the values;
//!   - `per_cluster(&self) -> Sources`: sources measured per CPU cluster rather than per CPU:
//!     `Source::CpuClock` when every CPU's `Cpu::mhz` is the average clock of the cluster of
//!     its kind. Linux returns none;
//!   - `unreadable(&self) -> usize`: how many processes the last `processes()` call could see
//!     but not measure because they belong to another user, and so left out. Races with exiting
//!     processes are not permission gaps and do not count; Linux returns 0;
//!   - `shadows(&self) -> Vec<Shadow>`: those unreadable processes again, as far as anyone may
//!     read them (pid, parent and name), so that what descends from them can still be drawn
//!     under them. Linux returns none, as every `/proc/<pid>/stat` is world-readable.
//! - Slow sources, read on the background thread every `model::BACKGROUND_PERIOD`:
//!   - `network() -> Network`;
//!   - `account(paths, state) -> HashMap<String, Unit>`: resource accounting of the named
//!     cgroups, given the state the previous call left; a reading that could not be taken, or
//!     that is a rate with no earlier reading, is None in the `Unit`;
//!   - `FileScan::new() -> FileScan` and `FileScan::sample(&mut self) -> HashMap<u32, Option<Files>>`:
//!     the regular files each process holds open, None for a table that could not be read; a
//!     pid the scan has not reached is absent;
//!   - `locks() -> Option<Locks>`: file locks with their holders and waiters, None when they
//!     cannot be read (the collector then reports `Source::FileLocks` missing);
//!   - `Gpu::load() -> Option<Gpu>` and `Gpu::sample(&self) -> HashMap<u32, GpuMemory>`: GPU
//!     memory per pid; without a `Gpu` the collector reports `Source::GpuMemory` missing.
//! - `journal() -> io::Result<(Child, JournalParser)>`: a running log follower with piped
//!   stdout and stderr, and the parser that reads one entry at a time from its stdout;
//!   `JOURNAL: &str` names the follower (`journalctl` or `log`) in the messages isotop prints
//!   when it cannot run or when it complains on stderr.
//! - `stop_journal(child: &mut Child)`: ends that follower and every process it started, and
//!   reaps it. The child may be a shell running the follower's commands in turn (macOS), so
//!   killing the child alone could leave the command it is waiting on running.
//! - `MEMORY_LABEL: &str` names what `Process::memory` measures (RSS on Linux), as the
//!   inspector and legends show it.
//! - `GROUP_LABEL: &str` names what `Process::group` stands for when `Source::Cgroups` is
//!   missing, and `WITHOUT_CGROUPS: &str` says how processes are grouped then and why, as the
//!   Cells and Coop legends show them.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, BufRead};
use std::net::IpAddr;

use crate::journal::Line;
use crate::model::{Blocker, IoBytes, Process, SyscallCounts};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

// The macOS collector's pure logic, so its tests also run in the Linux gates.
#[cfg(all(test, not(target_os = "macos")))]
#[path = "macos/logic.rs"]
mod macos_logic;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("isotop has platform support for Linux and macOS only");

// The macOS log parser is std-only; Linux test builds compile it too so its tests run in the
// Linux gates.
#[cfg(all(test, not(target_os = "macos")))]
#[expect(
    dead_code,
    reason = "on Linux only the parser's tests use it, not the macOS follower"
)]
#[path = "macos/journal.rs"]
mod macos_journal;

/// A system-wide source of measurements that a platform or a machine may lack. Where one is
/// missing, the values it would fill read None (0 for the system-wide pressures), and the views
/// say so instead of drawing them. A pull request that adds a source adds its variant here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// `Snapshot::pressure[0]`, the CPU stall percentage.
    CpuPressure,
    /// `Snapshot::pressure[1]`, the memory stall percentage.
    MemoryPressure,
    /// `Snapshot::pressure[2]`, the I/O stall percentage.
    IoPressure,
    /// `Process::core`, the CPU each process last ran on.
    LastCpu,
    /// `Cpu::mhz`, the clock of each CPU.
    CpuClock,
    /// `Cpu::wait`, the time tasks spent waiting for each CPU.
    RunQueue,
    /// Cgroup v2 accounting: no `Unit`s, so no limits, quotas, pressure or OOM kills.
    Cgroups,
    /// Per-connection byte counters and round-trip times of TCP sockets.
    SocketTraffic,
    /// The lock table: no process holds or waits for a lock.
    FileLocks,
    /// `Process::gpu_memory`, GPU memory per process.
    GpuMemory,
    /// `Cpu::kind`: every CPU is `CoreKind::Unknown`.
    CoreKinds,
    /// The socket scan: links between processes, loopback traffic and outside connections.
    Sockets,
    /// `Process::files`, each process's open files.
    OpenFiles,
    /// `Remote::place`, where remote addresses are, from the local GeoIP database.
    Locations,
    /// `Process::syscall_io`, the bytes and calls of read and write syscalls.
    SyscallIo,
    /// The system journal: `journalctl` on Linux, the unified log on macOS.
    Journal,
}

impl Source {
    /// Every source, in declaration order.
    pub const ALL: [Source; 16] = [
        Source::CpuPressure,
        Source::MemoryPressure,
        Source::IoPressure,
        Source::LastCpu,
        Source::CpuClock,
        Source::RunQueue,
        Source::Cgroups,
        Source::SocketTraffic,
        Source::FileLocks,
        Source::GpuMemory,
        Source::CoreKinds,
        Source::Sockets,
        Source::OpenFiles,
        Source::Locations,
        Source::SyscallIo,
        Source::Journal,
    ];

    /// The source's name in lower case, as messages show it.
    pub fn name(self) -> &'static str {
        match self {
            Source::CpuPressure => "cpu pressure",
            Source::MemoryPressure => "memory pressure",
            Source::IoPressure => "io pressure",
            Source::LastCpu => "last cpu",
            Source::CpuClock => "cpu clock",
            Source::RunQueue => "run queue",
            Source::Cgroups => "cgroups",
            Source::SocketTraffic => "socket traffic",
            Source::FileLocks => "file locks",
            Source::GpuMemory => "gpu memory",
            Source::CoreKinds => "core kinds",
            Source::Sockets => "sockets",
            Source::OpenFiles => "open files",
            Source::Locations => "remote locations",
            Source::SyscallIo => "syscall io",
            Source::Journal => "journal",
        }
    }

    const fn bit(self) -> u64 {
        1 << self as u32
    }
}

/// A set of sources, one bit per `Source`.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Sources(u64);

impl Sources {
    /// The set of `sources`, for constant contexts such as the view registry's rows.
    pub const fn of(sources: &[Source]) -> Sources {
        let mut bits = 0;
        let mut index = 0;
        while index < sources.len() {
            bits |= sources[index].bit();
            index += 1;
        }
        Sources(bits)
    }

    pub fn contains(self, source: Source) -> bool {
        self.0 & source.bit() != 0
    }

    pub fn insert(&mut self, source: Source) {
        self.0 |= source.bit();
    }

    /// Only tests ask; the views ask for the sources they draw.
    #[cfg(test)]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The sources in either set.
    pub fn union(self, other: Sources) -> Sources {
        Sources(self.0 | other.0)
    }

    /// The sources in the set, in declaration order.
    pub fn iter(self) -> impl Iterator<Item = Source> {
        Source::ALL
            .into_iter()
            .filter(move |&source| self.contains(source))
    }
}

impl From<Source> for Sources {
    fn from(source: Source) -> Self {
        Sources(source.bit())
    }
}

impl FromIterator<Source> for Sources {
    fn from_iter<I: IntoIterator<Item = Source>>(sources: I) -> Self {
        let mut set = Sources::default();
        for source in sources {
            set.insert(source);
        }
        set
    }
}

impl<const N: usize> From<[Source; N]> for Sources {
    fn from(sources: [Source; N]) -> Self {
        sources.into_iter().collect()
    }
}

/// Lists the names, so a failed assertion reads `{"cpu clock", "run queue"}`.
impl fmt::Debug for Sources {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_set()
            .entries(self.iter().map(Source::name))
            .finish()
    }
}

/// One process as the OS reports it, before isotop smooths anything.
pub struct RawProcess {
    /// Every `Process` field the OS gives directly: id, parent, name, command, group, kind,
    /// state, memory, threads, core, cgroup, priority, nice, written, policy and started. The
    /// derived fields (cpu, the I/O rates, system_share, syscall_io, major_faults, cpu_time and
    /// gpu_memory) are left at zero or None, and those the
    /// background thread fills (files, locks_held and blocked_on) at Pending or None, except
    /// that a Linux kernel thread's files are known to be none.
    pub process: Process,
    /// Cumulative CPU time in ticks of `Sampler::hz()`.
    pub ticks: u64,
    /// Cumulative storage I/O counters, or None when unreadable.
    pub io: Option<IoBytes>,
    /// Cumulative CPU time on performance cores, in ticks, from the same reading as `ticks`;
    /// None where the platform does not split CPU time by core kind.
    pub performance_ticks: Option<u64>,
    /// Cumulative time its threads were runnable, running or waiting for a CPU, in ticks, from
    /// the same reading as `ticks`; None when unmeasured. A kernel may bring it up to date only
    /// when a thread is switched onto a CPU or blocks (macOS does), so it can trail `ticks`
    /// while threads run and catch up later; the collector counts as waiting only the growth of
    /// `runnable_ticks - ticks` beyond its highest earlier value.
    pub runnable_ticks: Option<u64>,
    /// The part of `ticks` spent in kernel mode, in the same unit; the rest is user time.
    pub system_ticks: u64,
    /// Cumulative syscall-level I/O, or None when unreadable or not counted (macOS).
    pub syscalls: Option<SyscallCounts>,
    /// Cumulative major page faults (macOS: pages paged in), or None when unreadable.
    pub major_faults: Option<u64>,
}

/// A process the platform can see but not measure, because it belongs to another user: only
/// what any user may read of it. Nothing about its CPU, memory or threads is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shadow {
    pub pid: u32,
    pub parent: u32,
    /// Its short command name, unsanitized like every process name.
    pub name: String,
}

/// Reads one journal entry from the follower's output; `Ok(None)` at the end of the stream.
pub type JournalParser = fn(&mut dyn BufRead) -> io::Result<Option<Line>>;

/// Socket links between processes, by pid.
#[derive(Clone, Debug, Default)]
pub struct Network {
    /// Unordered pid pairs (smaller first) with the number of sockets connecting them.
    pub links: HashMap<(u32, u32), u32>,
    /// The pairs in `links` connected by at least one Unix socket. Empty on a platform that does
    /// not tell the protocols apart.
    pub unix: HashSet<(u32, u32)>,
    /// Established TCP connections whose far end is not a local socket, per pid.
    pub outside: HashMap<u32, u32>,
    /// Those outside connections in detail, from the kernel's TCP statistics.
    pub remotes: Vec<Remote>,
    /// Loopback TCP sockets between two known processes, for per-link traffic.
    pub loopback: Vec<Loopback>,
}

/// One end of an established loopback TCP connection between two different processes.
#[derive(Clone, Debug, PartialEq)]
pub struct Loopback {
    /// Identifies the socket while it is open (its inode on Linux).
    pub inode: u64,
    /// The process that owns this end and the one that owns the other end.
    pub pid: u32,
    pub peer: u32,
    /// Bytes received on this end since the connection opened. What one end receives the other
    /// end sent, so summing both ends counts every byte of the connection once.
    pub received: u64,
}

/// One established TCP connection to another machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub pid: u32,
    /// Identifies the socket while it is open (its inode on Linux).
    pub inode: u64,
    pub address: IpAddr,
    pub port: u16,
    /// Smoothed round-trip time in microseconds.
    pub rtt: u32,
    /// Bytes acknowledged by the peer and bytes received, since the connection opened.
    pub sent: u64,
    pub received: u64,
}

/// A regular file whose last link is gone while it is still open: its disk space is not returned
/// until every descriptor on it is closed. Identified by device and inode, so a file held through
/// several descriptors or by several processes counts once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeletedFile {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
}

/// The regular files one process holds open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Files {
    /// Distinct regular files: several descriptors on one file count once.
    pub open: u32,
    /// The deleted files among them, sorted. Found only among examined descriptors.
    pub deleted: Vec<DeletedFile>,
    /// Whether the table was larger than what was examined: `open` is then an estimate and
    /// `deleted` a lower bound.
    pub partial: bool,
}

impl Files {
    pub fn deleted_bytes(&self) -> u64 {
        self.deleted
            .iter()
            .fold(0, |total, file| total.saturating_add(file.size))
    }
}

/// File locks and the processes blocked on them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Locks {
    /// Locks and leases held, per pid.
    pub held: HashMap<u32, u32>,
    /// Pids blocked on a lock, with the holder of the lock that blocks them: `Blocker::Unnamed`
    /// for an OFD lock, which reports pid -1, or a holder outside this pid namespace (0).
    pub blocked: HashMap<u32, Blocker>,
    /// Open file description locks: they belong to an open file, not a process, and report pid
    /// -1, so no process can be named as their holder.
    pub unattributed: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_are_unique_and_match_previous_wording() {
        let names: Vec<&str> = Source::ALL.iter().map(|source| source.name()).collect();
        assert_eq!(
            names[..9],
            [
                "cpu pressure",
                "memory pressure",
                "io pressure",
                "last cpu",
                "cpu clock",
                "run queue",
                "cgroups",
                "socket traffic",
                "file locks",
            ],
            "the names `Snapshot::missing` held as strings"
        );
        let unique: HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        assert!(Source::ALL.len() <= 64, "a source is one bit of a u64");
    }

    #[test]
    fn sources_hold_each_source_once_and_list_them_in_declaration_order() {
        let mut sources = Sources::default();
        assert!(sources.is_empty());
        sources.insert(Source::FileLocks);
        sources.insert(Source::CpuPressure);
        sources.insert(Source::FileLocks);
        assert!(sources.contains(Source::FileLocks) && !sources.contains(Source::Cgroups));
        assert_eq!(
            sources.iter().collect::<Vec<_>>(),
            [Source::CpuPressure, Source::FileLocks]
        );
        assert_ne!(sources, Sources::from(Source::FileLocks));
        let both = Sources::from(Source::FileLocks)
            .union(Sources::from([Source::GpuMemory, Source::FileLocks]));
        assert_eq!(format!("{both:?}"), r#"{"file locks", "gpu memory"}"#);
        assert_eq!(
            Sources::of(&[Source::GpuMemory, Source::FileLocks, Source::GpuMemory]),
            both
        );
        assert!(Sources::of(&[]).is_empty());
        assert_eq!(
            Source::ALL.into_iter().collect::<Sources>().iter().count(),
            Source::ALL.len()
        );
    }
}
