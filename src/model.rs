use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::geo;
use crate::places::COUNTRIES;
use crate::platform::{self, RawProcess, Source, Sources};
use crate::simulation::{Rng, hash_identity, smoothing};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identity {
    pub pid: u32,
    pub start: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    Kernel,
    #[default]
    System,
    Session,
    Container,
}

#[derive(Clone, Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "policy and started are read by the views that come after P5 (#21)"
    )
)]
pub struct Process {
    pub id: Identity,
    pub parent: u32,
    pub name: String,
    pub command: String,
    pub group: String,
    pub kind: Kind,
    pub state: char,
    pub cpu: f32,
    pub memory: u64,
    /// Bytes per second read from and written to storage, together; None until a second
    /// reading exists or when /proc/<pid>/io is unreadable.
    pub io_rate: Option<f32>,
    /// The parts of `io_rate`, in bytes per second, with the same None rules.
    pub read_rate: Option<f32>,
    pub write_rate: Option<f32>,
    /// Bytes written to storage since the process started (`write_bytes`); None when unreadable.
    pub written: Option<u64>,
    /// Scheduler priority, /proc/<pid>/stat field 18. Normal tasks have 20 + nice (0 to 39);
    /// real-time tasks have -1 - rt_priority (negative). Lower runs first.
    pub priority: i32,
    /// Nice value, -20 (favoured) to 19 (background); /proc/<pid>/stat field 19.
    pub nice: i32,
    pub threads: u32,
    /// NVIDIA GPU memory; None when the process is not using a GPU, or no GPU source has
    /// reported it yet.
    pub gpu_memory: Option<GpuMemory>,
    /// The CPU it last ran on; None where the platform does not report it (macOS) or the
    /// field could not be read.
    pub core: Option<u32>,
    /// Its total CPU time in seconds.
    pub cpu_time: f32,
    /// Full cgroup v2 path, such as /system.slice/cron.service.
    pub cgroup: String,
    /// Share of its CPU time since the previous sample that ran on performance cores, 0 to 1.
    /// None where the platform does not split CPU time by core kind (Linux), on the first
    /// sample, and when the process used no CPU time in the interval.
    pub performance_share: Option<f32>,
    /// Average number of its threads that were runnable but waiting for a CPU since the
    /// previous sample, at most its thread count. None where the platform does not measure it
    /// (Linux), on the first sample, and when its counters went back (exec on macOS).
    pub waiting: Option<f32>,
    /// Regular files held open (memfds excluded), with those deleted while still open. Other
    /// users' descriptor tables are unreadable without privileges.
    pub files: Measured<platform::Files>,
    /// File locks and leases held. OFD locks name no process and are not counted.
    pub locks_held: Measured<u32>,
    /// The holder of the file lock this process is waiting for; None when it waits for none.
    pub blocked_on: Option<Blocker>,
    /// Share of its CPU time since the previous sample that ran in kernel mode, 0 to 1; the
    /// rest ran in user mode. None on the first sample and when no CPU time elapsed.
    pub system_share: Option<f32>,
    /// The scheduling policy of its main thread; `Policy::Unknown` where the platform does not
    /// say. On macOS only timesharing, round-robin and FIFO exist.
    pub policy: Policy,
    /// Syscall-level I/O per second, counting bytes that never reached storage (page cache,
    /// pipes, sockets) too. None until a second reading exists, after an exec or a counter that
    /// went backwards, when /proc/<pid>/io is unreadable, and always on macOS
    /// (`Source::SyscallIo`).
    pub syscall_io: Option<SyscallIo>,
    /// Major page faults per second: faults that had to read from storage. macOS counts the
    /// pages paged in instead (`pti_pageins`), which includes pages brought in ahead of faults.
    /// None until a second reading exists, and after an exec or a counter that went backwards.
    pub major_faults: Option<f32>,
    /// When it started, in seconds since the Unix epoch; 0 when the platform cannot tell (Linux
    /// without `btime` in /proc/stat).
    pub started: f64,
}

/// The scheduling class of a process's main thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    target_os = "macos",
    expect(dead_code, reason = "macOS has no deadline policy to report")
)]
pub enum Policy {
    /// The default time-sharing policy (`SCHED_OTHER`; macOS timeshare).
    Normal,
    /// For batch work that should not preempt interactive tasks (`SCHED_BATCH`).
    Batch,
    /// Runs only when nothing else wants the CPU (`SCHED_IDLE`).
    Idle,
    /// Real time, first in first out (`SCHED_FIFO`).
    Fifo,
    /// Real time, round robin (`SCHED_RR`).
    RoundRobin,
    /// Real time, earliest deadline first (`SCHED_DEADLINE`).
    Deadline,
    /// Not read, or a policy this table does not know.
    #[default]
    Unknown,
}

/// Syscall-level I/O rates of a process: bytes per second passed through `read` and `write`
/// style calls, and the calls per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SyscallIo {
    pub read_chars: f32,
    pub write_chars: f32,
    pub reads: f32,
    pub writes: f32,
}

impl Default for Process {
    /// A harmless placeholder: pid 0 (never a real process), a sleeping system process, and
    /// nothing measured yet.
    fn default() -> Self {
        Process {
            id: Identity::default(),
            parent: 0,
            name: String::new(),
            command: String::new(),
            group: String::new(),
            kind: Kind::default(),
            state: 'S',
            cpu: 0.0,
            memory: 0,
            io_rate: None,
            read_rate: None,
            write_rate: None,
            written: None,
            priority: 0,
            nice: 0,
            threads: 0,
            gpu_memory: None,
            core: None,
            cpu_time: 0.0,
            cgroup: String::new(),
            performance_share: None,
            waiting: None,
            files: Measured::Pending,
            locks_held: Measured::Pending,
            blocked_on: None,
            system_share: None,
            policy: Policy::Unknown,
            syscall_io: None,
            major_faults: None,
            started: 0.0,
        }
    }
}

impl Process {
    /// A synthetic row that stands in for a process isotop cannot read, such as a hub: nothing
    /// is measured, and nothing can be, so the background fields are unreadable rather than pending.
    pub fn placeholder(id: Identity) -> Process {
        Process {
            id,
            files: Measured::Unreadable,
            locks_held: Measured::Unreadable,
            ..Process::default()
        }
    }
}

/// A process's GPU memory, as far as the driver says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuMemory {
    Bytes(u64),
    /// The process uses the GPU but the driver does not report how much.
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "only the NVML reader on Linux produces it")
    )]
    Unreported,
}

impl GpuMemory {
    /// The bytes reported; 0 for `Unreported`, which a view that sizes by memory draws as the
    /// smallest in use.
    pub fn bytes(self) -> u64 {
        match self {
            GpuMemory::Bytes(bytes) => bytes,
            GpuMemory::Unreported => 0,
        }
    }

    /// Adds one driver reading (`None` for a process whose memory the driver does not report) to
    /// what is known of `pid`. A reported figure outweighs an unreported one, which adds nothing.
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "only the NVML reader on Linux calls it")
    )]
    pub fn accumulate(usage: &mut HashMap<u32, GpuMemory>, pid: u32, reported: Option<u64>) {
        let entry = usage.entry(pid).or_insert(GpuMemory::Unreported);
        *entry = match (*entry, reported) {
            (GpuMemory::Bytes(total), Some(bytes)) => GpuMemory::Bytes(total.saturating_add(bytes)),
            (GpuMemory::Unreported, Some(bytes)) => GpuMemory::Bytes(bytes),
            (known, None) => known,
        };
    }
}

/// The holder of a file lock that blocks a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    Process(u32),
    /// No process can be named: an OFD lock, which belongs to an open file, or a holder in
    /// another pid namespace.
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "only the Linux lock table produces it")
    )]
    Unnamed,
}

/// A measurement taken on the background thread, which may not have reached a process yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Measured<T> {
    /// Not measured yet: before the first background pass, a process born since the last one,
    /// or one a budget-limited scan has not reached.
    #[default]
    Pending,
    /// Could not be read, normally for lack of permission.
    Unreadable,
    Known(T),
}

