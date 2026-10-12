//! The out-of-memory ranking (`/proc/<pid>/oom_score` and `oom_score_adj`) and a process's memory
//! by kind (`/proc/<pid>/smaps_rollup`), read on the background thread.

use std::collections::HashMap;
use std::fs;
use std::io;

use crate::model::{Oom, Rollup};
use crate::platform::Taken;

/// Linux reads a process's memory by kind.
pub const MEMORY_DETAIL: bool = true;

/// The `oom_score_adj` that exempts a process from the out-of-memory killer.
const EXEMPT: i16 = -1000;

/// Each pid's ranking, None for a pid that could not be read. A pid that exited is absent.
pub fn oom_scores(pids: &[u32]) -> Option<HashMap<u32, Option<Oom>>> {
    let mut found = HashMap::with_capacity(pids.len());
    for &pid in pids {
        match oom(pid) {
            Taken::Value(oom) => found.insert(pid, Some(oom)),
            Taken::Unreadable => found.insert(pid, None),
            Taken::Gone => None,
        };
    }
    Some(found)
}

/// One process's memory by kind.
pub fn rollup(pid: u32) -> Taken<Rollup> {
    match fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
        Ok(text) => parse_rollup(&text).map_or(Taken::Unreadable, Taken::Value),
        // A kernel without the file (before 4.14) has the process directory all the same.
        Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
            if fs::metadata(format!("/proc/{pid}")).is_ok() {
                Taken::Unreadable
            } else {
                Taken::Gone
            }
        }
        Err(error) => failed(&error),
    }
}

fn oom(pid: u32) -> Taken<Oom> {
    let score = match fs::read_to_string(format!("/proc/{pid}/oom_score")) {
        Ok(text) => text,
        Err(error) => return failed(&error),
    };
    // The adjustment is a second file, so the process may have gone between the two.
    let adjust = match fs::read_to_string(format!("/proc/{pid}/oom_score_adj")) {
        Ok(text) => text,
        Err(error) => return failed(&error),
    };
    parse_oom(&score, &adjust).map_or(Taken::Unreadable, Taken::Value)
}

/// A failed read: a process that is gone (ENOENT, or ESRCH from a process that exited while it
/// was open) is no gap, and anything else, permissions first, leaves it unreadable.
fn failed<T>(error: &io::Error) -> Taken<T> {
    match error.raw_os_error() {
        Some(libc::ENOENT | libc::ESRCH) => Taken::Gone,
        _ => Taken::Unreadable,
    }
}

/// The ranking from the text of `oom_score` and `oom_score_adj`. The score is kept as read; an
/// adjustment that does not parse is unknown, which leaves the process not exempt.
fn parse_oom(score: &str, adjust: &str) -> Option<Oom> {
    let badness = score.trim().parse().ok()?;
    let adjust: Option<i16> = adjust.trim().parse().ok();
    Some(Oom {
        badness,
        adjust,
        exempt: adjust == Some(EXEMPT),
    })
}

