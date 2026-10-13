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
//!   - `PipeScan::new() -> PipeScan` and `PipeScan::sample(&mut self) -> Option<PipeSample>`:
//!     the pipe and FIFO descriptors each process holds, read only while `Source::Pipes` is
//!     demanded; bounded, so a pass may reach only some pids (`PipeSample::partial`); None when
//!     the platform cannot read them (the collector then reports `Source::Pipes` missing);
//!   - `Gpu::load() -> Option<Gpu>` and `Gpu::sample(&self) -> HashMap<u32, GpuMemory>`: GPU
//!     memory per pid; without a `Gpu` the collector reports `Source::GpuMemory` missing.
//!   - `oom_scores(pids: &[u32]) -> Option<HashMap<u32, Option<Oom>>>`: the out-of-memory ranking of
//!     each pid, None for one that could not be read; a pid that has exited is absent. None for
//!     the whole call when the platform cannot rank at all (the collector then reports
//!     `Source::Oom` missing);
//!   - `MEMORY_DETAIL: bool` and `rollup(pid: u32) -> Taken<Rollup>`: whether this platform reads
//!     a process's memory by kind, and one such reading (the collector keeps to the
//!     `model::ROLLUP_PROCESSES` largest and to a time budget). Without it the collector reports
//!     `Source::MemoryDetail` missing.
//! - Kernel counters, each read on the background thread while its source is demanded:
//!   - `interrupts() -> Option<Vec<Interrupt>>`: every row of the interrupt table with its
//!     counts per CPU, no rates and no affinity; None when it cannot be read
//!     (`Source::Interrupts`);
//!   - `softirqs() -> Option<Vec<Softirq>>`: every softirq with its counts per CPU
//!     (`Source::Softirqs`);
//!   - `irq_affinity(id: &str) -> Option<Vec<u32>>`: the CPUs that serve the numbered interrupt
//!     `id`, None when they cannot be read (`Source::IrqAffinity`);
//!   - `oom_kills() -> Option<u64>`: the kernel's count of OOM kills since boot
//!     (`Source::OomKills`). macOS reads none of the four, and `Sampler::missing` says so.
//!   - `Threads::new() -> Threads`, a `threads::ThreadReader`, read only while `Source::Threads`
//!     is demanded: `ticks_per_second()` is the unit of its cumulative CPU counters, and
//!     `list(target, limit)` lists the threads of one process, leader first, at most `limit` of
//!     them (`Listing::Unreadable` for another user's process). The pass that walks the
//!     processes within the caps and derives per-thread CPU is `threads::ThreadScan`, shared by
//!     both platforms.
//!   - `Descriptors::new() -> Descriptors`, an `io_detail::DescriptorReader`, read only while
//!     `Source::IoDetail` is demanded: `read(id, budget)` takes one process's regular files with
//!     their access mode, position and size, counts its other descriptors by kind, and (where
//!     `counts_bytes()`) brackets the walk with two readings of the byte counters of read and
//!     write calls (`Reading::Unreadable` for another user's process). The pass that walks the
//!     processes within the caps and derives per-file I/O is `io_detail::IoScan`, shared by both
//!     platforms;
//!   - `io_rank(process: &Process) -> Option<f64>`: how busy a process is, for choosing the
//!     `io_detail::IO_PROCESSES` that get a `Process::io_detail`: its syscall rate on Linux, its
//!     disk I/O rate on macOS, which has no syscall counters. None for a process that has no
//!     reading yet.
//! - Life events, read on the `isotop-events` thread while `Source::LifeEvents` is demanded:
//!   - `monotonic_ns() -> u64`: CLOCK_MONOTONIC in nanoseconds, the clock of `Observed::at`;
//!   - `open_events(queue, identities) -> Option<Sources>` (an `OpenEvents`): opens each event
//!     source this machine allows once (Linux: the proc connector and taskstats; macOS:
//!     kqueue, which watches the pids in `identities`, the latest sample's processes) and
//!     starts the `isotop-events` thread that reads them into `queue`, returning the sources it
//!     opened. None, with no thread started, when none opens: no permission, no kernel support,
//!     or a namespace other than the initial one. It never escalates privilege. The thread runs
//!     while anything besides itself holds `queue`, and ends within 200 ms after that, closing
//!     what it opened.
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
use std::sync::{Arc, Mutex, PoisonError};