impl<T> Measured<T> {
    pub fn known(&self) -> Option<&T> {
        match self {
            Measured::Known(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub latitude: f32,
    pub longitude: f32,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoreKind {
    Performance,
    Efficiency,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Cpu {
    pub id: u32,
    pub kind: CoreKind,
    /// Share of the last interval spent busy, 0 to 1; None on the first reading and when no time
    /// passed on the CPU's counters.
    pub busy: Option<f32>,
    /// The clock in MHz; None when the platform has no reading for it (see `Source::CpuClock`).
    pub mhz: Option<f32>,
    /// Average number of tasks that were runnable but waiting for this CPU; None on the first
    /// reading and where the platform does not measure it (see `Source::RunQueue`).
    pub wait: Option<f32>,
}

/// Resource accounting of one cgroup.
#[derive(Clone, Debug, Default)]
pub struct Unit {
    pub memory: u64,
    pub memory_max: Option<u64>,
    /// CPU use in percent of one core, and its quota; the use is None on the first reading,
    /// when its counter is unreadable, and when it went backwards.
    pub cpu: Option<f32>,
    pub cpu_max: Option<f32>,
    /// Whether the CPU quota throttled it since the previous reading; None on the first reading
    /// and when the counter is unreadable.
    pub throttled: Option<bool>,
    /// Pressure stall percentages (10 s average) for CPU, memory and I/O; None for a file that
    /// could not be read.
    pub pressure: [Option<f32>; 3],
    /// OOM kills since the cgroup was created; None when `memory.events` is unreadable.
    pub oom_kills: Option<u64>,
    /// Processes in the cgroup; None when `pids.current` is unreadable.
    pub pids: Option<u64>,
    pub pids_max: Option<u64>,
}

/// An established TCP connection to another machine.
#[derive(Clone, Debug)]
pub struct Remote {
    pub id: Identity,
    pub address: IpAddr,
    pub port: u16,
    /// Smoothed round-trip time in milliseconds.
    pub rtt: f32,
    /// Bytes per second sent and received since the previous scan; None until the connection has
    /// been seen twice, and when its counters went backwards.
    pub up: Option<f32>,
    pub down: Option<f32>,
    pub place: Option<Place>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub processes: Vec<Process>,
    /// None when the platform could not read them.
    pub memory_total: Option<u64>,
    pub memory_available: Option<u64>,
    pub cores: usize,
    pub elapsed: f64,
    /// Pressure stall percentages (10 s average) for CPU, memory and I/O.
    pub pressure: [f32; 3],
    pub links: Vec<(Identity, Identity, u32)>,
    /// Bytes per second over loopback TCP between the two processes of a pair, both directions
    /// together. Keys are ordered as in `links`; pairs with a measured connection but no traffic
    /// since the previous reading are present with 0, and pairs not yet measured are absent.
    pub link_traffic: HashMap<(Identity, Identity), f32>,
    /// The pairs in `links` connected by at least one Unix socket, keyed as in `links`. A pair
    /// can have Unix and TCP sockets both. Empty when the platform does not tell the protocols
    /// apart.
    pub unix_links: HashSet<(Identity, Identity)>,
    /// TCP connections leaving the machine, per process.
    pub outside: HashMap<Identity, u32>,
    pub cpus: Vec<Cpu>,
    /// Cgroups of the sampled processes, by path.
    pub units: HashMap<String, Unit>,
    pub remotes: Vec<Remote>,
    pub home: Option<Place>,
    /// Where remote locations come from, for the status line.
    pub geo: String,
    /// File locks no process can be named for: OFD locks, which belong to an open file and report
    /// pid -1.
    pub unattributed_locks: u32,
    /// System-wide sources that could not be read this sample or that the platform never has:
    /// the pressures `Sampler::pressure` could not read, the platform's gaps
    /// (`Sampler::missing`), and the lock table and GPU memory once the background thread has
    /// found them absent.
    pub missing: Sources,
    /// Processes the platform could see but not measure because they belong to another user.
    pub unreadable: usize,
    /// Sources the platform measures per CPU cluster rather than per CPU (`Sampler::per_cluster`):
    /// `Source::CpuClock` when every CPU's `mhz` is the average clock of the cluster of its kind.
    pub per_cluster: Sources,
    /// The processes counted in `unreadable`, as far as anyone may read them (`Sampler::shadows`),
    /// so that what they started can still be drawn under them.
    pub shadows: Vec<platform::Shadow>,
    /// How the background thread that measures the slow sources is doing.
    pub sampler: SamplerHealth,
}

/// How often the background thread runs its scans: sockets, GPU memory, cgroups, open files, file
/// locks and remote locations. What it measured reaches a sample up to this long plus one cycle
/// after it was read.
pub const BACKGROUND_PERIOD: Duration = Duration::from_secs(2);

/// Periods without a finished cycle after which a running background thread counts as stalled:
/// one of its scans is stuck, normally on a read that does not return.
const STALLED_PERIODS: u32 = 10;

/// The state of the background thread, as of one sample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SamplerHealth {
    /// The thread is alive. False when it could not be spawned, panicked, or ended because its
    /// shared state was poisoned; the slow sources then keep what it measured last.
    pub running: bool,
    /// How long ago its last cycle finished; None before the first.
    pub last_cycle: Option<Duration>,
    /// How long it has gone without finishing a cycle: since the last one finished, or since the
    /// thread started while the first is still running. `stalled` judges this, so a first cycle
    /// that never finishes counts too.
    pub quiet: Duration,
    /// How long each scan of that cycle took, by the source it reads, in the order they ran.
    pub scan_latency: Vec<(Source, Duration)>,
}

impl SamplerHealth {
    /// Alive, but no cycle has finished for `STALLED_PERIODS` periods.
    pub fn stalled(&self) -> bool {
        self.running && self.quiet > BACKGROUND_PERIOD * STALLED_PERIODS
    }
}

/// What the collector keeps of a process between samples.
struct Counters {
    ticks: u64,
    io: Option<IoBytes>,
    cpu: f32,
    performance_ticks: Option<u64>,
    /// The highest runnable-but-not-running total seen so far, in ticks: runnable ticks minus
    /// CPU ticks, which dips while threads run because the kernel may update the runnable total
    /// only when a thread is switched onto a CPU or blocks (see `RawProcess::runnable_ticks`).
    waited: Option<i128>,
    runnable_ticks: Option<u64>,
    threads: u32,
    system_ticks: u64,
    syscalls: Option<SyscallCounts>,
    major_faults: Option<u64>,
}

/// Cumulative bytes a process has read from and written to storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoBytes {
    pub read: u64,
    pub write: u64,
}

impl IoBytes {
    fn total(self) -> u64 {
        self.read.saturating_add(self.write)
    }

    /// Bytes per second of read and write combined, read, and write over `dt` seconds. None when
    /// a counter went backwards (a recycled pid starts again from zero), since the change is
    /// then unknown.
    fn rates(self, before: Self, dt: f32) -> Option<[f32; 3]> {
        Some(
            [
                self.total().checked_sub(before.total())?,
                self.read.checked_sub(before.read)?,
                self.write.checked_sub(before.write)?,
            ]
            .map(|bytes| bytes as f32 / dt),
        )
    }
}

/// Cumulative syscall-level I/O of a process: bytes and call counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyscallCounts {
    pub read_chars: u64,
    pub write_chars: u64,
    pub reads: u64,
    pub writes: u64,
}

impl SyscallCounts {
    /// Per-second rates over `dt` seconds; None when a counter went backwards.
    fn rates(self, before: Self, dt: f32) -> Option<SyscallIo> {
        let rate = |now: u64, then: u64| Some(now.checked_sub(then)? as f32 / dt);
        Some(SyscallIo {
            read_chars: rate(self.read_chars, before.read_chars)?,
            write_chars: rate(self.write_chars, before.write_chars)?,
            reads: rate(self.reads, before.reads)?,
            writes: rate(self.writes, before.writes)?,
        })
    }
}

/// Slow measurements gathered off the frame loop, keyed by the pids of `identities`.
#[derive(Default)]
struct Extras {
    /// The processes the frame loop had last sampled when this cycle began, by pid. Every scan
    /// read after that, so a pid in the scans is the process named here or one that took its pid
    /// since; `merge` keeps only the pids that still name the same process.
    identities: HashMap<u32, Identity>,
    network: platform::Network,
    gpu: HashMap<u32, GpuMemory>,
    units: HashMap<String, Unit>,
    /// Remote connections with the owning pid in `id.pid`.
    remotes: Vec<Remote>,
    /// Loopback traffic in bytes per second between pid pairs (smaller pid first).
    traffic: HashMap<(u32, u32), f32>,
    /// Open files per pid, None for an unreadable descriptor table; absent until scanned.
    files: HashMap<u32, Option<platform::Files>>,
    locks: Measured<platform::Locks>,
    /// The slow sources found absent: GPU memory without a `Gpu`, file locks without a table.
    missing: Sources,
    geo: String,
    /// How long each scan took, by the source it reads.
    latency: Vec<(Source, Duration)>,
    /// When the cycle that measured all this finished; None before the first.
    finished: Option<Instant>,
}

/// The background thread, as the frame loop sees it: what it measured last and whether it is
/// still alive.
struct Background {
    extras: Arc<Mutex<Extras>>,
    alive: Arc<AtomicBool>,
    started: Instant,
}

/// Clears the background thread's alive flag when the thread ends, by returning or by a panic.
struct Alive(Arc<AtomicBool>);

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

impl Background {
    /// Starts the background thread. It calls `start` once to set up its scans, then runs the
    /// cycle that returns every `BACKGROUND_PERIOD` and publishes what it measured, until the
    /// `Background` is dropped. None when no thread can be spawned.
    fn spawn<Cycle: FnMut() -> Extras>(
        start: impl FnOnce() -> Cycle + Send + 'static,
    ) -> Option<Background> {
        let extras = Arc::new(Mutex::new(Extras::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let writer = Arc::clone(&extras);
        let guard = Alive(Arc::clone(&alive));
        let started = Instant::now();
        std::thread::Builder::new()
            .name("isotop-sampler".into())
            .spawn(move || {
                let _guard = guard;
                let mut cycle = start();
                while Arc::strong_count(&writer) > 1 {
                    let mut measured = cycle();
                    measured.finished = Some(Instant::now());
                    match writer.lock() {
                        Ok(mut slot) => *slot = measured,
                        Err(_) => return,
                    }
                    std::thread::sleep(BACKGROUND_PERIOD);
                }
            })
            .ok()?;
        Some(Background {
            extras,
            alive,
            started,
        })
    }

    /// What it measured last. A poisoned lock still holds the last complete cycle: `slot` is
    /// replaced whole, never written in parts.
    fn extras(&self) -> MutexGuard<'_, Extras> {
        self.extras.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The thread's health at `now`. A poisoned lock ends the thread at its next publish, so it
    /// counts as stopped already.
    fn health(&self, now: Instant) -> SamplerHealth {
        let extras = self.extras();
        SamplerHealth {
            running: self.alive.load(Ordering::Relaxed) && !self.extras.is_poisoned(),
            last_cycle: extras
                .finished
                .map(|finished| now.saturating_duration_since(finished)),
            quiet: now.saturating_duration_since(extras.finished.unwrap_or(self.started)),
            scan_latency: extras.latency.clone(),
        }
    }
}

/// Runs one scan and records how long it took under the `source` it reads.
fn timed<T>(latency: &mut Vec<(Source, Duration)>, source: Source, scan: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = scan();
    latency.push((source, start.elapsed()));
    value
}

pub struct Collector {
    sampler: platform::Sampler,
    previous: HashMap<Identity, Counters>,
    last: Instant,
    origin: Instant,
    background: Option<Background>,
    /// Cgroup paths the background thread should account, and whether to locate remote
    /// addresses (only the globe shows them, and the first lookup loads the GeoIP database).
    wanted: Arc<Mutex<HashSet<String>>>,
    locating: Arc<AtomicBool>,
    /// The processes of the latest sample by pid, which the background scans are joined against.
    identities: Arc<Mutex<HashMap<u32, Identity>>>,
    geoip: Option<PathBuf>,
    home: Option<Place>,
    started: bool,
}

/// Samples sockets, GPU usage, cgroups, open files, file locks and remote locations every
/// `BACKGROUND_PERIOD` on a background thread; the thread stops once the collector is gone.
/// Returns None when no thread can be spawned.
fn background(
    geoip: Option<PathBuf>,
    wanted: Arc<Mutex<HashSet<String>>>,
    locating: Arc<AtomicBool>,
    identities: Arc<Mutex<HashMap<u32, Identity>>>,
) -> Option<Background> {
    Background::spawn(move || {
        let gpu = platform::Gpu::load();
        let mut geo = geo::Geo::open(geoip.as_deref());
        let mut units = HashMap::new();
        let mut sockets = HashMap::new();
        let mut loopback = LoopbackScan::default();
        let mut retained = HashSet::new();
        let mut files = platform::FileScan::new();
        let mut kept = KeptFiles::default();
        move || {
            // Taken before any scan reads, so that every pid the scans find is either the
            // process named here or one that took its pid since.
            let identities = identities.lock().map(|map| map.clone()).unwrap_or_default();
            let wanted_now = wanted.lock().map(|set| set.clone()).unwrap_or_default();
            let paths = accounted(wanted_now, &mut retained);
            let mut latency = Vec::new();
            let network = timed(&mut latency, Source::Sockets, platform::network);
            let locator = locating.load(Ordering::Relaxed).then_some(&mut geo);
            let remotes = timed(&mut latency, Source::Locations, || {
                remotes(&network.remotes, &mut sockets, locator)
            });
            let traffic = traffic(&network.loopback, &mut loopback, Instant::now());
            let gpu_memory = timed(&mut latency, Source::GpuMemory, || {
                gpu.as_ref().map(platform::Gpu::sample).unwrap_or_default()
            });
            let units = timed(&mut latency, Source::Cgroups, || {
                platform::account(&paths, &mut units)
            });
            let files = timed(&mut latency, Source::OpenFiles, || files.sample());
            let files = kept.update(files, &identities);
            let locks = timed(&mut latency, Source::FileLocks, || {
                platform::locks().map_or(Measured::Unreadable, Measured::Known)
            });
            let missing = background_gaps(gpu.is_some(), &locks);
            Extras {
                identities,
                network,
                gpu: gpu_memory,
                units,
                remotes,
                traffic,
                files,
                locks,
                missing,
                geo: geo.source.clone(),
                latency,
                finished: None,
            }
        }
    })
}

/// Open-file tables kept between background cycles, each with the process it was read for. The
/// file scan is bounded and reaches every pid only over several cycles, so a table read earlier
/// stands in until the scan comes round again, but only while its pid still names that process:
/// a reused pid starts out unscanned instead of showing its predecessor's files.
#[derive(Default)]
struct KeptFiles(HashMap<u32, (Identity, Option<platform::Files>)>);

impl KeptFiles {
    /// Takes the tables this cycle's scan read, against `identities` (the processes by pid when
    /// the cycle began), and returns every table that still belongs to the process its pid names
    /// there.
    fn update(
        &mut self,
        scanned: HashMap<u32, Option<platform::Files>>,
        identities: &HashMap<u32, Identity>,
    ) -> HashMap<u32, Option<platform::Files>> {
        self.0
            .retain(|pid, (id, _)| identities.get(pid) == Some(id));
        for (pid, files) in scanned {
            match identities.get(&pid) {
                Some(&id) => {
                    self.0.insert(pid, (id, files));
                }
                // Born since the cycle began, so it is not joined this cycle; its next scan
                // will have an identity to keep it under.
                None => {
                    self.0.remove(&pid);
                }
            }
        }
        self.0
            .iter()
            .map(|(&pid, (_, files))| (pid, files.clone()))
            .collect()
    }
}

/// Joins the background measurements onto a sample. They were scanned against
/// `extras.identities`, so a pid counts only while it still names the same process: data for a
/// process that has exited since is dropped, and a process that took its pid since is not
/// measured until the next cycle instead of inheriting what its predecessor had.
fn merge(snapshot: &mut Snapshot, extras: &Extras) {
    let current: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
    let id = |pid: &u32| {
        extras
            .identities
            .get(pid)
            .filter(|id| current.contains(id))
            .copied()
    };
    for process in &mut snapshot.processes {
        let scanned = extras.identities.get(&process.id.pid) == Some(&process.id);
        process.gpu_memory = extras.gpu.get(&process.id.pid).filter(|_| scanned).copied();
        merge_files(process, &extras.files, &extras.locks, scanned);
        // A holder that has exited since released its locks, so nothing waits on it any more.
        process.blocked_on = process.blocked_on.filter(|blocker| match blocker {
            Blocker::Process(holder) => id(holder).is_some(),
            Blocker::Unnamed => true,
        });
    }
    snapshot.unattributed_locks = extras.locks.known().map_or(0, |locks| locks.unattributed);
    snapshot.links = extras
        .network
        .links
        .iter()
        .filter_map(|(&(a, b), &count)| Some((id(&a)?, id(&b)?, count)))
        .collect();
    snapshot.outside = extras
        .network
        .outside
        .iter()
        .filter_map(|(pid, &count)| Some((id(pid)?, count)))
        .collect();
    snapshot.link_traffic = extras
        .traffic
        .iter()
        .filter_map(|(&(a, b), &rate)| Some(((id(&a)?, id(&b)?), rate)))
        .collect();
    snapshot.unix_links = extras
        .network
        .unix
        .iter()
        .filter_map(|(a, b)| Some((id(a)?, id(b)?)))
        .collect();
    snapshot.units = extras.units.clone();
    snapshot.remotes = extras
        .remotes
        .iter()
        .filter_map(|remote| {
            Some(Remote {
                id: id(&remote.id.pid)?,
                ..remote.clone()
            })
        })
        .collect();
    snapshot.geo = extras.geo.clone();
    snapshot.missing = snapshot.missing.union(extras.missing);
}

/// The slow sources a background pass found absent: GPU memory when no GPU library loaded, and
/// file locks when the lock table could not be read.
fn background_gaps(gpu: bool, locks: &Measured<platform::Locks>) -> Sources {
    let mut missing = Sources::default();
    if !gpu {
        missing.insert(Source::GpuMemory);
    }
    if matches!(locks, Measured::Unreadable) {
        missing.insert(Source::FileLocks);
    }
    missing
}

/// Sets a process's open files and locks from the background scans: absent from the file scan
/// means not read yet, not unreadable, and so does a process the scans did not run against
/// (`scanned` false: born since, or holding a pid that belonged to another process then). Kernel
/// threads keep the empty table the platform gave them (Linux); a kernel process the platform
/// leaves to the scan (kernel_task on macOS, which has a descriptor table like any process)
/// takes what the scan found.
fn merge_files(
    process: &mut Process,
    files: &HashMap<u32, Option<platform::Files>>,
    locks: &Measured<platform::Locks>,
    scanned: bool,
) {
    let pid = process.id.pid;
    if process.kind != Kind::Kernel || process.files == Measured::Pending {
        process.files = match files.get(&pid).filter(|_| scanned) {
            Some(Some(found)) => Measured::Known(found.clone()),
            Some(None) => Measured::Unreadable,
            None => Measured::Pending,
        };
    }
    (process.locks_held, process.blocked_on) = match locks {
        Measured::Known(locks) if scanned => (
            Measured::Known(locks.held.get(&pid).copied().unwrap_or(0)),
            locks.blocked.get(&pid).copied(),
        ),
        Measured::Known(_) | Measured::Pending => (Measured::Pending, None),
        Measured::Unreadable => (Measured::Unreadable, None),
    };
}

/// Byte rates from the change in each socket's counters since the previous scan, and locations.
fn remotes(
    sockets: &[platform::Remote],
    previous: &mut HashMap<u64, (Instant, u64, u64)>,
    mut geo: Option<&mut geo::Geo>,
) -> Vec<Remote> {
    let now = Instant::now();
    let mut next = HashMap::new();
    let found = sockets
        .iter()
        .map(|socket| {
            let rate = |now_bytes: u64, then_bytes: u64, dt: f32| {
                now_bytes
                    .checked_sub(then_bytes)
                    .map(|bytes| bytes as f32 / dt)
            };
            let (up, down) =
                previous
                    .get(&socket.inode)
                    .map_or((None, None), |&(then, sent, received)| {
                        let dt = now.duration_since(then).as_secs_f32().max(0.1);
                        (
                            rate(socket.sent, sent, dt),
                            rate(socket.received, received, dt),
                        )
                    });
            next.insert(socket.inode, (now, socket.sent, socket.received));
            Remote {
                id: Identity {
                    pid: socket.pid,
                    start: 0,
                },
                address: socket.address,
                port: socket.port,
                rtt: socket.rtt as f32 / 1000.0,
                up,
                down,
                place: geo.as_mut().and_then(|geo| geo.locate(socket.address)),
            }
        })
        .collect();
    *previous = next;
    found
}

/// The cgroup paths to account this cycle: those wanted now plus those wanted in the previous
/// cycle. A cgroup whose only process was just OOM-killed is no longer wanted, but its
/// memory.events still holds the kill, so it is read once more before being dropped.
fn accounted(wanted: HashSet<String>, previous: &mut HashSet<String>) -> HashSet<String> {
    let paths = wanted.union(previous).cloned().collect();
    *previous = wanted;
    paths
}

/// What the previous loopback scan saw: when it ran, and each socket's received counter with
/// the time it was read and the number of scans since it was last seen.
#[derive(Default)]
struct LoopbackScan {
    at: Option<Instant>,
    received: HashMap<u64, (u64, Instant, u32)>,
}

/// Scans a socket's last counter is kept after it drops out, so a socket that one scan missed
/// (an owner's fd table briefly unreadable) is measured from its old counter when it returns
/// instead of looking new. Socket inode numbers are not reused within that time.
const LOOPBACK_MISSES: u32 = 3;

/// Bytes per second between pid pairs (smaller pid first) from the change in each loopback
/// socket's received counter since the scan that last saw it, divided by the time since then.
/// Each end counts what it received, which is what the other end sent, so both directions
/// together count every byte once.
///
/// A socket not in the previous scan was opened inside the interval, so all of its received
/// bytes fall in it. On the very first scan nothing is known about the interval, and the pairs
/// stay absent rather than reading as measured and idle.
fn traffic(
    sockets: &[platform::Loopback],
    previous: &mut LoopbackScan,
    now: Instant,
) -> HashMap<(u32, u32), f32> {
    let interval = previous
        .at
        .map(|then| now.duration_since(then).as_secs_f32().max(0.1));
    let mut next = HashMap::new();
    let mut rates: HashMap<(u32, u32), f32> = HashMap::new();
    for socket in sockets {
        next.insert(socket.inode, (socket.received, now, 0));
        let Some(interval) = interval else {
            continue;
        };
        let (before, elapsed) = previous
            .received
            .get(&socket.inode)
            .map_or((0, interval), |&(received, seen, _)| {
                (received, now.duration_since(seen).as_secs_f32().max(0.1))
            });
        *rates
            .entry((socket.pid.min(socket.peer), socket.pid.max(socket.peer)))
            .or_default() += socket.received.saturating_sub(before) as f32 / elapsed;
    }
    for (&inode, &(received, seen, misses)) in &previous.received {
        if misses < LOOPBACK_MISSES {
            next.entry(inode).or_insert((received, seen, misses + 1));
        }
    }
    *previous = LoopbackScan {
        at: Some(now),
        received: next,
    };
    rates
}

impl Collector {
    pub fn new(geoip: Option<PathBuf>, home: Option<(f32, f32)>) -> Self {
        Self {
            sampler: platform::Sampler::new(),
            previous: HashMap::new(),
            last: Instant::now(),
            origin: Instant::now(),
            background: None,
            wanted: Arc::new(Mutex::new(HashSet::new())),
            locating: Arc::new(AtomicBool::new(false)),
            identities: Arc::new(Mutex::new(HashMap::new())),
            geoip,
            home: geo::home(home),
            started: false,
        }
    }

    /// Turns locating remote addresses on or off for the following samples.
    pub fn locate(&self, on: bool) {
        self.locating.store(on, Ordering::Relaxed);
    }

    pub fn sample(&mut self) -> std::io::Result<Snapshot> {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().max(0.001);
        let (pressure, pressure_missing) = self.sampler.pressure();
        let mut snapshot = Snapshot {
            cores: std::thread::available_parallelism().map_or(1, usize::from),
            elapsed: now.duration_since(self.origin).as_secs_f64(),
            pressure,
            home: self.home.clone(),
            ..Default::default()
        };
        (snapshot.memory_total, snapshot.memory_available) = self.sampler.memory();
        let hz = self.sampler.hz();
        let mut next = HashMap::new();
        for raw in self.sampler.processes()? {
            let previous = self.previous.get(&raw.process.id);
            let (process, counters) = measure(raw, previous, hz, dt);
            next.insert(process.id, counters);
            snapshot.processes.push(process);
        }
        // After the processes, which a platform may derive CPU readings from (the macOS
        // cluster clocks come from the processes' cycle counters).
        snapshot.cpus = self.sampler.cpus(dt);
        snapshot.missing = pressure_missing.union(self.sampler.missing());
        snapshot.per_cluster = self.sampler.per_cluster();
        snapshot.unreadable = self.sampler.unreadable();
        snapshot.shadows = self.sampler.shadows();
        snapshot.processes.sort_by_key(|p| p.id);
        if let Ok(mut wanted) = self.wanted.lock() {
            *wanted = snapshot
                .processes
                .iter()
                .filter(|p| !p.cgroup.is_empty())
                .map(|p| p.cgroup.clone())
                .collect();
        }
        if let Ok(mut identities) = self.identities.lock() {
            *identities = snapshot
                .processes
                .iter()
                .map(|p| (p.id.pid, p.id))
                .collect();
        }
        // Started once the first sample has published what to scan, so its first cycle has the
        // processes to join against and the cgroups to account.
        if !std::mem::replace(&mut self.started, true) {
            self.background = background(
                self.geoip.clone(),
                Arc::clone(&self.wanted),
                Arc::clone(&self.locating),
                Arc::clone(&self.identities),
            );
        }
        if let Some(background) = &self.background {
            snapshot.sampler = background.health(Instant::now());
            merge(&mut snapshot, &background.extras());
        }
        self.previous = next;
        self.last = now;
        Ok(snapshot)
    }
}

/// A process with its rates filled in from the change in its counters since `previous`, `dt`
/// seconds ago (CPU ticks count `hz` per second), and the counters to keep for the next sample.
fn measure(raw: RawProcess, previous: Option<&Counters>, hz: f32, dt: f32) -> (Process, Counters) {
    let RawProcess {
        mut process,
        ticks,
        io,
        performance_ticks,
        runnable_ticks,
        system_ticks,
        syscalls,
        major_faults,
    } = raw;
    process.cpu_time = ticks as f32 / hz;
    // Runnable time counts running time too, so what remains after the CPU time is waiting.
    let waited = runnable_ticks.map(|runnable| i128::from(runnable) - i128::from(ticks));
    if let Some(previous) = previous {
        let raw = ticks.saturating_sub(previous.ticks) as f32 / hz / dt * 100.0;
        let alpha = smoothing(dt, 1.5);
        process.cpu = previous.cpu + alpha * (raw - previous.cpu);
        if let Some([total, read, write]) = io
            .zip(previous.io)
            .and_then(|(current, before)| current.rates(before, dt))
        {
            process.io_rate = Some(total);
            process.read_rate = Some(read);
            process.write_rate = Some(write);
        }
        let reset = restarted(ticks, runnable_ticks, system_ticks, previous);
        // The kernel-mode part of the CPU time that elapsed; none when no time elapsed or the
        // counters went back.
        process.system_share = ticks
            .checked_sub(previous.ticks)
            .zip(system_ticks.checked_sub(previous.system_ticks))
            .filter(|&(elapsed, _)| elapsed > 0 && !reset)
            .map(|(elapsed, system)| (system as f32 / elapsed as f32).min(1.0));
        // After a reset (exec or wraparound) the old counters say nothing about the interval,
        // so the rates are unknown until the next reading, never zero or negative.
        process.syscall_io = syscalls
            .zip(previous.syscalls)
            .filter(|_| !reset)
            .and_then(|(now, before)| now.rates(before, dt));
        process.major_faults = major_faults
            .zip(previous.major_faults)
            .filter(|_| !reset)
            .and_then(|(now, before)| now.checked_sub(before))
            .map(|faults| faults as f32 / dt);
        // A counter that went backwards (a reused pid) gives no reading rather than a wrong one.
        process.performance_share = performance_ticks
            .zip(previous.performance_ticks)
            .and_then(|(now, before)| now.checked_sub(before))
            .zip(ticks.checked_sub(previous.ticks))
            .filter(|&(_, interval)| interval > 0)
            .map(|(performance, interval)| (performance as f32 / interval as f32).min(1.0));
        // Only growth beyond the highest total seen counts, so a dip while threads ran and its
        // recovery once they are switched out again are not mistaken for waiting. The first
        // reading may itself be dipped by however long a thread has run without a switch, so
        // the result is held to the threads there were: no more of them can have waited.
        let threads = process.threads.max(previous.threads) as f32;
        process.waiting = waited
            .zip(previous.waited)
            .filter(|_| !reset)
            .map(|(now, highest)| ((now - highest).max(0) as f32 / hz / dt).min(threads));
    }
    let highest = match (waited, previous) {
        (Some(now), Some(previous))
            if !restarted(ticks, runnable_ticks, system_ticks, previous) =>
        {
            Some(previous.waited.map_or(now, |highest| now.max(highest)))
        }
        (now, _) => now,
    };
    let counters = Counters {
        ticks,
        io,
        cpu: process.cpu,
        performance_ticks,
        waited: highest,
        runnable_ticks,
        threads: process.threads,
        system_ticks,
        syscalls,
        major_faults,
    };
    (process, counters)
}

/// Whether a process's CPU, kernel-mode or runnable total went backwards since the previous
/// reading. On macOS exec gives the process a new task that keeps only the exec'ing thread's
/// counters, so the old high-water mark of its waiting no longer applies, and no rate derived
/// from the old counters does either.
fn restarted(
    ticks: u64,
    runnable_ticks: Option<u64>,
    system_ticks: u64,
    previous: &Counters,
) -> bool {
    ticks < previous.ticks
        || system_ticks < previous.system_ticks
        || runnable_ticks
            .zip(previous.runnable_ticks)
            .is_some_and(|(now, before)| now < before)
}

pub fn bounded(value: f32, knee: f32) -> f32 {
    let value = value.max(0.0);
    value / (value + knee)
}

/// The demo's CPU use is `scale / 16 * (1 + sin(theta))^4 + bonus` with `theta = 0.55 t + 1.71 i`.
/// This is the exact integral of that CPU use (in percent-seconds) from time 0 to `time`, using
/// the antiderivative of (1 + sin)^4 = 1 + 4 sin + 6 sin^2 + 4 sin^3 + sin^4 with respect to theta:
/// 35/8 theta - 8 cos + 4/3 cos^3 - 7/4 sin 2theta + 1/32 sin 4theta.
fn demo_cpu_integral(time: f64, index: usize) -> f64 {
    const RATE: f64 = 0.55;
    let antiderivative = |theta: f64| {
        4.375 * theta - 8.0 * theta.cos() + 4.0 / 3.0 * theta.cos().powi(3)
            - 1.75 * (2.0 * theta).sin()
            + (4.0 * theta).sin() / 32.0
    };
    let start = index as f64 * 1.71;
    let (scale, bonus) = demo_cpu_shape(index);
    scale / 16.0 * (antiderivative(RATE * time + start) - antiderivative(start)) / RATE
        + bonus * time
}

/// The two numbers that shape demo process `index`'s CPU use: the `scale` of its swing and the
/// `bonus` that a few processes add on top. Its peak CPU use, in percent, is `scale + bonus`.
fn demo_cpu_shape(index: usize) -> (f64, f64) {
    let scale = 20.0 + (index % 9) as f64 * 24.0;
    let bonus = if index.is_multiple_of(47) { 110.0 } else { 0.0 };
    (scale, bonus)
}

/// The CPU use in percent of demo process `index` at `time`, in `f64`: the function that
/// `demo_cpu_integral` integrates. Base processes compute the same value in `f32` inside `demo`.
fn demo_cpu_percent(time: f64, index: usize) -> f64 {
    let (scale, bonus) = demo_cpu_shape(index);
    let phase = 0.55 * time + 1.71 * index as f64;
    (phase.sin() * 0.5 + 0.5).powi(4) * scale + bonus
}

/// The CPU time in seconds of demo process `index` at `time` when `DemoOptions::exact_cpu_time`
/// is on: the integral of its CPU use from time 0 (`demo_cpu_integral`, in percent-seconds,
/// divided by 100), plus the head start of `3.7 * index` seconds that `demo` gives each process.
/// It never decreases as `time` grows, because the CPU use is never negative. Before time 0 the
/// integral runs backwards, so the value can be negative there.
pub fn demo_cpu_seconds(time: f64, index: usize) -> f64 {
    demo_cpu_integral(time, index) / 100.0 + index as f64 * 3.7
}

/// The share of a demo process's I/O that is writes.
fn demo_write_share(index: usize) -> f32 {
    (1 + index % 3) as f32 / 4.0
}

/// When the demo machine booted, in seconds since the Unix epoch.
const DEMO_BOOT_TIME: f64 = 1_760_000_000.0;

/// Seconds between the starts of consecutive demo processes (`Identity::start` counts them).
const DEMO_START_SPACING: f64 = 30.0;

/// The kernel-mode share of a demo process's CPU time: kernel threads spend nearly all of it
/// there, and each family leans a different way, drifting a little over time.
fn demo_system_share(family: usize, kind: Kind, phase: f32) -> f32 {
    if kind == Kind::Kernel {
        0.97
    } else {
        (0.08 + 0.14 * (family % 5) as f32 + 0.06 * (phase * 0.3).sin()).clamp(0.0, 1.0)
    }
}

/// The scheduling policy of demo process `index`: real time where its priority is -51, batch or
/// idle where its nice is 19, normal for the rest.
fn demo_policy(index: usize, priority: i32, nice: i32) -> Policy {
    if priority == -51 {
        Policy::Fifo
    } else if nice == 19 {
        if index % 12 == 10 {
            Policy::Idle
        } else {
            Policy::Batch
        }
    } else {
        Policy::Normal
    }
}

/// Syscall I/O of demo process `index`, on the processes that do storage I/O (`io_rate`, of
/// which `write_rate` is writes): the bytes passed through read and write calls are at least
/// the bytes that reached storage, because the page cache absorbs some.
fn demo_syscall_io(index: usize, io_rate: f32, write_rate: f32) -> SyscallIo {
    if !index.is_multiple_of(7) {
        return SyscallIo::default();
    }
    let read_chars = (io_rate - write_rate) * (1.25 + 0.25 * (index % 4) as f32);
    let write_chars = write_rate * (1.1 + 0.1 * (index % 3) as f32);
    let call_size = 4096.0 * (1 + index % 8) as f32;
    SyscallIo {
        read_chars,
        write_chars,
        reads: read_chars / call_size,
        writes: write_chars / call_size,
    }
}

/// Major faults per second of demo process `index`: a few processes page in from storage.
fn demo_major_faults(index: usize, phase: f32) -> f32 {
    if index % 23 == 5 {
        4.0 * (1.1 + (phase * 0.7).sin())
    } else {
        0.0
    }
}

/// Demo open files, deleted-but-open files and file locks of process `index`, fixed for the whole
/// run so the clutches hold still: open files spread log-uniformly from 3 to about 380, a database
/// holding a rotated 1.2 GiB log and a browser two cache files after deleting them, four lock
/// holders, and two waiters in other flocks than their holders. Returns the open files, locks
/// held and the index of the holder it waits for.
fn demo_files(index: usize, kind: Kind) -> (platform::Files, u32, Option<usize>) {
    if kind == Kind::Kernel {
        return (platform::Files::default(), 0, None);
    }
    let spread = (index * 37 + 11) % 97;
    let open = (3.0 * (7.0 * spread as f32 / 96.0).exp2()).round() as u32;
    let deleted = |inode: u64, size: u64| platform::DeletedFile {
        device: 2049,
        inode,
        size,
    };
    let deleted = match index {
        69 => vec![deleted(6900, 1_288_490_189)],
        83 => vec![deleted(8300, 32 << 20), deleted(8301, 16 << 20)],
        _ => Vec::new(),
    };
    let held = match index {
        5 => 2,
        21 | 34 | 101 => 1,
        _ => 0,
    };
    let waiting = match index {
        40 => Some(5),
        70 => Some(21),
        _ => None,
    };
    let open = open.max(deleted.len() as u32);
    let files = platform::Files {
        open,
        deleted,
        partial: false,
    };
    (files, held, waiting)
}

/// What the demo workload synthesizes for a view beyond what `demo` gives every view. A view asks
/// for it on its registry row (`ViewInfo::demo`); the default is `demo` exactly, so a view that
/// asks for nothing gets today's frames, field for field.
///
/// Both options are pure functions of `time`, with no state, so a call at any time, in any order
/// and including negative times, gives the same snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DemoOptions {
    /// Adds transient processes after the base population, so base indices, pids and links stay
    /// as they are. There is one family per 16 base processes, and each family repeats a cycle
    /// of 24, 26, 28 or 30 seconds (by family number, staggered): a parent forks 1 to 6 children
    /// over 2 to 4 seconds, each child runs for 3 to 12 seconds and exits, and the parent exits
    /// 0.4 s after its last child. Some exits leave a `'Z'` entry for `DEMO_ZOMBIE_SECONDS`, one
    /// sample at the default rate. Once per 90 seconds, 60 s into its cycle, one more transient
    /// that grew above 1 GiB is killed with signal 9 (the out-of-memory case). Every transient
    /// lives inside its own cycle, so cycles never overlap, and pids come from a fixed pool
    /// above `1000 + count` and are reused with a new `Identity::start`. A transient's
    /// `cpu_time` is the integral of its own `cpu` since birth, and `demo_life` lists their forks
    /// and exits.
    pub churn: bool,
    /// Gives base processes the CPU time `demo_cpu_seconds` computes, the exact integral of
    /// their `cpu`, and enough threads for the cores they use: `threads >= ceil(peak cpu / 100)`,
    /// so a single-threaded process never exceeds one core. Off, `cpu_time` and `threads` are
    /// the unrelated values `demo` has always had.
    pub exact_cpu_time: bool,
}

/// One transient family per this many base processes.
const DEMO_FAMILY_SIZE: usize = 16;

/// The most children a family's parent forks in one cycle.
const DEMO_MAX_CHILDREN: usize = 6;

/// Seconds that a process which leaves a zombie stays listed as `'Z'` after it exited: one sample
/// at the default `--sample-ms`.
pub const DEMO_ZOMBIE_SECONDS: f64 = 1.0;

/// Seconds between the out-of-memory kills.
const DEMO_OOM_PERIOD: f64 = 90.0;

/// Added to a transient's birth in 1/100 s ticks to make `Identity::start`, which puts it above
/// every base `start` (those count processes from 1).
const DEMO_TRANSIENT_START: i64 = 1 << 40;

/// Seeds the generator of a (family, epoch) cycle, apart from other uses of the same identity.
const DEMO_CHURN_SALT: u64 = 0x5334_c4a2_d3e1;

/// The part a transient plays in its cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Forks the children of the cycle and waits for them.
    Parent,
    /// One of the parent's children.
    Child,
    /// The process that grows until the out-of-memory killer ends it.
    Victim,
}

/// One transient process of one cycle: when it is listed and everything else fixed about it.
#[derive(Clone, Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only demo_life reads the exit status, for B3 (#30)"
    )
)]
struct Transient {
    id: Identity,
    /// Its parent: the cycle's parent for a child, a base process otherwise.
    parent: Identity,
    family: usize,
    role: Role,
    /// The base process whose district it lives in.
    anchor: usize,
    /// Stands for the process index in the demo's CPU, memory and I/O formulas.
    shape: usize,
    /// It is listed from `birth` (inclusive) to `end` (exclusive), in demo seconds.
    birth: f64,
    /// When it exited. A zombie stays listed from `exit` until `end`; otherwise `end == exit`.
    exit: f64,
    end: f64,
    code: i32,
    signal: Option<i32>,
    /// The bytes the victim holds when it is killed; None follows the usual formula.
    peak_memory: Option<f64>,
}

