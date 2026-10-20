//! Kernel counters from `/proc`: the interrupt table, softirqs, IRQ affinity and the OOM kill
//! count. Everything here reads one file per call and parses its text; the rates, the demand and
//! the cadence belong to the background thread (`kernel.rs`).

use std::fs;

use crate::model::{CpuCount, Interrupt, Softirq};

/// A file's text with invalid UTF-8 replaced, so a device name that is not text is still read.
/// Names are kept as read; whoever shows one makes it `terminal::SafeText`.
fn read_text(path: &str) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// Every row of `/proc/interrupts` with its counts per CPU; None when it cannot be read. The
/// rows carry no rates and no affinity.
pub fn interrupts() -> Option<Vec<Interrupt>> {
    parse_interrupts(&read_text("/proc/interrupts")?)
}

/// Every row of `/proc/softirqs` with its counts per CPU; None when it cannot be read.
pub fn softirqs() -> Option<Vec<Softirq>> {
    parse_softirqs(&read_text("/proc/softirqs")?)
}

/// The CPUs that serve the numbered interrupt `id`, from `effective_affinity_list`. None when
/// `id` is not a number, when the file is absent or unreadable, and when it is empty (the kernel
/// prints nothing for an interrupt with no effective mask), never an empty list.
pub fn irq_affinity(id: &str) -> Option<Vec<u32>> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    parse_cpu_list(&read_text(&format!(
        "/proc/irq/{id}/effective_affinity_list"
    ))?)
}

/// The kernel's count of OOM kills since boot, `oom_kill` in `/proc/vmstat`; None before Linux
/// 4.13, which has no such line, and when the file cannot be read.
pub fn oom_kills() -> Option<u64> {
    parse_oom_kills(&read_text("/proc/vmstat")?)
}

/// The ids in a CPU list as the kernel writes it, `0-3,8`: single ids and inclusive ranges
/// separated by commas, with surrounding white space ignored. The ids come back sorted and
/// without repeats. None for an empty or malformed list, and for a range that runs backwards or
/// names more than 65536 ids, so a corrupt file cannot make a huge list.
pub fn parse_cpu_list(text: &str) -> Option<Vec<u32>> {
    const MOST: u32 = 1 << 16;
    let mut ids = Vec::new();
    for part in text.trim().split(',') {
        let part = part.trim();
        let (first, last) = match part.split_once('-') {
            Some((first, last)) => (first.trim().parse::<u32>().ok()?, last.trim().parse().ok()?),
            None => {
                let id = part.parse::<u32>().ok()?;
                (id, id)
            }
        };
        if last < first
            || last - first >= MOST
            || ids.len() as u64 + u64::from(last - first) >= u64::from(MOST)
        {
            return None;
        }
        ids.extend(first..=last);
    }
    ids.sort_unstable();
    ids.dedup();
    Some(ids)
}

/// The CPU ids in the header line of `/proc/interrupts` or `/proc/softirqs`, in column order:
/// a column is named by its `CPUn`, not by its position, because offline CPUs have none.
fn parse_header(line: &str) -> Option<Vec<u32>> {
    let ids: Option<Vec<u32>> = line
        .split_whitespace()
        .map(|name| name.strip_prefix("CPU")?.parse().ok())
        .collect();
    ids.filter(|ids| !ids.is_empty())
}

/// The first white-space separated token of `text` and what follows it.
fn token(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(char::is_whitespace) {
        Some(end) => (&text[..end], &text[end..]),
        None => (text, ""),
    }
}

/// Up to `limit` counts at the start of `text`, and the trimmed text after them.
fn take_counts(mut text: &str, limit: usize) -> (Vec<u64>, &str) {
    let mut counts = Vec::new();
    while counts.len() < limit {
        let (word, after) = token(text);
        let Ok(count) = word.parse::<u64>() else {
            break;
        };
        counts.push(count);
        text = after;
    }
    (counts, text.trim())
}

