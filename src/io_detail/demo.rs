//! The detail the demo synthesizes for `DemoOptions::io_detail`: what a pass with the real
//! attribution maths would have read of a demo process, one `BACKGROUND_PERIOD` after the pass
//! before it. Everything is a pure function of the time and the process's index and identity.

use std::time::{Duration, Instant};

use super::{Characters, Counts, Held, IoDetail, OpenFile, Seen, detail};
use crate::model::{BACKGROUND_PERIOD, Process, demo_syscall_chars};
use crate::platform::AccessMode;
use crate::simulation::{hash, hash_identity};

/// Seeds the draws that shape a demo process's descriptors, apart from other uses of an identity.
const DEMO_IO_SALT: u64 = 0x10de_7a11_f10e;

/// The bytes the demo's counters and positions start with at time 0, so that they stay positive
/// for the demo times before it.
const HEAD: f64 = 2.0e9;

/// A file a demo process may hold.
struct Template {
    /// Where it is; `{}` becomes the pid, for a file of the process's own.
    path: &'static str,
    device: u64,
    access: AccessMode,
    /// Opened `O_APPEND`: its position says nothing about bytes.
    appending: bool,
    /// Deleted while open.
    deleted: bool,
    /// The fractions of the process's read and write bytes that move this descriptor's position.
    /// A write-only file reads nothing, and the fractions of all the files a process holds sum to
    /// well under one, which leaves the rest unattributed (pipes, sockets, the terminal, `pread`).
    read: f64,
    write: f64,
}

const TEMPLATES: [Template; 8] = [
    Template {
        path: "/usr/lib/x86_64-linux-gnu/libLLVM.so.18",
        device: 2049,
        access: AccessMode::Read,
        appending: false,
        deleted: false,
        read: 0.26,
        write: 0.0,
    },
    Template {
        path: "/usr/lib/locale/locale-archive",
        device: 2049,
        access: AccessMode::Read,
        appending: false,
        deleted: false,
        read: 0.08,
        write: 0.0,
    },
    Template {
        path: "/home/demo/src/project/target/debug/deps/main.o",
        device: 2050,
        access: AccessMode::Write,
        appending: false,
        deleted: false,
        read: 0.0,
        write: 0.22,
    },
    Template {
        path: "/home/demo/.cache/demo/index.db",
        device: 2050,
        access: AccessMode::Both,
        appending: false,
        deleted: false,
        read: 0.14,
        write: 0.10,
    },
    Template {
        path: "/var/log/demo/service.log",
        device: 2051,
        access: AccessMode::Write,
        appending: true,
        deleted: false,
        read: 0.0,
        write: 0.12,
    },
    Template {
        path: "/var/log/demo/service.log.1",
        device: 2051,
        access: AccessMode::Write,
        appending: false,
        deleted: true,
        read: 0.0,
        write: 0.10,
    },
    Template {
        path: "/tmp/demo-scratch-{}",
        device: 26,
        access: AccessMode::Both,
        appending: false,
        deleted: false,
        read: 0.06,
        write: 0.08,
    },
    Template {
        path: "/home/demo/Documents/notes.txt",
        device: 2050,
        access: AccessMode::Read,
        appending: false,
        deleted: false,
        read: 0.05,
        write: 0.0,
    },
];

/// The byte counters of demo process `index` at `time`, cumulative from `HEAD`.
fn counters(index: usize, time: f64) -> (f64, f64) {
    let (read, write) = demo_syscall_chars(time, index);
    ((HEAD + read).max(0.0), (HEAD + write).max(0.0))
}