/// The number of transient families for `count` base processes. The victim is family number
/// `families`.
fn demo_families(count: usize) -> usize {
    count.div_ceil(DEMO_FAMILY_SIZE)
}

/// The length of a family's cycle in seconds.
fn demo_cycle_period(family: usize, families: usize) -> f64 {
    if family == families {
        DEMO_OOM_PERIOD
    } else {
        24.0 + 2.0 * (family % 4) as f64
    }
}

/// When a family's cycle number 0 starts, in demo seconds. Staggered, so families do not fork in
/// step.
fn demo_cycle_offset(family: usize) -> f64 {
    5.9 * family as f64
}

/// The number of the cycle of `family` that `time` falls in; negative before the first.
fn demo_epoch(time: f64, family: usize, families: usize) -> i64 {
    ((time - demo_cycle_offset(family)) / demo_cycle_period(family, families)).floor() as i64
}

/// The identity of the transient in pool slot `slot`. The pid is above every base pid, and the
/// start is the birth in 1/100 s ticks, so a reused pid gets a new identity.
fn demo_transient_identity(count: usize, slot: usize, birth: f64) -> Identity {
    let ticks = (birth * 100.0).floor() as i64;
    Identity {
        pid: 1001 + (count + slot) as u32,
        start: (DEMO_TRANSIENT_START + ticks) as u64,
    }
}

/// The identity of base process `index`.
fn demo_base_identity(index: usize) -> Identity {
    Identity {
        pid: 1000 + index as u32,
        start: index as u64 + 1,
    }
}

/// Every transient of `family`'s cycle number `epoch`, the parent first. All of them are listed
/// inside the cycle, whatever the draws: a parent is born less than a second in, forks until at
/// most 4.2 s after that, its children run for at most 12 s, and the last zombie window ends
/// 1.4 s after the last of them, so under 19 s of the shortest cycle's 24. The victim lives 35
/// to 55 s, is killed 60 s into its cycle and is a zombie for a second after, inside the 90.
fn demo_cycle(count: usize, family: usize, epoch: i64) -> Vec<Transient> {
    let families = demo_families(count);
    if count == 0 || family > families {
        return Vec::new();
    }
    let start = demo_cycle_offset(family) + epoch as f64 * demo_cycle_period(family, families);
    let mut rng = Rng::new(hash_identity(
        Identity {
            pid: family as u32,
            start: epoch as u64,
        },
        DEMO_CHURN_SALT,
    ));
    // Any district but a kernel thread's, which fork nothing here.
    let mut district = (rng.next_u64() % families as u64) as usize;
    if district % 4 == 3 {
        district -= 1;
    }
    let anchor = (district * DEMO_FAMILY_SIZE + 1 + (rng.next_u64() % 15) as usize).min(count - 1);
    let anchor_id = demo_base_identity(anchor);
    let shape = |rng: &mut Rng| 1 + (rng.next_u64() % 4000) as usize;
    let slot = family * (DEMO_MAX_CHILDREN + 1);
    if family == families {
        // Killed 60 s into every cycle, so the kills are exactly one period apart.
        let exit = start + 60.0;
        let birth = exit - 35.0 - 20.0 * f64::from(rng.unit());
        return vec![Transient {
            id: demo_transient_identity(count, slot, birth),
            parent: anchor_id,
            family,
            role: Role::Victim,
            anchor,
            shape: shape(&mut rng),
            birth,
            exit,
            end: exit + DEMO_ZOMBIE_SECONDS,
            code: 0,
            signal: Some(9),
            peak_memory: Some((1.1 + 0.5 * f64::from(rng.unit())) * 1073741824.0),
        }];
    }
    let parent_birth = start + f64::from(rng.unit());
    let children = 1 + (rng.next_u64() % DEMO_MAX_CHILDREN as u64) as usize;
    let span = 2.0 + 2.0 * f64::from(rng.unit());
    let parent_id = demo_transient_identity(count, slot, parent_birth);
    let parent_shape = shape(&mut rng);
    let mut cycle = Vec::with_capacity(children + 1);
    let mut last_exit = parent_birth;
    for child in 1..=children {
        let birth = parent_birth
            + 0.2
            + span * ((child - 1) as f64 + f64::from(rng.unit())) / children as f64;
        let exit = birth + 3.0 + 9.0 * f64::from(rng.unit());
        let zombie = rng.unit() < 0.35;
        let code = i32::from(rng.unit() < 0.15);
        last_exit = last_exit.max(exit);
        cycle.push(Transient {
            id: demo_transient_identity(count, slot + child, birth),
            parent: parent_id,
            family,
            role: Role::Child,
            anchor,
            shape: shape(&mut rng),
            birth,
            exit,
            end: exit + if zombie { DEMO_ZOMBIE_SECONDS } else { 0.0 },
            code,
            signal: None,
            peak_memory: None,
        });
    }
    let exit = last_exit + 0.4;
    let zombie = rng.unit() < 0.5;
    cycle.insert(
        0,
        Transient {
            id: parent_id,
            parent: anchor_id,
            family,
            role: Role::Parent,
            anchor,
            shape: parent_shape,
            birth: parent_birth,
            exit,
            end: exit + if zombie { DEMO_ZOMBIE_SECONDS } else { 0.0 },
            code: 0,
            signal: None,
            peak_memory: None,
        },
    );
    cycle
}

