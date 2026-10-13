//! The demo's interrupt table for the switchboard (`DemoOptions::exchange`): 48 numbered lines on
//! 16 CPUs, in the prototype's order and with its rates, the named rows each operator needs and
//! the softirqs, all pure functions of the time. `kernel::demo` is the plain one the OOM views
//! use; this one adds what a telephone exchange has to show: a line that irqbalance moves between
//! two CPUs, a non-maskable interrupt now and then, and rescheduling interrupts that follow how
//! busy each CPU is.

use super::kernel::{
    DEMO_COUNTERS, DEMO_SOFTIRQS, DEMO_UPTIME, Wave, counted, local_timer, on_cpu,
};
use super::{Cpu, CpuCount, Interrupt, Measured, Softirq};

/// The numbered lines: interrupt number, device name, mean events per second (the prototype's
/// rates, scaled to 41,203 a second in all) and the CPU that serves it. The names group as the
/// switchboard groups them: `nvme0` (9), `eth0` (8), `iwlwifi` (8), `idma64.` (2), `dmar` (2)
/// and the rest alone.
const LINES: [(u32, &str, f64, u32); 48] = [
    (24, "nvme0q0", 28.51, 0),
    (25, "nvme0q1", 1734.66, 0),
    (26, "nvme0q2", 1164.36, 1),
    (27, "nvme0q3", 2625.75, 2),
    (28, "nvme0q4", 760.4, 3),
    (29, "nvme0q5", 1568.32, 4),
    (30, "nvme0q6", 487.13, 5),
    (31, "nvme0q7", 902.97, 6),
    (32, "nvme0q8", 617.82, 7),
    (33, "ahci", 100.99, 11),
    (34, "xhci_hcd", 1817.83, 4),
    (35, "i915", 2863.37, 15),
    (36, "eth0-rx-0", 4647.93, 0),
    (37, "eth0-rx-1", 3386.15, 2),
    (38, "eth0-rx-2", 3687.93, 3),
    (39, "eth0-rx-3", 2661.39, 5),
    (40, "eth0-tx-0", 974.26, 6),
    (41, "eth0-tx-1", 760.4, 7),
    (42, "eth0-tx-2", 843.57, 3),
    (43, "eth0-tx-3", 534.65, 10),
    (44, "hda_intel", 1140.6, 11),
    (45, "mei_me", 16.63, 12),
    (46, "intel_ish", 47.52, 14),
    (47, "i801_smbus", 3.56, 0),
    (48, "iwlwifi", 451.49, 1),
    (49, "iwlwifi:q1", 1473.27, 8),
    (50, "iwlwifi:q2", 1021.78, 2),
    (51, "iwlwifi:q3", 1699.01, 5),
    (52, "iwlwifi:q4", 617.82, 9),
    (53, "iwlwifi:q5", 1140.6, 7),
    (54, "iwlwifi:q6", 356.44, 10),
    (55, "iwlwifi:q7", 724.75, 12),
    (56, "thunderbolt", 7.13, 13),
    (57, "ucsi_acpi", 2.38, 0),
    (58, "pinctrl", 13.07, 14),
    (59, "dw_dmac", 0.48, 15),
    (0, "timer", 1.19, 0),
    (8, "rtc0", 1.19, 0),
    (9, "acpi", 26.14, 0),
    (60, "i2c_dw.0", 140.2, 9),
    (61, "hid-i2c", 142.57, 9),
    (62, "idma64.0", 5.94, 8),
    (63, "idma64.1", 0.24, 8),
    (64, "dmar0", 0.12, 0),
    (65, "dmar1", 0.12, 0),
    (66, "aerdrv", 0.05, 0),
    (67, "PCIe PME", 0.05, 0),
    (68, "pciehp", 0.36, 0),
];