use crate::journal::Line;
use crate::model::{Blocker, Identity, IoBytes, Process, Queue, SyscallCounts};

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
    /// `Remote::location`, where remote addresses are, from the local GeoIP database.
    Locations,
    /// `Process::syscall_io`, the bytes and calls of read and write syscalls.
    SyscallIo,
    /// The system journal: `journalctl` on Linux, the unified log on macOS.
    Journal,
    /// The fork, exec and exit stream, `Snapshot::life`, which a view demands. While it is
    /// demanded, `Snapshot::sources` holds it with the event sources that deliver it, and
    /// `Snapshot::missing` the kinds of detail none of them can give (`ForkEvents`,
    /// `ExitRecords`).
    LifeEvents,
    /// Fork, exec and exit events from the Linux proc connector: exact times, parents and exit
    /// statuses.
    ProcConnector,
    /// Linux taskstats exit records over generic netlink; they need CAP_NET_ADMIN.
    Taskstats,
    /// Exec and exit events of the sampled processes from kqueue on macOS: exact times, no exit
    /// status.
    Kqueue,
    /// Births, exits and execs found by comparing two samples, which always works but misses a
    /// process that lives between them.
    SampleDiff,
    /// Forks with an exact time and both identities. Without them births come from sample
    /// diffing, at the time of the sample that saw them.
    ForkEvents,
    /// Exit records (`ExitRecord`): an exit has no command, CPU, memory or I/O totals.
    ExitRecords,
    /// `Process::oom`, the out-of-memory ranking: Linux `oom_score` and `oom_score_adj`; macOS
    /// jetsam priorities, which only root can list.
    Oom,
    /// `Process::memory_detail`, `smaps_rollup` for the largest processes: none on macOS.
    MemoryDetail,
    /// The split of the proportional set size into anonymous, file and shared-memory pages
    /// (`Rollup::pss_anon` and the two after it), which needs Linux 5.3.
    MemorySplit,
    /// `Snapshot::interrupts`, the interrupt counts and rates per CPU (`/proc/interrupts`).
    Interrupts,
    /// `Snapshot::softirqs`, the softirq counts and rates per CPU (`/proc/softirqs`).
    Softirqs,
    /// `Interrupt::affinity`, the CPUs that serve each numbered interrupt. It fills the rows of
    /// `Interrupts`, so a view that draws it declares both.
    IrqAffinity,
    /// `Snapshot::oom_kills`, the kernel's count of OOM kills (`oom_kill` in `/proc/vmstat`).
    OomKills,
    /// `Process::thread_list`, each process's threads with their names, CPU and states.
    Threads,
    /// `Snapshot::pipes`: the pipes and FIFOs that connect processes, with direction. A view
    /// demands it; the unread bytes of Unix sockets (`Snapshot::unix_queues`) come with the
    /// socket scan and are not behind it.
    Pipes,
    /// `Process::io_detail`: which open files the busiest processes read and write (`io_detail`
    /// module), from the positions of their descriptors.
    IoDetail,
    /// `IoDetail::unattributed`, the part of a process's read and write calls that no file
    /// position explains. It needs the byte counters of read and write calls (`rchar` and
    /// `wchar`), which macOS lacks, and a demanded `Source::IoDetail` is then read without them:
    /// nothing bounds a file's claim and nothing says what remains.
    UnattributedIo,
}

impl Source {
    /// Every source, in declaration order.
    pub const ALL: &[Source] = &[
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
        Source::LifeEvents,
        Source::ProcConnector,
        Source::Taskstats,
        Source::Kqueue,
        Source::SampleDiff,
        Source::ForkEvents,
        Source::ExitRecords,
        Source::Oom,
        Source::MemoryDetail,
        Source::MemorySplit,
        Source::Interrupts,
        Source::Softirqs,
        Source::IrqAffinity,
        Source::OomKills,
        Source::Threads,
        Source::Pipes,
        Source::IoDetail,
        Source::UnattributedIo,
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
            Source::LifeEvents => "life events",
            Source::ProcConnector => "proc connector",
            Source::Taskstats => "taskstats",
            Source::Kqueue => "kqueue",
            Source::SampleDiff => "sample diff",
            Source::ForkEvents => "fork events",
            Source::ExitRecords => "exit records",
            Source::Oom => "oom score",
            Source::MemoryDetail => "memory detail",
            Source::MemorySplit => "memory split",
            Source::Interrupts => "interrupts",
            Source::Softirqs => "softirqs",
            Source::IrqAffinity => "irq affinity",
            Source::OomKills => "oom kills",
            Source::Threads => "threads",
            Source::Pipes => "pipes",
            Source::IoDetail => "io detail",
            Source::UnattributedIo => "unattributed io",
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
            .iter()
            .copied()
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

/// How reading one process's detail came out.
#[cfg_attr(
    all(target_os = "macos", not(test)),
    expect(dead_code, reason = "only Linux reads a process's memory by kind")
)]
pub enum Taken<T> {
    Value(T),
    /// It exists but this user may not read it, or the kernel lacks the file.
    Unreadable,
    /// It exited before or while it was read, so there is nothing to keep, not even a gap.
    Gone,
}

