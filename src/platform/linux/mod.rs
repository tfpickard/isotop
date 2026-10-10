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

use crate::model::{CoreKind, Cpu, Identity, IoBytes, Kind, Measured, Process, Unit};
use crate::platform::{Files, Network, RawProcess, Shadow};

pub use files::{FileScan, locks};
pub use journal::{JOURNAL, journal};
pub use nvml::Nvml as Gpu;

/// What `Process::memory` measures here: the resident set size.
pub const MEMORY_LABEL: &str = "RSS";

/// Reads processes, memory, pressure and CPUs, keeping the CPU counters between readings.
pub struct Sampler {
    cpu_previous: HashMap<u32, Ticks>,
    core_kinds: HashMap<u32, CoreKind>,
    hz: f32,
    page_size: u64,
}

impl Sampler {
    pub fn new() -> Self {
        Self {
            cpu_previous: HashMap::new(),
            core_kinds: core_kinds(),
            // These sysconf selectors return positive values on supported Linux systems.
            hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f32,
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
        }
    }

    /// USER_HZ, the unit of the CPU times in /proc/<pid>/stat.
    pub fn hz(&self) -> f32 {
        self.hz
    }

    /// Every process in /proc whose stat file could be read and parsed.
    pub fn processes(&mut self) -> io::Result<Vec<RawProcess>> {
        let mut processes = Vec::new();
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
            let io = fs::read_to_string(path.join("io"))
                .ok()
                .map(|text| parse_io(&text));
            process.written = io.map(|io| io.write);
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
            processes.push(RawProcess {
                process,
                ticks,
                io,
                performance_ticks: None,
                runnable_ticks: None,
            });
        }
        Ok(processes)
    }

    /// Total and available memory in bytes from /proc/meminfo; 0 when unreadable.
    pub fn memory(&self) -> (u64, u64) {
        let (mut total, mut available) = (0, 0);
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
                    "MemTotal:" => total = value,
                    "MemAvailable:" => available = value,
                    _ => {}
                }
            }
        }
        (total, available)
    }

    /// Pressure stall percentages, and whether PSI is available.
    pub fn pressure(&self) -> ([f32; 3], bool) {
        pressure()
    }

    /// Per-CPU use since the previous call, `dt` seconds ago.
    pub fn cpus(&mut self, dt: f32) -> Vec<Cpu> {
        cpus(&mut self.cpu_previous, &self.core_kinds, dt)
    }

    /// Linux measures everything isotop shows.
    pub fn missing(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Linux measures every CPU on its own.
    pub fn per_cluster(&self) -> Vec<&'static str> {
        Vec::new()
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

/// Socket links between processes.
pub fn network() -> Network {
    net::sample()
}

/// CPU counters from /proc/stat and /proc/schedstat: busy and total ticks, waiting nanoseconds.
#[derive(Clone, Copy)]
struct Ticks {
    busy: u64,
    total: u64,
    wait: u64,
}

/// Reads the accounting files of each cgroup. CPU use comes from the change in usage since the
/// previous reading.
pub fn account(
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

/// Cumulative `read_bytes` and `write_bytes` from /proc/<pid>/io.
fn parse_io(text: &str) -> IoBytes {
    let mut bytes = IoBytes::default();
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
    let kernel = number(6)? & PF_KTHREAD != 0;
    // Kernel threads hold no descriptors of their own (their tables read empty even as root), so
    // they are known to have no files; other processes wait for the background scan.
    let files = if kernel {
        Measured::Known(Files::default())
    } else {
        Measured::Pending
    };
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
            gpu_memory: 0,
            // Field 39 of /proc/<pid>/stat, counted from the state as field 3.
            core: number(36).unwrap_or(0) as u32,
            cpu_time: 0.0,
            cgroup: String::new(),
            performance_share: None,
            waiting: None,
            files,
            locks_held: Measured::Pending,
            blocked_on: None,
        },
        number(11)?.saturating_add(number(12)?),
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
        let (p, ticks) = parse_stat(73, &text, 4096).unwrap();
        assert_eq!(p.name, "a tricky ) name");
        assert_eq!(p.parent, 42);
        assert_eq!(p.id.start, 999);
        assert_eq!(ticks, 130);
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
        assert_eq!(p.core, 17);
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
    fn io_counters_come_from_read_bytes_and_write_bytes() {
        let text = "rchar: 99\nwchar: 98\nsyscr: 1\nsyscw: 1\nread_bytes: 4096\nwrite_bytes: 1000\ncancelled_write_bytes: 0\n";
        assert_eq!(
            parse_io(text),
            IoBytes {
                read: 4096,
                write: 1000
            }
        );
    }
}
