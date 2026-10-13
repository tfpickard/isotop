//! The kernel counters on the background thread: what each pass reads, the rates it turns the
//! counts into, and the demo's synthesized values. The platform reads and parses one file per
//! call (`platform::interrupts` and the others); this module decides whether to read at all
//! (`Reads`, so only while a shown view declares the source), joins the affinity onto the rows
//! and divides the change in each count by the time between two passes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{CpuCount, Interrupt, Measured, Reads, Softirq, timed};
use crate::platform::{self, Source};

/// The row of `interrupts` that counts the timer: `LOC` on x86; where there is none (arm64), the
/// first numbered row whose device is `arch_timer`.
pub fn timer_row(rows: &[Interrupt]) -> Option<&Interrupt> {
    rows.iter().find(|row| row.id == "LOC").or_else(|| {
        rows.iter()
            .find(|row| row.devices.iter().any(|device| device == "arch_timer"))
    })
}

impl Interrupt {
    /// Whether the row is an interrupt number, which has devices and an affinity, and not a named
    /// counter such as `LOC`.
    pub fn is_numbered(&self) -> bool {
        !self.id.is_empty() && self.id.bytes().all(|byte| byte.is_ascii_digit())
    }
}

/// What one pass read, as `Snapshot` holds it.
#[derive(Clone, Debug, Default)]
pub struct KernelReadings {
    pub interrupts: Measured<Vec<Interrupt>>,
    pub softirqs: Measured<Vec<Softirq>>,
    pub oom_kills: Option<u64>,
}

/// The platform reads one pass makes; the tests replace them.
#[derive(Clone, Copy)]
pub struct Readers {
    pub interrupts: fn() -> Option<Vec<Interrupt>>,
    pub softirqs: fn() -> Option<Vec<Softirq>>,
    pub affinity: fn(&str) -> Option<Vec<u32>>,
    pub oom_kills: fn() -> Option<u64>,
}

impl Readers {
    pub const PLATFORM: Readers = Readers {
        interrupts: platform::interrupts,
        softirqs: platform::softirqs,
        affinity: platform::irq_affinity,
        oom_kills: platform::oom_kills,
    };
}

/// The counts the previous pass read from one table, and when.
#[derive(Default)]
struct Previous {
    at: Option<Instant>,
    counts: HashMap<String, Vec<(u32, u64)>>,
}

/// The shortest interval a rate divides by, in seconds, so two reads in quick succession do not
/// give a huge rate.
const SHORTEST_INTERVAL: f32 = 0.1;

impl Previous {
    /// Fills in the rate of every count in `rows` (a row's id and its counts) from the change since
    /// the previous pass, over the time between the two reads, and keeps these counts for the
    /// next. A row, or a CPU of a row, the previous pass did not have gets no rate, nor does a
    /// count that went backwards (an interrupt number reused for another device): a reading that
    /// was not taken is None, never 0. A row that disappeared is forgotten.
    fn rates<'a>(
        &mut self,
        rows: impl Iterator<Item = (&'a str, &'a mut Vec<CpuCount>)>,
        now: Instant,
    ) {
        let interval = self.at.map(|then| {
            now.saturating_duration_since(then)
                .as_secs_f32()
                .max(SHORTEST_INTERVAL)
        });
        let mut next = HashMap::with_capacity(self.counts.len());
        for (id, counts) in rows {
            if let (Some(interval), Some(before)) = (interval, self.counts.get(id)) {
                for (index, entry) in counts.iter_mut().enumerate() {
                    // Columns normally line up with the previous pass; look the CPU up when a
                    // CPU went offline or came back.
                    let earlier = before
                        .get(index)
                        .filter(|&&(cpu, _)| cpu == entry.0)
                        .or_else(|| before.iter().find(|&&(cpu, _)| cpu == entry.0));
                    entry.2 = earlier
                        .and_then(|&(_, count)| entry.1.checked_sub(count))
                        .map(|change| change as f32 / interval);
                }
            }
            next.insert(
                id.to_owned(),
                counts.iter().map(|&(cpu, count, _)| (cpu, count)).collect(),
            );
        }
        self.at = Some(now);
        self.counts = next;
    }
}

/// The state the background thread keeps between passes: the counts each table had.
#[derive(Default)]
pub struct KernelScan {
    interrupts: Previous,
    softirqs: Previous,
}