/// One process as the OS reports it, before isotop smooths anything.
pub struct RawProcess {
    /// Every `Process` field the OS gives directly: id, parent, name, command, group, kind,
    /// state, memory, threads, core, cgroup, priority, nice, written, policy and started. The
    /// derived fields (cpu, the I/O rates, system_share, syscall_io, major_faults, cpu_time and
    /// gpu_memory) are left at zero or None, and those the
    /// background thread fills (files, locks_held, blocked_on, oom, memory_detail, thread_list
    /// and io_detail) at Pending or None, except that a Linux kernel thread's files are known to
    /// be none.
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
    /// Changes when the process calls exec, within one `Identity`: macOS's pid version
    /// (`p_idversion`). None where the platform has no such number (Linux).
    pub generation: Option<i64>,
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

/// One fork, exec or exit, as `Snapshot::life` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct LifeEvent {
    /// When it happened, in seconds of `Snapshot::elapsed`: the kernel's timestamp for the proc
    /// connector, when it was received for taskstats and kqueue, and the sample that saw the
    /// change for sample diffing.
    pub at: f64,
    pub kind: LifeKind,
    pub source: LifeSource,
    /// An identity in it is a guess: a forked child gone before its start could be read has the
    /// start its fork's timestamp gives, and a process nothing names has start 0. An estimated
    /// identity never matches a sampled process.
    pub estimated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LifeKind {
    /// A new process; a new thread is not one.
    Fork {
        parent: Identity,
        child: Identity,
    },
    Exec {
        id: Identity,
    },
    /// A process ended. `record` comes from taskstats only.
    Exit {
        id: Identity,
        cause: Cause,
        record: Option<ExitRecord>,
    },
}

/// Where an event came from. For an exit the merge prefers them in this order, the most detailed
/// first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifeSource {
    Taskstats,
    Connector,
    Kqueue,
    Diff,
}

impl LifeSource {
    /// Every mechanism, most detailed first: with `source`, what picks the delivering ones out of
    /// `Snapshot::sources`, which also holds the demanded stream (`Source::LifeEvents`) and other
    /// views' sources.
    pub const ALL: [LifeSource; 4] = [
        LifeSource::Taskstats,
        LifeSource::Connector,
        LifeSource::Kqueue,
        LifeSource::Diff,
    ];

    /// The `Source` that `Snapshot::sources` lists while this one delivers.
    pub fn source(self) -> Source {
        match self {
            LifeSource::Taskstats => Source::Taskstats,
            LifeSource::Connector => Source::ProcConnector,
            LifeSource::Kqueue => Source::Kqueue,
            LifeSource::Diff => Source::SampleDiff,
        }
    }

    /// "taskstats", "proc connector", "kqueue" or "sample diff".
    pub fn name(self) -> &'static str {
        self.source().name()
    }
}

/// Why a process ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// It exited with this status.
    Code(i32),
    /// A signal killed it, and it dumped core or not.
    Signal { number: i32, core: bool },
    /// Sample diffing and kqueue see that a process is gone, not why.
    Unknown,
}

impl Cause {
    /// The cause a wait status gives (`exit_code` of the proc connector, `ac_exitcode` of
    /// taskstats): the low 7 bits are the signal that killed it, 0 for a normal exit, bit 7 says
    /// it dumped core, and the next byte is the exit status.
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "only Linux sources report a wait status")
    )]
    pub fn of_status(status: u32) -> Cause {
        match status & 0x7f {
            0 => Cause::Code(((status >> 8) & 0xff) as i32),
            number => Cause::Signal {
                number: number as i32,
                core: status & 0x80 != 0,
            },
        }
    }
}