/// The memory by kind from the text of `smaps_rollup`, whose values are in kibibytes. None when a
/// field every kernel since 4.14 prints is missing. The split of the proportional set size (the
/// three `Pss_` fields) came with 5.3 and stays None before.
fn parse_rollup(text: &str) -> Option<Rollup> {
    let mut fields: HashMap<&str, u64> = HashMap::new();
    for line in text.lines() {
        // The first line names the mapping's range, which has colons of its own and no value.
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if let Some(kibibytes) = rest.split_whitespace().next().and_then(|v| v.parse().ok()) {
            fields.insert(key, kibibytes);
        }
    }
    let bytes = |key: &str| {
        fields
            .get(key)
            .map(|kibibytes| kibibytes.saturating_mul(1024))
    };
    Some(Rollup {
        rss: bytes("Rss")?,
        pss: bytes("Pss")?,
        pss_anon: bytes("Pss_Anon"),
        pss_file: bytes("Pss_File"),
        pss_shmem: bytes("Pss_Shmem"),
        shared_clean: bytes("Shared_Clean")?,
        shared_dirty: bytes("Shared_Dirty")?,
        private_clean: bytes("Private_Clean")?,
        private_dirty: bytes("Private_Dirty")?,
        swap: bytes("Swap")?,
        swap_pss: bytes("SwapPss")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `smaps_rollup` of a Linux 6.8 process.
    const ROLLUP: &str = "\
5584d1e00000-7ffd7e3f2000 ---p 00000000 00:00 0                          [rollup]
Rss:               34816 kB
Pss:               12288 kB
Pss_Dirty:          6144 kB
Pss_Anon:           8192 kB
Pss_File:           3584 kB
Pss_Shmem:           512 kB
Shared_Clean:      16384 kB
Shared_Dirty:        128 kB
Private_Clean:      9728 kB
Private_Dirty:      8576 kB
Referenced:        30720 kB
Anonymous:          8704 kB
KSM:                   0 kB
LazyFree:              0 kB
AnonHugePages:      2048 kB
ShmemPmdMapped:        0 kB
FilePmdMapped:         0 kB
Shared_Hugetlb:        0 kB
Private_Hugetlb:       0 kB
Swap:               2048 kB
SwapPss:            1024 kB
Locked:                0 kB
";

    /// The same file from a kernel before 5.3: no `Pss_` split.
    const BEFORE_SPLIT: &str = "\
55d5b3a00000-7ffe3fd8c000 ---p 00000000 00:00 0                          [rollup]
Rss:               34816 kB
Pss:               12288 kB
Shared_Clean:      16384 kB
Shared_Dirty:        128 kB
Private_Clean:      9728 kB
Private_Dirty:      8576 kB
Referenced:        30720 kB
Anonymous:          8704 kB
LazyFree:              0 kB
AnonHugePages:      2048 kB
ShmemPmdMapped:        0 kB
Shared_Hugetlb:        0 kB
Private_Hugetlb:       0 kB
Swap:               2048 kB
SwapPss:            1024 kB
Locked:                0 kB
";

    #[test]
    fn smaps_rollup_parses_every_field_in_bytes() {
        let rollup = parse_rollup(ROLLUP).unwrap();
        assert_eq!(
            rollup,
            Rollup {
                rss: 34816 * 1024,
                pss: 12288 * 1024,
                pss_anon: Some(8192 * 1024),
                pss_file: Some(3584 * 1024),
                pss_shmem: Some(512 * 1024),
                shared_clean: 16384 * 1024,
                shared_dirty: 128 * 1024,
                private_clean: 9728 * 1024,
                private_dirty: 8576 * 1024,
                swap: 2048 * 1024,
                swap_pss: 1024 * 1024,
            }
        );
        let parts =
            rollup.shared_clean + rollup.shared_dirty + rollup.private_clean + rollup.private_dirty;
        assert_eq!(parts, rollup.rss, "the fixture is a consistent kernel's");
    }

    #[test]
    fn smaps_rollup_without_the_split_leaves_it_unknown() {
        let rollup = parse_rollup(BEFORE_SPLIT).unwrap();
        assert_eq!(
            (rollup.pss_anon, rollup.pss_file, rollup.pss_shmem),
            (None, None, None)
        );
        assert_eq!(rollup.pss, 12288 * 1024);
        assert_eq!(rollup.swap_pss, 1024 * 1024);
    }

    #[test]
    fn a_rollup_missing_a_field_every_kernel_prints_is_not_read() {
        let without_swap = BEFORE_SPLIT.replace("Swap:", "Unrelated:");
        assert_eq!(parse_rollup(&without_swap), None);
        assert_eq!(parse_rollup(""), None, "a kernel thread's file is empty");
    }

    #[test]
    fn oom_keeps_the_score_as_read_and_exempts_minus_one_thousand() {
        assert_eq!(
            parse_oom("667\n", "200\n"),
            Some(Oom {
                badness: 667,
                adjust: Some(200),
                exempt: false
            })
        );
        // The kernel reports 0 for a process it never picks.
        assert_eq!(
            parse_oom("0\n", "-1000\n"),
            Some(Oom {
                badness: 0,
                adjust: Some(-1000),
                exempt: true
            })
        );
        let unparsed = parse_oom("12\n", "garbage").unwrap();
        assert_eq!((unparsed.adjust, unparsed.exempt), (None, false));
        assert_eq!(parse_oom("", "0"), None);
    }

    #[test]
    fn this_process_ranks_and_reads_its_own_rollup() {
        let me = std::process::id();
        let found = oom_scores(&[me, u32::MAX]).expect("Linux always ranks");
        assert!(found[&me].is_some_and(|oom| oom.adjust.is_some()));
        assert!(!found.contains_key(&u32::MAX), "a pid that does not exist");
        let Taken::Value(own) = rollup(me) else {
            panic!("a process reads its own smaps_rollup");
        };
        assert!(own.rss > 0 && own.pss <= own.rss);
        assert!(matches!(rollup(u32::MAX), Taken::Gone));
    }
}