/// What demo process `index` (the position of its pid after 1000) holds at `time`.
fn held(index: usize, process: &Process, time: f64) -> Held {
    let seed = hash_identity(process.id, DEMO_IO_SALT);
    let (read, write) = counters(index, time);
    let count = 3 + (seed % 4) as usize;
    let first = ((seed >> 8) % TEMPLATES.len() as u64) as usize;
    let files = (0..count)
        .map(|offset| {
            let template = &TEMPLATES[(first + offset) % TEMPLATES.len()];
            let path = template.path.replace("{}", &process.id.pid.to_string());
            let position = (template.read * read + template.write * write) as u64;
            // A file in use stays a little larger than where its descriptor is, and one opened to
            // append is always at its end.
            let slack = if template.appending {
                0
            } else {
                (1 + hash::fnv1a_64(&path) % 64) << 20
            };
            OpenFile {
                fd: 3 + offset as u32,
                device: template.device,
                inode: 1000 + hash::fnv1a_64(&path) % 9_000_000,
                size: position + slack,
                path,
                deleted: template.deleted,
                access: template.access,
                appending: template.appending,
                position,
            }
        })
        .collect();
    Held {
        files,
        counts: Counts {
            pipes: 1 + ((seed >> 16) % 3) as u32,
            sockets: 1 + ((seed >> 24) % 4) as u32,
            ttys: ((seed >> 32) % 2) as u32,
            anonymous: 1 + ((seed >> 40) % 3) as u32,
            other: ((seed >> 48) % 3) as u32,
        },
        characters: Some((
            Characters {
                read: read as u64,
                write: write as u64,
            },
            Characters {
                read: read as u64,
                write: write as u64,
            },
        )),
        examined: 0,
        partial: false,
    }
}