impl Cause {
    /// The name of signal `number` in Linux's numbering (signal(7)), which every wait status
    /// isotop reads uses: macOS reports no exit status. None for a real-time or unknown signal.
    pub fn signal_name(number: i32) -> Option<&'static str> {
        const NAMES: [&str; 31] = [
            "SIGHUP",
            "SIGINT",
            "SIGQUIT",
            "SIGILL",
            "SIGTRAP",
            "SIGABRT",
            "SIGBUS",
            "SIGFPE",
            "SIGKILL",
            "SIGUSR1",
            "SIGSEGV",
            "SIGUSR2",
            "SIGPIPE",
            "SIGALRM",
            "SIGTERM",
            "SIGSTKFLT",
            "SIGCHLD",
            "SIGCONT",
            "SIGSTOP",
            "SIGTSTP",
            "SIGTTIN",
            "SIGTTOU",
            "SIGURG",
            "SIGXCPU",
            "SIGXFSZ",
            "SIGVTALRM",
            "SIGPROF",
            "SIGWINCH",
            "SIGIO",
            "SIGPWR",
            "SIGSYS",
        ];
        usize::try_from(number)
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| NAMES.get(index).copied())
    }
}

/// What taskstats reports of a process that ended, summed over its threads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExitRecord {
    /// Its command name (`ac_comm`), unsanitized like every process name.
    pub comm: String,
    /// When it started, in seconds since the Unix epoch.
    pub btime: u64,
    /// How long it ran, and the user plus system CPU time of all its threads, in microseconds.
    pub etime: u64,
    pub cpu: u64,
    /// The most resident memory it held, in bytes.
    pub hiwater_rss: u64,
    /// Bytes it read from and wrote to storage, over all its threads.
    pub read_bytes: u64,
    pub write_bytes: u64,
    /// Voluntary and involuntary context switches of all its threads.
    pub nvcsw: u64,
    pub nivcsw: u64,
    /// It forked and never called exec (AFORK).
    pub forked_never_execd: bool,
    /// How many thread records were summed into it.
    pub threads: u32,
}

/// One thing an event source saw, before the collector names the processes in it and merges it
/// into `Snapshot::life`.
#[derive(Clone, Debug, PartialEq)]
pub struct Observed {
    /// When, in nanoseconds of `monotonic_ns()`: the kernel's timestamp where the source gives
    /// one (the proc connector), the time it was received otherwise.
    pub at: u64,
    pub source: LifeSource,
    pub what: Observation,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Observation {
    /// A new process, `parent` the process (tgid) that forked it. Each start is None when it
    /// could not be read; `estimated_start` is the child's start as the event's timestamp gives
    /// it, which stands in for an unread one.
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "kqueue names no forked child")
    )]
    Fork {
        parent: u32,
        child: u32,
        parent_start: Option<u64>,
        child_start: Option<u64>,
        estimated_start: u64,
    },
    /// `start` when the source knows which process it was, None for the collector to name the
    /// pid.
    Exec { pid: u32, start: Option<u64> },
    Exit {
        pid: u32,
        start: Option<u64>,
        cause: Cause,
    },
    /// A taskstats record of the process `tgid`, its threads' records summed. `start` is when it
    /// began in the unit of a Linux `Identity::start` (ticks since boot), None when the boot time
    /// is unknown.
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "taskstats is Linux only")
    )]
    Record {
        tgid: u32,
        record: ExitRecord,
        cause: Cause,
        start: Option<u64>,
    },
}

/// The bounded queue from the event thread to `Collector::sample`, which drains it every sample.
#[derive(Default)]
pub struct LifeQueue(Mutex<Queued>);

#[derive(Default)]
struct Queued {
    observed: Vec<Observed>,
    dropped: u32,
}

impl LifeQueue {
    /// The most observations it holds; what arrives while it is full is counted and dropped.
    pub const CAPACITY: usize = 4096;

    pub fn push(&self, observed: Observed) {
        let mut queued = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if queued.observed.len() < Self::CAPACITY {
            queued.observed.push(observed);
        } else {
            queued.dropped = queued.dropped.saturating_add(1);
        }
    }

    /// Counts one observation lost before it reached the queue: the kernel had more for the
    /// socket than its buffer held.
    #[cfg(target_os = "linux")]
    pub fn overrun(&self) {
        let mut queued = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        queued.dropped = queued.dropped.saturating_add(1);
    }

