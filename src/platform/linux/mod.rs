//! Linux: processes from /proc, CPUs from /proc/stat, /proc/schedstat and cpufreq, pressure
//! stall information, cgroup v2 accounting, sockets, open files and file locks, NVML and the
//! systemd journal.

mod files;
mod journal;
mod net;
mod nvml;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;
use std::time::Instant;

use crate::model::{
    CoreKind, Cpu, Identity, IoBytes, Kind, Measured, Policy, Process, SyscallCounts, Unit,
};
use crate::platform::{Files, Network, RawProcess, Shadow, Source, Sources};

pub use files::{FileScan, locks};
pub use journal::{JOURNAL, journal, stop_journal};
pub use nvml::Nvml as Gpu;

/// What `Process::memory` measures here: the resident set size.
pub const MEMORY_LABEL: &str = "RSS";

/// Without cgroup v2 a process's group is the service or scope its cgroup lines name.
pub const GROUP_LABEL: &str = "service";
pub const WITHOUT_CGROUPS: &str = "groups by service; no cgroup v2 accounting here";

/// Samples between re-reads of a process's cmdline and cgroup when its comm has not changed.
/// They change only on exec, which changes comm, on a cgroup migration and when a process
/// rewrites its own arguments (setproctitle), which keep it; those show within this many samples.
const TEXT_REFRESH: u64 = 10;

/// Reads processes, memory, pressure and CPUs, keeping the CPU counters between readings.
pub struct Sampler {
    cpu_previous: HashMap<u32, Ticks>,
    core_kinds: HashMap<u32, CoreKind>,
    hz: f32,
    page_size: u64,
    /// When the machine booted, in seconds since the Unix epoch (`btime` in /proc/stat), read
    /// once; None when it is missing.
    boot_time: Option<f64>,
    /// What this machine lacks for good: core kinds, and cgroup v2 accounting.
    fixed_gaps: Sources,
    /// What the last `cpus` call could not read.
    cpu_gaps: Sources,
    /// Whether any process in the last `processes` call sat in a cgroup v2 cgroup below the
    /// root; inside a container every process may read as the root `0::/`.
    grouped: bool,
    /// What each process's cmdline and cgroup said when last read; only the processes of the
    /// latest sample are kept.
    texts: HashMap<Identity, Texts>,
    /// Samples taken so far, the clock of `TEXT_REFRESH`.
    samples: u64,
}

/// A process's command line and cgroup as last read from /proc/<pid>/cmdline and cgroup, with
/// the comm it had then.
struct Texts {
    comm: String,
    /// The arguments joined by spaces; None when empty (kernel threads, zombies) or unreadable.
    command: Option<String>,
    group: String,
    /// The kind its cgroup implies, for a process that is not a kernel thread.
    kind: Kind,
    cgroup: String,
}

impl Texts {
    /// Reads them from the process directory `path`.
    fn read(path: &Path, comm: &str) -> Texts {
        let command = fs::read(path.join("cmdline"))
            .ok()
            .map(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .replace('\0', " ")
                    .trim()
                    .to_owned()
            })
            .filter(|text| !text.is_empty());
        let cgroup = fs::read_to_string(path.join("cgroup")).ok();
        Texts {
            comm: comm.to_owned(),
            command,
            group: cgroup
                .as_deref()
                .and_then(cgroup_name)
                .unwrap_or_else(|| "ungrouped".into()),
            kind: cgroup.as_deref().map_or(Kind::System, classify),
            cgroup: cgroup.as_deref().and_then(cgroup_path).unwrap_or_default(),
        }
    }

    fn apply(&self, process: &mut Process) {
        if let Some(command) = &self.command {
            process.command.clone_from(command);
        }
        process.group.clone_from(&self.group);
        if process.kind != Kind::Kernel {
            process.kind = self.kind;
            process.cgroup.clone_from(&self.cgroup);
        }
    }
}