/// The detail of demo process `index` at demo `time` (`DemoOptions::io_detail`): its files, the
/// descriptors that are not files, and the rates over the `BACKGROUND_PERIOD` that ends at `time`,
/// made by `IoScan`'s own attribution from two readings that many seconds apart. The positions
/// advance with the integral of the process's I/O (`demo_syscall_chars`), so the rates are
/// averages of what `Process::syscall_io` says and never exceed it; every file holds a fraction of
/// that I/O, so the part that remains unattributed is never zero while the process does I/O.
///
/// A process that does no I/O (`index` not a multiple of 7) holds the same files and reads zero
/// everywhere.
pub fn demo_detail(index: usize, process: &Process, time: f64) -> IoDetail {
    let seconds = BACKGROUND_PERIOD.as_secs_f64();
    let earlier = held(index, process, time - seconds);
    let later = held(index, process, time);
    let began = Instant::now();
    let seen = Seen {
        held: earlier,
        began,
    };
    detail(
        Some(&seen),
        &later,
        began + Duration::from_secs_f64(seconds),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io_detail::{FileFlow, Flow, IO_PROCESSES};
    use crate::model::{DemoOptions, Measured, Snapshot, demo, demo_with};
    use crate::platform::{Source, Sources};
    use std::sync::Arc;

    const WITH: DemoOptions = DemoOptions {
        io_detail: true,
        ..DemoOptions::NONE
    };

    fn details(snapshot: &Snapshot) -> Vec<(u32, Arc<IoDetail>)> {
        snapshot
            .processes
            .iter()
            .filter_map(|process| Some((process.id.pid, Arc::clone(process.io_detail.known()?))))
            .collect()
    }

    #[test]
    fn demo_io_detail_is_repeatable() {
        let first = demo_with(12.0, 192, WITH);
        // A pure function of the time: other times in between, and in any order, change nothing.
        for time in [3.0, -4.0, 400.0, 12.0] {
            let other = demo_with(time, 192, WITH);
            assert!(!details(&other).is_empty(), "time {time}");
        }
        let again = demo_with(12.0, 192, WITH);
        assert_eq!(details(&first), details(&again));
        assert!(!details(&first).is_empty());
        // The positions move with the time, so the rates do.
        let later = demo_with(15.5, 192, WITH);
        assert_ne!(details(&first), details(&later));
    }

    #[test]
    fn demo_io_detail_is_off_unless_a_row_asks_and_then_changes_nothing_else() {
        let plain = demo(12.0, 192);
        assert!(plain.sources.is_empty());
        assert!(
            plain
                .processes
                .iter()
                .all(|process| process.io_detail == Measured::Pending)
        );
        let options = demo_with(12.0, 192, WITH);
        assert_eq!(options.sources, Sources::from(Source::IoDetail));
        // Every other field of every process is as in the plain demo.
        let mut stripped = options.processes.clone();
        for process in &mut stripped {
            process.io_detail = Measured::Pending;
        }
        assert_eq!(
            format!("{stripped:?}"),
            format!("{:?}", plain.processes),
            "only io_detail differs"
        );
        assert_eq!(options.links, plain.links);
    }

    #[test]
    fn demo_io_detail_synthesizes_every_field() {
        let snapshot = demo_with(12.0, 192, WITH);
        let all = details(&snapshot);
        assert!(
            !all.is_empty() && all.len() <= IO_PROCESSES,
            "{}",
            all.len()
        );
        let files: Vec<&FileFlow> = all.iter().flat_map(|(_, d)| &d.files).collect();
        for prefix in ["/usr/lib", "/home/demo", "/var/log", "/tmp"] {
            assert!(
                files.iter().any(|file| file.path.starts_with(prefix)),
                "no file under {prefix}"
            );
        }
        assert!(files.iter().any(|file| file.deleted), "a deleted file");
        assert!(
            files.iter().all(|file| !file.path.ends_with(" (deleted)")),
            "the suffix is taken off"
        );
        assert!(files.iter().any(|file| file.flow == Flow::Unpositioned));
        assert!(files.iter().any(|file| matches!(
            file.flow,
            Flow::Measured { read, .. } if read > 0.0
        )));
        assert!(files.iter().any(|file| matches!(
            file.flow,
            Flow::Measured { written, .. } if written > 0.0
        )));
        assert!(files.iter().any(|file| matches!(
            file.flow,
            Flow::Measured { either, .. } if either > 0.0
        )));
        for count in [
            |d: &IoDetail| d.pipes,
            |d: &IoDetail| d.sockets,
            |d: &IoDetail| d.ttys,
            |d: &IoDetail| d.anonymous,
            |d: &IoDetail| d.other,
        ] {
            assert!(all.iter().any(|(_, d)| count(d) > 0));
        }
        for (pid, detail) in &all {
            let left = detail
                .unattributed
                .unwrap_or_else(|| panic!("pid {pid} has a remainder"));
            assert!(
                left.read > 0.0 && left.write > 0.0,
                "pid {pid}: files take a part of its I/O, not all: {left:?}"
            );
            assert!(!detail.partial);
            let mut seen = std::collections::HashSet::new();
            assert!(
                detail
                    .files
                    .iter()
                    .all(|file| seen.insert((file.device, file.inode))),
                "one entry per file"
            );
        }
    }

    #[test]
    fn demo_io_detail_never_attributes_more_than_the_process_reads_and_writes() {
        let seconds = BACKGROUND_PERIOD.as_secs_f64();
        for time in [0.0, 12.0, 37.5, 90.0, -3.0] {
            let snapshot = demo_with(time, 192, WITH);
            for process in &snapshot.processes {
                let Some(detail) = process.io_detail.known() else {
                    continue;
                };
                let index = (process.id.pid - 1000) as usize;
                let (read_now, write_now) = demo_syscall_chars(time, index);
                let (read_then, write_then) = demo_syscall_chars(time - seconds, index);
                let (read, write) = (
                    (read_now - read_then) / seconds,
                    (write_now - write_then) / seconds,
                );
                let (mut read_claimed, mut write_claimed, mut either_claimed) = (0.0, 0.0, 0.0);
                for file in &detail.files {
                    if let Flow::Measured {
                        read,
                        written,
                        either,
                    } = file.flow
                    {
                        read_claimed += f64::from(read);
                        write_claimed += f64::from(written);
                        either_claimed += f64::from(either);
                    }
                }
                // The counters and positions are whole bytes, so a byte per interval of rounding.
                let slack = 1.0 + 1e-4 * (read + write);
                assert!(
                    read_claimed <= read + slack,
                    "{time} {index}: {read_claimed} > {read}"
                );
                assert!(write_claimed <= write + slack, "{time} {index}");
                assert!(
                    read_claimed + write_claimed + either_claimed <= read + write + slack,
                    "{time} {index}"
                );
                let left = detail.unattributed.unwrap();
                assert!(left.read >= 0.0 && left.write >= 0.0 && left.either >= 0.0);
            }
        }
    }

    #[test]
    fn demo_io_detail_goes_to_the_base_processes_among_the_busiest_by_syscall_rate() {
        let options = DemoOptions {
            churn: true,
            io_detail: true,
            ..DemoOptions::NONE
        };
        let snapshot = demo_with(40.0, 192, options);
        let busiest = crate::io_detail::select(
            &snapshot.processes[..192],
            crate::io_detail::by_syscall_rate,
        );
        let chosen: Vec<_> = snapshot
            .processes
            .iter()
            .filter(|process| process.io_detail.known().is_some())
            .map(|process| process.id)
            .collect();
        assert!(chosen.len() > 1);
        assert!(
            chosen.iter().all(|id| busiest.contains(id)),
            "only the busiest are read"
        );
        assert!(
            snapshot.processes[192..]
                .iter()
                .all(|process| process.io_detail == Measured::Pending),
            "the transients are not"
        );
    }
}