/// The transients listed at `time`, family by family with the parent of a cycle before its
/// children: those born at or before `time` and not yet gone. A cycle holds its transients
/// inside itself, so only the cycle that `time` falls in can have any.
fn demo_listed(time: f64, count: usize) -> Vec<Transient> {
    let families = demo_families(count);
    (0..=families)
        .flat_map(|family| demo_cycle(count, family, demo_epoch(time, family, families)))
        .filter(|transient| transient.birth <= time && time < transient.end)
        .collect()
}

impl Transient {
    /// Whether it has exited by `time`; a zombie is listed from then on.
    fn exited(&self, time: f64) -> bool {
        time >= self.exit
    }

    /// The percent-seconds of CPU it has used by `time`: the exact integral of its CPU use since
    /// birth, which stops growing at the exit.
    fn cpu_integral(&self, time: f64) -> f64 {
        let until = time.min(self.exit);
        demo_cpu_integral(until, self.shape) - demo_cpu_integral(self.birth, self.shape)
    }

    /// The bytes it holds while running at `time`. A new process starts at 30% of its usual size
    /// and reaches that in 3 s; the victim leaks from 300 MiB to its peak with the square of its
    /// age.
    fn running_memory(&self, time: f64) -> f64 {
        let age = time.min(self.exit) - self.birth;
        if let Some(peak) = self.peak_memory {
            let grown = (age / (self.exit - self.birth)).clamp(0.0, 1.0);
            return 300.0 * 1048576.0 + (peak - 300.0 * 1048576.0) * grown * grown;
        }
        let phase = (0.55 * time + 1.71 * self.shape as f64) as f32;
        let usual =
            (12.0 + (self.shape * 71 % 900) as f32 * (0.8 + 0.2 * (phase * 0.2).sin())) * 1048576.0;
        f64::from(usual) * (0.3 + 0.7 * (age / 3.0).clamp(0.0, 1.0))
    }

    /// The `Process` listed at `time`, which is between `birth` and `end`, among `count` base
    /// processes. Once it has exited it is a zombie: no CPU, no memory, no readable descriptor
    /// table, and no rate, because there is nothing running to measure one on.
    fn process(&self, time: f64, count: usize) -> Process {
        let index = self.shape;
        let running = !self.exited(time);
        let phase = (0.55 * time + 1.71 * index as f64) as f32;
        let cpu = if running {
            demo_cpu_percent(time, index) as f32
        } else {
            0.0
        };
        let family = self.anchor / DEMO_FAMILY_SIZE;
        let kind = [Kind::System, Kind::Session, Kind::Container, Kind::Kernel][family % 4];
        let group = format!("district-{:02}.service", family);
        let cgroup = demo_slice(kind).map_or_else(String::new, |slice| format!("{slice}/{group}"));
        let stem = match self.role {
            Role::Parent => ["make", "cargo", "ninja", "pytest"][self.family % 4],
            Role::Child => ["cc1", "rustc", "ld", "test"][index % 4],
            Role::Victim => "leak",
        };
        let (priority, nice) = demo_priority(index, kind);
        let io_rate = if index.is_multiple_of(7) {
            cpu * 65536.0
        } else {
            0.0
        };
        let write_rate = io_rate * demo_write_share(index);
        // Written bytes are the integral of write_rate, as in `demo`, counted from birth.
        let written = if index.is_multiple_of(7) {
            (self.cpu_integral(time) * 65536.0 * demo_write_share(index) as f64) as u64
        } else {
            0
        };
        let core = ((index * 7 + (time / (5.0 + (index % 5) as f64)) as usize) % 16) as u32;
        // Exhaustive on purpose: a new field must be decided here.
        Process {
            id: self.id,
            parent: self.parent.pid,
            name: format!("{stem}-{}", self.id.pid),
            command: format!("{stem} --demo --job {}", self.id.pid),
            cgroup,
            group,
            kind,
            state: if !running {
                'Z'
            } else if cpu > 30.0 {
                'R'
            } else {
                'S'
            },
            cpu,
            memory: if running {
                self.running_memory(time) as u64
            } else {
                0
            },
            io_rate: running.then_some(io_rate),
            read_rate: running.then_some(io_rate - write_rate),
            write_rate: running.then_some(write_rate),
            written: Some(written),
            priority,
            nice,
            threads: demo_threads(index, true),
            gpu_memory: None,
            core: Some(core),
            cpu_time: (self.cpu_integral(time) / 100.0) as f32,
            performance_share: (cpu > 0.0).then(|| {
                let mixing = 0.15 * (0.5 + 0.5 * (phase * 0.37).sin());
                if core < 8 { 1.0 - mixing } else { mixing }
            }),
            waiting: Some(1.5 * bounded(cpu, 80.0) * (0.6 + 0.4 * (phase * 0.8).cos())),
            files: if running {
                Measured::Known(platform::Files {
                    open: 3 + (index % 12) as u32,
                    deleted: Vec::new(),
                    partial: false,
                })
            } else {
                Measured::Unreadable
            },
            locks_held: Measured::Known(0),
            blocked_on: None,
            system_share: (cpu > 0.0).then(|| demo_system_share(self.family, kind, phase)),
            policy: demo_policy(index, priority, nice),
            syscall_io: running.then(|| demo_syscall_io(index, io_rate, write_rate)),
            major_faults: running.then(|| demo_major_faults(index, phase)),
            // Demo time 0 is the instant after the last base process started.
            started: DEMO_BOOT_TIME + (count as u64 + 1) as f64 * DEMO_START_SPACING + self.birth,
        }
    }
}

/// What happened to a transient process, for the event stream of `demo_life`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "B3 (#30) builds its demo event stream from demo_life"
    )
)]
pub enum DemoLifeKind {
    Fork,
    Exit,
}

/// One fork or exit of a transient process.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "B3 (#30) builds its demo event stream from demo_life"
    )
)]
pub struct DemoLife {
    pub kind: DemoLifeKind,
    pub id: Identity,
    pub parent: Identity,
    /// Demo seconds: the birth for a fork; for an exit, the moment the process stops being
    /// listed, which is `DEMO_ZOMBIE_SECONDS` after it died when it left a zombie.
    pub time: f64,
    /// The bytes it holds: at the fork, and at the moment it died for an exit.
    pub memory: u64,
    /// The CPU seconds it has used: 0 at the fork, its whole life at the exit.
    pub cpu_seconds: f64,
    pub threads: u32,
    /// The exit status; 0 for a fork, and for a process a signal ended.
    pub code: i32,
    /// The signal that killed it; None for a fork and for a normal exit.
    pub signal: Option<i32>,
}

/// The forks and exits of the transient processes (`DemoOptions::churn`) among `count` base
/// processes whose time falls in `(from, to]`, ordered by time. A transient is listed from its
/// fork to its exit, so with `a` the sample at `from` and `b` the one at `to`:
///
/// - a fork without an exit of the same identity is a process listed at `b` and not at `a`;
/// - an exit without a fork is one listed at `a` and not at `b`;
/// - a process that both forked and exited in the interval is in neither sample and has both
///   events, so splitting an interval anywhere gives the same events in total.
///
/// An interval shorter than 3 s, the shortest a process is listed, has no such pair, and its
/// events are exactly the difference between the two populations. The cycles are 24 to 30 s long
/// (90 s for the one process that outgrows 1 GiB and is killed with signal 9), so an interval of
/// that length or more holds every kind of event.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "B3 (#30) builds its demo event stream from demo_life"
    )
)]
pub fn demo_life(from: f64, to: f64, count: usize) -> Vec<DemoLife> {
    let families = demo_families(count);
    let mut events = Vec::new();
    for family in 0..=families {
        let epochs = demo_epoch(from, family, families)..=demo_epoch(to, family, families);
        for transient in epochs.flat_map(|epoch| demo_cycle(count, family, epoch)) {
            let event = |kind, time, running_at: f64, cpu_seconds| DemoLife {
                kind,
                id: transient.id,
                parent: transient.parent,
                time,
                memory: transient.running_memory(running_at) as u64,
                cpu_seconds,
                threads: demo_threads(transient.shape, true),
                code: if kind == DemoLifeKind::Exit {
                    transient.code
                } else {
                    0
                },
                signal: if kind == DemoLifeKind::Exit {
                    transient.signal
                } else {
                    None
                },
            };
            if from < transient.birth && transient.birth <= to {
                events.push(event(
                    DemoLifeKind::Fork,
                    transient.birth,
                    transient.birth,
                    0.0,
                ));
            }
            if from < transient.end && transient.end <= to {
                events.push(event(
                    DemoLifeKind::Exit,
                    transient.end,
                    transient.exit,
                    transient.cpu_integral(transient.exit) / 100.0,
                ));
            }
        }
    }
    events.sort_by(|a, b| a.time.total_cmp(&b.time));
    events
}

/// The thread count of demo process `index`: one, or a power of two times four for every fifth.
/// With `exact` it is also at least one thread per 100% of the process's peak CPU use, so a
/// process that never has two threads never uses more than one core.
fn demo_threads(index: usize, exact: bool) -> u32 {
    let threads = if index.is_multiple_of(5) {
        4 << (index % 6)
    } else {
        1
    };
    if exact {
        let (scale, bonus) = demo_cpu_shape(index);
        threads.max(((scale + bonus) / 100.0).ceil() as u32)
    } else {
        threads
    }
}

/// The scheduler priority and nice value of demo process `index` of `kind`.
fn demo_priority(index: usize, kind: Kind) -> (i32, i32) {
    if kind == Kind::Kernel && index.is_multiple_of(3) {
        (0, -20)
    } else if index % 61 == 17 {
        (-51, 0)
    } else if index % 6 == 4 {
        (39, 19)
    } else {
        (20, 0)
    }
}

/// The cgroup slice that districts of `kind` live under; kernel threads are in none.
fn demo_slice(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Kernel => None,
        Kind::Session => Some("/user.slice/user-1000.slice/user@1000.service/app.slice"),
        Kind::Container => Some("/machine.slice"),
        Kind::System => Some("/system.slice"),
    }
}

/// The demo workload at `time` for `count` base processes, with the extras `options` ask for.
/// `DemoOptions::default()` gives the workload every view has always had.
#[expect(clippy::too_many_lines, reason = "split in #90 (A31)")]
pub fn demo_with(time: f64, count: usize, options: DemoOptions) -> Snapshot {
    let names = [
        "init",
        "shell",
        "compiler",
        "browser",
        "renderer",
        "database",
        "worker",
        "language-server",
    ];
    let mut processes = Vec::with_capacity(count);
    for i in 0..count {
        let family = i / 16;
        let phase = time as f32 * 0.55 + i as f32 * 1.71;
        let cpu = ((phase.sin() * 0.5 + 0.5).powi(4) * (20.0 + (i % 9) as f32 * 24.0))
            + if i % 47 == 0 { 110.0 } else { 0.0 };
        let memory = (12.0 + (i * 71 % 900) as f32 * (0.8 + 0.2 * (phase * 0.2).sin())) * 1048576.0;
        let kind = [Kind::System, Kind::Session, Kind::Container, Kind::Kernel][family % 4];
        let group = format!("district-{:02}.service", family);
        let slice = demo_slice(kind);
        let io_rate = if i % 7 == 0 { cpu * 65536.0 } else { 0.0 };
        let write_rate = io_rate * demo_write_share(i);
        // Written bytes are the integral of write_rate, which is the CPU integral scaled alike.
        let written = if i % 7 == 0 {
            (demo_cpu_integral(time, i) * 65536.0 * demo_write_share(i) as f64) as u64
        } else {
            0
        };
        let (priority, nice) = demo_priority(i, kind);
        let stalled = (time * 0.3 + i as f64 * 2.1).sin() > 0.7;
        let core = ((i * 7 + (time / (5.0 + (i % 5) as f64)) as usize) % 16) as u32;
        let (files, locks_held, waiting_for) = demo_files(i, kind);
        // Exhaustive on purpose: a new field must be decided here.
        processes.push(Process {
            id: Identity {
                pid: 1000 + i as u32,
                start: i as u64 + 1,
            },
            parent: if i == 0 {
                0
            } else if i % 16 == 0 {
                1000
            } else {
                1000 + (family * 16) as u32
            },
            name: format!("{}-{}", names[i % names.len()], i),
            command: format!("{} --demo --instance {}", names[i % names.len()], i),
            cgroup: slice.map_or_else(String::new, |slice| format!("{slice}/{group}")),
            group,
            kind,
            state: if i % 41 == 0 {
                'Z'
            } else if i == 9 {
                'T'
            } else if i % 13 == 6 && kind != Kind::Kernel && cpu <= 30.0 && stalled {
                'D'
            } else if cpu > 30.0 {
                'R'
            } else {
                'S'
            },
            cpu,
            memory: memory as u64,
            io_rate: Some(io_rate),
            read_rate: Some(io_rate - write_rate),
            write_rate: Some(write_rate),
            written: Some(written),
            priority,
            nice,
            threads: demo_threads(i, options.exact_cpu_time),
            gpu_memory: (i % 37 == 5).then(|| GpuMemory::Bytes((300 + i as u64 * 7) << 20)),
            core: Some(core),
            cpu_time: if options.exact_cpu_time {
                demo_cpu_seconds(time, i) as f32
            } else {
                (time as f32 + i as f32) * cpu / 100.0 + i as f32 * 3.7
            },
            // Mostly on the kind of core it last ran on (0 to 7 are performance cores), with
            // some of its time on the other cluster; no share when it used no CPU.
            performance_share: (cpu > 0.0).then(|| {
                let mixing = 0.15 * (0.5 + 0.5 * (phase * 0.37).sin());
                if core < 8 { 1.0 - mixing } else { mixing }
            }),
            waiting: Some(1.5 * bounded(cpu, 80.0) * (0.6 + 0.4 * (phase * 0.8).cos())),
            files: Measured::Known(files),
            locks_held: Measured::Known(locks_held),
            // Holders come before their waiters, so a waiter's holder always exists.
            blocked_on: waiting_for.map(|holder| Blocker::Process(1000 + holder as u32)),
            system_share: (cpu > 0.0).then(|| demo_system_share(family, kind, phase)),
            policy: demo_policy(i, priority, nice),
            syscall_io: Some(demo_syscall_io(i, io_rate, write_rate)),
            major_faults: Some(demo_major_faults(i, phase)),
            started: DEMO_BOOT_TIME + (i as u64 + 1) as f64 * DEMO_START_SPACING,
        });
    }
    let id = |i: usize| processes[i % count].id;
    let links = (1..count)
        .filter(|i| i % 3 == 0)
        .map(|i| (id(i), id(i * 7 + 16), 1 + (i % 4) as u32))
        .collect();
    // Every third link is silent, a few carry megabytes, the rest kilobytes.
    let link_traffic = (1..count)
        .filter(|i| i % 3 == 0)
        .map(|i| {
            let pulse = ((time * 0.9 + i as f64 * 0.7).sin() as f32) * 0.5 + 0.5;
            let rate = match (i / 3) % 12 {
                0 | 3 | 6 | 9 => 0.0,
                7 => 1.0e6 * (1.0 + 2.0 * pulse),
                _ => 1024.0 * (1 + i % 5) as f32 * pulse,
            };
            ((id(i), id(i * 7 + 16)), rate)
        })
        .collect();
    // Links of four sockets have a Unix socket among them, including some silent TCP ones.
    let unix_links = (1..count)
        .filter(|i| i % 3 == 0 && i % 4 == 3)
        .map(|i| (id(i), id(i * 7 + 16)))
        .collect();
    let outside = (0..count)
        .filter(|i| i % 11 == 0)
        .map(|i| (id(i), 1 + (i % 3) as u32))
        .collect();
    let wave = |rate: f64, scale: f32| ((time * rate).sin() as f32).max(0.0) * scale;
    let cpus = (0..16)
        .map(|id| {
            let load = (time as f32 * 0.4 + id as f32 * 0.9).sin() * 0.5 + 0.5;
            Cpu {
                id,
                kind: if id < 8 {
                    CoreKind::Performance
                } else {
                    CoreKind::Efficiency
                },
                busy: Some(load * if id < 8 { 0.9 } else { 0.6 }),
                mhz: Some(if id < 8 {
                    1400.0 + 3400.0 * load
                } else {
                    1000.0 + 2200.0 * load
                }),
                wait: Some((load - 0.6).max(0.0) * 3.0),
            }
        })
        .collect();
    let mut units: HashMap<String, Unit> = HashMap::new();
    for p in processes.iter().filter(|p| !p.cgroup.is_empty()) {
        let unit = units.entry(p.cgroup.clone()).or_default();
        unit.memory += p.memory;
        *unit.cpu.get_or_insert(0.0) += p.cpu;
        *unit.pids.get_or_insert(0) += 1;
    }
    for (k, (path, unit)) in units.iter_mut().enumerate() {
        let family = path.trim_end_matches(".service").rsplit('-').next();
        let family: usize = family.and_then(|f| f.parse().ok()).unwrap_or(k);
        if family.is_multiple_of(3) {
            unit.memory_max = Some(unit.memory * 3 / 2);
        }
        if family % 4 == 1 {
            unit.cpu_max = Some(120.0);
        }
        unit.throttled = Some(unit.cpu_max.is_some() && unit.cpu.is_some_and(|cpu| cpu > 120.0));
        unit.pressure = [
            Some(wave(0.3 + family as f64 * 0.05, 40.0)),
            Some(wave(0.2, 10.0)),
            Some(0.0),
        ];
        unit.oom_kills = Some(if family == 4 { (time / 23.0) as u64 } else { 0 });
    }
    let remotes = processes
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 11 == 0)
        .flat_map(|(i, p)| {
            (0..1 + i % 3).map(move |k| {
                let (code, latitude, longitude, name) =
                    COUNTRIES[(i * 37 + k * 101) % COUNTRIES.len()];
                let pulse = ((time * 0.7 + i as f64 + k as f64) as f32).sin().max(0.0);
                Remote {
                    id: p.id,
                    address: IpAddr::from([203, 0, 113, (i + k) as u8]),
                    port: [443, 22, 5432][k % 3],
                    rtt: 20.0 + (i * 13 % 180) as f32,
                    up: Some(2048.0 * pulse),
                    down: Some(65536.0 * pulse * pulse),
                    place: (k < 2).then(|| Place {
                        latitude,
                        longitude,
                        name: format!("{name} ({code})"),
                    }),
                }
            })
        })
        .collect();
    if options.churn {
        // The units keep the limits they were given from the base processes, and the transients
        // add what they use, so a unit's pressure shows them.
        let transients: Vec<Process> = demo_listed(time, count)
            .iter()
            .map(|transient| transient.process(time, count))
            .collect();
        for transient in &transients {
            if let Some(unit) = units.get_mut(&transient.cgroup) {
                unit.memory += transient.memory;
                *unit.cpu.get_or_insert(0.0) += transient.cpu;
                *unit.pids.get_or_insert(0) += 1;
            }
        }
        for unit in units.values_mut() {
            unit.throttled =
                Some(unit.cpu_max.is_some() && unit.cpu.is_some_and(|cpu| cpu > 120.0));
        }
        processes.extend(transients);
    }
    Snapshot {
        processes,
        memory_total: Some(32 * 1024_u64.pow(3)),
        memory_available: Some(12 * 1024_u64.pow(3)),
        cores: 16,
        elapsed: time,
        pressure: [wave(0.21, 30.0), wave(0.13, 18.0), wave(0.17, 24.0)],
        links,
        link_traffic,
        unix_links,
        outside,
        cpus,
        units,
        remotes,
        unattributed_locks: 1,
        missing: Sources::default(),
        unreadable: 0,
        per_cluster: Sources::default(),
        shadows: Vec::new(),
        sampler: SamplerHealth {
            running: true,
            last_cycle: Some(Duration::ZERO),
            quiet: Duration::ZERO,
            scan_latency: Vec::new(),
        },
        home: Some(Place {
            latitude: 52.37,
            longitude: 4.9,
            name: "Amsterdam".into(),
        }),
        geo: "demo locations".into(),
    }
}