/// Whether to read a process's cmdline and cgroup again rather than keep `cached`, the comm they
/// were read with: when nothing is cached, after an exec (its comm is no longer `cached`), and
/// once every `TEXT_REFRESH` samples, staggered by pid so the re-reads spread over the samples.
fn reread(cached: Option<&str>, comm: &str, pid: u32, sample: u64) -> bool {
    cached != Some(comm) || (sample + u64::from(pid)).is_multiple_of(TEXT_REFRESH)
}

impl Sampler {
    pub fn new() -> Self {
        let core_kinds = core_kinds();
        let controllers = fs::read_to_string("/sys/fs/cgroup/cgroup.controllers").ok();
        Self {
            cpu_previous: HashMap::new(),
            fixed_gaps: fixed_gaps(&core_kinds, controllers.as_deref()),
            core_kinds,
            // SAFETY: sysconf takes no pointers and has no preconditions; for a selector it
            // does not know it returns -1, which max(1) clamps. These selectors return positive
            // values on supported Linux systems.
            hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f32,
            // SAFETY: as above.
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
            boot_time: fs::read_to_string("/proc/stat")
                .ok()
                .as_deref()
                .and_then(parse_boot_time),
            cpu_gaps: Sources::default(),
            grouped: false,
            texts: HashMap::new(),
            samples: 0,
        }
    }

    /// USER_HZ, the unit of the CPU times in /proc/<pid>/stat.
    pub fn hz(&self) -> f32 {
        self.hz
    }

    /// Every process in /proc whose stat file could be read and parsed. Each sample reads its
    /// stat and io; its cmdline and cgroup come from `texts` unless `reread` says otherwise.
    pub fn processes(&mut self) -> io::Result<Vec<RawProcess>> {
        let mut processes = Vec::new();
        let mut texts = HashMap::with_capacity(self.texts.len());
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
            let Some((mut process, counters)) = parse_stat(pid, &stat, self.page_size) else {
                continue;
            };
            process.started = self
                .boot_time
                .map_or(0.0, |boot| start_time(boot, process.id.start, self.hz));
            let (io, syscalls) = fs::read_to_string(path.join("io"))
                .ok()
                .map(|text| parse_io(&text))
                .unzip();
            process.written = io.map(|io| io.write);
            let cached = self.texts.remove(&process.id);
            let text = match cached {
                Some(cached) if !reread(Some(&cached.comm), &process.name, pid, self.samples) => {
                    cached
                }
                _ => Texts::read(&path, &process.name),
            };
            text.apply(&mut process);
            texts.insert(process.id, text);
            processes.push(RawProcess {
                process,
                ticks: counters.ticks,
                io,
                performance_ticks: None,
                runnable_ticks: None,
                system_ticks: counters.system_ticks,
                syscalls,
                major_faults: Some(counters.major_faults),
            });
        }
        self.grouped = processes.iter().any(|raw| !raw.process.cgroup.is_empty());
        self.texts = texts;
        self.samples += 1;
        Ok(processes)
    }

    /// Total and available memory in bytes from /proc/meminfo; each None when it could not be
    /// read.
    pub fn memory(&self) -> (Option<u64>, Option<u64>) {
        fs::read_to_string("/proc/meminfo").map_or((None, None), |info| meminfo(&info))
    }

    /// Pressure stall percentages, and the pressures that could not be read.
    pub fn pressure(&self) -> ([f32; 3], Sources) {
        pressure(
            ["cpu", "memory", "io"]
                .map(|resource| fs::read_to_string(format!("/proc/pressure/{resource}")).ok()),
        )
    }

    /// Per-CPU use since the previous call, `dt` seconds ago.
    pub fn cpus(&mut self, dt: f32) -> Vec<Cpu> {
        let (cpus, gaps) = cpus(&mut self.cpu_previous, &self.core_kinds, dt);
        self.cpu_gaps = gaps;
        cpus
    }

    /// What this machine lacks: the gaps found at start, those of the last CPU readings, and
    /// cgroups when no process sat in a cgroup v2 cgroup below the root.
    pub fn missing(&self) -> Sources {
        let mut missing = self.fixed_gaps.union(self.cpu_gaps);
        if !self.grouped {
            missing.insert(Source::Cgroups);
        }
        missing
    }

    /// Linux measures every CPU on its own.
    pub fn per_cluster(&self) -> Sources {
        Sources::default()
    }

    /// Always 0: a process that vanishes between readdir and the read is a race, not a gap.
    pub fn unreadable(&self) -> usize {
        0
    }

    /// None: every process's stat is world-readable, so no process is left out.
    pub fn shadows(&self) -> Vec<Shadow> {
        Vec::new()
    }
}

