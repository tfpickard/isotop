use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::geo;
use crate::places::COUNTRIES;
use crate::platform::{self, RawProcess};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identity {
    pub pid: u32,
    pub start: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Kernel,
    System,
    Session,
    Container,
}

#[derive(Clone, Debug)]
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
    /// NVIDIA GPU memory in bytes; 1 means in use with unreported memory.
    pub gpu_memory: u64,
    /// The CPU it last ran on, and its total CPU time in seconds.
    pub core: u32,
    pub cpu_time: f32,
    /// Full cgroup v2 path, such as /system.slice/cron.service.
    pub cgroup: String,
    /// Share of its CPU time since the previous sample that ran on performance cores, 0 to 1.
    /// None where the platform does not split CPU time by core kind (Linux), on the first
    /// sample, and when the process used no CPU time in the interval.
    pub performance_share: Option<f32>,
    /// Average number of its threads that were runnable but waiting for a CPU since the
    /// previous sample. None where the platform does not measure it (Linux) and on the first
    /// sample.
    pub waiting: Option<f32>,
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
    /// Share of the last interval spent busy, 0 to 1.
    pub busy: f32,
    pub mhz: f32,
    /// Average number of tasks that were runnable but waiting for this CPU.
    pub wait: f32,
}

/// Resource accounting of one cgroup.
#[derive(Clone, Debug, Default)]
pub struct Unit {
    pub memory: u64,
    pub memory_max: Option<u64>,
    /// CPU use and quota in percent of one core.
    pub cpu: f32,
    pub cpu_max: Option<f32>,
    /// The CPU quota throttled it since the previous reading.
    pub throttled: bool,
    /// Pressure stall percentages (10 s average) for CPU, memory and I/O.
    pub pressure: [f32; 3],
    pub oom_kills: u64,
    pub pids: u64,
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
    /// Bytes per second sent and received.
    pub up: f32,
    pub down: f32,
    pub place: Option<Place>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub processes: Vec<Process>,
    pub memory_total: u64,
    pub memory_available: u64,
    pub cores: usize,
    pub elapsed: f64,
    /// Pressure stall percentages (10 s average) for CPU, memory and I/O.
    pub pressure: [f32; 3],
    pub links: Vec<(Identity, Identity, u32)>,
    /// Bytes per second over loopback TCP between the two processes of a pair, both directions
    /// together. Keys are ordered as in `links`; pairs with a measured connection but no traffic
    /// since the previous reading are present with 0, and pairs not yet measured are absent.
    pub link_traffic: HashMap<(Identity, Identity), f32>,
    /// TCP connections leaving the machine, per process.
    pub outside: HashMap<Identity, u32>,
    pub cpus: Vec<Cpu>,
    /// Cgroups of the sampled processes, by path.
    pub units: HashMap<String, Unit>,
    pub remotes: Vec<Remote>,
    pub home: Option<Place>,
    /// Where remote locations come from, for the status line.
    pub geo: String,
    /// System-wide sources that could not be read this sample or that the platform never has:
    /// "cpu pressure", "memory pressure" and "io pressure" when pressure stall information is
    /// unreadable, then the platform's permanent gaps (`Sampler::missing`).
    pub missing: Vec<&'static str>,
    /// Processes the platform could see but not measure because they belong to another user.
    pub unreadable: usize,
    /// Sources the platform measures per CPU cluster rather than per CPU (`Sampler::per_cluster`):
    /// "cpu clock" when every CPU's `mhz` is the average clock of the cluster of its kind.
    pub per_cluster: Vec<&'static str>,
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

    /// Bytes per second of read and write combined, read, and write over `dt` seconds.
    fn rates(self, before: Self, dt: f32) -> [f32; 3] {
        [
            self.total().saturating_sub(before.total()),
            self.read.saturating_sub(before.read),
            self.write.saturating_sub(before.write),
        ]
        .map(|bytes| bytes as f32 / dt)
    }
}

/// Slow measurements gathered off the frame loop.
#[derive(Default)]
struct Extras {
    network: platform::Network,
    gpu: HashMap<u32, u64>,
    units: HashMap<String, Unit>,
    /// Remote connections with the owning pid in `id.pid`.
    remotes: Vec<Remote>,
    /// Loopback traffic in bytes per second between pid pairs (smaller pid first).
    traffic: HashMap<(u32, u32), f32>,
    geo: String,
}