    /// Everything queued, in the order it arrived, and how many were dropped since the last
    /// drain.
    pub fn drain(&self) -> (Vec<Observed>, u32) {
        let mut queued = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let dropped = std::mem::take(&mut queued.dropped);
        (std::mem::take(&mut queued.observed), dropped)
    }
}

/// `open_events`, or what a test puts in its place.
pub type OpenEvents = fn(Arc<LifeQueue>, Arc<Mutex<HashMap<u32, Identity>>>) -> Option<Sources>;

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
    /// What waits unread on the connected Unix sockets between two different processes, by
    /// (reader pid, writer pid), summed over the sockets of the pair: the reader owns the socket
    /// whose receive queue it is, the writer owns its peer. A pair is present once its queue was
    /// read, empty or not.
    pub unix_queues: HashMap<(u32, u32), Queue>,
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

/// How a process may use a descriptor, from its open flags: one end of a pipe or FIFO, or an open
/// file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessMode {
    Read,
    Write,
    /// A FIFO opened for reading and writing.
    Both,
}

impl AccessMode {
    pub fn reads(self) -> bool {
        self != AccessMode::Write
    }

    pub fn writes(self) -> bool {
        self != AccessMode::Read
    }

    /// What a process may do with a pipe through two descriptors of it.
    pub fn union(self, other: AccessMode) -> AccessMode {
        if self == other {
            self
        } else {
            AccessMode::Both
        }
    }
}

/// What the descriptors of one pipe or FIFO have in common, whichever process holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PipeId {
    /// An anonymous pipe: its inode on Linux, the lower of its handle and its peer's on macOS.
    Anonymous(u64),
    /// A named pipe, by the device and inode of its file.
    Fifo { device: u64, inode: u64 },
}

/// One descriptor of a pipe or FIFO that a process holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeEnd {
    pub id: PipeId,
    pub access: AccessMode,
    /// Bytes waiting in the pipe, where the platform tells (macOS, for anonymous pipes); None
    /// where it does not (Linux exposes no count for a pipe another process holds, and no
    /// platform counts a FIFO's).
    pub buffered: Option<u64>,
}

/// What one `PipeScan::sample` pass read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PipeSample {
    /// The pipe and FIFO descriptors of each process the pass reached, an empty list for a
    /// process that holds none. A pid the pass did not reach, or could not read, is absent.
    pub ends: HashMap<u32, Vec<PipeEnd>>,
    /// Whether the pass stopped at its cap before reaching every pid, or read only part of one
    /// process's descriptors. The next pass resumes after the last pid it reached.
    pub partial: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_all_lists_every_source_in_declaration_order() {
        // `Sources` is a bit set indexed by the discriminant, and `iter` walks `ALL`, so the list
        // must be the enum in order. The last variant is the one a new source is appended after.
        for (index, source) in Source::ALL.iter().enumerate() {
            assert_eq!(*source as usize, index, "{} is out of order", source.name());
        }
        assert_eq!(Source::UnattributedIo as usize + 1, Source::ALL.len());
    }

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
    fn signals_are_named_in_linux_numbering() {
        assert_eq!(Cause::signal_name(9), Some("SIGKILL"));
        assert_eq!(Cause::signal_name(15), Some("SIGTERM"));
        assert_eq!(Cause::signal_name(11), Some("SIGSEGV"));
        assert_eq!(Cause::signal_name(31), Some("SIGSYS"));
        assert_eq!(Cause::signal_name(0), None);
        assert_eq!(
            Cause::signal_name(34),
            None,
            "real-time signals have no name"
        );
        assert_eq!(Cause::signal_name(-1), None);
    }

    #[test]
    fn life_sources_are_named_as_the_sources_they_report() {
        let named = [
            LifeSource::Connector,
            LifeSource::Taskstats,
            LifeSource::Kqueue,
            LifeSource::Diff,
        ]
        .map(LifeSource::name);
        assert_eq!(
            named,
            ["proc connector", "taskstats", "kqueue", "sample diff"]
        );
        let reported: Sources = LifeSource::ALL
            .map(LifeSource::source)
            .into_iter()
            .collect();
        assert_eq!(
            reported.iter().collect::<Vec<_>>(),
            [
                Source::ProcConnector,
                Source::Taskstats,
                Source::Kqueue,
                Source::SampleDiff
            ],
            "every mechanism is listed once, and none is the demanded stream"
        );
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
            Source::ALL
                .iter()
                .copied()
                .collect::<Sources>()
                .iter()
                .count(),
            Source::ALL.len()
        );
    }
}