impl KernelScan {
    /// One pass: reads each source that is demanded (and only those: an undemanded source's file
    /// is not opened), records how long each read took in `latency`, and turns counts into
    /// rates. A table that is not read this pass is forgotten, so rates after a pause in demand
    /// begin again with a first pass instead of averaging over the pause.
    ///
    /// The affinity is read for the numbered rows of the interrupt table, so `IrqAffinity`
    /// without `Interrupts` has no rows to read it for and is missing.
    pub(super) fn scan(
        &mut self,
        readers: Readers,
        reads: &mut Reads,
        latency: &mut Vec<(Source, Duration)>,
    ) -> KernelReadings {
        let mut interrupts = reads.measure(Source::Interrupts, || {
            timed(latency, Source::Interrupts, readers.interrupts)
        });
        let mut softirqs = reads.measure(Source::Softirqs, || {
            timed(latency, Source::Softirqs, readers.softirqs)
        });
        let now = Instant::now();
        match &mut interrupts {
            Measured::Known(rows) => self.interrupts.rates(
                rows.iter_mut()
                    .map(|row| (row.id.as_str(), &mut row.per_cpu)),
                now,
            ),
            _ => self.interrupts = Previous::default(),
        }
        match &mut softirqs {
            Measured::Known(rows) => self.softirqs.rates(
                rows.iter_mut()
                    .map(|(name, counts)| (name.as_str(), counts)),
                now,
            ),
            _ => self.softirqs = Previous::default(),
        }
        let table = match &mut interrupts {
            Measured::Known(rows) => Some(rows),
            _ => None,
        };
        reads.read(Source::IrqAffinity, || {
            let rows = table?;
            timed(latency, Source::IrqAffinity, || {
                let mut any = false;
                for row in rows.iter_mut().filter(|row| row.is_numbered()) {
                    row.affinity = (readers.affinity)(&row.id);
                    any |= row.affinity.is_some();
                }
                any.then_some(())
            })
        });
        let oom_kills = reads.read(Source::OomKills, || {
            timed(latency, Source::OomKills, readers.oom_kills)
        });
        KernelReadings {
            interrupts,
            softirqs,
            oom_kills,
        }
    }
}

/// Seconds in the day of uptime the demo's counts start from, so that they are positive at
/// negative times too.
pub(super) const DEMO_UPTIME: f64 = 86_400.0;

/// Seconds the moving interrupt stays on one CPU.
const DEMO_MOVE_PERIOD: f64 = 37.0;

/// A rate in events per second that wanders: `base * (mean + swing * sin(speed * t + phase))`,
/// never negative while `swing <= mean`. Its count since time 0 is the integral.
#[derive(Clone, Copy)]
pub(super) struct Wave {
    pub(super) base: f64,
    pub(super) mean: f64,
    pub(super) swing: f64,
    pub(super) speed: f64,
    pub(super) phase: f64,
}

impl Wave {
    pub(super) fn rate(self, time: f64) -> f64 {
        self.base * (self.mean + self.swing * (self.speed * time + self.phase).sin())
    }

    /// The events from time 0 to `time`: the integral of `rate`.
    pub(super) fn total(self, time: f64) -> f64 {
        self.base
            * (self.mean * time
                + self.swing / self.speed
                    * (self.phase.cos() - (self.speed * time + self.phase).cos()))
    }

    pub(super) fn between(self, from: f64, to: f64) -> f64 {
        self.total(to) - self.total(from)
    }

    /// The count since boot at `time`, taking `share` of the events.
    pub(super) fn count(self, time: f64, share: f64) -> u64 {
        (share * (self.base * self.mean * DEMO_UPTIME + self.total(time))).max(0.0) as u64
    }
}