pub struct Collector {
    sampler: platform::Sampler,
    previous: HashMap<Identity, Counters>,
    last: Instant,
    origin: Instant,
    extras: Option<Arc<Mutex<Extras>>>,
    /// Cgroup paths the background thread should account, and whether to locate remote
    /// addresses (only the globe shows them, and the first lookup loads the GeoIP database).
    wanted: Arc<Mutex<HashSet<String>>>,
    locating: Arc<AtomicBool>,
    geoip: Option<PathBuf>,
    home: Option<Place>,
    started: bool,
}

/// Samples sockets, GPU usage, cgroups and remote locations every two seconds on a background
/// thread; the thread stops once the collector is gone. Returns None when no thread can be spawned.
fn background(
    geoip: Option<PathBuf>,
    wanted: Arc<Mutex<HashSet<String>>>,
    locating: Arc<AtomicBool>,
) -> Option<Arc<Mutex<Extras>>> {
    let shared = Arc::new(Mutex::new(Extras::default()));
    let writer = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("isotop-sampler".into())
        .spawn(move || {
            let gpu = platform::Gpu::load();
            let mut geo = geo::Geo::open(geoip.as_deref());
            let mut units = HashMap::new();
            let mut sockets = HashMap::new();
            let mut loopback = LoopbackScan::default();
            let mut retained = HashSet::new();
            while Arc::strong_count(&writer) > 1 {
                let wanted_now = wanted.lock().map(|set| set.clone()).unwrap_or_default();
                let paths = accounted(wanted_now, &mut retained);
                let network = platform::network();
                let locator = locating.load(Ordering::Relaxed).then_some(&mut geo);
                let remotes = remotes(&network.remotes, &mut sockets, locator);
                let traffic = traffic(&network.loopback, &mut loopback, Instant::now());
                let extras = Extras {
                    network,
                    gpu: gpu.as_ref().map(platform::Gpu::sample).unwrap_or_default(),
                    units: platform::account(&paths, &mut units),
                    remotes,
                    traffic,
                    geo: geo.source.clone(),
                };
                match writer.lock() {
                    Ok(mut slot) => *slot = extras,
                    Err(_) => return,
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .ok()
        .map(|_| shared)
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
            let (up, down) =
                previous
                    .get(&socket.inode)
                    .map_or((0.0, 0.0), |&(then, sent, received)| {
                        let dt = now.duration_since(then).as_secs_f32().max(0.1);
                        (
                            socket.sent.saturating_sub(sent) as f32 / dt,
                            socket.received.saturating_sub(received) as f32 / dt,
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
/// the number of scans since it was last seen.
#[derive(Default)]
struct LoopbackScan {
    at: Option<Instant>,
    received: HashMap<u64, (u64, u32)>,
}

/// Scans a socket's last counter is kept after it drops out, so a socket that one scan missed
/// (an owner's fd table briefly unreadable) is measured from its old counter when it returns
/// instead of looking new. Socket inode numbers are not reused within that time.
const LOOPBACK_MISSES: u32 = 3;

/// Bytes per second between pid pairs (smaller pid first) from the change in each loopback
/// socket's received counter since the previous scan. Each end counts what it received, which
/// is what the other end sent, so both directions together count every byte once.
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
        next.insert(socket.inode, (socket.received, 0));
        let Some(interval) = interval else {
            continue;
        };
        let before = previous
            .received
            .get(&socket.inode)
            .map_or(0, |&(received, _)| received);
        *rates
            .entry((socket.pid.min(socket.peer), socket.pid.max(socket.peer)))
            .or_default() += socket.received.saturating_sub(before) as f32 / interval;
    }
    for (&inode, &(received, misses)) in &previous.received {
        if misses < LOOPBACK_MISSES {
            next.entry(inode).or_insert((received, misses + 1));
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
            extras: None,
            wanted: Arc::new(Mutex::new(HashSet::new())),
            locating: Arc::new(AtomicBool::new(false)),
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
        if !std::mem::replace(&mut self.started, true) {
            self.extras = background(
                self.geoip.clone(),
                Arc::clone(&self.wanted),
                Arc::clone(&self.locating),
            );
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().max(0.001);
        let (pressure, pressure_readable) = self.sampler.pressure();
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
        snapshot.missing = missing(pressure_readable, self.sampler.missing());
        snapshot.per_cluster = self.sampler.per_cluster();
        snapshot.unreadable = self.sampler.unreadable();
        snapshot.processes.sort_by_key(|p| p.id);
        if let Ok(mut wanted) = self.wanted.lock() {
            *wanted = snapshot
                .processes
                .iter()
                .filter(|p| !p.cgroup.is_empty())
                .map(|p| p.cgroup.clone())
                .collect();
        }
        if let Some(shared) = &self.extras
            && let Ok(extras) = shared.lock()
        {
            let ids: HashMap<u32, Identity> = snapshot
                .processes
                .iter()
                .map(|p| (p.id.pid, p.id))
                .collect();
            for process in &mut snapshot.processes {
                process.gpu_memory = extras.gpu.get(&process.id.pid).copied().unwrap_or(0);
            }
            snapshot.links = extras
                .network
                .links
                .iter()
                .filter_map(|(&(a, b), &count)| Some((*ids.get(&a)?, *ids.get(&b)?, count)))
                .collect();
            snapshot.outside = extras
                .network
                .outside
                .iter()
                .filter_map(|(pid, &count)| Some((*ids.get(pid)?, count)))
                .collect();
            snapshot.link_traffic = extras
                .traffic
                .iter()
                .filter_map(|(&(a, b), &rate)| Some(((*ids.get(&a)?, *ids.get(&b)?), rate)))
                .collect();
            snapshot.units = extras.units.clone();
            snapshot.remotes = extras
                .remotes
                .iter()
                .filter_map(|remote| {
                    Some(Remote {
                        id: *ids.get(&remote.id.pid)?,
                        ..remote.clone()
                    })
                })
                .collect();
            snapshot.geo = extras.geo.clone();
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
    } = raw;
    process.cpu_time = ticks as f32 / hz;
    // Runnable time counts running time too, so what remains after the CPU time is waiting.
    let waited = runnable_ticks.map(|runnable| i128::from(runnable) - i128::from(ticks));
    if let Some(previous) = previous {
        let raw = ticks.saturating_sub(previous.ticks) as f32 / hz / dt * 100.0;
        let alpha = 1.0 - (-dt / 1.5).exp();
        process.cpu = previous.cpu + alpha * (raw - previous.cpu);
        if let Some([total, read, write]) = io
            .zip(previous.io)
            .map(|(current, before)| current.rates(before, dt))
        {
            process.io_rate = Some(total);
            process.read_rate = Some(read);
            process.write_rate = Some(write);
        }
        // A counter that went backwards (a reused pid) gives no reading rather than a wrong one.
        process.performance_share = performance_ticks
            .zip(previous.performance_ticks)
            .and_then(|(now, before)| now.checked_sub(before))
            .zip(ticks.checked_sub(previous.ticks))
            .filter(|&(_, interval)| interval > 0)
            .map(|(performance, interval)| (performance as f32 / interval as f32).min(1.0));
        // Only growth beyond the highest total seen counts, so a dip while threads ran and its
        // recovery once they are switched out again are not mistaken for waiting.
        process.waiting = waited
            .zip(previous.waited)
            .map(|(now, highest)| (now - highest).max(0) as f32 / hz / dt);
    }
    let highest = match (waited, previous.and_then(|previous| previous.waited)) {
        (Some(now), Some(highest)) => Some(now.max(highest)),
        (now, _) => now,
    };
    let counters = Counters {
        ticks,
        io,
        cpu: process.cpu,
        performance_ticks,
        waited: highest,
    };
    (process, counters)
}

/// The names for `Snapshot::missing`: the three pressure sources when pressure could not be read
/// at all, then the platform's permanent gaps, each name once.
fn missing(pressure_readable: bool, permanent: Vec<&'static str>) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = if pressure_readable {
        Vec::new()
    } else {
        vec!["cpu pressure", "memory pressure", "io pressure"]
    };
    for name in permanent {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
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
    let scale = 20.0 + (index % 9) as f64 * 24.0;
    let bonus = if index.is_multiple_of(47) { 110.0 } else { 0.0 };
    scale / 16.0 * (antiderivative(RATE * time + start) - antiderivative(start)) / RATE
        + bonus * time
}

/// The share of a demo process's I/O that is writes.
fn demo_write_share(index: usize) -> f32 {
    (1 + index % 3) as f32 / 4.0
}

pub fn demo(time: f64, count: usize) -> Snapshot {
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
            gpu_memory: if i % 37 == 5 {
                (300 + i as u64 * 7) << 20
            } else {
                0
            },
            core,
            cpu_time: (time as f32 + i as f32) * cpu / 100.0 + i as f32 * 3.7,
            // Mostly on the kind of core it last ran on (0 to 7 are performance cores), with
            // some of its time on the other cluster; no share when it used no CPU.
            performance_share: (cpu > 0.0).then(|| {
                let mixing = 0.15 * (0.5 + 0.5 * (phase * 0.37).sin());
                if core < 8 { 1.0 - mixing } else { mixing }
            }),
            waiting: Some(1.5 * bounded(cpu, 80.0) * (0.6 + 0.4 * (phase * 0.8).cos())),
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
                busy: load * if id < 8 { 0.9 } else { 0.6 },
                mhz: if id < 8 {
                    1400.0 + 3400.0 * load
                } else {
                    1000.0 + 2200.0 * load
                },
                wait: (load - 0.6).max(0.0) * 3.0,
            }
        })
        .collect();
    let mut units: HashMap<String, Unit> = HashMap::new();
    for p in processes.iter().filter(|p| !p.cgroup.is_empty()) {
        let unit = units.entry(p.cgroup.clone()).or_default();
        unit.memory += p.memory;
        unit.cpu += p.cpu;
        unit.pids += 1;
    }
    for (k, (path, unit)) in units.iter_mut().enumerate() {
        let family = path.trim_end_matches(".service").rsplit('-').next();
        let family: usize = family.and_then(|f| f.parse().ok()).unwrap_or(k);
        if family.is_multiple_of(3) {
            unit.memory_max = Some(unit.memory * 3 / 2);
        }
        if family % 4 == 1 {
            unit.cpu_max = Some(120.0);
            unit.throttled = unit.cpu > 120.0;
        }
        unit.pressure = [wave(0.3 + family as f64 * 0.05, 40.0), wave(0.2, 10.0), 0.0];
        unit.oom_kills = if family == 4 { (time / 23.0) as u64 } else { 0 };
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
                    up: 2048.0 * pulse,
                    down: 65536.0 * pulse * pulse,
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
        memory_total: 32 * 1024_u64.pow(3),
        memory_available: 12 * 1024_u64.pow(3),
        cores: 16,
        elapsed: time,
        pressure: [wave(0.21, 30.0), wave(0.13, 18.0), wave(0.17, 24.0)],
        links,
        link_traffic,
        outside,
        cpus,
        units,
        remotes,
        missing: Vec::new(),
        unreadable: 0,
        per_cluster: Vec::new(),
        home: Some(Place {
            latitude: 52.37,
            longitude: 4.9,
            name: "Amsterdam".into(),
        }),
        geo: "demo locations".into(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(after.rates(before, 2.0), [4000.0, 3000.0, 1000.0]);
        // A counter that went backwards (a recycled pid's reset) reads as no traffic.
        assert_eq!(before.rates(after, 2.0), [0.0, 0.0, 0.0]);
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
    fn a_loopback_socket_missed_by_one_scan_is_measured_from_its_old_counter() {
        let start = Instant::now();
        let mut previous = LoopbackScan::default();
        let at = |seconds| start + Duration::from_secs(seconds);
        traffic(&[loopback(7, 40, 41, 0)], &mut previous, at(0));
        traffic(&[loopback(7, 40, 41, 900_000_000)], &mut previous, at(2));
        // One scan cannot see the socket, then it is back with 2 MB more.
        assert!(traffic(&[], &mut previous, at(4)).is_empty());
        let rates = traffic(&[loopback(7, 40, 41, 902_000_000)], &mut previous, at(6));
        assert_eq!(rates, HashMap::from([((40, 41), 1_000_000.0)]));
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
    fn missing_names_the_pressures_then_the_platform_gaps_once_each() {
        assert!(missing(true, Vec::new()).is_empty());
        assert_eq!(
            missing(true, vec!["last cpu", "cgroups"]),
            ["last cpu", "cgroups"]
        );
        assert_eq!(
            missing(false, Vec::new()),
            ["cpu pressure", "memory pressure", "io pressure"]
        );
        assert_eq!(
            missing(false, vec!["cpu pressure", "io pressure", "last cpu"]),
            ["cpu pressure", "memory pressure", "io pressure", "last cpu"],
            "a name the platform repeats is listed once"
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
        assert!(gaps.iter().all(|name| snapshot.missing.contains(name)));
        assert_eq!(snapshot.per_cluster, sampler.per_cluster());
        if cfg!(target_os = "linux") {
            assert!(gaps.is_empty() && snapshot.unreadable == 0);
            assert!(snapshot.per_cluster.is_empty());
        }
    }

    /// A raw reading of the demo's first process with the given counters, in nanosecond ticks,
    /// with the derived fields unset as a platform leaves them.
    fn reading(ticks: u64, performance: Option<u64>, runnable: Option<u64>) -> RawProcess {
        RawProcess {
            process: Process {
                performance_share: None,
                waiting: None,
                ..demo(0.0, 1).processes[0].clone()
            },
            ticks,
            io: None,
            performance_ticks: performance,
            runnable_ticks: runnable,
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
    }
}
