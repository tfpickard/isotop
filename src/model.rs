use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{net, nvml};

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
    pub io_rate: Option<f32>,
    pub threads: u32,
    /// NVIDIA GPU memory in bytes; 1 means in use with unreported memory.
    pub gpu_memory: u64,
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
    /// TCP connections leaving the machine, per process.
    pub outside: HashMap<Identity, u32>,
}

struct Counters {
    ticks: u64,
    io: Option<u64>,
    cpu: f32,
}

/// Slow measurements gathered off the frame loop.
#[derive(Default)]
struct Extras {
    network: net::Network,
    gpu: HashMap<u32, u64>,
}

pub struct Collector {
    previous: HashMap<Identity, Counters>,
    last: Instant,
    origin: Instant,
    hz: f32,
    page_size: u64,
    extras: Option<Arc<Mutex<Extras>>>,
    started: bool,
}

/// Samples sockets and GPU usage every two seconds on a background thread; the thread stops
/// once the collector is gone. Returns None when no thread can be spawned.
fn background() -> Option<Arc<Mutex<Extras>>> {
    let shared = Arc::new(Mutex::new(Extras::default()));
    let writer = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("isotop-sampler".into())
        .spawn(move || {
            let nvml = nvml::Nvml::load();
            while Arc::strong_count(&writer) > 1 {
                let extras = Extras {
                    network: net::sample(),
                    gpu: nvml.as_ref().map(nvml::Nvml::sample).unwrap_or_default(),
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

fn pressure() -> [f32; 3] {
    ["cpu", "memory", "io"].map(|resource| {
        fs::read_to_string(format!("/proc/pressure/{resource}"))
            .ok()
            .and_then(|text| parse_pressure(&text))
            .unwrap_or(0.0)
    })
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
    pub fn new() -> Self {
        Self {
            previous: HashMap::new(),
            last: Instant::now(),
            origin: Instant::now(),
            // These sysconf selectors return positive values on supported Linux systems.
            hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f32,
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
            extras: None,
            started: false,
        }
    }

    pub fn sample(&mut self) -> std::io::Result<Snapshot> {
        if !std::mem::replace(&mut self.started, true) {
            self.extras = background();
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().max(0.001);
        let mut snapshot = Snapshot {
            cores: std::thread::available_parallelism().map_or(1, usize::from),
            elapsed: now.duration_since(self.origin).as_secs_f64(),
            pressure: pressure(),
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
            let io = fs::read_to_string(path.join("io")).ok().map(|text| {
                text.lines()
                    .filter_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        matches!(key, "read_bytes" | "write_bytes")
                            .then(|| value.trim().parse::<u64>().ok())
                            .flatten()
                    })
                    .sum::<u64>()
            });
            if let Some(previous) = self.previous.get(&process.id) {
                let raw = ticks.saturating_sub(previous.ticks) as f32 / self.hz / dt * 100.0;
                let alpha = 1.0 - (-dt / 1.5).exp();
                process.cpu = previous.cpu + alpha * (raw - previous.cpu);
                process.io_rate = io
                    .zip(previous.io)
                    .map(|(a, b)| a.saturating_sub(b) as f32 / dt);
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
            threads: number(17)? as u32,
            gpu_memory: 0,
        },
        number(11)?.saturating_add(number(12)?),
    ))
}

pub fn bounded(value: f32, knee: f32) -> f32 {
    let value = value.max(0.0);
    value / (value + knee)
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
            group: format!("district-{:02}.service", family),
            kind: [Kind::System, Kind::Session, Kind::Container, Kind::Kernel][family % 4],
            state: if i % 41 == 0 {
                'Z'
            } else if cpu > 30.0 {
                'R'
            } else {
                'S'
            },
            cpu,
            memory: memory as u64,
            io_rate: Some(if i % 7 == 0 { cpu * 65536.0 } else { 0.0 }),
            threads: if i % 5 == 0 { 4 << (i % 6) } else { 1 },
            gpu_memory: if i % 37 == 5 {
                (300 + i as u64 * 7) << 20
            } else {
                0
            },
        });
    }
    let id = |i: usize| processes[i % count].id;
    let links = (1..count)
        .filter(|i| i % 3 == 0)
        .map(|i| (id(i), id(i * 7 + 16), 1 + (i % 4) as u32))
        .collect();
    let outside = (0..count)
        .filter(|i| i % 11 == 0)
        .map(|i| (id(i), 1 + (i % 3) as u32))
        .collect();
    let wave = |rate: f64, scale: f32| ((time * rate).sin() as f32).max(0.0) * scale;
    Snapshot {
        processes,
        memory_total: 32 * 1024_u64.pow(3),
        memory_available: 12 * 1024_u64.pow(3),
        cores: 16,
        elapsed: time,
        pressure: [wave(0.21, 30.0), wave(0.13, 18.0), wave(0.17, 24.0)],
        links,
        outside,
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
}