/// Which CPUs serve a numbered demo interrupt.
#[derive(Clone, Copy)]
enum Serving {
    Fixed(&'static [u32]),
    /// One CPU at a time, a new one every `DEMO_MOVE_PERIOD`.
    Moving,
}

/// The CPU that serves the moving interrupt in epoch `epoch` (counted from time 0): a step of 5
/// visits all 16 CPUs before it repeats.
fn moving_cpu(epoch: i64, cores: usize) -> u32 {
    (3 + 5 * epoch).rem_euclid(cores.max(1) as i64) as u32
}

/// A numbered demo interrupt.
struct DemoDevice {
    id: &'static str,
    chip: &'static str,
    hwirq: u64,
    trigger: &'static str,
    device: &'static str,
    serving: Serving,
    wave: Wave,
}

const fn device(
    id: &'static str,
    chip: &'static str,
    hwirq: u64,
    device: &'static str,
    serving: Serving,
    base: f64,
    speed: f64,
) -> DemoDevice {
    DemoDevice {
        id,
        chip,
        hwirq,
        trigger: "edge",
        device,
        serving,
        wave: Wave {
            base,
            mean: 0.6,
            swing: 0.4,
            speed,
            phase: hwirq as f64 * 0.37,
        },
    }
}

const DEMO_DEVICES: [DemoDevice; 13] = [
    device(
        "27",
        "IR-PCI-MSIX-0000:01:00.0",
        0,
        "nvme0q0",
        Serving::Fixed(&[0]),
        0.3,
        0.11,
    ),
    device(
        "28",
        "IR-PCI-MSIX-0000:01:00.0",
        1,
        "nvme0q1",
        Serving::Fixed(&[2, 3]),
        140.0,
        0.23,
    ),
    device(
        "29",
        "IR-PCI-MSIX-0000:01:00.0",
        2,
        "nvme0q2",
        Serving::Fixed(&[4, 5]),
        110.0,
        0.19,
    ),
    device(
        "30",
        "IR-PCI-MSIX-0000:01:00.0",
        3,
        "nvme0q3",
        Serving::Fixed(&[6, 7]),
        90.0,
        0.29,
    ),
    device(
        "31",
        "IR-PCI-MSIX-0000:01:00.0",
        4,
        "nvme0q4",
        Serving::Fixed(&[8, 9]),
        75.0,
        0.17,
    ),
    device(
        "32",
        "IR-PCI-MSIX-0000:01:00.0",
        5,
        "nvme0q5",
        Serving::Fixed(&[10, 11]),
        60.0,
        0.31,
    ),
    device(
        "33",
        "IR-PCI-MSIX-0000:01:00.0",
        6,
        "nvme0q6",
        Serving::Fixed(&[12, 13]),
        45.0,
        0.13,
    ),
    device(
        "34",
        "IR-PCI-MSIX-0000:01:00.0",
        7,
        "nvme0q7",
        Serving::Fixed(&[14, 15]),
        30.0,
        0.27,
    ),
    device(
        "35",
        "IR-PCI-MSI-0000:00:14.0",
        327680,
        "xhci_hcd",
        Serving::Fixed(&[4]),
        6.0,
        0.21,
    ),
    device(
        "36",
        "IR-PCI-MSIX-0000:00:14.3",
        0,
        "iwlwifi",
        Serving::Moving,
        60.0,
        0.15,
    ),
    device(
        "37",
        "IR-PCI-MSI-0000:00:02.0",
        32768,
        "i915",
        Serving::Fixed(&[6]),
        90.0,
        0.25,
    ),
    device(
        "38",
        "IR-PCI-MSI-0000:00:17.0",
        376832,
        "ahci[0000:00:17.0]",
        Serving::Fixed(&[8]),
        12.0,
        0.33,
    ),
    device(
        "39",
        "IR-PCI-MSI-0000:00:1f.6",
        0,
        "e1000e",
        Serving::Fixed(&[1]),
        25.0,
        0.35,
    ),
];

/// A named demo row, counted on every CPU with a wave of its own.
pub(super) struct DemoCounter {
    pub(super) id: &'static str,
    pub(super) description: &'static str,
    pub(super) wave: Wave,
}

const fn counter(
    id: &'static str,
    description: &'static str,
    base: f64,
    mean: f64,
    speed: f64,
) -> DemoCounter {
    DemoCounter {
        id,
        description,
        wave: Wave {
            base,
            mean,
            swing: mean,
            speed,
            phase: base,
        },
    }
}

/// The named rows after the LOC row, which follows the CPU's busy share.
pub(super) const DEMO_COUNTERS: [DemoCounter; 4] = [
    counter("NMI", "Non-maskable interrupts", 0.1, 0.5, 0.07),
    counter("RES", "Rescheduling interrupts", 30.0, 0.6, 0.23),
    counter("CAL", "Function call interrupts", 12.0, 0.7, 0.31),
    counter("TLB", "TLB shootdowns", 18.0, 0.5, 0.41),
];

/// The softirqs in the kernel's order, each with the events per second per CPU.
pub(super) const DEMO_SOFTIRQS: [(&str, f64, f64, f64); 10] = [
    ("HI", 0.5, 0.5, 0.13),
    ("TIMER", 230.0, 0.7, 0.4),
    ("NET_TX", 6.0, 0.5, 0.21),
    ("NET_RX", 70.0, 0.5, 0.17),
    ("BLOCK", 90.0, 0.5, 0.11),
    ("IRQ_POLL", 0.0, 0.5, 0.19),
    ("TASKLET", 3.0, 0.5, 0.29),
    ("SCHED", 320.0, 0.7, 0.37),
    ("HRTIMER", 45.0, 0.6, 0.19),
    ("RCU", 210.0, 0.6, 0.33),
];

/// The wave of `LOC` on one CPU: about 250 Hz scaled by the CPU's busy share, which the demo's
/// `Cpu::busy` computes as `(0.5 + 0.5 sin(0.4 t + 0.9 id))` times 0.9 on the first eight CPUs and
/// 0.6 on the others, so a busier CPU takes more timer interrupts.
pub(super) fn local_timer(cpu: u32) -> Wave {
    let scale = if cpu < 8 { 0.9 } else { 0.6 };
    Wave {
        base: 250.0,
        mean: 0.3 + 0.35 * scale,
        swing: 0.35 * scale,
        speed: 0.4,
        phase: 0.9 * f64::from(cpu),
    }
}

/// The wave of a per-CPU counter on `cpu`: the row's own, shifted by the CPU.
pub(super) fn on_cpu(wave: Wave, cpu: u32) -> Wave {
    Wave {
        phase: wave.phase + 0.9 * f64::from(cpu),
        ..wave
    }
}

pub(super) fn counted(wave: Wave, cpu: u32, time: f64) -> CpuCount {
    (cpu, wave.count(time, 1.0), Some(wave.rate(time) as f32))
}

/// The demo's interrupt table and softirqs at `time` on `cores` CPUs, a pure function of the
/// time: counts are the integral of the rates, so they only grow.
pub fn demo(time: f64, cores: usize) -> (Measured<Vec<Interrupt>>, Measured<Vec<Softirq>>) {
    let cpus = 0..cores as u32;
    let epoch = ((time / DEMO_MOVE_PERIOD).floor() as i64).max(0);
    let mut interrupts = Vec::new();
    for device in &DEMO_DEVICES {
        let serving: Vec<u32> = match device.serving {
            Serving::Fixed(cpus) => cpus
                .iter()
                .copied()
                .filter(|&cpu| (cpu as usize) < cores)
                .collect(),
            Serving::Moving => vec![moving_cpu(epoch, cores)],
        };
        let per_cpu = cpus
            .clone()
            .map(|cpu| match device.serving {
                Serving::Fixed(_) if serving.contains(&cpu) => {
                    let share = 1.0 / serving.len() as f64;
                    (
                        cpu,
                        device.wave.count(time, share),
                        Some((device.wave.rate(time) * share) as f32),
                    )
                }
                Serving::Fixed(_) => (cpu, 0, Some(0.0)),
                Serving::Moving => moved(device, cpu, time, cores),
            })
            .collect();
        interrupts.push(Interrupt {
            id: device.id.to_owned(),
            chip: device.chip.to_owned(),
            hwirq: Some(device.hwirq),
            trigger: device.trigger.to_owned(),
            devices: vec![device.device.to_owned()],
            description: String::new(),
            per_cpu,
            total_only: false,
            affinity: Some(serving),
        });
    }
    let named = |id: &str, description: &str, wave: &dyn Fn(u32) -> Wave| Interrupt {
        id: id.to_owned(),
        description: description.to_owned(),
        per_cpu: cpus
            .clone()
            .map(|cpu| counted(wave(cpu), cpu, time))
            .collect(),
        ..Interrupt::default()
    };
    interrupts.push(named(
        DEMO_COUNTERS[0].id,
        DEMO_COUNTERS[0].description,
        &|cpu| on_cpu(DEMO_COUNTERS[0].wave, cpu),
    ));
    interrupts.push(named("LOC", "Local timer interrupts", &local_timer));
    for counter in &DEMO_COUNTERS[1..] {
        interrupts.push(named(counter.id, counter.description, &|cpu| {
            on_cpu(counter.wave, cpu)
        }));
    }
    let softirqs = DEMO_SOFTIRQS
        .iter()
        .enumerate()
        .map(|(index, &(name, base, mean, speed))| {
            let wave = Wave {
                base,
                mean,
                swing: mean * 0.8,
                speed,
                phase: index as f64,
            };
            let per_cpu = cpus
                .clone()
                .map(|cpu| counted(on_cpu(wave, cpu), cpu, time))
                .collect();
            (name.to_owned(), per_cpu)
        })
        .collect();
    (Measured::Known(interrupts), Measured::Known(softirqs))
}

/// The count and rate of the moving interrupt on `cpu`: the integral of its wave over the epochs
/// that CPU served, and the wave itself while it serves.
fn moved(device: &DemoDevice, cpu: u32, time: f64, cores: usize) -> CpuCount {
    let epoch = ((time / DEMO_MOVE_PERIOD).floor() as i64).max(0);
    let mut served = 0.0;
    // The CPUs repeat every `cores` epochs, so only this CPU's epochs are visited.
    let first = (0..cores as i64)
        .find(|&k| moving_cpu(k, cores) == cpu)
        .unwrap_or(0);
    let mut k = first;
    while k <= epoch {
        let start = k as f64 * DEMO_MOVE_PERIOD;
        let end = time.min(start + DEMO_MOVE_PERIOD);
        if end > start {
            served += device.wave.between(start, end);
        }
        k += cores as i64;
    }
    let count =
        (device.wave.base * device.wave.mean * DEMO_UPTIME / cores as f64 + served).max(0.0) as u64;
    let rate = if moving_cpu(epoch, cores) == cpu {
        device.wave.rate(time)
    } else {
        0.0
    };
    (cpu, count, Some(rate as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Demand, DemoOptions, demo_with};
    use crate::platform::Sources;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn row(id: &str, devices: &[&str], counts: &[u64]) -> Interrupt {
        Interrupt {
            id: id.to_owned(),
            devices: devices.iter().map(|name| (*name).to_owned()).collect(),
            per_cpu: counts
                .iter()
                .enumerate()
                .map(|(cpu, &count)| (cpu as u32, count, None))
                .collect(),
            ..Interrupt::default()
        }
    }

    fn rates(row: &Interrupt) -> Vec<Option<f32>> {
        row.per_cpu.iter().map(|&(_, _, rate)| rate).collect()
    }

    #[test]
    fn the_timer_row_is_loc_on_x86_and_the_arch_timer_device_elsewhere() {
        let x86 = [row("0", &["timer"], &[1]), row("LOC", &[], &[1])];
        assert_eq!(timer_row(&x86).map(|row| row.id.as_str()), Some("LOC"));
        let arm = [
            row("9", &["vgic"], &[1]),
            row("10", &["arch_timer"], &[1]),
            row("IPI0", &[], &[1]),
        ];
        assert_eq!(timer_row(&arm).map(|row| row.id.as_str()), Some("10"));
        assert!(timer_row(&arm[..1]).is_none() && timer_row(&[]).is_none());
        assert!(arm[0].is_numbered() && !arm[2].is_numbered() && !row("", &[], &[]).is_numbered());
    }

    #[test]
    fn rates_need_two_passes_and_never_go_negative() {
        let start = Instant::now();
        let mut previous = Previous::default();
        let mut first = [row("1", &[], &[100, 200]), row("2", &[], &[5, 5])];
        previous.rates(
            first
                .iter_mut()
                .map(|row| (row.id.as_str(), &mut row.per_cpu)),
            start,
        );
        assert_eq!(
            rates(&first[0]),
            [None, None],
            "the first pass has counts but no rates"
        );

        let mut second = vec![
            row("1", &[], &[300, 200]),
            row("2", &[], &[4, 9]),
            row("3", &[], &[7, 7]),
        ];
        previous.rates(
            second
                .iter_mut()
                .map(|row| (row.id.as_str(), &mut row.per_cpu)),
            start + Duration::from_secs(2),
        );
        assert_eq!(rates(&second[0]), [Some(100.0), Some(0.0)]);
        assert_eq!(
            rates(&second[1]),
            [None, Some(2.0)],
            "a count that went backwards is unknown"
        );
        assert_eq!(
            rates(&second[2]),
            [None, None],
            "a new row has no earlier count"
        );

        // Rows 2 and 3 are gone, and the CPUs of row 1 are in another order.
        let mut third = [row("1", &[], &[500, 0, 200])];
        third[0].per_cpu[1].0 = 2;
        third[0].per_cpu[2].0 = 1;
        previous.rates(
            third
                .iter_mut()
                .map(|row| (row.id.as_str(), &mut row.per_cpu)),
            start + Duration::from_secs(4),
        );
        assert_eq!(
            rates(&third[0]),
            [Some(100.0), None, Some(0.0)],
            "CPUs are matched by id"
        );
        let mut returned = [row("2", &[], &[50, 90])];
        previous.rates(
            returned
                .iter_mut()
                .map(|row| (row.id.as_str(), &mut row.per_cpu)),
            start + Duration::from_secs(6),
        );
        assert_eq!(
            rates(&returned[0]),
            [None, None],
            "a row that dropped out starts again"
        );
        assert!(
            third
                .iter()
                .chain(&returned)
                .chain(&second)
                .flat_map(rates)
                .flatten()
                .all(|rate| rate >= 0.0)
        );

        // Two reads in the same instant do not divide by zero.
        let mut again = [row("1", &[], &[600, 10, 19])];
        again[0].per_cpu[1].0 = 2;
        again[0].per_cpu[2].0 = 1;
        previous.rates(
            again
                .iter_mut()
                .map(|row| (row.id.as_str(), &mut row.per_cpu)),
            start + Duration::from_secs(4),
        );
        assert!(
            rates(&again[0])
                .iter()
                .flatten()
                .all(|rate| rate.is_finite())
        );
    }

    static INTERRUPT_READS: AtomicU32 = AtomicU32::new(0);
    static SOFTIRQ_READS: AtomicU32 = AtomicU32::new(0);
    static AFFINITY_READS: AtomicU32 = AtomicU32::new(0);
    static OOM_READS: AtomicU32 = AtomicU32::new(0);

    fn counting() -> Readers {
        Readers {
            interrupts: || {
                INTERRUPT_READS.fetch_add(1, Ordering::SeqCst);
                Some(vec![row("1", &["timer"], &[10]), row("LOC", &[], &[10])])
            },
            softirqs: || {
                SOFTIRQ_READS.fetch_add(1, Ordering::SeqCst);
                Some(vec![("TIMER".to_owned(), vec![(0, 10, None)])])
            },
            affinity: |id| {
                AFFINITY_READS.fetch_add(1, Ordering::SeqCst);
                (id == "1").then(|| vec![0])
            },
            oom_kills: || {
                OOM_READS.fetch_add(1, Ordering::SeqCst);
                Some(2)
            },
        }
    }

    fn reads_of() -> [u32; 4] {
        [
            &INTERRUPT_READS,
            &SOFTIRQ_READS,
            &AFFINITY_READS,
            &OOM_READS,
        ]
        .map(|count| count.load(Ordering::SeqCst))
    }

    #[test]
    fn kernel_counters_are_not_read_without_demand() {
        let mut scan = KernelScan::default();
        let mut latency = Vec::new();
        let mut idle = Reads::new(Demand::default());
        let readings = scan.scan(counting(), &mut idle, &mut latency);
        assert_eq!(
            reads_of(),
            [0, 0, 0, 0],
            "nothing is opened with nothing demanded"
        );
        assert!(latency.is_empty() && idle.active.is_empty() && idle.missing.is_empty());
        assert_eq!(
            (readings.interrupts, readings.softirqs, readings.oom_kills),
            (Measured::Pending, Measured::Pending, None)
        );

        // Each source is read for its own demand, and the affinity only with the table.
        let all = Sources::from([
            Source::Interrupts,
            Source::Softirqs,
            Source::IrqAffinity,
            Source::OomKills,
        ]);
        let mut reads = Reads::new(Demand::from(all));
        let readings = scan.scan(counting(), &mut reads, &mut latency);
        assert_eq!(
            reads_of(),
            [1, 1, 1, 1],
            "the affinity is read for the one numbered row"
        );
        assert_eq!((reads.active, reads.missing), (all, Sources::default()));
        let Measured::Known(rows) = readings.interrupts else {
            panic!("the table was read");
        };
        assert_eq!(rows[0].affinity, Some(vec![0]));
        assert_eq!(rows[1].affinity, None, "named rows have no affinity");
        assert_eq!(readings.oom_kills, Some(2));
        assert_eq!(latency.len(), 4);
    }

    #[test]
    fn a_demanded_source_that_cannot_be_read_is_unreadable_and_missing() {
        let mut scan = KernelScan::default();
        let mut latency = Vec::new();
        let unreadable = Readers {
            interrupts: || None,
            softirqs: || None,
            affinity: |_| None,
            oom_kills: || None,
        };
        let demanded = Sources::from([Source::Interrupts, Source::Softirqs, Source::OomKills]);
        let mut reads = Reads::new(Demand::from(demanded));
        let readings = scan.scan(unreadable, &mut reads, &mut latency);
        assert_eq!(readings.interrupts, Measured::Unreadable);
        assert_eq!(readings.softirqs, Measured::Unreadable);
        assert_eq!(readings.oom_kills, None);
        assert_eq!(reads.missing, demanded);
        assert!(reads.active.is_empty());

        // The affinity needs the table: without it, or when no row has one, it is missing.
        let mut reads = Reads::new(Demand::from(Sources::from(Source::IrqAffinity)));
        scan.scan(unreadable, &mut reads, &mut latency);
        assert_eq!(reads.missing, Sources::from(Source::IrqAffinity));
        let mut reads = Reads::new(Demand::from(Sources::from([
            Source::Interrupts,
            Source::IrqAffinity,
        ])));
        let table = Readers {
            interrupts: || Some(vec![row("1", &["timer"], &[10])]),
            ..unreadable
        };
        let readings = scan.scan(table, &mut reads, &mut latency);
        assert_eq!(reads.missing, Sources::from(Source::IrqAffinity));
        assert_eq!(reads.active, Sources::from(Source::Interrupts));
        assert!(
            matches!(readings.interrupts, Measured::Known(ref rows) if rows[0].affinity.is_none())
        );
    }

    #[test]
    fn rates_start_again_after_the_demand_lapses() {
        let mut scan = KernelScan::default();
        let mut latency = Vec::new();
        let table = Readers {
            interrupts: || Some(vec![row("LOC", &[], &[10])]),
            ..counting()
        };
        let demanded = Demand::from(Sources::from(Source::Interrupts));
        scan.scan(table, &mut Reads::new(demanded), &mut latency);
        scan.scan(table, &mut Reads::new(Demand::default()), &mut latency);
        let readings = scan.scan(table, &mut Reads::new(demanded), &mut latency);
        let rows = readings.interrupts.known().expect("read");
        assert_eq!(
            rates(&rows[0]),
            [None],
            "the pause is not averaged into a rate"
        );
    }

    fn demo_rows(time: f64) -> (Vec<Interrupt>, Vec<Softirq>) {
        let snapshot = demo_with(
            time,
            192,
            DemoOptions {
                kernel_counters: true,
                ..DemoOptions::NONE
            },
        );
        (
            snapshot.interrupts.known().expect("synthesized").clone(),
            snapshot.softirqs.known().expect("synthesized").clone(),
        )
    }

    #[test]
    fn demo_kernel_counters_are_off_unless_a_row_asks_for_them() {
        let plain = demo_with(12.0, 192, DemoOptions::NONE);
        assert_eq!(plain.interrupts, Measured::Pending);
        assert_eq!(plain.softirqs, Measured::Pending);
        assert_eq!(plain.oom_kills, None);
    }

    #[test]
    fn demo_interrupts_cover_the_devices_and_counters_the_views_draw() {
        let (interrupts, softirqs) = demo_rows(12.0);
        let ids: Vec<&str> = interrupts.iter().map(|row| row.id.as_str()).collect();
        let devices: Vec<&str> = interrupts
            .iter()
            .flat_map(|row| row.devices.iter().map(String::as_str))
            .collect();
        for wanted in [
            "nvme0q0", "nvme0q1", "nvme0q2", "nvme0q3", "nvme0q4", "nvme0q5", "nvme0q6", "nvme0q7",
            "xhci_hcd", "iwlwifi", "i915",
        ] {
            assert!(devices.contains(&wanted), "{wanted}");
        }
        assert!(devices.iter().any(|name| name.starts_with("ahci")));
        for wanted in ["NMI", "LOC", "RES", "CAL", "TLB"] {
            assert!(ids.contains(&wanted), "{wanted}");
        }
        assert!(
            interrupts
                .iter()
                .all(|row| row.per_cpu.len() == 16 && !row.total_only)
        );
        assert!(
            interrupts
                .iter()
                .all(|row| row.is_numbered() == row.affinity.is_some()),
            "numbered rows have an affinity and named ones do not"
        );
        let names: Vec<&str> = softirqs.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "HI", "TIMER", "NET_TX", "NET_RX", "BLOCK", "IRQ_POLL", "TASKLET", "SCHED",
                "HRTIMER", "RCU"
            ]
        );
        assert!(softirqs.iter().all(|(_, per_cpu)| per_cpu.len() == 16));
        let local = interrupts.iter().find(|row| row.id == "LOC").unwrap();
        let hertz: Vec<f32> = local
            .per_cpu
            .iter()
            .filter_map(|&(_, _, rate)| rate)
            .collect();
        assert!(
            hertz.iter().all(|&rate| (60.0..=250.0).contains(&rate)),
            "about 250 Hz scaled by busy: {hertz:?}"
        );
        assert_eq!(
            timer_row(&interrupts).map(|row| row.id.as_str()),
            Some("LOC")
        );
    }