/// The line irqbalance moves, and the two CPUs it moves it between.
const MOVING: &str = "eth0-rx-2";
const MOVE_FROM: u32 = 3;
const MOVE_TO: u32 = 9;
/// When it first moves, and how long it stays on a CPU. With samples a second apart the first
/// sample to see the move is the one at 11 s, so a frame at `--time 12` is a second into the
/// swing.
const MOVE_AT: f64 = 10.2;
const MOVE_EVERY: f64 = 30.0;

/// The CPU that gets the non-maskable interrupts, and when one arrives (every 45 s from 10.7 s,
/// so a frame at `--time 12` finds the red lamp lit: it burns for two seconds).
const NMI_CPU: u32 = 0;
const NMI_AT: f64 = 10.7;
const NMI_EVERY: f64 = 45.0;
/// Rescheduling interrupts per second on a CPU that is fully busy.
const RES_PER_BUSY: f64 = 330.0;

/// The rate of one line: its mean, wandering by 30% on a period of its own.
fn wave(index: usize) -> Wave {
    Wave {
        base: LINES[index].2,
        mean: 1.0,
        swing: 0.3,
        speed: 0.13 + 0.011 * (index % 9) as f64,
        phase: index as f64 * 0.7,
    }
}

/// Which of its two homes the moving line is on at `time`: an epoch number, even on `MOVE_TO`.
fn epoch(time: f64) -> i64 {
    ((time - MOVE_AT) / MOVE_EVERY).floor() as i64
}

/// The CPU that serves line `index` at `time`.
fn serving(index: usize, time: f64) -> u32 {
    if LINES[index].1 == MOVING {
        if epoch(time) % 2 == 0 {
            MOVE_TO
        } else {
            MOVE_FROM
        }
    } else {
        LINES[index].3
    }
}

/// The CPUs a line has served: its own, or for the moving line the two.
fn serves(index: usize, cpu: u32) -> bool {
    if LINES[index].1 == MOVING {
        cpu == MOVE_FROM || cpu == MOVE_TO
    } else {
        LINES[index].3 == cpu
    }
}

/// The count of line `index` on `cpu` at `time`: a day of uptime's share, plus every event since
/// 0 that `cpu` served. The moving line adds the events of the epochs it spent on `cpu`.
fn line_count(index: usize, cpu: u32, time: f64) -> u64 {
    let wave = wave(index);
    if !serves(index, cpu) {
        return 0;
    }
    let time = time.max(0.0);
    if LINES[index].1 != MOVING {
        return wave.count(time, 1.0);
    }
    let boot = wave.base * wave.mean * DEMO_UPTIME * 0.5;
    let mut served = 0.0;
    let mut k = epoch(0.0);
    while MOVE_AT + k as f64 * MOVE_EVERY < time {
        let on_to = k % 2 == 0;
        if on_to == (cpu == MOVE_TO) {
            let start = (MOVE_AT + k as f64 * MOVE_EVERY).max(0.0);
            let end = time.min(MOVE_AT + (k + 1) as f64 * MOVE_EVERY);
            if end > start {
                served += wave.between(start, end);
            }
        }
        k += 1;
    }
    (boot + served).max(0.0) as u64
}

/// The chip a device's interrupt comes through.
fn chip(name: &str) -> &'static str {
    match name {
        "timer" | "rtc0" | "acpi" => "IO-APIC",
        _ if name.starts_with("nvme") => "IR-PCI-MSIX-0000:01:00.0",
        _ if name.starts_with("eth0") => "IR-PCI-MSIX-0000:03:00.0",
        _ if name.starts_with("iwlwifi") => "IR-PCI-MSIX-0000:00:14.3",
        _ => "IR-PCI-MSI-0000:00:1f.0",
    }
}

/// The scale a CPU's busy share runs at in the demo (`Cpu::busy`): 0.9 on the first eight.
fn scale(cpu: u32) -> f64 {
    if cpu < 8 { 0.9 } else { 0.6 }
}

