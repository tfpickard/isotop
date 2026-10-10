use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::places::COUNTRIES;
use crate::{geo, net, nvml};

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
    /// System-wide sources that could not be read this sample, such as "pressure". The set of
    /// names grows as platform ports add sources of their own (the macOS port will add more).
    pub missing: Vec<&'static str>,
}

struct Counters {
    ticks: u64,
    io: Option<IoBytes>,
    cpu: f32,
}

/// Cumulative `read_bytes` and `write_bytes` from /proc/<pid>/io.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IoBytes {
    read: u64,
    write: u64,
}

impl IoBytes {
    fn parse(text: &str) -> Self {
        let mut bytes = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let Ok(value) = value.trim().parse() else {
                continue;
            };
            match key {
                "read_bytes" => bytes.read = value,
                "write_bytes" => bytes.write = value,
                _ => {}
            }
        }
        bytes
    }

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
    network: net::Network,
    gpu: HashMap<u32, u64>,
    units: HashMap<String, Unit>,
    /// Remote connections with the owning pid in `id.pid`.
    remotes: Vec<Remote>,
    /// Loopback traffic in bytes per second between pid pairs (smaller pid first).
    traffic: HashMap<(u32, u32), f32>,
    geo: String,
}

/// CPU counters from /proc/stat and /proc/schedstat: busy and total ticks, waiting nanoseconds.
#[derive(Clone, Copy)]
struct Ticks {
    busy: u64,
    total: u64,
    wait: u64,
}