/// The rows of `/proc/interrupts`. A row with fewer counts than there are CPUs (`ERR:`, `MIS:`,
/// `Err:`) is one total (`Interrupt::total_only`). A row with no counts is not a row.
fn parse_interrupts(text: &str) -> Option<Vec<Interrupt>> {
    let mut lines = text.lines();
    let cpus = parse_header(lines.next()?)?;
    let rows = lines
        .filter_map(|line| {
            let (label, rest) = line.split_once(':')?;
            let id = label.trim();
            let (counts, tail) = take_counts(rest, cpus.len());
            if id.is_empty() || counts.is_empty() {
                return None;
            }
            let total_only = counts.len() != cpus.len();
            let per_cpu: Vec<CpuCount> = if total_only {
                let total = counts
                    .iter()
                    .fold(0, |sum: u64, &count| sum.saturating_add(count));
                vec![(cpus[0], total, None)]
            } else {
                cpus.iter()
                    .zip(counts)
                    .map(|(&cpu, count)| (cpu, count, None))
                    .collect()
            };
            let mut row = Interrupt {
                id: id.to_owned(),
                chip: String::new(),
                hwirq: None,
                trigger: String::new(),
                devices: Vec::new(),
                description: String::new(),
                per_cpu,
                total_only,
                affinity: None,
            };
            if id.bytes().all(|byte| byte.is_ascii_digit()) {
                describe_numbered(&mut row, tail);
            } else {
                row.description = tail.to_owned();
            }
            Some(row)
        })
        .collect();
    Some(rows)
}

/// Fills in what follows the counts of a numbered row: the chip, then the hardware interrupt
/// number and trigger, then the device names, as `show_interrupts` in `kernel/irq/proc.c` prints
/// them. x86 joins the number and the handler's name, `9-fasteoi`; arm64 and others separate the
/// number from `Level` or `Edge`; a chip without a domain prints no number, and a row may have
/// no devices. The device names are separated by commas and may hold spaces.
fn describe_numbered(row: &mut Interrupt, tail: &str) {
    let (chip, mut rest) = token(tail);
    row.chip = chip.to_owned();
    let (word, after) = token(rest);
    let (number, joined) = word.split_once('-').unwrap_or((word, ""));
    if let Some(trigger) = word.strip_prefix('-') {
        row.trigger = trigger.to_owned();
        rest = after;
    } else if let Ok(hwirq) = number.parse::<u64>() {
        row.hwirq = Some(hwirq);
        rest = after;
        if !joined.is_empty() {
            row.trigger = joined.to_owned();
        } else {
            let (next, more) = token(after);
            if matches!(next, "Level" | "Edge") {
                row.trigger = next.to_owned();
                rest = more;
            }
        }
    }
    row.devices = rest
        .split(", ")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
}

/// The rows of `/proc/softirqs`, in file order, one count per CPU of the header. A row with no
/// counts is skipped.
fn parse_softirqs(text: &str) -> Option<Vec<Softirq>> {
    let mut lines = text.lines();
    let cpus = parse_header(lines.next()?)?;
    let rows = lines
        .filter_map(|line| {
            let (label, rest) = line.split_once(':')?;
            let (counts, _) = take_counts(rest, cpus.len());
            if label.trim().is_empty() || counts.is_empty() {
                return None;
            }
            let per_cpu = cpus
                .iter()
                .zip(counts)
                .map(|(&cpu, count)| (cpu, count, None))
                .collect();
            Some((label.trim().to_owned(), per_cpu))
        })
        .collect();
    Some(rows)
}