/// The count and rate of the rescheduling interrupts on `cpu`: `RES_PER_BUSY` times the busy
/// share, which the demo's `Cpu::busy` computes as `(0.5 + 0.5 sin(0.4 t + 0.9 id)) * scale`.
fn rescheduling(cpu: u32) -> Wave {
    Wave {
        base: RES_PER_BUSY * scale(cpu),
        mean: 0.5,
        swing: 0.5,
        speed: 0.4,
        phase: 0.9 * f64::from(cpu),
    }
}

/// The non-maskable interrupts so far on `cpu`, and the rate over the second that ends at `time`:
/// one event an `NMI_EVERY` on `NMI_CPU`, none elsewhere.
fn non_maskable(cpu: u32, time: f64) -> CpuCount {
    if cpu != NMI_CPU {
        return (cpu, 0, Some(0.0));
    }
    let events = |until: f64| ((until - NMI_AT) / NMI_EVERY).floor() as i64 + 1;
    let before = events(time).max(0);
    let lately = before - events(time - 1.0).max(0);
    (cpu, 3 + before as u64, Some(lately as f32))
}

/// The demo's interrupt table and softirqs at `time` on `cpus`.
pub(super) fn demo(time: f64, cpus: &[Cpu]) -> (Measured<Vec<Interrupt>>, Measured<Vec<Softirq>>) {
    let mut interrupts = Vec::with_capacity(LINES.len() + 5);
    for (index, &(irq, name, ..)) in LINES.iter().enumerate() {
        let home = serving(index, time);
        let wave = wave(index);
        let per_cpu = cpus
            .iter()
            .map(|cpu| {
                let rate = if cpu.id == home { wave.rate(time) } else { 0.0 };
                (cpu.id, line_count(index, cpu.id, time), Some(rate as f32))
            })
            .collect();
        interrupts.push(Interrupt {
            id: irq.to_string(),
            chip: chip(name).to_owned(),
            hwirq: Some(u64::from(irq)),
            trigger: "edge".to_owned(),
            devices: vec![name.to_owned()],
            description: String::new(),
            per_cpu,
            total_only: false,
            affinity: Some(vec![home]),
        });
    }
    let named = |id: &str, description: &str, count: &dyn Fn(u32) -> CpuCount| Interrupt {
        id: id.to_owned(),
        description: description.to_owned(),
        per_cpu: cpus.iter().map(|cpu| count(cpu.id)).collect(),
        ..Interrupt::default()
    };
    interrupts.push(named("NMI", "Non-maskable interrupts", &|cpu| {
        non_maskable(cpu, time)
    }));
    interrupts.push(named("LOC", "Local timer interrupts", &|cpu| {
        counted(local_timer(cpu), cpu, time)
    }));
    interrupts.push(named("RES", "Rescheduling interrupts", &|cpu| {
        counted(rescheduling(cpu), cpu, time)
    }));
    for counter in &DEMO_COUNTERS[2..] {
        interrupts.push(named(counter.id, counter.description, &|cpu| {
            counted(on_cpu(counter.wave, cpu), cpu, time)
        }));
    }
    (
        Measured::Known(interrupts),
        Measured::Known(softirqs(time, cpus)),
    )
}

/// The share of a line's events that also raise a softirq, by device: the receive and transmit
/// queues of the network, the block layer and the wireless card.
fn raises(name: &str) -> Option<(&'static str, f64)> {
    match name {
        _ if name.starts_with("eth0-rx") => Some(("NET_RX", 0.8)),
        _ if name.starts_with("eth0-tx") => Some(("NET_TX", 0.5)),
        _ if name.starts_with("nvme") => Some(("BLOCK", 0.9)),
        _ if name.starts_with("iwlwifi") => Some(("NET_RX", 0.4)),
        _ => None,
    }
}