/// Total and available memory in bytes from the text of /proc/meminfo.
fn meminfo(text: &str) -> (Option<u64>, Option<u64>) {
    let (mut total, mut available) = (None, None);
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let key = fields.next();
        let kibibytes = fields.next().and_then(|v| v.parse::<u64>().ok());
        let bytes = kibibytes.map(|kibibytes| kibibytes.saturating_mul(1024));
        match key {
            Some("MemTotal:") => total = bytes,
            Some("MemAvailable:") => available = bytes,
            _ => {}
        }
    }
    (total, available)
}

/// Socket links between processes.
pub fn network() -> Network {
    net::sample()
}

/// CPU counters from /proc/stat and /proc/schedstat: busy and total ticks, waiting nanoseconds
/// when /proc/schedstat counts them.
#[derive(Clone, Copy)]
struct Ticks {
    busy: u64,
    total: u64,
    wait: Option<u64>,
}

/// Reads the accounting files of each cgroup. CPU use comes from the change in usage since the
/// previous reading.
pub fn account(
    paths: &HashSet<String>,
    previous: &mut HashMap<String, (Instant, Option<u64>, Option<u64>)>,
) -> HashMap<String, Unit> {
    account_under(Path::new("/sys/fs/cgroup"), paths, previous)
}

/// `account` for the cgroup hierarchy mounted at `root`. A reading that is missing, and one that
/// needs an earlier reading to become a rate and has none, is None in the `Unit`, never 0.
fn account_under(
    root: &Path,
    paths: &HashSet<String>,
    previous: &mut HashMap<String, (Instant, Option<u64>, Option<u64>)>,
) -> HashMap<String, Unit> {
    let now = Instant::now();
    let mut next = HashMap::new();
    let mut units = HashMap::new();
    for path in paths {
        let base = root.join(path.trim_start_matches('/'));
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
        let usage = field(&stat, "usage_usec ");
        let throttles = field(&stat, "nr_throttled ");
        // A counter that went backwards (a cgroup recreated under the same path) gives no rate.
        let (cpu, throttled) = previous
            .get(path)
            .map_or((None, None), |&(then, used, count)| {
                let dt = now.duration_since(then).as_secs_f32().max(0.1);
                (
                    usage
                        .zip(used)
                        .and_then(|(now, then)| now.checked_sub(then))
                        .map(|used| used as f32 / 1e6 / dt * 100.0),
                    throttles
                        .zip(count)
                        .and_then(|(now, then)| now.checked_sub(then))
                        .map(|throttled| throttled > 0),
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
                    read(&format!("{resource}.pressure")).and_then(|text| parse_pressure(&text))
                }),
                oom_kills: field(&read("memory.events"), "oom_kill "),
                pids: limit("pids.current"),
                pids_max: limit("pids.max"),
            },
        );
    }
    *previous = next;
    units
}