/// The demo workload every view has always had: `demo_with` and no options.
#[cfg(test)]
pub fn demo(time: f64, count: usize) -> Snapshot {
    demo_with(time, count, DemoOptions::default())
}

pub fn bytes(value: u64) -> String {
    let value = value as f64;
    if value >= 1073741824.0 {
        format!("{:.1} GiB", value / 1073741824.0)
    } else if value >= 1048576.0 {
        format!("{:.1} MiB", value / 1048576.0)
    } else {
        format!("{:.0} KiB", value / 1024.0)
    }
}

/// A short, human description of what a process is, for labels and the idle tour.
pub fn describe(process: &Process) -> String {
    const KERNEL: [(&str, &str); 14] = [
        ("kthreadd", "spawns every kernel thread"),
        ("kworker", "kernel worker thread"),
        ("ksoftirqd", "deferred interrupt work"),
        ("migration", "moves tasks between CPUs"),
        ("rcu", "RCU grace-period housekeeping"),
        ("kswapd", "reclaims memory under pressure"),
        ("kcompactd", "defragments memory"),
        ("khugepaged", "assembles huge pages"),
        ("irq/", "interrupt handler thread"),
        ("jbd2", "ext4 journal writer"),
        ("cpuhp", "CPU hotplug"),
        ("watchdog", "lockup detector"),
        ("oom_reaper", "frees memory of killed tasks"),
        ("nvidia", "NVIDIA driver thread"),
    ];
    // comm is truncated to 15 bytes, so entries match by prefix and the more specific come first.
    const KNOWN: [(&str, &str); 44] = [
        ("systemd-journal", "system log collector"),
        ("systemd-udevd", "device event manager"),
        ("systemd-logind", "login and seat manager"),
        ("systemd-resolve", "DNS resolver"),
        ("systemd-timesyn", "network clock sync"),
        ("systemd-oomd", "out-of-memory killer"),
        ("dbus-daemon", "message bus"),
        ("dbus-broker", "message bus"),
        ("NetworkManager", "network manager"),
        ("wpa_supplicant", "Wi-Fi authentication"),
        ("sshd", "SSH server"),
        ("cron", "job scheduler"),
        ("dockerd", "Docker engine"),
        ("containerd-shim", "supervises one container"),
        ("containerd", "container runtime"),
        ("pipewire-pulse", "PulseAudio compatibility"),
        ("pipewire", "audio and video server"),
        ("wireplumber", "PipeWire session manager"),
        ("Xwayland", "X11 compatibility server"),
        ("niri", "Wayland compositor"),
        ("sway", "Wayland compositor"),
        ("gnome-shell", "desktop shell"),
        ("kwin", "window manager"),
        ("ghostty", "terminal emulator"),
        ("kitty", "terminal emulator"),
        ("alacritty", "terminal emulator"),
        ("tmux", "terminal multiplexer"),
        ("zsh", "shell"),
        ("bash", "shell"),
        ("fish", "shell"),
        ("nvim", "text editor"),
        ("code", "VS Code"),
        ("firefox", "web browser"),
        ("chrome", "web browser"),
        ("opencode", "AI coding agent"),
        ("node", "Node.js runtime"),
        ("python", "Python interpreter"),
        ("cupsd", "print server"),
        ("avahi-daemon", "mDNS service discovery"),
        ("bluetoothd", "Bluetooth daemon"),
        ("polkitd", "privilege authorization"),
        ("gvfsd", "virtual filesystem daemon"),
        ("xdg-desktop-por", "desktop portal"),
        ("isotop", "you are here"),
    ];
    let name = process.name.as_str();
    let found = |table: &[(&str, &'static str)]| {
        table
            .iter()
            .find(|(prefix, _)| name.starts_with(prefix))
            .map(|&(_, text)| text)
    };
    let text = match (process.kind, name) {
        (Kind::Kernel, _) => found(&KERNEL).unwrap_or("kernel thread"),
        (_, "systemd") if process.id.pid == 1 => "init: the service manager",
        (_, "systemd") => "per-user service manager",
        (kind, _) => found(&KNOWN).unwrap_or(match kind {
            Kind::Session => "app in your session",
            Kind::Container => "container process",
            _ => "system service",
        }),
    };
    text.into()
}

/// Constructors shared by the tests of every module.
#[cfg(test)]
pub mod testing {
    use super::*;

    /// A quiet sleeping process with a 1 MiB footprint, a normal priority and one thread, in
    /// the group `g`; its background measurements are pending. Tests set what they care about
    /// with struct update.
    pub fn process(pid: u32, parent: u32) -> Process {
        Process {
            id: Identity {
                pid,
                start: pid as u64,
            },
            parent,
            name: format!("p{pid}"),
            group: "g".into(),
            memory: 1 << 20,
            priority: 20,
            threads: 1,
            cgroup: "/system.slice/g.service".into(),
            ..Process::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn resource_scale_is_monotonic_and_bounded_for_multicore_cpu() {
        let values: Vec<_> = [0.0, 1.0, 100.0, 1600.0, 100000.0]
            .map(|v| bounded(v, 60.0))
            .into();
        assert_eq!(values[0], 0.0);
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
    }

    #[test]
    fn demo_carries_every_data_source() {
        let snapshot = demo(30.0, 128);
        assert_eq!(snapshot.cpus.len(), 16);
        assert!(snapshot.units.len() >= 4);
        assert!(snapshot.remotes.iter().any(|r| r.place.is_some()));
        assert!(snapshot.remotes.iter().any(|r| r.place.is_none()));
        assert!(
            snapshot
                .processes
                .iter()
                .any(|p| p.core != snapshot.processes[0].core)
        );
    }

    #[test]
    fn io_read_and_write_rates_come_from_separate_counters() {
        let before = IoBytes {
            read: 4096,
            write: 1000,
        };
        let after = IoBytes {
            read: 4096 + 6000,
            write: 1000 + 2000,
        };
        assert_eq!(after.rates(before, 2.0), Some([4000.0, 3000.0, 1000.0]));
    }

    fn loopback(inode: u64, pid: u32, peer: u32, received: u64) -> platform::Loopback {
        platform::Loopback {
            inode,
            pid,
            peer,
            received,
        }
    }

    #[test]
    fn loopback_traffic_becomes_a_rate_from_counter_deltas_and_counts_each_byte_once() {
        let start = Instant::now();
        let mut previous = LoopbackScan::default();
        // Nothing is known about the first interval, so the pair stays unmeasured.
        let first = [loopback(1, 30, 20, 1000), loopback(2, 20, 30, 500)];
        assert!(traffic(&first, &mut previous, start).is_empty());
        // 30 received 4000 bytes and 20 received 1000 more; 5000 bytes crossed in 2 seconds.
        let second = [loopback(1, 30, 20, 5000), loopback(2, 20, 30, 1500)];
        let rates = traffic(&second, &mut previous, start + Duration::from_secs(2));
        assert_eq!(rates, HashMap::from([((20, 30), 2500.0)]));
        // The interval is floored at 0.1 s, and a new socket on a known pair counts all it has
        // received, because it was opened inside the interval.
        let third = [
            loopback(1, 30, 20, 5100),
            loopback(2, 20, 30, 1500),
            loopback(3, 20, 30, 777),
        ];
        let rates = traffic(&third, &mut previous, start + Duration::from_millis(2010));
        assert_eq!(rates, HashMap::from([((20, 30), 1000.0 + 7770.0)]));
    }

    #[test]
    fn a_loopback_connection_seen_once_counts_its_bytes_over_the_scan_interval() {
        let start = Instant::now();
        let mut previous = LoopbackScan::default();
        assert!(traffic(&[], &mut previous, start).is_empty());
        // A connection opened and closed between scans is seen once, with 5 MB received.
        let seen = [
            loopback(7, 40, 41, 5_000_000),
            loopback(8, 41, 40, 1_000_000),
        ];
        let rates = traffic(&seen, &mut previous, start + Duration::from_secs(2));
        assert_eq!(rates, HashMap::from([((40, 41), 3_000_000.0)]));
        // It is gone at the next scan, so the pair is absent again rather than idle.
        let rates = traffic(&[], &mut previous, start + Duration::from_secs(4));
        assert!(rates.is_empty());
    }

    #[test]
    fn a_loopback_socket_missed_by_scans_is_averaged_over_the_time_since_it_was_last_seen() {
        let start = Instant::now();
        let mut previous = LoopbackScan::default();
        let at = |seconds| start + Duration::from_secs(seconds);
        traffic(&[loopback(7, 40, 41, 0)], &mut previous, at(0));
        traffic(&[loopback(7, 40, 41, 900_000_000)], &mut previous, at(2));
        // One scan cannot see the socket, then it is back with 2 MB more, sent over the 4
        // seconds since it was last seen rather than the 2 since the previous scan.
        assert!(traffic(&[], &mut previous, at(4)).is_empty());
        let rates = traffic(&[loopback(7, 40, 41, 902_000_000)], &mut previous, at(6));
        assert_eq!(rates, HashMap::from([((40, 41), 500_000.0)]));
        // Two missed scans spread 3 MB over 6 seconds.
        assert!(traffic(&[], &mut previous, at(8)).is_empty());
        assert!(traffic(&[], &mut previous, at(10)).is_empty());
        let rates = traffic(&[loopback(7, 40, 41, 905_000_000)], &mut previous, at(12));
        assert_eq!(rates, HashMap::from([((40, 41), 500_000.0)]));
    }

    #[test]
    fn a_cgroup_that_stops_being_wanted_is_accounted_for_one_more_cycle() {
        let set = |paths: &[&str]| -> HashSet<String> {
            paths.iter().map(|path| path.to_string()).collect()
        };
        let mut previous = HashSet::new();
        assert_eq!(
            accounted(set(&["/a", "/b"]), &mut previous),
            set(&["/a", "/b"])
        );
        assert_eq!(accounted(set(&["/a"]), &mut previous), set(&["/a", "/b"]));
        assert_eq!(accounted(set(&["/a"]), &mut previous), set(&["/a"]));
        assert_eq!(
            accounted(set(&["/a", "/b"]), &mut previous),
            set(&["/a", "/b"])
        );
    }

    #[test]
    fn demo_read_and_write_rates_add_up_to_io_rate() {
        for time in [0.0, 3.3, 41.0] {
            for (i, p) in demo(time, 128).processes.iter().enumerate() {
                let (io, read, write) = (
                    p.io_rate.unwrap(),
                    p.read_rate.unwrap(),
                    p.write_rate.unwrap(),
                );
                let expected = if i % 7 == 0 { p.cpu * 65536.0 } else { 0.0 };
                assert_eq!(io, expected);
                assert!(read >= 0.0 && write >= 0.0);
                assert!((read + write - io).abs() <= io * 1e-6);
            }
        }
    }

    #[test]
    fn demo_written_bytes_never_decrease_and_start_at_zero() {
        let mut last = vec![0_u64; 64];
        for step in 0..300 {
            let snapshot = demo(step as f64 * 0.7, 64);
            for (before, p) in last.iter_mut().zip(&snapshot.processes) {
                let written = p.written.unwrap();
                assert!(written >= *before, "{} went backwards", p.name);
                *before = written;
            }
        }
        assert!(last.iter().any(|&written| written > 0));
        assert!(demo(0.0, 64).processes.iter().all(|p| p.written == Some(0)));
    }

    #[test]
    fn demo_written_bytes_grow_at_write_rate() {
        // Trapezoid integration of the sampled write_rate against the closed-form total.
        let (end, step) = (20.0, 0.05);
        for index in [0, 7, 14, 21, 49] {
            let rate = |time: f64| demo(time, 64).processes[index].write_rate.unwrap() as f64;
            let steps = (end / step) as usize;
            let integral: f64 = (0..steps)
                .map(|k| (rate(k as f64 * step) + rate((k + 1) as f64 * step)) / 2.0 * step)
                .sum();
            let written = demo(end, 64).processes[index].written.unwrap() as f64;
            assert!(
                (written - integral).abs() <= integral * 0.005,
                "process {index}: {written} vs {integral}"
            );
        }
    }

    #[test]
    fn background_reports_gpu_memory_and_file_locks_only_when_absent() {
        let table = Measured::Known(platform::Locks::default());
        assert!(background_gaps(true, &table).is_empty());
        assert!(background_gaps(true, &Measured::Pending).is_empty());
        assert_eq!(background_gaps(false, &table), Source::GpuMemory.into());
        assert_eq!(
            background_gaps(false, &Measured::Unreadable),
            Sources::from([Source::GpuMemory, Source::FileLocks])
        );
    }

    #[test]
    fn this_platform_reports_its_gaps_in_the_first_sample() {
        let mut collector = Collector::new(None, None);
        let Ok(snapshot) = collector.sample() else {
            return; // live mode is unavailable here (the macOS stub refuses)
        };
        // A sampler that has read the processes once, as the collector's had: on macOS whether
        // the clock is missing depends on whether they carried cycle counts.
        let mut sampler = platform::Sampler::new();
        let _ = sampler.processes();
        let gaps = sampler.missing();
        assert!(gaps.iter().all(|source| snapshot.missing.contains(source)));
        assert_eq!(snapshot.per_cluster, sampler.per_cluster());
        if cfg!(target_os = "linux") {
            // Linux reads the last CPU and socket traffic everywhere; what else it lacks
            // depends on the machine.
            assert!(!snapshot.missing.contains(Source::LastCpu));
            assert!(!snapshot.missing.contains(Source::SocketTraffic));
            assert!(snapshot.unreadable == 0 && snapshot.per_cluster.is_empty());
        }
    }

    /// A raw reading of the demo's first process with the given counters, in nanosecond ticks,
    /// with the derived fields unset as a platform leaves them.
    fn reading(ticks: u64, performance: Option<u64>, runnable: Option<u64>) -> RawProcess {
        RawProcess {
            process: Process {
                performance_share: None,
                waiting: None,
                io_rate: None,
                read_rate: None,
                write_rate: None,
                system_share: None,
                syscall_io: None,
                major_faults: None,
                ..demo(0.0, 1).processes[0].clone()
            },
            ticks,
            io: None,
            performance_ticks: performance,
            runnable_ticks: runnable,
            system_ticks: 0,
            syscalls: None,
            major_faults: None,
        }
    }

    const SECOND: u64 = 1_000_000_000;

    #[test]
    fn performance_share_is_the_share_of_the_intervals_cpu_time_on_performance_cores() {
        let (_, first) = measure(reading(SECOND, Some(SECOND / 2), None), None, 1e9, 1.0);
        let (process, second) = measure(
            reading(3 * SECOND, Some(2 * SECOND), None),
            Some(&first),
            1e9,
            1.0,
        );
        // 1.5 s of the 2 s since the first reading ran on performance cores.
        assert_eq!(process.performance_share, Some(0.75));
        let (idle, third) = measure(
            reading(3 * SECOND, Some(2 * SECOND), None),
            Some(&second),
            1e9,
            1.0,
        );
        assert_eq!(idle.performance_share, None, "no CPU time, no share");
        let (reset, _) = measure(reading(4 * SECOND, Some(0), None), Some(&third), 1e9, 1.0);
        assert_eq!(reset.performance_share, None, "a counter that went back");
        let (linux, _) = measure(reading(4 * SECOND, None, None), Some(&third), 1e9, 1.0);
        assert_eq!((linux.performance_share, linux.waiting), (None, None));
    }

    #[test]
    fn performance_share_and_waiting_need_a_previous_reading() {
        let (process, _) = measure(reading(SECOND, Some(SECOND), Some(SECOND)), None, 1e9, 1.0);
        assert_eq!((process.performance_share, process.waiting), (None, None));
    }

    #[test]
    fn waiting_counts_runnable_time_beyond_cpu_time_per_second_of_wall_time() {
        let at = |ticks, runnable, previous: Option<&Counters>| {
            measure(reading(ticks, None, Some(runnable)), previous, 1e9, 2.0)
        };
        let (_, first) = at(SECOND, 2 * SECOND, None);
        // Over 2 s the threads ran 1 s and were runnable 4 s, so waited 3 s: 1.5 on average.
        let (process, second) = at(2 * SECOND, 6 * SECOND, Some(&first));
        assert_eq!(process.waiting, Some(1.5));
        // A thread ran 2 s without being switched out, so the runnable total has not caught up
        // yet: nothing waited.
        let (running, third) = at(4 * SECOND, 6 * SECOND, Some(&second));
        assert_eq!(running.waiting, Some(0.0));
        // It is switched out and the runnable total catches up with the 2 s it ran plus 0.5 s
        // of waiting; only the waiting counts.
        let (caught_up, _) = at(4 * SECOND, 8_500_000_000, Some(&third));
        assert_eq!(caught_up.waiting, Some(0.25));
    }

    /// `reading` with the kernel-mode time and the cumulative syscall and fault counters set.
    fn counted(
        ticks: u64,
        system_ticks: u64,
        syscalls: SyscallCounts,
        major_faults: u64,
    ) -> RawProcess {
        RawProcess {
            system_ticks,
            syscalls: Some(syscalls),
            major_faults: Some(major_faults),
            ..reading(ticks, None, None)
        }
    }

    fn calls(read_chars: u64, write_chars: u64, reads: u64, writes: u64) -> SyscallCounts {
        SyscallCounts {
            read_chars,
            write_chars,
            reads,
            writes,
        }
    }

    #[test]
    fn syscall_io_and_major_fault_rates_come_from_counter_deltas() {
        let (first, counters) = measure(
            counted(SECOND, 0, calls(1000, 500, 10, 4), 20),
            None,
            1e9,
            2.0,
        );
        assert_eq!((first.syscall_io, first.major_faults), (None, None));
        let (second, _) = measure(
            counted(2 * SECOND, 0, calls(5000, 1500, 30, 14), 26),
            Some(&counters),
            1e9,
            2.0,
        );
        assert_eq!(
            second.syscall_io,
            Some(SyscallIo {
                read_chars: 2000.0,
                write_chars: 500.0,
                reads: 10.0,
                writes: 5.0
            })
        );
        assert_eq!(second.major_faults, Some(3.0));
    }

    #[test]
    fn counter_resets_read_as_unknown_rates_not_negative() {
        let (_, before) = measure(
            counted(10 * SECOND, 4 * SECOND, calls(9000, 9000, 90, 90), 50),
            None,
            1e9,
            1.0,
        );
        // Exec: every counter went back.
        let (exec, after) = measure(
            counted(SECOND, SECOND / 2, calls(100, 100, 1, 1), 2),
            Some(&before),
            1e9,
            1.0,
        );
        assert_eq!(exec.syscall_io, None);
        assert_eq!(exec.major_faults, None);
        assert_eq!(exec.system_share, None);
        // A single counter that wrapped while the CPU time did not is unknown as well.
        let (wrapped, _) = measure(
            counted(2 * SECOND, SECOND, calls(50, 400, 0, 5), 1),
            Some(&after),
            1e9,
            1.0,
        );
        assert_eq!(wrapped.syscall_io, None);
        assert_eq!(wrapped.major_faults, None);
        // Counters that only grew give rates again.
        let (grown, _) = measure(
            counted(3 * SECOND, SECOND, calls(150, 700, 2, 9), 4),
            Some(&after),
            1e9,
            1.0,
        );
        let io = grown.syscall_io.unwrap();
        assert_eq!((io.read_chars, io.write_chars), (50.0, 600.0));
        assert_eq!((io.reads, io.writes), (1.0, 8.0));
        assert_eq!(grown.major_faults, Some(2.0));
    }

    #[test]
    fn system_share_is_the_kernel_part_of_the_intervals_cpu_time() {
        let at = |ticks, system, previous: Option<&Counters>| {
            measure(
                counted(ticks, system, SyscallCounts::default(), 0),
                previous,
                1e9,
                1.0,
            )
        };
        let (_, first) = at(2 * SECOND, SECOND, None);
        // 4 s of CPU time passed, 1 s of it in the kernel.
        let (process, _) = at(6 * SECOND, 2 * SECOND, Some(&first));
        assert_eq!(process.system_share, Some(0.25));
        // Rounding in a unit conversion can put the kernel part above the total.
        let (capped, _) = at(3 * SECOND, 4 * SECOND, Some(&first));
        assert_eq!(capped.system_share, Some(1.0));
    }

    #[test]
    fn system_share_is_none_until_cpu_time_elapses() {
        let at = |ticks, system, previous: Option<&Counters>| {
            measure(
                counted(ticks, system, SyscallCounts::default(), 0),
                previous,
                1e9,
                1.0,
            )
        };
        let (first, counters) = at(2 * SECOND, SECOND, None);
        assert_eq!(first.system_share, None, "no previous sample");
        let (idle, _) = at(2 * SECOND, SECOND, Some(&counters));
        assert_eq!(idle.system_share, None, "no CPU time elapsed");
    }

    #[test]
    fn demo_fills_every_new_field() {
        let snapshot = demo(12.0, 128);
        let policies = |wanted: Policy| {
            snapshot
                .processes
                .iter()
                .filter(move |p| p.policy == wanted)
        };
        for process in &snapshot.processes {
            assert!(process.syscall_io.is_some() && process.major_faults.is_some());
            assert!(process.started >= DEMO_BOOT_TIME);
            assert_eq!(
                process.system_share.is_some(),
                process.cpu > 0.0,
                "{}",
                process.name
            );
            if let Some(share) = process.system_share {
                assert!((0.0..=1.0).contains(&share));
            }
            let io = process.syscall_io.unwrap();
            assert!(
                io.read_chars >= process.read_rate.unwrap(),
                "{}",
                process.name
            );
            assert!(io.write_chars >= process.write_rate.unwrap());
            assert_eq!(process.policy == Policy::Fifo, process.priority == -51);
            assert_eq!(
                matches!(process.policy, Policy::Batch | Policy::Idle),
                process.nice == 19
            );
        }
        assert!(policies(Policy::Fifo).count() > 0);
        assert!(policies(Policy::Batch).count() > 0 && policies(Policy::Idle).count() > 0);
        assert!(policies(Policy::Normal).count() > 0);
        assert!(
            snapshot
                .processes
                .iter()
                .any(|p| p.major_faults > Some(0.0))
        );
        let busy = |p: &&Process| p.syscall_io.is_some_and(|io| io.reads > 0.0);
        assert!(snapshot.processes.iter().filter(busy).count() > 0);
        let starts: Vec<f64> = snapshot.processes.iter().map(|p| p.started).collect();
        assert!(starts.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(snapshot.missing.is_empty());
    }

    #[test]
    fn the_demo_keeps_its_new_fields_to_closed_forms_of_time_and_index() {
        // The same index at the same time reads the same however many processes surround it.
        let small = demo(12.0, 20);
        let large = demo(12.0, 128);
        for (a, b) in small.processes.iter().zip(&large.processes) {
            assert_eq!(a.policy, b.policy);
            assert_eq!(a.system_share, b.system_share);
            assert_eq!(a.syscall_io, b.syscall_io);
            assert_eq!(a.major_faults, b.major_faults);
            assert_eq!(a.started, b.started);
        }
    }

    #[test]
    fn waiting_starts_over_when_exec_resets_the_counters() {
        let at = |ticks, runnable, previous: Option<&Counters>| {
            measure(reading(ticks, None, Some(runnable)), previous, 1e9, 2.0)
        };
        let (_, first) = at(10 * SECOND, 20 * SECOND, None);
        // Exec kept only one thread's counters, so both totals went back.
        let (exec, second) = at(2 * SECOND, 3 * SECOND, Some(&first));
        assert_eq!(exec.waiting, None);
        // The new image waits 2 s over 2 s, measured from its own totals.
        let (after, _) = at(3 * SECOND, 6 * SECOND, Some(&second));
        assert_eq!(after.waiting, Some(1.0));
    }

    #[test]
    fn waiting_never_exceeds_the_threads_there_were() {
        let at = |ticks, runnable, previous: Option<&Counters>| {
            let mut raw = reading(ticks, None, Some(runnable));
            raw.process.threads = 1;
            measure(raw, previous, 1e9, 1.0)
        };
        // First seen while its one thread had run 60 s without a switch, so the runnable total
        // lags by that much; when the thread is switched out it catches up all at once.
        let (_, first) = at(60 * SECOND, SECOND, None);
        let (process, _) = at(61 * SECOND, 62 * SECOND, Some(&first));
        assert_eq!(process.waiting, Some(1.0));
    }

    #[test]
    fn demo_scheduling_classes_states_and_traffic_cover_the_coop_cases() {
        let mut stalled = false;
        for step in 0..200 {
            let snapshot = demo(step as f64 * 1.3, 128);
            assert!(snapshot.missing.is_empty() && snapshot.unreadable == 0);
            for (i, p) in snapshot.processes.iter().enumerate() {
                assert_eq!(p.state == 'Z', i % 41 == 0);
                assert_eq!(p.state == 'T', i == 9);
                if p.state == 'D' {
                    stalled = true;
                    assert!(p.kind != Kind::Kernel && p.cpu <= 30.0);
                }
            }
        }
        assert!(stalled);
        let snapshot = demo(30.0, 128);
        let processes = &snapshot.processes;
        let count = |wanted: (i32, i32)| {
            processes
                .iter()
                .filter(|p| (p.priority, p.nice) == wanted)
                .count()
        };
        assert!(count((20, 0)) > processes.len() / 2);
        assert!(count((39, 19)) > 0 && count((-51, 0)) > 0 && count((0, -20)) > 0);
        assert!(
            processes
                .iter()
                .filter(|p| p.priority == 0)
                .all(|p| p.kind == Kind::Kernel)
        );
        let rates: Vec<f32> = snapshot.link_traffic.values().copied().collect();
        assert!(rates.contains(&0.0));
        assert!(rates.iter().any(|&rate| rate > 0.0 && rate < 100_000.0));
        assert!(rates.iter().any(|&rate| rate >= 1.0e6));
        assert_eq!(demo(30.0, 128).link_traffic, snapshot.link_traffic);
        // Unix links are links with room for a second socket, one of them over a silent TCP link.
        let links: HashMap<(Identity, Identity), u32> = snapshot
            .links
            .iter()
            .map(|&(a, b, count)| ((a, b), count))
            .collect();
        assert!(!snapshot.unix_links.is_empty());
        assert!(snapshot.unix_links.iter().all(|pair| links[pair] >= 2));
        assert!(
            snapshot
                .unix_links
                .iter()
                .any(|pair| snapshot.link_traffic[pair] == 0.0)
        );
    }

    #[test]
    fn demo_files_and_locks_hold_still_and_cover_the_coop_cases() {
        let files = |snapshot: &Snapshot| {
            snapshot
                .processes
                .iter()
                .map(|p| (p.files.clone(), p.locks_held.clone(), p.blocked_on))
                .collect::<Vec<_>>()
        };
        let snapshot = demo(30.0, 192);
        assert_eq!(files(&demo(3.0, 192)), files(&snapshot));
        let processes = &snapshot.processes;
        let open: Vec<u32> = processes
            .iter()
            .filter(|p| p.kind != Kind::Kernel)
            .map(|p| p.files.known().unwrap().open)
            .collect();
        assert!(open.iter().all(|&n| (3..=400).contains(&n)));
        assert!(open.iter().any(|&n| n < 8) && open.iter().any(|&n| n > 200));
        assert!(
            processes
                .iter()
                .filter(|p| p.kind == Kind::Kernel)
                .all(|p| p.files == Measured::Known(platform::Files::default())
                    && p.locks_held == Measured::Known(0))
        );
        let deleted: usize = processes
            .iter()
            .map(|p| p.files.known().unwrap().deleted.len())
            .sum();
        assert_eq!(deleted, 3);
        assert!(processes.iter().any(|p| {
            let files = p.files.known().unwrap();
            files.deleted.len() == 1 && files.deleted_bytes() == 1_288_490_189
        }));
        let holders = processes
            .iter()
            .filter(|p| p.locks_held.known().is_some_and(|&held| held > 0))
            .count();
        assert_eq!(holders, 4);
        let waiters: Vec<&Process> = processes
            .iter()
            .filter(|p| p.blocked_on.is_some())
            .collect();
        assert_eq!(waiters.len(), 2);
        for waiter in waiters {
            let holder = processes
                .iter()
                .find(|p| waiter.blocked_on == Some(Blocker::Process(p.id.pid)))
                .unwrap();
            assert!(holder.locks_held.known().is_some_and(|&held| held > 0));
            assert_ne!(holder.cgroup, waiter.cgroup, "waits in another flock");
        }
    }

    /// A raw reading of the demo's first process with cumulative I/O counters.
    fn reading_with_io(ticks: u64, io: IoBytes) -> RawProcess {
        RawProcess {
            io: Some(io),
            ..reading(ticks, None, None)
        }
    }

    fn socket(inode: u64, sent: u64, received: u64) -> platform::Remote {
        platform::Remote {
            pid: 1000,
            inode,
            address: IpAddr::from([203, 0, 113, 7]),
            port: 443,
            rtt: 20_000,
            sent,
            received,
        }
    }

    #[test]
    fn first_sample_rates_are_unknown_not_zero() {
        let counters = IoBytes {
            read: 4096,
            write: 8192,
        };
        let (first, kept) = measure(reading_with_io(SECOND, counters), None, 1e9, 1.0);
        assert_eq!(
            (first.io_rate, first.read_rate, first.write_rate),
            (None, None, None),
            "a process seen for the first time has no I/O rate yet"
        );
        let more = IoBytes {
            read: 4096,
            write: 8192 + 2048,
        };
        let (second, _) = measure(reading_with_io(2 * SECOND, more), Some(&kept), 1e9, 1.0);
        assert_eq!(second.io_rate, Some(2048.0));
        assert_eq!(second.read_rate, Some(0.0), "idle is a measured zero");
        // A connection seen for the first time has no byte rate either; the second scan has.
        let mut scans = HashMap::new();
        let first = remotes(&[socket(5, 1000, 4000)], &mut scans, None);
        assert_eq!((first[0].up, first[0].down), (None, None));
        let second = remotes(&[socket(5, 1000, 4500)], &mut scans, None);
        assert_eq!(second[0].up, Some(0.0));
        assert!(second[0].down.is_some_and(|rate| rate > 0.0));
        let fresh = remotes(&[socket(5, 1000, 4500), socket(6, 0, 0)], &mut scans, None);
        assert_eq!((fresh[1].up, fresh[1].down), (None, None), "a new socket");
    }

    #[test]
    fn counter_reversal_reads_as_unknown() {
        let before = IoBytes {
            read: 4096,
            write: 1000,
        };
        let after = IoBytes {
            read: 4096 + 6000,
            write: 1000 + 2000,
        };
        assert_eq!(before.rates(after, 2.0), None, "a recycled pid's reset");
        // Only one counter going back leaves the others unmeasurable too: the total is wrong.
        let write_reset = IoBytes {
            read: 4096 + 6000,
            write: 0,
        };
        assert_eq!(write_reset.rates(after, 2.0), None);
        let (_, kept) = measure(reading_with_io(SECOND, after), None, 1e9, 1.0);
        let (process, _) = measure(reading_with_io(2 * SECOND, before), Some(&kept), 1e9, 1.0);
        assert_eq!(
            (process.io_rate, process.read_rate, process.write_rate),
            (None, None, None)
        );
        // A socket whose counters went back reads as unknown in that direction only.
        let mut scans = HashMap::new();
        remotes(&[socket(5, 9000, 4000)], &mut scans, None);
        let next = remotes(&[socket(5, 100, 4800)], &mut scans, None);
        assert_eq!(next[0].up, None);
        assert!(next[0].down.is_some());
    }

    #[test]
    fn gpu_memory_distinguishes_unreported_from_absent() {
        let mut usage = HashMap::new();
        GpuMemory::accumulate(&mut usage, 10, None);
        GpuMemory::accumulate(&mut usage, 20, Some(4096));
        GpuMemory::accumulate(&mut usage, 20, None);
        GpuMemory::accumulate(&mut usage, 30, None);
        GpuMemory::accumulate(&mut usage, 30, Some(1 << 20));
        GpuMemory::accumulate(&mut usage, 30, Some(1 << 20));
        GpuMemory::accumulate(&mut usage, 40, Some(0));
        assert_eq!(usage.get(&10), Some(&GpuMemory::Unreported));
        assert_eq!(usage.get(&20), Some(&GpuMemory::Bytes(4096)));
        assert_eq!(usage.get(&30), Some(&GpuMemory::Bytes(2 << 20)));
        assert_eq!(usage.get(&40), Some(&GpuMemory::Bytes(0)));
        assert_eq!(usage.get(&50), None, "a process the driver never listed");
        assert_eq!(GpuMemory::Unreported.bytes(), 0);
        // The demo lists some processes and leaves the rest absent.
        let demo = demo(1.0, 64);
        let using: Vec<_> = demo
            .processes
            .iter()
            .filter(|p| p.gpu_memory.is_some())
            .collect();
        assert!(!using.is_empty() && using.len() < demo.processes.len());
        assert!(
            using
                .iter()
                .all(|p| matches!(p.gpu_memory, Some(GpuMemory::Bytes(bytes)) if bytes > 0))
        );
    }

    #[test]
    fn a_process_the_file_scan_has_not_reached_is_pending_not_unreadable() {
        let mut process = demo(0.0, 2).processes[1].clone();
        let locks = Measured::Known(platform::Locks {
            held: HashMap::from([(1001, 2)]),
            ..platform::Locks::default()
        });
        let scanned = platform::Files {
            open: 4,
            ..platform::Files::default()
        };
        let mut found = HashMap::new();
        merge_files(&mut process, &found, &Measured::Pending, true);
        assert_eq!(process.files, Measured::Pending, "not reached");
        assert_eq!(
            process.locks_held,
            Measured::Pending,
            "before the first pass"
        );
        found.insert(1001, None);
        merge_files(&mut process, &found, &Measured::Unreadable, true);
        assert_eq!(process.files, Measured::Unreadable);
        assert_eq!(process.locks_held, Measured::Unreadable);
        found.insert(1001, Some(scanned.clone()));
        merge_files(&mut process, &found, &locks, true);
        assert_eq!(process.files, Measured::Known(scanned));
        assert_eq!(process.locks_held, Measured::Known(2));
    }

    #[test]
    fn kernel_threads_keep_their_empty_table_and_kernel_task_takes_the_scan() {
        let scanned = platform::Files {
            open: 3,
            ..platform::Files::default()
        };
        let found = HashMap::from([(0, Some(scanned.clone())), (2, Some(scanned.clone()))]);
        let mut thread = demo(0.0, 1).processes[0].clone();
        thread.kind = Kind::Kernel;
        thread.id.pid = 2;
        thread.files = Measured::Known(platform::Files::default());
        merge_files(&mut thread, &found, &Measured::Pending, true);
        assert_eq!(thread.files, Measured::Known(platform::Files::default()));
        // kernel_task on macOS comes from the sampler not read yet, like every process there.
        let mut task = thread.clone();
        task.id.pid = 0;
        task.files = Measured::Pending;
        merge_files(&mut task, &found, &Measured::Pending, true);
        assert_eq!(task.files, Measured::Known(scanned));
        task.files = Measured::Pending;
        merge_files(
            &mut task,
            &HashMap::from([(0, None)]),
            &Measured::Pending,
            true,
        );
        assert_eq!(task.files, Measured::Unreadable, "without root");
    }

    #[test]
    fn background_data_is_not_inherited_by_a_reused_pid() {
        let open = |count| {
            Some(platform::Files {
                open: count,
                ..platform::Files::default()
            })
        };
        // The cycle scanned against pid 7 as the process that started at 70, and pid 8.
        let extras = Extras {
            identities: HashMap::from([
                (7, Identity { pid: 7, start: 70 }),
                (8, Identity { pid: 8, start: 8 }),
            ]),
            network: platform::Network {
                links: HashMap::from([((7, 8), 1)]),
                unix: HashSet::from([(7, 8)]),
                outside: HashMap::from([(7, 2), (8, 1)]),
                ..platform::Network::default()
            },
            gpu: HashMap::from([(7, GpuMemory::Bytes(500)), (8, GpuMemory::Bytes(600))]),
            remotes: [7, 8]
                .map(|pid| Remote {
                    id: Identity { pid, start: 0 },
                    address: IpAddr::from([203, 0, 113, 1]),
                    port: 443,
                    rtt: 10.0,
                    up: Some(1.0),
                    down: Some(1.0),
                    place: None,
                })
                .into(),
            traffic: HashMap::from([((7, 8), 100.0)]),
            files: HashMap::from([(7, open(4)), (8, open(2))]),
            locks: Measured::Known(platform::Locks {
                held: HashMap::from([(7, 1)]),
                blocked: HashMap::from([(8, Blocker::Process(7))]),
                unattributed: 0,
            }),
            ..Extras::default()
        };
        let sample = |start_of_7| {
            let mut snapshot = Snapshot {
                processes: vec![
                    Process {
                        id: Identity {
                            pid: 7,
                            start: start_of_7,
                        },
                        ..testing::process(7, 1)
                    },
                    testing::process(8, 1),
                ],
                ..Snapshot::default()
            };
            merge(&mut snapshot, &extras);
            snapshot
        };
        // Pid 7 now belongs to a process that started at 71.
        let reused = sample(71);
        let (newcomer, survivor) = (&reused.processes[0], &reused.processes[1]);
        assert_eq!(newcomer.gpu_memory, None);
        assert_eq!(newcomer.files, Measured::Pending);
        assert_eq!(newcomer.locks_held, Measured::Pending);
        assert_eq!(survivor.gpu_memory, Some(GpuMemory::Bytes(600)));
        assert_eq!(survivor.files, Measured::Known(open(2).unwrap()));
        assert_eq!(survivor.blocked_on, None, "its lock's holder has exited");
        assert!(reused.links.is_empty() && reused.unix_links.is_empty());
        assert!(reused.link_traffic.is_empty());
        assert_eq!(
            reused.outside,
            HashMap::from([(survivor.id, 1)]),
            "the old owner's connections are dropped"
        );
        assert_eq!(
            reused.remotes.iter().map(|r| r.id).collect::<Vec<_>>(),
            [survivor.id]
        );
        // The process the cycle scanned is still there: everything joins.
        let same = sample(70);
        let (scanned, other) = (&same.processes[0], &same.processes[1]);
        assert_eq!(scanned.gpu_memory, Some(GpuMemory::Bytes(500)));
        assert_eq!(scanned.files, Measured::Known(open(4).unwrap()));
        assert_eq!(scanned.locks_held, Measured::Known(1));
        assert_eq!(other.blocked_on, Some(Blocker::Process(7)));
        assert_eq!(same.links, [(scanned.id, other.id, 1)]);
        assert_eq!(same.link_traffic.len(), 1);
        assert_eq!(same.remotes.len(), 2);
    }

    #[test]
    fn a_reused_pid_does_not_keep_its_predecessors_open_files() {
        let table = |open| {
            Some(platform::Files {
                open,
                ..platform::Files::default()
            })
        };
        let first = HashMap::from([
            (7, Identity { pid: 7, start: 70 }),
            (8, Identity { pid: 8, start: 8 }),
        ]);
        let mut kept = KeptFiles::default();
        let files = kept.update(HashMap::from([(7, table(4)), (8, table(2))]), &first);
        assert_eq!(files.len(), 2);
        // The scan did not reach either pid this cycle; pid 7 now names a process that started
        // at 71, pid 8 the same process as before.
        let second = HashMap::from([
            (7, Identity { pid: 7, start: 71 }),
            (8, Identity { pid: 8, start: 8 }),
        ]);
        let files = kept.update(HashMap::new(), &second);
        assert_eq!(files.get(&7), None, "the predecessor's table is dropped");
        assert_eq!(
            files.get(&8),
            Some(&table(2)),
            "kept until the scan comes round"
        );
        // A table scanned for a pid the cycle had no identity for is not kept.
        let files = kept.update(HashMap::from([(9, table(1))]), &second);
        assert_eq!(files.get(&9), None);
        let files = kept.update(HashMap::from([(7, table(5))]), &second);
        assert_eq!(files.get(&7), Some(&table(5)));
    }

    /// The health of `background` once its thread has had `within` to end.
    fn settled(background: &Background, within: Duration) -> SamplerHealth {
        let deadline = Instant::now() + within;
        while background.alive.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        background.health(Instant::now())
    }

    #[test]
    fn a_dead_sampler_thread_is_reported() {
        let healthy = Background::spawn(|| {
            || Extras {
                latency: vec![(Source::Sockets, Duration::from_millis(3))],
                ..Extras::default()
            }
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut health = healthy.health(Instant::now());
        while health.last_cycle.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            health = healthy.health(Instant::now());
        }
        assert!(health.running && !health.stalled());
        assert!(health.last_cycle.is_some_and(|age| age < BACKGROUND_PERIOD));
        assert_eq!(
            health.scan_latency,
            [(Source::Sockets, Duration::from_millis(3))]
        );

        // A scan that panics ends the thread.
        let panicked = Background::spawn(|| || -> Extras { panic!("a scan failed") }).unwrap();
        let health = settled(&panicked, Duration::from_secs(5));
        assert!(!health.running);
        assert_eq!(health.last_cycle, None);

        // A poisoned lock stops it too, and reads as stopped at once.
        let poisoned = Arc::clone(&healthy.extras);
        let _ = std::thread::spawn(move || {
            let _held = poisoned.lock();
            panic!("poisoned while publishing");
        })
        .join();
        let frozen = healthy.health(Instant::now());
        assert!(!frozen.running);
        assert_eq!(
            frozen.scan_latency,
            [(Source::Sockets, Duration::from_millis(3))],
            "what it measured last is kept"
        );

        // A collector whose thread could not be spawned has no background and reports so.
        assert!(!SamplerHealth::default().running);

        let stuck = SamplerHealth {
            running: true,
            last_cycle: Some(BACKGROUND_PERIOD * (STALLED_PERIODS + 1)),
            quiet: BACKGROUND_PERIOD * (STALLED_PERIODS + 1),
            scan_latency: Vec::new(),
        };
        assert!(stuck.stalled());
    }

    #[test]
    fn a_first_cycle_that_never_finishes_counts_as_stalled() {
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let hung = Background::spawn(move || {
            move || {
                let _ = blocked.recv();
                Extras::default()
            }
        })
        .unwrap();
        let soon = hung.health(Instant::now());
        assert!(soon.running && !soon.stalled());
        assert_eq!(soon.last_cycle, None);
        let later = hung.health(hung.started + BACKGROUND_PERIOD * (STALLED_PERIODS + 1));
        assert_eq!(later.last_cycle, None, "no cycle has finished");
        assert!(later.stalled(), "{later:?}");
        drop(release);
    }

    #[test]
    fn default_process_is_a_harmless_placeholder() {
        let process = Process::default();
        assert_eq!(
            process.id.pid, 0,
            "pid 0 is never looked up as a real process"
        );
        assert_eq!(process.kind, Kind::System);
        assert_eq!(process.files, Measured::Pending);
        let hub = Process::placeholder(Identity { pid: 7, start: 1 });
        assert_eq!((hub.id.pid, hub.kind), (7, Kind::System));
        assert_eq!(hub.files, Measured::Unreadable);
        assert_eq!(hub.locks_held, Measured::Unreadable);
    }

    /// A verbatim copy of `demo` as it was before `DemoOptions`, which the default options must
    /// keep reproducing.
    #[expect(clippy::too_many_lines, reason = "split in #90 (A31)")]
    fn original_demo(time: f64, count: usize) -> Snapshot {
        let names = [
            "init",
            "shell",
            "compiler",
            "browser",
            "renderer",
            "database",
            "worker",
            "language-server",
        ];
        let mut processes = Vec::with_capacity(count);
        for i in 0..count {
            let family = i / 16;
            let phase = time as f32 * 0.55 + i as f32 * 1.71;
            let cpu = ((phase.sin() * 0.5 + 0.5).powi(4) * (20.0 + (i % 9) as f32 * 24.0))
                + if i % 47 == 0 { 110.0 } else { 0.0 };
            let memory =
                (12.0 + (i * 71 % 900) as f32 * (0.8 + 0.2 * (phase * 0.2).sin())) * 1048576.0;
            let kind = [Kind::System, Kind::Session, Kind::Container, Kind::Kernel][family % 4];
            let group = format!("district-{:02}.service", family);
            let slice = match kind {
                Kind::Kernel => None,
                Kind::Session => Some("/user.slice/user-1000.slice/user@1000.service/app.slice"),
                Kind::Container => Some("/machine.slice"),
                Kind::System => Some("/system.slice"),
            };
            let io_rate = if i % 7 == 0 { cpu * 65536.0 } else { 0.0 };
            let write_rate = io_rate * demo_write_share(i);
            // Written bytes are the integral of write_rate, which is the CPU integral scaled alike.
            let written = if i % 7 == 0 {
                (demo_cpu_integral(time, i) * 65536.0 * demo_write_share(i) as f64) as u64
            } else {
                0
            };
            let (priority, nice) = if kind == Kind::Kernel && i % 3 == 0 {
                (0, -20)
            } else if i % 61 == 17 {
                (-51, 0)
            } else if i % 6 == 4 {
                (39, 19)
            } else {
                (20, 0)
            };
            let stalled = (time * 0.3 + i as f64 * 2.1).sin() > 0.7;
            let core = ((i * 7 + (time / (5.0 + (i % 5) as f64)) as usize) % 16) as u32;
            let (files, locks_held, waiting_for) = demo_files(i, kind);
            // Exhaustive on purpose: a new field must be decided here.
            processes.push(Process {
                id: Identity {
                    pid: 1000 + i as u32,
                    start: i as u64 + 1,
                },
                parent: if i == 0 {
                    0
                } else if i % 16 == 0 {
                    1000
                } else {
                    1000 + (family * 16) as u32
                },
                name: format!("{}-{}", names[i % names.len()], i),
                command: format!("{} --demo --instance {}", names[i % names.len()], i),
                cgroup: slice.map_or_else(String::new, |slice| format!("{slice}/{group}")),
                group,
                kind,
                state: if i % 41 == 0 {
                    'Z'
                } else if i == 9 {
                    'T'
                } else if i % 13 == 6 && kind != Kind::Kernel && cpu <= 30.0 && stalled {
                    'D'
                } else if cpu > 30.0 {
                    'R'
                } else {
                    'S'
                },
                cpu,
                memory: memory as u64,
                io_rate: Some(io_rate),
                read_rate: Some(io_rate - write_rate),
                write_rate: Some(write_rate),
                written: Some(written),
                priority,
                nice,
                threads: if i % 5 == 0 { 4 << (i % 6) } else { 1 },
                gpu_memory: (i % 37 == 5).then(|| GpuMemory::Bytes((300 + i as u64 * 7) << 20)),
                core: Some(core),
                cpu_time: (time as f32 + i as f32) * cpu / 100.0 + i as f32 * 3.7,
                // Mostly on the kind of core it last ran on (0 to 7 are performance cores), with
                // some of its time on the other cluster; no share when it used no CPU.
                performance_share: (cpu > 0.0).then(|| {
                    let mixing = 0.15 * (0.5 + 0.5 * (phase * 0.37).sin());
                    if core < 8 { 1.0 - mixing } else { mixing }
                }),
                waiting: Some(1.5 * bounded(cpu, 80.0) * (0.6 + 0.4 * (phase * 0.8).cos())),
                files: Measured::Known(files),
                locks_held: Measured::Known(locks_held),
                // Holders come before their waiters, so a waiter's holder always exists.
                blocked_on: waiting_for.map(|holder| Blocker::Process(1000 + holder as u32)),
                system_share: (cpu > 0.0).then(|| demo_system_share(family, kind, phase)),
                policy: demo_policy(i, priority, nice),
                syscall_io: Some(demo_syscall_io(i, io_rate, write_rate)),
                major_faults: Some(demo_major_faults(i, phase)),
                started: DEMO_BOOT_TIME + (i as u64 + 1) as f64 * DEMO_START_SPACING,
            });
        }
        let id = |i: usize| processes[i % count].id;
        let links = (1..count)
            .filter(|i| i % 3 == 0)
            .map(|i| (id(i), id(i * 7 + 16), 1 + (i % 4) as u32))
            .collect();
        // Every third link is silent, a few carry megabytes, the rest kilobytes.
        let link_traffic = (1..count)
            .filter(|i| i % 3 == 0)
            .map(|i| {
                let pulse = ((time * 0.9 + i as f64 * 0.7).sin() as f32) * 0.5 + 0.5;
                let rate = match (i / 3) % 12 {
                    0 | 3 | 6 | 9 => 0.0,
                    7 => 1.0e6 * (1.0 + 2.0 * pulse),
                    _ => 1024.0 * (1 + i % 5) as f32 * pulse,
                };
                ((id(i), id(i * 7 + 16)), rate)
            })
            .collect();
        // Links of four sockets have a Unix socket among them, including some silent TCP ones.
        let unix_links = (1..count)
            .filter(|i| i % 3 == 0 && i % 4 == 3)
            .map(|i| (id(i), id(i * 7 + 16)))
            .collect();
        let outside = (0..count)
            .filter(|i| i % 11 == 0)
            .map(|i| (id(i), 1 + (i % 3) as u32))
            .collect();
        let wave = |rate: f64, scale: f32| ((time * rate).sin() as f32).max(0.0) * scale;
        let cpus = (0..16)
            .map(|id| {
                let load = (time as f32 * 0.4 + id as f32 * 0.9).sin() * 0.5 + 0.5;
                Cpu {
                    id,
                    kind: if id < 8 {
                        CoreKind::Performance
                    } else {
                        CoreKind::Efficiency
                    },
                    busy: Some(load * if id < 8 { 0.9 } else { 0.6 }),
                    mhz: Some(if id < 8 {
                        1400.0 + 3400.0 * load
                    } else {
                        1000.0 + 2200.0 * load
                    }),
                    wait: Some((load - 0.6).max(0.0) * 3.0),
                }
            })
            .collect();
        let mut units: HashMap<String, Unit> = HashMap::new();
        for p in processes.iter().filter(|p| !p.cgroup.is_empty()) {
            let unit = units.entry(p.cgroup.clone()).or_default();
            unit.memory += p.memory;
            *unit.cpu.get_or_insert(0.0) += p.cpu;
            *unit.pids.get_or_insert(0) += 1;
        }
        for (k, (path, unit)) in units.iter_mut().enumerate() {
            let family = path.trim_end_matches(".service").rsplit('-').next();
            let family: usize = family.and_then(|f| f.parse().ok()).unwrap_or(k);
            if family.is_multiple_of(3) {
                unit.memory_max = Some(unit.memory * 3 / 2);
            }
            if family % 4 == 1 {
                unit.cpu_max = Some(120.0);
            }
            unit.throttled =
                Some(unit.cpu_max.is_some() && unit.cpu.is_some_and(|cpu| cpu > 120.0));
            unit.pressure = [
                Some(wave(0.3 + family as f64 * 0.05, 40.0)),
                Some(wave(0.2, 10.0)),
                Some(0.0),
            ];
            unit.oom_kills = Some(if family == 4 { (time / 23.0) as u64 } else { 0 });
        }
        let remotes = processes
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 11 == 0)
            .flat_map(|(i, p)| {
                (0..1 + i % 3).map(move |k| {
                    let (code, latitude, longitude, name) =
                        COUNTRIES[(i * 37 + k * 101) % COUNTRIES.len()];
                    let pulse = ((time * 0.7 + i as f64 + k as f64) as f32).sin().max(0.0);
                    Remote {
                        id: p.id,
                        address: IpAddr::from([203, 0, 113, (i + k) as u8]),
                        port: [443, 22, 5432][k % 3],
                        rtt: 20.0 + (i * 13 % 180) as f32,
                        up: Some(2048.0 * pulse),
                        down: Some(65536.0 * pulse * pulse),
                        place: (k < 2).then(|| Place {
                            latitude,
                            longitude,
                            name: format!("{name} ({code})"),
                        }),
                    }
                })
            })
            .collect();
        Snapshot {
            processes,
            memory_total: Some(32 * 1024_u64.pow(3)),
            memory_available: Some(12 * 1024_u64.pow(3)),
            cores: 16,
            elapsed: time,
            pressure: [wave(0.21, 30.0), wave(0.13, 18.0), wave(0.17, 24.0)],
            links,
            link_traffic,
            unix_links,
            outside,
            cpus,
            units,
            remotes,
            unattributed_locks: 1,
            missing: Sources::default(),
            unreadable: 0,
            per_cluster: Sources::default(),
            shadows: Vec::new(),
            sampler: SamplerHealth {
                running: true,
                last_cycle: Some(Duration::ZERO),
                quiet: Duration::ZERO,
                scan_latency: Vec::new(),
            },
            home: Some(Place {
                latitude: 52.37,
                longitude: 4.9,
                name: "Amsterdam".into(),
            }),
            geo: "demo locations".into(),
        }
    }

    /// Every field of `snapshot` as text. The maps and sets are sorted, so two equal snapshots give
    /// the same text, and a changed bit in any float shows. Exhaustive on purpose: a new
    /// `Snapshot` field must be decided here.
    fn fingerprint(snapshot: &Snapshot) -> String {
        let Snapshot {
            processes,
            memory_total,
            memory_available,
            cores,
            elapsed,
            pressure,
            links,
            link_traffic,
            unix_links,
            outside,
            cpus,
            units,
            remotes,
            home,
            geo,
            unattributed_locks,
            missing,
            unreadable,
            per_cluster,
            shadows,
            sampler,
        } = snapshot;
        let link_traffic: BTreeMap<_, _> = link_traffic.iter().collect();
        let unix_links: BTreeSet<_> = unix_links.iter().collect();
        let outside: BTreeMap<_, _> = outside.iter().collect();
        let units: BTreeMap<_, _> = units.iter().collect();
        format!(
            "{processes:?}\n{memory_total:?} {memory_available:?} {cores} {elapsed:?} {pressure:?}\n\
             {links:?}\n{link_traffic:?}\n{unix_links:?}\n{outside:?}\n{cpus:?}\n{units:?}\n\
             {remotes:?}\n{home:?} {geo:?} {unattributed_locks} {missing:?} {unreadable} \
             {per_cluster:?}\n{shadows:?}\n{sampler:?}"
        )
    }

    const CHURN: DemoOptions = DemoOptions {
        churn: true,
        exact_cpu_time: false,
    };
    const EXACT: DemoOptions = DemoOptions {
        churn: false,
        exact_cpu_time: true,
    };
    const BOTH: DemoOptions = DemoOptions {
        churn: true,
        exact_cpu_time: true,
    };

    /// The transients of `snapshot`, which come after the `count` base processes.
    fn transients(snapshot: &Snapshot, count: usize) -> &[Process] {
        &snapshot.processes[count..]
    }

    #[test]
    fn default_options_reproduce_the_existing_demo() {
        for count in [16, 192, 1000] {
            for time in [-75.5, -1.0, 0.0, 0.25, 12.0, 61.7, 300.0, 5000.0] {
                let before = original_demo(time, count);
                assert_eq!(
                    fingerprint(&demo_with(time, count, DemoOptions::default())),
                    fingerprint(&before),
                    "demo_with at {time} for {count}"
                );
                assert_eq!(
                    fingerprint(&demo(time, count)),
                    fingerprint(&before),
                    "demo at {time} for {count}"
                );
            }
        }
    }

    #[test]
    fn churn_is_a_pure_function_of_time() {
        let times = [95.5, -10.0, 12.0, 0.0, -300.25, 12.0, 7001.0, 95.5];
        let first: Vec<String> = times
            .iter()
            .map(|&time| fingerprint(&demo_with(time, 192, BOTH)))
            .collect();
        let backwards: Vec<String> = times
            .iter()
            .rev()
            .map(|&time| fingerprint(&demo_with(time, 192, BOTH)))
            .collect();
        let mut reversed = backwards;
        reversed.reverse();
        assert_eq!(first, reversed);
        assert_eq!(first[2], first[5], "the same time gives the same snapshot");
        assert!(demo_with(5.0, 0, BOTH).processes.is_empty());
    }

    #[test]
    fn churn_keeps_base_indices_pids_and_links() {
        for count in [16, 192, 1000] {
            for time in [-40.0, 3.0, 12.0, 61.0, 100.0] {
                let plain = demo_with(time, count, DemoOptions::default());
                let busy = demo_with(time, count, CHURN);
                let kept = plain.processes.len();
                assert_eq!(kept, count);
                assert_eq!(
                    format!("{:?}", &busy.processes[..kept]),
                    format!("{:?}", plain.processes),
                    "base processes at {time} for {count}"
                );
                assert_eq!(busy.links, plain.links);
                assert_eq!(busy.unix_links, plain.unix_links);
                assert_eq!(busy.outside, plain.outside);
                assert_eq!(busy.link_traffic, plain.link_traffic);
                assert_eq!(
                    format!("{:?}", busy.remotes),
                    format!("{:?}", plain.remotes)
                );
                assert!(
                    transients(&busy, count)
                        .iter()
                        .all(|process| process.id.pid >= 1000 + count as u32
                            && process.id.start > count as u64),
                    "transients have pids above the base ones and starts above theirs"
                );
                for (path, unit) in &plain.units {
                    let (with, without) = (&busy.units[path], unit);
                    assert_eq!(with.memory_max, without.memory_max, "limits stay put");
                    assert_eq!(with.cpu_max, without.cpu_max);
                }
            }
        }
        let busy = demo_with(12.0, 192, CHURN);
        assert!(!transients(&busy, 192).is_empty(), "something is running");
    }

    #[test]
    fn churn_runs_families_of_one_to_six_children_with_zombies_and_a_victim() {
        let count = 192;
        let mut children_seen = BTreeSet::new();
        let (mut zombies, mut victims) = (0, 0);
        for second in 0..400 {
            let snapshot = demo_with(f64::from(second), count, CHURN);
            let listed = transients(&snapshot, count);
            let mut children: HashMap<u32, usize> = HashMap::new();
            for process in listed {
                if listed.iter().any(|other| other.id.pid == process.parent) {
                    *children.entry(process.parent).or_default() += 1;
                }
                zombies += usize::from(process.state == 'Z');
                victims += usize::from(process.name.starts_with("leak"));
            }
            children_seen.extend(children.values().copied());
        }
        assert!(zombies > 0 && victims > 0);
        assert!(
            children_seen.contains(&1) && children_seen.iter().all(|&n| (1..=6).contains(&n)),
            "{children_seen:?}"
        );
        assert!(children_seen.iter().max() >= Some(&4), "{children_seen:?}");
    }

    #[test]
    fn zombies_stay_listed_for_one_sample_and_hold_nothing() {
        let count = 192;
        let mut runs: HashMap<Identity, usize> = HashMap::new();
        for second in 0..300 {
            let snapshot = demo_with(f64::from(second), count, CHURN);
            for process in transients(&snapshot, count) {
                if process.state != 'Z' {
                    continue;
                }
                *runs.entry(process.id).or_default() += 1;
                assert_eq!((process.memory, process.cpu), (0, 0.0));
                assert_eq!(process.files, Measured::Unreadable);
                assert_eq!(process.io_rate, None);
                assert_eq!(process.major_faults, None);
                assert_eq!(process.system_share, None);
            }
        }
        assert!(!runs.is_empty());
        assert!(runs.values().all(|&samples| samples == 1), "{runs:?}");
    }

    #[test]
    fn demo_life_matches_population_differences_between_two_times() {
        let count = 192;
        let population = |time: f64| -> BTreeSet<Identity> {
            let snapshot = demo_with(time, count, CHURN);
            transients(&snapshot, count)
                .iter()
                .map(|process| process.id)
                .collect()
        };
        for from in [-61.5, 0.0, 7.3, 20.0, 96.25, 311.0] {
            for length in [0.4, 1.0, 2.9, 5.5, 20.0, 75.0, 200.0] {
                let to = from + length;
                let events = demo_life(from, to, count);
                assert!(
                    events.windows(2).all(|pair| pair[0].time <= pair[1].time),
                    "ordered by time"
                );
                assert!(events.iter().all(|e| from < e.time && e.time <= to));
                let ids = |kind| -> BTreeSet<Identity> {
                    events
                        .iter()
                        .filter(|event| event.kind == kind)
                        .map(|event| event.id)
                        .collect()
                };
                let (forks, exits) = (ids(DemoLifeKind::Fork), ids(DemoLifeKind::Exit));
                let (before, after) = (population(from), population(to));
                let entered: BTreeSet<_> = after.difference(&before).copied().collect();
                let left: BTreeSet<_> = before.difference(&after).copied().collect();
                let both: BTreeSet<_> = forks.intersection(&exits).copied().collect();
                assert_eq!(
                    forks.difference(&both).copied().collect::<BTreeSet<_>>(),
                    entered,
                    "forks from {from} for {length}"
                );
                assert_eq!(
                    exits.difference(&both).copied().collect::<BTreeSet<_>>(),
                    left,
                    "exits from {from} for {length}"
                );
                assert!(
                    both.iter()
                        .all(|id| !before.contains(id) && !after.contains(id)),
                    "a process that forked and exited in the interval is in neither sample"
                );
                if length < 3.0 {
                    assert!(both.is_empty(), "no process lives under 3 s");
                }
                assert_eq!(
                    events.len(),
                    forks.len() + exits.len(),
                    "one fork, one exit"
                );
            }
        }
    }

    #[test]
    fn demo_life_over_split_intervals_equals_one_long_interval() {
        for count in [16, 192] {
            for (from, middle, to) in [(-50.0, 13.7, 120.0), (0.0, 61.0, 400.0), (31.0, 31.5, 33.0)]
            {
                let whole = demo_life(from, to, count);
                let mut parts = demo_life(from, middle, count);
                parts.extend(demo_life(middle, to, count));
                assert_eq!(whole, parts, "{from}..{middle}..{to} for {count}");
                assert!(!whole.is_empty() || to - from < 5.0);
            }
        }
        let thirds: Vec<DemoLife> = (0..300)
            .flat_map(|second| demo_life(f64::from(second), f64::from(second + 1), 192))
            .collect();
        assert_eq!(thirds, demo_life(0.0, 300.0, 192));
    }

    #[test]
    fn a_reused_transient_pid_gets_a_new_start() {
        let count = 192;
        let families = demo_families(count);
        let mut starts: HashMap<u32, BTreeSet<u64>> = HashMap::new();
        for second in 0..400 {
            let snapshot = demo_with(f64::from(second), count, CHURN);
            for process in transients(&snapshot, count) {
                starts
                    .entry(process.id.pid)
                    .or_default()
                    .insert(process.id.start);
            }
        }
        let pool = 1001 + count as u32..1001 + (count + 7 * (families + 1)) as u32;
        assert!(starts.keys().all(|pid| pool.contains(pid)), "{starts:?}");
        let reused = starts.values().filter(|set| set.len() > 1).count();
        assert!(reused > starts.len() / 2, "{reused} of {}", starts.len());
        // Within a pid, consecutive cycles never overlap in time.
        for family in 0..=families {
            let parents: Vec<_> = (0..5)
                .map(|epoch| demo_cycle(count, family, epoch)[0].clone())
                .collect();
            assert!(
                parents
                    .windows(2)
                    .all(|pair| pair[0].id.pid == pair[1].id.pid
                        && pair[0].id.start != pair[1].id.start
                        && pair[0].end <= pair[1].birth)
            );
        }
    }

    #[test]
    fn a_large_transient_exits_every_ninety_seconds() {
        let count = 192;
        let families = demo_families(count);
        let kills: Vec<DemoLife> = demo_life(-500.0, 1000.0, count)
            .into_iter()
            .filter(|event| event.kind == DemoLifeKind::Exit && event.signal == Some(9))
            .collect();
        assert!(kills.len() >= 16, "{}", kills.len());
        for pair in kills.windows(2) {
            assert!(
                (pair[1].time - pair[0].time - 90.0).abs() < 1e-6,
                "{pair:?}"
            );
        }
        for kill in &kills {
            assert!(kill.memory > 1 << 30, "{} bytes", kill.memory);
            assert_eq!(kill.code, 0);
            // It was a real process for most of the half minute before, growing as it went.
            let just_before = demo_with(kill.time - DEMO_ZOMBIE_SECONDS - 0.01, count, CHURN);
            let victim = transients(&just_before, count)
                .iter()
                .find(|process| process.id == kill.id)
                .expect("listed until it is killed");
            assert!(victim.memory > 1 << 30 && victim.state != 'Z');
            let earlier = demo_with(kill.time - 40.0, count, CHURN);
            let young = transients(&earlier, count)
                .iter()
                .find(|process| process.id == kill.id);
            assert!(young.is_none_or(|process| process.memory < victim.memory));
        }
        assert_eq!(
            demo_life(0.0, 90.0 * 10.0, count)
                .iter()
                .filter(|event| event.signal == Some(9))
                .count()
                .abs_diff(10),
            0,
            "ten kills in ten periods ({families} families)"
        );
    }

    #[test]
    fn exact_cpu_time_is_monotone_and_integrates_cpu() {
        let count = 100;
        let indices = [0, 1, 5, 47, 94, 99];
        let mut previous = [f32::NEG_INFINITY; 6];
        for step in 0..=1200 {
            let time = f64::from(step) * 0.05;
            let snapshot = demo_with(time, count, EXACT);
            for (slot, &index) in indices.iter().enumerate() {
                let cpu_time = snapshot.processes[index].cpu_time;
                assert!(cpu_time >= previous[slot], "process {index} at {time}");
                previous[slot] = cpu_time;
                assert!(
                    (f64::from(cpu_time) - demo_cpu_seconds(time, index)).abs() < 1e-3,
                    "{index} at {time}"
                );
            }
        }
        // Simpson's rule over the sampled CPU use against the change in CPU time.
        let intervals = 400;
        let width = 20.0 / f64::from(intervals);
        let cpu_at =
            |time: f64, index: usize| f64::from(demo_with(time, count, EXACT).processes[index].cpu);
        for index in indices {
            let mut sum = cpu_at(0.0, index) + cpu_at(20.0, index);
            for step in 1..intervals {
                let weight = if step % 2 == 1 { 4.0 } else { 2.0 };
                sum += weight * cpu_at(f64::from(step) * width, index);
            }
            let integral = sum * width / 3.0 / 100.0;
            let gained = demo_cpu_seconds(20.0, index) - demo_cpu_seconds(0.0, index);
            assert!(
                (integral - gained).abs() < 2e-3 * gained.max(1.0),
                "process {index}: quadrature {integral}, cpu time {gained}"
            );
        }
        let plain = demo_with(20.0, count, DemoOptions::default());
        assert_ne!(
            plain.processes[47].cpu_time,
            demo_with(20.0, count, EXACT).processes[47].cpu_time
        );
    }

    #[test]
    fn a_transients_cpu_time_integrates_its_own_cpu_since_birth() {
        let count = 192;
        let start = 12.0;
        let first = demo_with(start, count, CHURN);
        let mut checked = 0;
        for tracked in transients(&first, count).iter().filter(|p| p.state != 'Z') {
            let intervals = 100;
            let width = 1.0 / f64::from(intervals);
            let series: Vec<Option<Process>> = (0..=intervals)
                .map(|step| {
                    demo_with(start + f64::from(step) * width, count, CHURN)
                        .processes
                        .into_iter()
                        .find(|process| process.id == tracked.id && process.state != 'Z')
                })
                .collect();
            if series.iter().any(Option::is_none) {
                continue;
            }
            let cpu: Vec<f64> = series
                .iter()
                .map(|p| f64::from(p.as_ref().unwrap().cpu))
                .collect();
            let mut sum = cpu[0] + cpu[intervals as usize];
            for (step, &value) in cpu.iter().enumerate().take(intervals as usize).skip(1) {
                sum += if step % 2 == 1 { 4.0 } else { 2.0 } * value;
            }
            let integral = sum * width / 3.0 / 100.0;
            let gained = f64::from(series[intervals as usize].as_ref().unwrap().cpu_time)
                - f64::from(series[0].as_ref().unwrap().cpu_time);
            assert!(
                (integral - gained).abs() < 1e-3,
                "{integral} against {gained}"
            );
            checked += 1;
        }
        assert!(checked >= 2, "only {checked} transients to follow");
        let born = demo_life(-1.0, 100.0, count)
            .into_iter()
            .find(|event| event.kind == DemoLifeKind::Fork)
            .expect("something forks");
        assert_eq!(born.cpu_seconds, 0.0);
        let just_born = demo_with(born.time, count, CHURN);
        let process = just_born
            .processes
            .iter()
            .find(|p| p.id == born.id)
            .unwrap();
        assert_eq!((process.cpu_time, process.written.unwrap_or(0)), (0.0, 0));
        assert_eq!(process.memory, born.memory);
    }

    #[test]
    fn no_single_threaded_demo_process_exceeds_one_core() {
        for options in [EXACT, BOTH] {
            for count in [16, 192, 1000] {
                for time in [-20.0, 0.0, 3.3, 12.0, 77.0, 160.0] {
                    let snapshot = demo_with(time, count, options);
                    for (index, process) in snapshot.processes.iter().enumerate() {
                        if process.threads == 1 {
                            assert!(
                                process.cpu <= 100.0 + 1e-3,
                                "{index} at {time}: {}",
                                process.cpu
                            );
                        }
                        if index < count {
                            let (scale, bonus) = demo_cpu_shape(index);
                            assert!(
                                f64::from(process.threads) >= ((scale + bonus) / 100.0).ceil(),
                                "{index}"
                            );
                        }
                    }
                }
            }
        }
        let overloaded = (0..600).any(|step| {
            demo_with(f64::from(step), 100, DemoOptions::default())
                .processes
                .iter()
                .any(|process| process.threads == 1 && process.cpu > 100.0)
        });
        assert!(
            overloaded,
            "the default demo keeps the threads it always had"
        );
    }
}