/// The ten softirqs: each one's own wave on every CPU, and on the CPUs that serve network and
/// block devices the share of those devices' events (a softirq runs where its interrupt did).
fn softirqs(time: f64, cpus: &[Cpu]) -> Vec<Softirq> {
    DEMO_SOFTIRQS
        .iter()
        .enumerate()
        .map(|(row, &(name, base, mean, speed))| {
            let own = Wave {
                base,
                mean,
                swing: mean * 0.8,
                speed,
                phase: row as f64,
            };
            let per_cpu = cpus
                .iter()
                .map(|cpu| {
                    let own = on_cpu(own, cpu.id);
                    let (mut count, mut rate) = (own.count(time, 1.0), own.rate(time) as f32);
                    for (index, &(_, device, ..)) in LINES.iter().enumerate() {
                        let Some((raised, share)) = raises(device) else {
                            continue;
                        };
                        if raised == name && serving(index, time) == cpu.id {
                            rate += (wave(index).rate(time) * share) as f32;
                        }
                        if raised == name {
                            count += (line_count(index, cpu.id, time) as f64 * share) as u64;
                        }
                    }
                    (cpu.id, count, Some(rate))
                })
                .collect();
            (name.to_owned(), per_cpu)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DemoOptions, demo_with};

    fn at(time: f64) -> Vec<Interrupt> {
        let options = DemoOptions {
            exchange: true,
            ..DemoOptions::NONE
        };
        match demo_with(time, 16, options).interrupts {
            Measured::Known(rows) => rows,
            other => panic!("the exchange demo reads its table: {other:?}"),
        }
    }

    fn row<'a>(rows: &'a [Interrupt], id: &str) -> &'a Interrupt {
        rows.iter().find(|row| row.id == id).expect(id)
    }

    fn rate_on(row: &Interrupt, cpu: u32) -> f32 {
        row.per_cpu
            .iter()
            .find(|entry| entry.0 == cpu)
            .and_then(|entry| entry.2)
            .expect("a rate")
    }

    #[test]
    fn the_exchange_demo_has_the_prototypes_48_lines_on_16_cpus_with_about_41_thousand_a_second() {
        let rows = at(12.0);
        let numbered: Vec<&Interrupt> = rows.iter().filter(|row| row.is_numbered()).collect();
        assert_eq!(numbered.len(), 48);
        assert!(numbered.iter().all(|row| row.per_cpu.len() == 16));
        let total: f32 = numbered
            .iter()
            .flat_map(|row| row.per_cpu.iter())
            .filter_map(|entry| entry.2)
            .sum();
        assert!((31_000.0..52_000.0).contains(&total), "{total} irq/s");
        for name in ["NMI", "LOC", "RES", "CAL", "TLB"] {
            assert!(rows.iter().any(|row| row.id == name), "{name}");
        }
        assert!(
            numbered
                .iter()
                .all(|row| row.affinity.as_ref().unwrap().len() == 1),
            "every line has an effective affinity of one CPU"
        );
    }

    #[test]
    fn the_moving_line_changes_cpu_every_30_seconds_from_10_2() {
        let cpu_of = |time: f64| {
            let rows = at(time);
            let line = row(&rows, "38");
            assert_eq!(line.devices, [MOVING]);
            let served = line.affinity.clone().unwrap();
            let busiest = line
                .per_cpu
                .iter()
                .max_by(|a, b| a.2.unwrap().total_cmp(&b.2.unwrap()))
                .unwrap()
                .0;
            assert_eq!(served, [busiest], "the rates follow the affinity at {time}");
            busiest
        };
        assert_eq!(cpu_of(10.0), 3);
        assert_eq!(cpu_of(11.0), 9, "the first sample after 10.2 sees the move");
        assert_eq!(cpu_of(40.0), 9);
        assert_eq!(cpu_of(41.0), 3);
    }

    #[test]
    fn the_counts_of_a_moving_line_only_grow_on_the_cpu_that_serves_it() {
        let count = |time: f64, cpu: u32| {
            let rows = at(time);
            row(&rows, "38")
                .per_cpu
                .iter()
                .find(|entry| entry.0 == cpu)
                .unwrap()
                .1
        };
        assert!(count(11.0, 9) > count(10.0, 9), "cpu 9 serves after 10.2");
        assert_eq!(count(11.0, 3), count(10.5, 3), "cpu 3 does not");
        assert!(count(45.0, 3) > count(41.0, 3), "and takes it back at 40.2");
    }

    #[test]
    fn one_nmi_arrives_on_cpu0_every_45_seconds_and_lights_one_sample() {
        let nmi = |time: f64| {
            let rows = at(time);
            row(&rows, "NMI").per_cpu[0]
        };
        assert_eq!(nmi(10.0).2, Some(0.0));
        assert_eq!(nmi(11.0).2, Some(1.0), "the sample after 10.7 sees it");
        assert_eq!(nmi(12.0).2, Some(0.0));
        assert_eq!(nmi(56.0).2, Some(1.0), "and again 45 s later");
        assert_eq!(nmi(11.0).1, nmi(10.0).1 + 1);
        let rows = at(11.0);
        assert!(
            row(&rows, "NMI").per_cpu[1..]
                .iter()
                .all(|entry| entry.2 == Some(0.0)),
            "only cpu0 takes them"
        );
    }

    #[test]
    fn rescheduling_interrupts_follow_how_busy_each_cpu_is() {
        let options = DemoOptions {
            exchange: true,
            ..DemoOptions::NONE
        };
        for time in [3.0, 12.0, 40.5] {
            let snapshot = demo_with(time, 16, options);
            let Measured::Known(rows) = &snapshot.interrupts else {
                panic!("the table is read");
            };
            let res = row(rows, "RES");
            for cpu in &snapshot.cpus {
                let busy = cpu.busy.unwrap();
                let rate = rate_on(res, cpu.id);
                assert!(
                    (rate - 330.0 * busy).abs() < 0.01,
                    "cpu{} at {time}: {rate} against busy {busy}",
                    cpu.id
                );
            }
        }
    }

    #[test]
    fn the_exchange_demo_gives_every_softirq_a_rate_on_every_cpu_and_net_rx_follows_the_network() {
        let options = DemoOptions {
            exchange: true,
            ..DemoOptions::NONE
        };
        let snapshot = demo_with(12.0, 16, options);
        let Measured::Known(softirqs) = &snapshot.softirqs else {
            panic!("the softirqs are read");
        };
        assert_eq!(
            softirqs
                .iter()
                .map(|row| row.0.as_str())
                .collect::<Vec<_>>(),
            [
                "HI", "TIMER", "NET_TX", "NET_RX", "BLOCK", "IRQ_POLL", "TASKLET", "SCHED",
                "HRTIMER", "RCU"
            ]
        );
        assert!(
            softirqs
                .iter()
                .all(|row| row.1.len() == 16 && row.1.iter().all(|entry| entry.2.is_some()))
        );
        let net_rx = &softirqs[3].1;
        let cpu0 = net_rx[0].2.unwrap();
        let cpu15 = net_rx[15].2.unwrap();
        assert!(
            cpu0 > cpu15 + 1000.0,
            "cpu0 serves eth0-rx-0 ({cpu0}), cpu15 serves no network line ({cpu15})"
        );
    }

    #[test]
    fn the_plain_kernel_demo_is_what_it_was_without_the_option() {
        let plain = demo_with(
            12.0,
            16,
            DemoOptions {
                kernel_counters: true,
                ..DemoOptions::NONE
            },
        );
        let Measured::Known(rows) = &plain.interrupts else {
            panic!("read");
        };
        assert_eq!(rows.iter().filter(|row| row.is_numbered()).count(), 13);
        let off = demo_with(12.0, 16, DemoOptions::NONE);
        assert!(matches!(off.interrupts, Measured::Pending));
    }
}