    #[test]
    fn demo_counts_only_grow_and_rates_vary_with_time() {
        let mut before = demo_rows(0.0);
        let mut seen = std::collections::HashSet::new();
        for step in 1..=400 {
            let after = demo_rows(f64::from(step) * 1.5);
            for (earlier, later) in before.0.iter().zip(&after.0) {
                for (&(_, then, _), &(cpu, now, rate)) in earlier.per_cpu.iter().zip(&later.per_cpu)
                {
                    assert!(
                        now >= then,
                        "interrupt {} on CPU {cpu} went backwards",
                        later.id
                    );
                    assert!(rate.is_some_and(|rate| rate >= 0.0));
                }
            }
            for ((_, earlier), (_, later)) in before.1.iter().zip(&after.1) {
                assert!(earlier.iter().zip(later).all(|(a, b)| b.1 >= a.1));
            }
            seen.insert(
                after.0.iter().find(|row| row.id == "28").unwrap().per_cpu[2]
                    .2
                    .unwrap()
                    .to_bits(),
            );
            before = after;
        }
        assert!(seen.len() > 100, "the rate wanders");
        assert_eq!(
            demo_rows(55.5),
            demo_rows(55.5),
            "a pure function of the time"
        );
        let negative = demo_rows(-40.0);
        assert!(negative.0.iter().all(|row| {
            row.per_cpu
                .iter()
                .all(|&(_, count, _)| count > 0 || row.id != "LOC")
        }));
    }