/// Per-CPU busy share and wait from /proc/stat and /proc/schedstat, and clock from cpufreq,
/// with what could not be read of the latter two.
fn cpus(
    previous: &mut HashMap<u32, Ticks>,
    kinds: &HashMap<u32, CoreKind>,
    dt: f32,
) -> (Vec<Cpu>, Sources) {
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
                    wait: None,
                },
            );
        }
    }
    let schedstat = fs::read_to_string("/proc/schedstat").ok();
    if let Some(text) = &schedstat {
        for (id, values) in text.lines().filter_map(cpu_line) {
            if let (Some(entry), Some(&wait)) = (ticks.get_mut(&id), values.get(WAIT_FIELD)) {
                entry.wait = Some(wait);
            }
        }
    }
    let mut cpus: Vec<Cpu> = ticks
        .iter()
        .map(|(&id, now)| {
            // No elapsed ticks, or counters that went backwards, give no reading.
            let (busy, wait) = previous.get(&id).map_or((None, None), |before| {
                let busy = now
                    .total
                    .checked_sub(before.total)
                    .filter(|&total| total > 0)
                    .zip(now.busy.checked_sub(before.busy))
                    .map(|(total, busy)| busy as f32 / total as f32);
                let wait = now
                    .wait
                    .zip(before.wait)
                    .and_then(|(now, before)| now.checked_sub(before))
                    .map(|wait| wait as f32 / 1e9 / dt.max(0.001));
                (busy, wait)
            });
            let mhz = fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{id}/cpufreq/scaling_cur_freq"
            ))
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|&khz| khz > 0.0)
            .map(|khz| khz / 1000.0);
            Cpu {
                id,
                kind: kinds.get(&id).copied().unwrap_or(CoreKind::Unknown),
                busy: busy.map(|busy| busy.clamp(0.0, 1.0)),
                mhz,
                wait,
            }
        })
        .collect();
    cpus.sort_by_key(|cpu| cpu.id);
    *previous = ticks;
    let clocks: Vec<Option<f32>> = cpus.iter().map(|cpu| cpu.mhz).collect();
    let gaps = cpu_gaps(schedstat.as_deref(), &clocks);
    (cpus, gaps)
}

/// Index of a /proc/schedstat cpuN value (field 8 of the line, schedstat version 15 and later)
/// that counts nanoseconds spent waiting to run.
const WAIT_FIELD: usize = 7;

/// What a CPU reading lacks: the clock when no CPU has a cpufreq reading (common in virtual
/// machines), and the run queue when /proc/schedstat is unreadable or too old to count waiting.
fn cpu_gaps(schedstat: Option<&str>, clocks: &[Option<f32>]) -> Sources {
    let mut gaps = Sources::default();
    if clocks.iter().all(Option::is_none) {
        gaps.insert(Source::CpuClock);
    }
    let waits = schedstat.is_some_and(|text| {
        text.lines()
            .filter_map(cpu_line)
            .any(|(_, values)| values.len() > WAIT_FIELD)
    });
    if !waits {
        gaps.insert(Source::RunQueue);
    }
    gaps
}