pub struct Collector {
    previous: HashMap<Identity, Counters>,
    cpu_previous: HashMap<u32, Ticks>,
    core_kinds: HashMap<u32, CoreKind>,
    last: Instant,
    origin: Instant,
    hz: f32,
    page_size: u64,
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
            let nvml = nvml::Nvml::load();
            let mut geo = geo::Geo::open(geoip.as_deref());
            let mut units = HashMap::new();
            let mut sockets = HashMap::new();
            let mut loopback = LoopbackScan::default();
            let mut retained = HashSet::new();
            while Arc::strong_count(&writer) > 1 {
                let wanted_now = wanted.lock().map(|set| set.clone()).unwrap_or_default();
                let paths = accounted(wanted_now, &mut retained);
                let network = net::sample();
                let locator = locating.load(Ordering::Relaxed).then_some(&mut geo);
                let remotes = remotes(&network.remotes, &mut sockets, locator);
                let traffic = traffic(&network.loopback, &mut loopback, Instant::now());
                let extras = Extras {
                    network,
                    gpu: nvml.as_ref().map(nvml::Nvml::sample).unwrap_or_default(),
                    units: account(&paths, &mut units),
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
    sockets: &[net::Remote],
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
    sockets: &[net::Loopback],
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

/// Reads the accounting files of each cgroup. CPU use comes from the change in usage since the
/// previous reading.
fn account(
    paths: &HashSet<String>,
    previous: &mut HashMap<String, (Instant, u64, u64)>,
) -> HashMap<String, Unit> {
    let now = Instant::now();
    let mut next = HashMap::new();
    let mut units = HashMap::new();
    for path in paths {
        let base = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
        let read = |name: &str| fs::read_to_string(base.join(name)).ok();
        let Some(memory) = read("memory.current").and_then(|v| v.trim().parse().ok()) else {
            continue;
        };
        let limit = |name: &str| read(name).and_then(|v| v.trim().parse::<u64>().ok());
        let field = |text: &Option<String>, key: &str| {
            text.as_deref()?
                .lines()
                .find_map(|line| line.strip_prefix(key)?.trim().parse::<u64>().ok())
        };
        let stat = read("cpu.stat");
        let usage = field(&stat, "usage_usec ").unwrap_or(0);
        let throttles = field(&stat, "nr_throttled ").unwrap_or(0);
        let (cpu, throttled) = previous
            .get(path)
            .map_or((0.0, false), |&(then, used, count)| {
                let dt = now.duration_since(then).as_secs_f32().max(0.1);
                (
                    usage.saturating_sub(used) as f32 / 1e6 / dt * 100.0,
                    throttles > count,
                )
            });
        next.insert(path.clone(), (now, usage, throttles));
        let cpu_max = read("cpu.max").and_then(|text| {
            let mut parts = text.split_whitespace();
            let quota: f32 = parts.next()?.parse().ok()?;
            let period: f32 = parts.next()?.parse().ok()?;
            Some(quota / period * 100.0)
        });
        units.insert(
            path.clone(),
            Unit {
                memory,
                memory_max: limit("memory.max"),
                cpu,
                cpu_max,
                throttled,
                pressure: ["cpu", "memory", "io"].map(|resource| {
                    read(&format!("{resource}.pressure"))
                        .and_then(|text| parse_pressure(&text))
                        .unwrap_or(0.0)
                }),
                oom_kills: field(&read("memory.events"), "oom_kill ").unwrap_or(0),
                pids: limit("pids.current").unwrap_or(0),
                pids_max: limit("pids.max"),
            },
        );
    }
    *previous = next;
    units
}

/// Per-CPU busy share and wait from /proc/stat and /proc/schedstat, and clock from cpufreq.
fn cpus(previous: &mut HashMap<u32, Ticks>, kinds: &HashMap<u32, CoreKind>, dt: f32) -> Vec<Cpu> {
    let mut ticks: HashMap<u32, Ticks> = HashMap::new();
    if let Ok(text) = fs::read_to_string("/proc/stat") {
        for (id, values) in text.lines().filter_map(cpu_line) {
            let total: u64 = values.iter().take(8).sum();
            let idle = values.get(3).copied().unwrap_or(0) + values.get(4).copied().unwrap_or(0);
            ticks.insert(
                id,
                Ticks {
                    busy: total.saturating_sub(idle),
                    total,
                    wait: 0,
                },
            );
        }
    }
    if let Ok(text) = fs::read_to_string("/proc/schedstat") {
        // Field 8 of a cpuN line (schedstat version 15 and later) is nanoseconds spent waiting.
        for (id, values) in text.lines().filter_map(cpu_line) {
            if let (Some(entry), Some(&wait)) = (ticks.get_mut(&id), values.get(7)) {
                entry.wait = wait;
            }
        }
    }
    let mut cpus: Vec<Cpu> = ticks
        .iter()
        .map(|(&id, now)| {
            let (busy, wait) = previous.get(&id).map_or((0.0, 0.0), |before| {
                let total = now.total.saturating_sub(before.total).max(1) as f32;
                (
                    now.busy.saturating_sub(before.busy) as f32 / total,
                    now.wait.saturating_sub(before.wait) as f32 / 1e9 / dt.max(0.001),
                )
            });
            let mhz = fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{id}/cpufreq/scaling_cur_freq"
            ))
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .map_or(0.0, |khz| khz / 1000.0);
            Cpu {
                id,
                kind: kinds.get(&id).copied().unwrap_or(CoreKind::Unknown),
                busy: busy.clamp(0.0, 1.0),
                mhz,
                wait,
            }
        })
        .collect();
    cpus.sort_by_key(|cpu| cpu.id);
    *previous = ticks;
    cpus
}

/// A `cpuN v1 v2 ...` line as (N, values); the aggregate `cpu` line is skipped.
fn cpu_line(line: &str) -> Option<(u32, Vec<u64>)> {
    let mut fields = line.split_whitespace();
    let id = fields.next()?.strip_prefix("cpu")?.parse().ok()?;
    Some((id, fields.filter_map(|v| v.parse().ok()).collect()))
}

/// Hybrid CPUs list their performance and efficiency cores as ranges such as `0-7,16`.
fn core_kinds() -> HashMap<u32, CoreKind> {
    let mut kinds = HashMap::new();
    for (path, kind) in [
        ("/sys/devices/cpu_core/cpus", CoreKind::Performance),
        ("/sys/devices/cpu_atom/cpus", CoreKind::Efficiency),
    ] {
        for id in fs::read_to_string(path)
            .map(|t| ranges(&t))
            .unwrap_or_default()
        {
            kinds.insert(id, kind);
        }
    }
    kinds
}

fn ranges(text: &str) -> Vec<u32> {
    text.trim()
        .split(',')
        .filter_map(|part| {
            let (a, b) = part.split_once('-').unwrap_or((part, part));
            Some(a.trim().parse::<u32>().ok()?..=b.trim().parse::<u32>().ok()?)
        })
        .flatten()
        .collect()
}

/// Pressure stall percentages, and whether /proc/pressure/cpu could be read at all (kernels
/// without PSI have no such file). Unreadable or unparsable values count as 0.
fn pressure() -> ([f32; 3], bool) {
    let texts = ["cpu", "memory", "io"]
        .map(|resource| fs::read_to_string(format!("/proc/pressure/{resource}")).ok());
    let readable = texts[0].is_some();
    let values = texts.map(|text| text.as_deref().and_then(parse_pressure).unwrap_or(0.0));
    (values, readable)
}

/// The `some avg10=` value from a /proc/pressure file.
fn parse_pressure(text: &str) -> Option<f32> {
    text.lines()
        .next()?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse()
        .ok()
}

impl Collector {
    pub fn new(geoip: Option<PathBuf>, home: Option<(f32, f32)>) -> Self {
        Self {
            previous: HashMap::new(),
            cpu_previous: HashMap::new(),
            core_kinds: core_kinds(),
            last: Instant::now(),
            origin: Instant::now(),
            // These sysconf selectors return positive values on supported Linux systems.
            hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f32,
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
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
        let (pressure, pressure_readable) = pressure();
        let mut snapshot = Snapshot {
            cores: std::thread::available_parallelism().map_or(1, usize::from),
            elapsed: now.duration_since(self.origin).as_secs_f64(),
            pressure,
            missing: if pressure_readable {
                Vec::new()
            } else {
                vec!["pressure"]
            },
            cpus: cpus(&mut self.cpu_previous, &self.core_kinds, dt),
            home: self.home.clone(),
            ..Default::default()
        };
        if let Ok(info) = fs::read_to_string("/proc/meminfo") {
            for line in info.lines() {
                let mut fields = line.split_whitespace();
                let key = fields.next().unwrap_or_default();
                let value = fields
                    .next()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
                    * 1024;
                match key {
                    "MemTotal:" => snapshot.memory_total = value,
                    "MemAvailable:" => snapshot.memory_available = value,
                    _ => {}
                }
            }
        }
        let mut next = HashMap::new();
        for entry in fs::read_dir("/proc")?.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let path = entry.path();
            let Ok(stat) = fs::read_to_string(path.join("stat")) else {
                continue;
            };
            let Some((mut process, ticks)) = parse_stat(pid, &stat, self.page_size) else {
                continue;
            };
            process.cpu_time = ticks as f32 / self.hz;
            let io = fs::read_to_string(path.join("io"))
                .ok()
                .map(|text| IoBytes::parse(&text));
            process.written = io.map(|io| io.write);
            if let Some(previous) = self.previous.get(&process.id) {
                let raw = ticks.saturating_sub(previous.ticks) as f32 / self.hz / dt * 100.0;
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
            }
            if let Ok(command) = fs::read(path.join("cmdline")) {
                let text = String::from_utf8_lossy(&command).replace('\0', " ");
                if !text.trim().is_empty() {
                    process.command = text.trim().to_owned();
                }
            }
            let cgroup = fs::read_to_string(path.join("cgroup")).ok();
            process.group = cgroup
                .as_deref()
                .and_then(cgroup_name)
                .unwrap_or_else(|| "ungrouped".into());
            if process.kind != Kind::Kernel {
                process.kind = cgroup.as_deref().map_or(Kind::System, classify);
                process.cgroup = cgroup.as_deref().and_then(cgroup_path).unwrap_or_default();
            }
            next.insert(
                process.id,
                Counters {
                    ticks,
                    io,
                    cpu: process.cpu,
                },
            );
            snapshot.processes.push(process);
        }
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

fn cgroup_name(text: &str) -> Option<String> {
    let paths: Vec<&str> = text
        .lines()
        .filter_map(|line| line.splitn(3, ':').nth(2))
        .collect();
    for path in &paths {
        if let Some(service) = path
            .split('/')
            .rev()
            .find(|part| part.ends_with(".service") || part.ends_with(".scope"))
        {
            return Some(service.to_owned());
        }
    }
    paths.iter().find_map(|path| {
        path.split('/')
            .rev()
            .find(|part| !part.is_empty())
            .map(str::to_owned)
    })
}

/// The unified (cgroup v2) hierarchy path from /proc/<pid>/cgroup.
fn cgroup_path(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("0::"))
        .filter(|path| *path != "/")
        .map(str::to_owned)
}

fn classify(cgroup: &str) -> Kind {
    const CONTAINERS: [&str; 6] = [
        "docker",
        "containerd",
        "libpod",
        "machine.slice",
        "kubepods",
        "lxc",
    ];
    if CONTAINERS.iter().any(|marker| cgroup.contains(marker)) {
        Kind::Container
    } else if cgroup.contains("user.slice") || cgroup.contains("/user@") {
        Kind::Session
    } else {
        Kind::System
    }
}

fn parse_stat(pid: u32, text: &str, page_size: u64) -> Option<(Process, u64)> {
    const PF_KTHREAD: u64 = 0x0020_0000;
    // comm can contain both whitespace and closing parentheses; only the last ')' delimits it.
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let fields: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    let number = |index: usize| fields.get(index)?.parse::<u64>().ok();
    let signed = |index: usize| fields.get(index)?.parse::<i32>().ok();
    let name = text.get(open + 1..close)?.to_owned();
    let rss = fields.get(21)?.parse::<i64>().ok()?.max(0) as u64;
    Some((
        Process {
            id: Identity {
                pid,
                start: number(19)?,
            },
            parent: number(1)? as u32,
            command: name.clone(),
            name,
            group: String::new(),
            kind: if number(6)? & PF_KTHREAD != 0 {
                Kind::Kernel
            } else {
                Kind::System
            },
            state: fields.first()?.chars().next()?,
            cpu: 0.0,
            memory: rss.saturating_mul(page_size),
            io_rate: None,
            read_rate: None,
            write_rate: None,
            written: None,
            // Fields 18 and 19 of /proc/<pid>/stat; priority is negative for real-time tasks.
            priority: signed(15)?,
            nice: signed(16)?,
            threads: number(17)? as u32,
            gpu_memory: 0,
            // Field 39 of /proc/<pid>/stat, counted from the state as field 3.
            core: number(36).unwrap_or(0) as u32,
            cpu_time: 0.0,
            cgroup: String::new(),
        },
        number(11)?.saturating_add(number(12)?),
    ))
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
            core: ((i * 7 + (time / (5.0 + (i % 5) as f64)) as usize) % 16) as u32,
            cpu_time: (time as f32 + i as f32) * cpu / 100.0 + i as f32 * 3.7,
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
    fn kernel_threads_and_cgroups_are_classified() {
        let mut fields = vec!["0"; 22];
        fields[0] = "S";
        fields[1] = "2";
        fields[6] = "2129984";
        fields[17] = "3";
        fields[19] = "5";
        let text = format!("40 (kworker/0:1) {}", fields.join(" "));
        let (p, _) = parse_stat(40, &text, 4096).unwrap();
        assert_eq!(p.kind, Kind::Kernel);
        assert_eq!(p.threads, 3);
        assert_eq!(describe(&p), "kernel worker thread");
        assert_eq!(
            parse_pressure("some avg10=12.50 avg60=3.00 avg300=1.00 total=9\nfull avg10=1.00"),
            Some(12.5)
        );
        assert_eq!(
            classify("0::/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope"),
            Kind::Session
        );
        assert_eq!(
            classify("0::/system.slice/docker-abc.scope"),
            Kind::Container
        );
        assert_eq!(classify("0::/system.slice/cron.service"), Kind::System);
    }

    #[test]
    fn stat_handles_spaces_parentheses_and_negative_rss() {
        let mut fields = vec!["0"; 22];
        fields[0] = "S";
        fields[1] = "42";
        fields[11] = "123";
        fields[12] = "7";
        fields[19] = "999";
        fields[21] = "-1";
        let text = format!("73 (a tricky ) name) {}", fields.join(" "));
        let (p, ticks) = parse_stat(73, &text, 4096).unwrap();
        assert_eq!(p.name, "a tricky ) name");
        assert_eq!(p.parent, 42);
        assert_eq!(p.id.start, 999);
        assert_eq!(ticks, 130);
        assert_eq!(p.memory, 0);
        assert!(parse_stat(73, "73 (short) S", 4096).is_none());
    }

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
    fn cpu_lines_core_ranges_and_cgroup_paths_parse() {
        assert_eq!(
            cpu_line("cpu3 0 0 0 0 0 0 6891 2519 25"),
            Some((3, vec![0, 0, 0, 0, 0, 0, 6891, 2519, 25]))
        );
        assert_eq!(cpu_line("cpu  1 2 3"), None);
        assert_eq!(ranges("0-3,8\n"), vec![0, 1, 2, 3, 8]);
        assert_eq!(
            cgroup_path("0::/system.slice/cron.service\n"),
            Some("/system.slice/cron.service".into())
        );
        assert_eq!(cgroup_path("0::/\n"), None);
        let mut fields = vec!["0"; 40];
        fields[0] = "R";
        fields[36] = "17";
        let (p, _) = parse_stat(9, &format!("9 (x) {}", fields.join(" ")), 4096).unwrap();
        assert_eq!(p.core, 17);
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
    fn grouping_prefers_service_over_slice() {
        assert_eq!(
            cgroup_name("0::/user.slice/user-1000.slice/session-2.scope\n"),
            Some("session-2.scope".into())
        );
        assert_eq!(
            cgroup_name("0::/system.slice/example.service\n"),
            Some("example.service".into())
        );
    }

    fn stat_line(edit: impl FnOnce(&mut Vec<&str>)) -> String {
        let mut fields = vec!["0"; 22];
        fields[0] = "S";
        edit(&mut fields);
        format!("5 (x) {}", fields.join(" "))
    }

    #[test]
    fn stat_reads_priority_and_nice_including_real_time_priorities() {
        let normal = stat_line(|fields| {
            fields[15] = "39";
            fields[16] = "19";
        });
        let (process, _) = parse_stat(5, &normal, 4096).unwrap();
        assert_eq!((process.priority, process.nice), (39, 19));
        let real_time = stat_line(|fields| fields[15] = "-51");
        let (process, _) = parse_stat(5, &real_time, 4096).unwrap();
        assert_eq!((process.priority, process.nice), (-51, 0));
        let favoured = stat_line(|fields| {
            fields[15] = "0";
            fields[16] = "-20";
        });
        let (process, _) = parse_stat(5, &favoured, 4096).unwrap();
        assert_eq!((process.priority, process.nice), (0, -20));
    }

    #[test]
    fn io_read_and_write_rates_and_written_come_from_separate_counters() {
        let text = "rchar: 99\nwchar: 98\nsyscr: 1\nsyscw: 1\nread_bytes: 4096\nwrite_bytes: 1000\ncancelled_write_bytes: 0\n";
        let before = IoBytes::parse(text);
        assert_eq!(
            before,
            IoBytes {
                read: 4096,
                write: 1000
            }
        );
        let after = IoBytes {
            read: 4096 + 6000,
            write: 1000 + 2000,
        };
        assert_eq!(after.rates(before, 2.0), [4000.0, 3000.0, 1000.0]);
        // A counter that went backwards (a recycled pid's reset) reads as no traffic.
        assert_eq!(before.rates(after, 2.0), [0.0, 0.0, 0.0]);
    }

    fn loopback(inode: u64, pid: u32, peer: u32, received: u64) -> net::Loopback {
        net::Loopback {
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
    fn demo_scheduling_classes_states_and_traffic_cover_the_coop_cases() {
        let mut stalled = false;
        for step in 0..200 {
            let snapshot = demo(step as f64 * 1.3, 128);
            assert!(snapshot.missing.is_empty());
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