    #[test]
    fn the_moving_demo_interrupt_changes_cpu_every_37_seconds() {
        let served = |time: f64| {
            let (interrupts, _) = demo_rows(time);
            let wifi = interrupts
                .iter()
                .find(|row| row.devices == ["iwlwifi"])
                .unwrap()
                .clone();
            let busy: Vec<u32> = wifi
                .per_cpu
                .iter()
                .filter(|&&(_, _, rate)| rate.is_some_and(|rate| rate > 0.0))
                .map(|&(cpu, ..)| cpu)
                .collect();
            assert_eq!(
                wifi.affinity.as_ref(),
                Some(&busy),
                "the affinity names the CPU taking it"
            );
            busy
        };
        assert_eq!(served(0.0), served(36.9));
        assert_ne!(served(36.9), served(37.1));
        assert_eq!(served(37.5), served(73.9));
        let cpus: std::collections::HashSet<u32> = (0..16)
            .map(|epoch| served(f64::from(epoch) * 37.0 + 1.0)[0])
            .collect();
        assert_eq!(cpus.len(), 16, "it visits every CPU before it repeats");
    }

    #[test]
    fn demo_oom_kills_equal_the_sum_of_unit_oom_kills() {
        for churn in [false, true] {
            for mortality in [false, true] {
                let options = DemoOptions {
                    churn: churn || mortality,
                    mortality,
                    kernel_counters: true,
                    ..DemoOptions::NONE
                };
                let mut last = 0;
                for time in [-30.0, 0.0, 12.0, 23.5, 70.0, 150.0, 400.0, 3000.0] {
                    let snapshot = demo_with(time, 192, options);
                    let summed: u64 = snapshot
                        .units
                        .values()
                        .filter_map(|unit| unit.oom_kills)
                        .sum();
                    assert_eq!(snapshot.oom_kills, Some(summed), "{options:?} at {time}");
                    assert!(summed >= last, "cumulative: {options:?} at {time}");
                    last = summed;
                }
                assert!(last > 0, "{options:?}: something was killed by 3000 s");
            }
        }
    }