/// What a machine lacks for as long as isotop runs: core kinds when the hybrid sysfs names none
/// (every CPU that is not a hybrid Intel one, ARM big.LITTLE included), and cgroups when the
/// unified hierarchy is not mounted at /sys/fs/cgroup (cgroup v1, or none) or offers no memory
/// controller, without which `account` finds no units.
fn fixed_gaps(kinds: &HashMap<u32, CoreKind>, controllers: Option<&str>) -> Sources {
    let mut gaps = Sources::default();
    if kinds.is_empty() {
        gaps.insert(Source::CoreKinds);
    }
    if !controllers.is_some_and(|text| text.split_whitespace().any(|name| name == "memory")) {
        gaps.insert(Source::Cgroups);
    }
    gaps
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

/// Pressure stall percentages from the texts of /proc/pressure/cpu, memory and io, each judged
/// on its own: a file that could not be read (kernels without PSI, or with it turned off) or
/// parsed reads 0 and is missing.
fn pressure(texts: [Option<String>; 3]) -> ([f32; 3], Sources) {
    let mut missing = Sources::default();
    let mut values = [0.0; 3];
    let sources = [
        Source::CpuPressure,
        Source::MemoryPressure,
        Source::IoPressure,
    ];
    for ((text, value), source) in texts.iter().zip(&mut values).zip(sources) {
        match text.as_deref().and_then(parse_pressure) {
            Some(reading) => *value = reading,
            None => missing.insert(source),
        }
    }
    (values, missing)
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

/// The `btime` line of /proc/stat: when the machine booted, in seconds since the Unix epoch.
fn parse_boot_time(text: &str) -> Option<f64> {
    text.lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()
}

/// When a process that started `start_ticks` ticks after boot (`USER_HZ` ticks per second) began,
/// in seconds since the Unix epoch.
fn start_time(boot_time: f64, start_ticks: u64, hz: f32) -> f64 {
    boot_time + start_ticks as f64 / f64::from(hz)
}

/// The scheduling policy of a `sched.h` policy number (field 41 of /proc/<pid>/stat).
fn policy(number: u64) -> Policy {
    match number {
        0 => Policy::Normal,
        1 => Policy::Fifo,
        2 => Policy::RoundRobin,
        3 => Policy::Batch,
        5 => Policy::Idle,
        6 => Policy::Deadline,
        _ => Policy::Unknown,
    }
}

/// Cumulative `read_bytes` and `write_bytes` of storage, and the syscall-level `rchar`, `wchar`,
/// `syscr` and `syscw`, from /proc/<pid>/io.
fn parse_io(text: &str) -> (IoBytes, SyscallCounts) {
    let mut bytes = IoBytes::default();
    let mut syscalls = SyscallCounts::default();
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
            "rchar" => syscalls.read_chars = value,
            "wchar" => syscalls.write_chars = value,
            "syscr" => syscalls.reads = value,
            "syscw" => syscalls.writes = value,
            _ => {}
        }
    }
    (bytes, syscalls)
}