fn parse_oom_kills(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("oom_kill ")?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::kernel::timer_row;

    /// A fixture without its header: the lines that start with `#` say where it came from.
    fn fixture(text: &str) -> String {
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn x86() -> Vec<Interrupt> {
        parse_interrupts(&fixture(include_str!("fixtures/x86_interrupts.txt"))).unwrap()
    }

    fn row<'a>(rows: &'a [Interrupt], id: &str) -> &'a Interrupt {
        rows.iter().find(|row| row.id == id).expect(id)
    }

    fn counts(row: &Interrupt) -> Vec<u64> {
        row.per_cpu.iter().map(|&(_, count, _)| count).collect()
    }

    #[test]
    fn x86_interrupts_parse_numeric_and_named_rows() {
        let rows = x86();
        assert_eq!(rows.len(), 28);
        let timer = row(&rows, "0");
        assert_eq!(
            (timer.chip.as_str(), timer.hwirq, timer.trigger.as_str()),
            ("IO-APIC", Some(2), "edge")
        );
        assert_eq!(timer.devices, ["timer"]);
        assert_eq!(counts(timer), [10, 0, 0, 0]);
        assert_eq!(timer.per_cpu[0], (0, 10, None), "no rate in a parsed row");
        let acpi = row(&rows, "9");
        assert_eq!((acpi.hwirq, acpi.trigger.as_str()), (Some(9), "fasteoi"));
        let nvme = row(&rows, "27");
        assert_eq!(nvme.chip, "IR-PCI-MSIX-0000:01:00.0");
        assert_eq!(
            (nvme.hwirq, nvme.devices.clone()),
            (Some(0), vec!["nvme0q0".to_owned()])
        );
        let xhci = row(&rows, "41");
        assert_eq!(xhci.hwirq, Some(327680));
        let shared = row(&rows, "24");
        assert_eq!(
            shared.devices,
            ["PCIe PME", "pciehp"],
            "names split at commas, not at spaces"
        );
        let local = row(&rows, "LOC");
        assert_eq!(local.description, "Local timer interrupts");
        assert_eq!(
            (local.chip.as_str(), local.hwirq, local.devices.len()),
            ("", None, 0)
        );
        assert_eq!(counts(local), [9100322, 9001201, 8999812, 9043321]);
        assert_eq!(row(&rows, "TLB").description, "TLB shootdowns");
        assert_eq!(
            rows.iter().position(|row| row.id == "41"),
            Some(8),
            "the numbered rows come first, in file order"
        );
        assert!(rows.iter().all(|row| row.affinity.is_none()));
        assert_eq!(timer_row(&rows).map(|row| row.id.as_str()), Some("LOC"));
    }

    #[test]
    fn arm64_interrupts_parse_ipi_rows_and_find_the_timer_row() {
        let rows =
            parse_interrupts(&fixture(include_str!("fixtures/arm64_interrupts.txt"))).unwrap();
        let timer = timer_row(&rows).expect("arch_timer");
        assert_eq!(timer.id, "10");
        assert_eq!(
            (timer.chip.as_str(), timer.hwirq, timer.trigger.as_str()),
            ("GICv3", Some(27), "Level")
        );
        assert_eq!(timer.devices, ["arch_timer"]);
        assert_eq!(
            timer
                .per_cpu
                .iter()
                .map(|&(cpu, ..)| cpu)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        let message = row(&rows, "28");
        assert_eq!(
            (
                message.chip.as_str(),
                message.hwirq,
                message.trigger.as_str()
            ),
            ("MSI", Some(81922), "Edge")
        );
        assert_eq!(message.devices, ["virtio1-output"]);
        let acpi = row(&rows, "13");
        assert_eq!((acpi.chip.as_str(), acpi.hwirq), ("ARMH0061:00", Some(6)));
        assert_eq!(acpi.devices, ["ACPI:Event"]);
        for (index, description) in [
            "Rescheduling interrupts",
            "Function call interrupts",
            "CPU stop interrupts",
            "CPU stop NMIs",
            "Timer broadcast interrupts",
            "IRQ work interrupts",
            "CPU backtrace interrupts",
            "KGDB roundup interrupts",
        ]
        .into_iter()
        .enumerate()
        {
            let ipi = row(&rows, &format!("IPI{index}"));
            assert_eq!(ipi.description, description);
            assert!(!ipi.total_only && ipi.per_cpu.len() == 4);
        }
        assert_eq!(counts(row(&rows, "IPI0")), [691978, 705816, 718470, 703202]);
        assert!(
            rows.iter().all(|row| row.id != "LOC"),
            "arm64 has no LOC row"
        );
    }

    #[test]
    fn rows_with_fewer_columns_than_cpus_are_kept_as_totals() {
        let rows = x86();
        for id in ["ERR", "MIS"] {
            let total = row(&rows, id);
            assert!(total.total_only, "{id}");
            assert_eq!(total.per_cpu, [(0, 0, None)], "{id}");
        }
        assert!(rows.iter().filter(|row| row.total_only).count() == 2);
        let arm =
            parse_interrupts(&fixture(include_str!("fixtures/arm64_interrupts.txt"))).unwrap();
        let error = row(&arm, "Err");
        assert!(error.total_only && error.per_cpu.len() == 1);
        let mixed = "           CPU0       CPU1\nMIS:          3          4\nIPI0:    1    2    x\nERR:\nPIN:  7\n";
        let rows = parse_interrupts(mixed).unwrap();
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(
            ids,
            ["MIS", "IPI0", "PIN"],
            "a row with no counts is no row"
        );
        assert_eq!(counts(&rows[0]), [3, 4]);
        assert_eq!(rows[1].description, "x");
        assert!(!rows[0].total_only && !rows[1].total_only && rows[2].total_only);
    }

    #[test]
    fn columns_follow_the_header_when_cpus_are_offline() {
        let rows = parse_interrupts(&fixture(include_str!(
            "fixtures/x86_interrupts_cpu2_offline.txt"
        )))
        .unwrap();
        let nvme = row(&rows, "27");
        assert_eq!(nvme.per_cpu, [(0, 41230, None), (1, 0, None), (3, 9, None)]);
        assert_eq!(
            row(&rows, "LOC")
                .per_cpu
                .iter()
                .map(|&(cpu, ..)| cpu)
                .collect::<Vec<_>>(),
            [0, 1, 3]
        );
        let error = row(&rows, "ERR");
        assert!(error.total_only);
        assert_eq!(
            error.per_cpu[0].0, 0,
            "a total is filed under the first column"
        );
        let softirqs = parse_softirqs(&fixture(include_str!(
            "fixtures/x86_softirqs_cpu2_offline.txt"
        )))
        .unwrap();
        assert_eq!(
            softirqs[3].1,
            [(0, 2000, None), (1, 3000, None), (3, 4000, None)]
        );
        assert_eq!(parse_header("   CPU0  CPU10 CPU11"), Some(vec![0, 10, 11]));
        assert_eq!(parse_header("   CPU0  oops"), None);
        assert_eq!(parse_interrupts(""), None);
    }

    #[test]
    fn softirqs_parse_one_column_per_cpu() {
        let rows = parse_softirqs(&fixture(include_str!("fixtures/arm64_softirqs.txt"))).unwrap();
        let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "HI", "TIMER", "NET_TX", "NET_RX", "BLOCK", "IRQ_POLL", "TASKLET", "SCHED",
                "HRTIMER", "RCU"
            ]
        );
        assert!(rows.iter().all(|(_, per_cpu)| per_cpu.len() == 4));
        let net_receive = &rows[3].1;
        assert_eq!(
            net_receive
                .iter()
                .map(|&(cpu, count, rate)| (cpu, count, rate))
                .collect::<Vec<_>>(),
            [
                (0, 440294, None),
                (1, 106297, None),
                (2, 107675, None),
                (3, 106420, None)
            ]
        );
        assert_eq!(rows[4].1[0].1, 755467);
    }

    #[test]
    fn cpu_lists_parse_ranges_and_singles() {
        assert_eq!(parse_cpu_list("0-3,8\n"), Some(vec![0, 1, 2, 3, 8]));
        assert_eq!(parse_cpu_list("5"), Some(vec![5]));
        assert_eq!(
            parse_cpu_list("8, 0-1,1"),
            Some(vec![0, 1, 8]),
            "sorted, without repeats"
        );
        assert_eq!(parse_cpu_list("0-0"), Some(vec![0]));
        for bad in [
            "",
            "\n",
            "3-1",
            "a",
            "1,,2",
            "1-",
            "-2",
            "0-4294967295",
            "0-1,x",
        ] {
            assert_eq!(parse_cpu_list(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn unreadable_affinity_is_none_not_empty() {
        assert_eq!(irq_affinity("4294967295"), None, "no such interrupt");
        assert_eq!(irq_affinity(""), None);
        assert_eq!(
            irq_affinity("../self/stat"),
            None,
            "only numbers name an interrupt"
        );
        assert_eq!(
            irq_affinity("LOC"),
            None,
            "named rows have no affinity file"
        );
        assert_eq!(parse_cpu_list(""), None, "an empty file says nothing");
    }

    #[test]
    fn oom_kill_is_read_from_vmstat_and_absent_on_old_kernels() {
        let real = fixture(include_str!("fixtures/vmstat.txt"));
        assert_eq!(parse_oom_kills(&real), Some(0));
        assert_eq!(
            parse_oom_kills("nr_free_pages 5\noom_kill 12\npgfault 1\n"),
            Some(12)
        );
        let before_4_13: String = real
            .lines()
            .filter(|line| !line.starts_with("oom_kill"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(before_4_13.contains("pgfault"));
        assert_eq!(parse_oom_kills(&before_4_13), None);
        assert_eq!(parse_oom_kills("oom_kill_x 3\noom_kill many\n"), None);
    }

    #[test]
    fn this_machines_counter_files_parse_when_the_kernel_shows_them() {
        // Sandboxes hide or empty some of these files, so only what is shown is checked.
        if let Some(rows) = softirqs() {
            let names: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
            assert!(
                names.contains(&"TIMER") && names.contains(&"RCU"),
                "{names:?}"
            );
        }
        if let Some(rows) = interrupts() {
            assert!(!rows.is_empty());
            assert!(rows.iter().all(|row| !row.per_cpu.is_empty()));
        }
        if let Some(kills) = oom_kills() {
            assert!(kills < u64::MAX);
        }
    }
}