    #[test]
    fn the_real_thread_reads_the_demanded_counters_and_says_what_it_could_not() {
        let four = Sources::from([
            Source::Interrupts,
            Source::Softirqs,
            Source::IrqAffinity,
            Source::OomKills,
        ]);
        let thread = super::super::background(
            None,
            Default::default(),
            std::sync::Arc::new(std::sync::Mutex::new(Demand::from(four))),
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .expect("the sampler thread starts");
        let deadline = Instant::now() + Duration::from_secs(60);
        while thread.extras().finished.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let extras = thread.extras();
        assert!(extras.finished.is_some(), "the first cycle finished");
        for source in four.iter() {
            assert!(
                extras.sources.contains(source) != extras.missing.contains(source),
                "{} is read or missing, not both or neither",
                source.name()
            );
        }
        let kernel = &extras.kernel;
        assert_eq!(
            kernel.interrupts == Measured::Unreadable,
            extras.missing.contains(Source::Interrupts)
        );
        assert_eq!(
            kernel.oom_kills.is_none(),
            extras.missing.contains(Source::OomKills)
        );
        if cfg!(target_os = "macos") {
            assert_eq!(
                extras.missing.iter().filter(|s| four.contains(*s)).count(),
                4
            );
            assert_eq!(kernel.softirqs, Measured::Unreadable);
        }
        assert!(
            kernel.interrupts.known().is_none_or(|rows| rows
                .iter()
                .all(|row| row.per_cpu.iter().all(|c| c.2.is_none()))),
            "the first pass has counts but no rates"
        );
    }
}