/// The cumulative counters of /proc/<pid>/stat that rates are made from.
struct StatCounters {
    /// utime plus stime, in `USER_HZ` ticks.
    ticks: u64,
    /// stime, the kernel-mode part of `ticks`.
    system_ticks: u64,
    /// majflt.
    major_faults: u64,
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

fn parse_stat(pid: u32, text: &str, page_size: u64) -> Option<(Process, StatCounters)> {
    const PF_KTHREAD: u64 = 0x0020_0000;
    // comm can contain both whitespace and closing parentheses; only the last ')' delimits it.
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let fields: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    let number = |index: usize| fields.get(index)?.parse::<u64>().ok();
    let signed = |index: usize| fields.get(index)?.parse::<i32>().ok();
    let name = text.get(open + 1..close)?.to_owned();
    let rss = fields.get(21)?.parse::<i64>().ok()?.max(0) as u64;
    let kernel = number(6)? & PF_KTHREAD != 0;
    // Kernel threads hold no descriptors of their own (their tables read empty even as root), so
    // they are known to have no files; other processes wait for the background scan.
    let files = if kernel {
        Measured::Known(Files::default())
    } else {
        Measured::Pending
    };
    Some((
        // Exhaustive on purpose: a new field must be decided here.
        Process {
            id: Identity {
                pid,
                start: number(19)?,
            },
            parent: number(1)? as u32,
            command: name.clone(),
            name,
            group: String::new(),
            kind: if kernel { Kind::Kernel } else { Kind::System },
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
            gpu_memory: None,
            // Field 39 of /proc/<pid>/stat, counted from the state as field 3.
            core: number(36).map(|core| core as u32),
            cpu_time: 0.0,
            cgroup: String::new(),
            performance_share: None,
            waiting: None,
            files,
            locks_held: Measured::Pending,
            blocked_on: None,
            system_share: None,
            // Field 41 of /proc/<pid>/stat; a kernel without it reads as unknown.
            policy: number(38).map_or(Policy::Unknown, policy),
            syscall_io: None,
            major_faults: None,
            // Needs the boot time, which `Sampler::processes` adds.
            started: 0.0,
        },
        StatCounters {
            ticks: number(11)?.saturating_add(number(12)?),
            system_ticks: number(12)?,
            // Field 12 of /proc/<pid>/stat.
            major_faults: number(9)?,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::describe;

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
        let (p, counters) = parse_stat(73, &text, 4096).unwrap();
        assert_eq!(p.name, "a tricky ) name");
        assert_eq!(p.parent, 42);
        assert_eq!(p.id.start, 999);
        assert_eq!(counters.ticks, 130);
        assert_eq!(p.memory, 0);
        assert!(parse_stat(73, "73 (short) S", 4096).is_none());
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
        assert_eq!(p.core, Some(17));
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
    fn linux_reports_missing_cpufreq_and_schedstat() {
        let schedstat = "version 15\ntimestamp 1\ncpu0 0 0 0 0 0 0 6891 2519 25\ndomain0 3 0\n";
        assert!(cpu_gaps(Some(schedstat), &[Some(2400.0), None]).is_empty());
        assert_eq!(
            cpu_gaps(Some(schedstat), &[None, None]),
            Sources::from(Source::CpuClock),
            "no CPU has a cpufreq reading"
        );
        assert_eq!(cpu_gaps(Some(schedstat), &[]), Source::CpuClock.into());
        assert_eq!(
            cpu_gaps(None, &[Some(2400.0)]),
            Source::RunQueue.into(),
            "/proc/schedstat is unreadable"
        );
        assert_eq!(
            cpu_gaps(Some("version 14\ncpu0 0 0 0 0 0 0 6891\n"), &[Some(2400.0)]),
            Source::RunQueue.into(),
            "a schedstat too old to count waiting"
        );
        assert_eq!(
            cpu_gaps(None, &[None]),
            Sources::from([Source::CpuClock, Source::RunQueue])
        );
    }

    #[test]
    fn cmdline_is_reread_after_exec_and_cached_otherwise() {
        let pid = 4321;
        assert!(reread(None, "bash", pid, 7), "nothing cached yet");
        let due = (0..TEXT_REFRESH)
            .filter(|&sample| reread(Some("bash"), "bash", pid, sample))
            .count();
        assert_eq!(due, 1, "the same comm is re-read once per refresh period");
        for sample in 0..TEXT_REFRESH {
            assert!(reread(Some("bash"), "python3", pid, sample), "exec");
        }
        let staggered: HashSet<u64> = (0..TEXT_REFRESH as u32)
            .map(|pid| (0..TEXT_REFRESH).find(|&sample| reread(Some("x"), "x", pid, sample)))
            .map(Option::unwrap)
            .collect();
        assert_eq!(
            staggered.len(),
            TEXT_REFRESH as usize,
            "spread over the samples"
        );
    }

    #[test]
    fn linux_reports_missing_core_kinds_and_cgroup_v2() {
        let hybrid = HashMap::from([(0, CoreKind::Performance), (1, CoreKind::Efficiency)]);
        let unified = "cpuset cpu io memory hugetlb pids rdma misc\n";
        assert!(fixed_gaps(&hybrid, Some(unified)).is_empty());
        assert_eq!(
            fixed_gaps(&HashMap::new(), Some(unified)),
            Source::CoreKinds.into(),
            "no hybrid sysfs, as on ARM big.LITTLE or a CPU of one kind"
        );
        assert_eq!(
            fixed_gaps(&hybrid, None),
            Source::Cgroups.into(),
            "cgroup v1, or no cgroups mounted"
        );
        assert_eq!(
            fixed_gaps(&hybrid, Some("cpu io pids\n")),
            Source::Cgroups.into(),
            "no memory controller, so no units"
        );
    }

    #[test]
    fn linux_sampler_reports_cgroups_missing_when_no_process_sits_in_cgroup_v2() {
        let mut sampler = Sampler::new();
        sampler.fixed_gaps = Sources::default();
        sampler.grouped = true;
        assert!(sampler.missing().is_empty());
        sampler.grouped = false;
        assert_eq!(sampler.missing(), Source::Cgroups.into());
    }

    #[test]
    fn pressure_is_judged_per_file() {
        let reading = |value: &str| Some(format!("some avg10={value} avg60=0.00\nfull avg10=0.00"));
        let (values, missing) = pressure([reading("4.00"), reading("2.50"), reading("1.00")]);
        assert_eq!(values, [4.0, 2.5, 1.0]);
        assert!(missing.is_empty());
        let (values, missing) = pressure([reading("4.00"), None, Some("garbled".into())]);
        assert_eq!(values, [4.0, 0.0, 0.0]);
        assert_eq!(
            missing,
            Sources::from([Source::MemoryPressure, Source::IoPressure])
        );
        let (_, missing) = pressure([None, None, None]);
        assert_eq!(
            missing,
            Sources::from([
                Source::CpuPressure,
                Source::MemoryPressure,
                Source::IoPressure
            ])
        );
    }

    /// A cgroup directory under a fresh root, holding the given files.
    fn cgroup_with(name: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, HashSet<String>) {
        let root = std::env::temp_dir().join(format!("isotop-{name}-{}", std::process::id()));
        let cgroup = root.join("system.slice/x.service");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&cgroup).unwrap();
        for (file, text) in files {
            fs::write(cgroup.join(file), text).unwrap();
        }
        (root, HashSet::from(["/system.slice/x.service".to_string()]))
    }

    #[test]
    fn unreadable_cgroup_pressure_is_none() {
        let (root, paths) = cgroup_with(
            "pressure",
            &[
                ("memory.current", "4096\n"),
                (
                    "cpu.pressure",
                    "some avg10=1.50 avg60=0.00\nfull avg10=0.00\n",
                ),
                ("memory.pressure", "garbled"),
            ],
        );
        let units = account_under(&root, &paths, &mut HashMap::new());
        let unit = &units["/system.slice/x.service"];
        assert_eq!(unit.memory, 4096);
        // Only the file that parses is a reading: a missing or garbled one is not 0.0.
        assert_eq!(unit.pressure, [Some(1.5), None, None]);
        assert_eq!((unit.pids, unit.oom_kills), (None, None));
        assert_eq!((unit.cpu, unit.throttled), (None, None));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cgroup_rates_need_an_earlier_reading_and_counters_that_did_not_go_back() {
        let stat =
            |usage: u64, throttled: u64| format!("usage_usec {usage}\nnr_throttled {throttled}\n");
        let (root, paths) = cgroup_with(
            "rates",
            &[
                ("memory.current", "4096\n"),
                ("cpu.stat", &stat(5_000_000, 3)),
                ("pids.current", "7\n"),
                ("memory.events", "low 0\noom_kill 2\n"),
            ],
        );
        let cgroup = root.join("system.slice/x.service");
        let mut previous = HashMap::new();
        let first = &account_under(&root, &paths, &mut previous)["/system.slice/x.service"];
        assert_eq!((first.cpu, first.throttled), (None, None), "first reading");
        assert_eq!((first.pids, first.oom_kills), (Some(7), Some(2)));
        fs::write(cgroup.join("cpu.stat"), stat(7_000_000, 3)).unwrap();
        let second = &account_under(&root, &paths, &mut previous)["/system.slice/x.service"];
        assert!(second.cpu.is_some_and(|cpu| cpu > 0.0));
        assert_eq!(second.throttled, Some(false));
        fs::write(cgroup.join("cpu.stat"), stat(100, 1)).unwrap();
        let reset = &account_under(&root, &paths, &mut previous)["/system.slice/x.service"];
        assert_eq!(
            (reset.cpu, reset.throttled),
            (None, None),
            "counters went back"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn meminfo_reads_each_figure_on_its_own() {
        let text = "MemTotal:       32768 kB\nMemFree: 1 kB\nMemAvailable:   16384 kB\n";
        assert_eq!(meminfo(text), (Some(32768 * 1024), Some(16384 * 1024)));
        assert_eq!(meminfo("MemTotal: 8 kB\n"), (Some(8192), None));
        assert_eq!(meminfo("MemAvailable: lots kB\n"), (None, None));
        assert_eq!(meminfo(""), (None, None));
    }

    #[test]
    fn stat_without_a_last_cpu_field_has_no_core() {
        let (process, _) = parse_stat(5, &stat_line(|_| {}), 4096).unwrap();
        assert_eq!(process.core, None);
    }

    #[test]
    fn text_cache_holds_only_the_processes_of_the_latest_sample() {
        let mut sampler = Sampler::new();
        for _ in 0..2 {
            let processes = sampler.processes().unwrap();
            let ids: HashSet<Identity> = processes.iter().map(|raw| raw.process.id).collect();
            let cached: HashSet<Identity> = sampler.texts.keys().copied().collect();
            assert_eq!(cached, ids);
            let own = processes
                .iter()
                .find(|raw| raw.process.id.pid == std::process::id())
                .unwrap();
            assert!(
                own.process.command.contains("isotop"),
                "{}",
                own.process.command
            );
        }
    }

    #[test]
    fn io_counters_come_from_read_bytes_and_write_bytes() {
        let text = "rchar: 99\nwchar: 98\nsyscr: 1\nsyscw: 1\nread_bytes: 4096\nwrite_bytes: 1000\ncancelled_write_bytes: 0\n";
        assert_eq!(
            parse_io(text).0,
            IoBytes {
                read: 4096,
                write: 1000
            }
        );
    }

    #[test]
    fn syscall_io_rates_come_from_rchar_wchar_syscr_and_syscw() {
        let text = "rchar: 99\nwchar: 98\nsyscr: 7\nsyscw: 5\nread_bytes: 4096\nwrite_bytes: 1000\ncancelled_write_bytes: 0\n";
        assert_eq!(
            parse_io(text).1,
            SyscallCounts {
                read_chars: 99,
                write_chars: 98,
                reads: 7,
                writes: 5
            }
        );
        assert_eq!(parse_io("read_bytes: 1\n").1, SyscallCounts::default());
    }

    #[test]
    fn stat_parsing_keeps_user_and_system_time_apart() {
        let text = stat_line(|fields| {
            fields[11] = "123";
            fields[12] = "7";
        });
        let (_, counters) = parse_stat(5, &text, 4096).unwrap();
        assert_eq!(counters.ticks, 130);
        assert_eq!(counters.system_ticks, 7);
    }

    #[test]
    fn policy_and_major_faults_parse_from_stat() {
        let with_policy = |number: &'static str| {
            let mut fields = vec!["0"; 40];
            fields[0] = "S";
            fields[9] = "812";
            fields[38] = number;
            format!("5 (x) {}", fields.join(" "))
        };
        let (fifo, counters) = parse_stat(5, &with_policy("1"), 4096).unwrap();
        assert_eq!(fifo.policy, Policy::Fifo);
        assert_eq!(counters.major_faults, 812);
        let (batch, _) = parse_stat(5, &with_policy("3"), 4096).unwrap();
        assert_eq!(batch.policy, Policy::Batch);
        for (number, expected) in [
            ("0", Policy::Normal),
            ("2", Policy::RoundRobin),
            ("5", Policy::Idle),
            ("6", Policy::Deadline),
            ("4", Policy::Unknown),
        ] {
            let (process, _) = parse_stat(5, &with_policy(number), 4096).unwrap();
            assert_eq!(process.policy, expected, "policy {number}");
        }
        let (short, _) = parse_stat(5, &stat_line(|_| {}), 4096).unwrap();
        assert_eq!(
            short.policy,
            Policy::Unknown,
            "a stat line without field 41"
        );
    }

    #[test]
    fn start_time_converts_to_seconds_since_the_epoch() {
        let stat = "cpu  1 2 3\nintr 5\nbtime 1700000000\nprocesses 99\n";
        assert_eq!(parse_boot_time(stat), Some(1_700_000_000.0));
        assert_eq!(parse_boot_time("cpu  1 2 3\n"), None);
        assert_eq!(start_time(1_700_000_000.0, 250, 100.0), 1_700_000_002.5);
        assert_eq!(start_time(1_700_000_000.0, 0, 100.0), 1_700_000_000.0);
    }
}
