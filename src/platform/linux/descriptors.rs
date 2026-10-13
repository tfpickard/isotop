//! The descriptors of a process for `Source::IoDetail`: its regular files with their access mode,
//! position and size (from `/proc/<pid>/fd` and `fdinfo`), its other descriptors by kind, and the
//! byte counters of its read and write calls (`rchar` and `wchar` of `/proc/<pid>/io`) read just
//! before and just after the walk. The pass that makes rates of them is `io_detail::IoScan`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::fdinfo::parse_fdinfo;
use super::files::{Regular, Target, attributes, classify};
use super::{split_stat, timing};
use crate::io_detail::{
    Budget, Characters, DescriptorReader, Held, OpenFile, Reading, by_syscall_rate,
};
use crate::model::{Identity, Process};

/// Reads the descriptors of one process after another.
pub struct Descriptors {
    root: PathBuf,
    /// Whether this kernel counts the bytes of read and write calls: `io` is in `/proc/self`.
    /// Kernels built without task I/O accounting have no such file.
    counted: bool,
}

impl Descriptors {
    pub fn new() -> Self {
        Self::at("/proc")
    }

    fn at(root: &str) -> Self {
        let root = PathBuf::from(root);
        let counted = fs::read_to_string(root.join("self/io"))
            .is_ok_and(|text| parse_characters(&text).is_some());
        Descriptors { root, counted }
    }

    /// The regular file that descriptor `fd` of the process in `process` holds, if it is still
    /// open and can be read.
    fn open_file(&self, process: &Path, fd: u32, file: Regular) -> Opened {
        let text = match fs::read_to_string(process.join("fdinfo").join(fd.to_string())) {
            Ok(text) => text,
            // It closed since the directory was listed.
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Opened::Closed,
            Err(_) => return Opened::Unreadable,
        };
        let Some(info) = parse_fdinfo(&text) else {
            return Opened::Unreadable;
        };
        let Some(access) = info.access() else {
            return Opened::Unreadable;
        };
        // The size was read before the position, so a descriptor that wrote in between sits past
        // it. Read it again, so that a position past the end of the file means a seek, not a
        // late measurement.
        let mut size = file.size;
        if info.pos > size
            && let Some(status) = attributes(&process.join("fd").join(fd.to_string()))
        {
            size = size.max(status.stx_size);
        }
        Opened::File(OpenFile {
            fd,
            device: file.device,
            inode: file.inode,
            size,
            path: String::from_utf8_lossy(&file.path).into_owned(),
            deleted: file.deleted,
            access,
            appending: info.appending(),
            position: info.pos,
        })
    }
}

/// What reading one regular file's descriptor came to.
enum Opened {
    File(OpenFile),
    /// The descriptor closed since the directory was listed: it is not there to count.
    Closed,
    /// It is open but its position could not be read.
    Unreadable,
}

/// `rchar` and `wchar` from the text of `/proc/<pid>/io`; None when either is missing, since a
/// counter that was not read is not zero.
fn parse_characters(text: &str) -> Option<Characters> {
    let (mut read, mut write) = (None, None);
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key {
            "rchar" => read = value.trim().parse().ok(),
            "wchar" => write = value.trim().parse().ok(),
            _ => {}
        }
    }
    Some(Characters {
        read: read?,
        write: write?,
    })
}

/// The start time of the process in `process` (`starttime` of its stat file, in ticks after boot),
/// which with its pid is its `Identity`; None when it cannot be read, as for a process that has
/// exited.
fn start_of(process: &Path) -> Option<u64> {
    let text = fs::read_to_string(process.join("stat")).ok()?;
    let (_, fields) = split_stat(&text)?;
    Some(timing(&fields)?.start)
}

impl DescriptorReader for Descriptors {
    fn counts_bytes(&self) -> bool {
        self.counted
    }

    /// The byte counters are read before the walk and after it, so that the interval between two
    /// readings contains the one the positions span. Another user's process is `Unreadable`
    /// (`io` and `fd` need the same permission). The pid is checked against the identity after
    /// the walk: if the process that is there now is the one that was asked for, it was there
    /// throughout, since a pid is not given to another process while it lives.
    fn read(&mut self, id: Identity, budget: Budget) -> Reading {
        let process = self.root.join(id.pid.to_string());
        let before = if self.counted {
            match fs::read_to_string(process.join("io")) {
                Ok(text) => parse_characters(&text),
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    return Reading::Unreadable;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Reading::Gone,
                Err(_) => None,
            }
        } else {
            None
        };
        let entries = match fs::read_dir(process.join("fd")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Reading::Unreadable;
            }
            Err(_) => return Reading::Gone,
        };
        let mut held = Held::default();
        for entry in entries.flatten() {
            let Some(fd) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
            else {
                continue;
            };
            if held.examined == budget.descriptors {
                held.partial = true;
                break;
            }
            held.examined += 1;
            match classify(&entry.path()) {
                Target::Regular(file) => {
                    if held.files.len() == budget.files {
                        held.partial = true;
                        continue;
                    }
                    match self.open_file(&process, fd, file) {
                        Opened::File(file) => held.files.push(file),
                        Opened::Closed => {}
                        Opened::Unreadable => {
                            held.counts.other += 1;
                            held.partial = true;
                        }
                    }
                }
                Target::Pipe => held.counts.pipes += 1,
                Target::Socket => held.counts.sockets += 1,
                Target::Tty => held.counts.ttys += 1,
                Target::Anonymous => held.counts.anonymous += 1,
                Target::Other => held.counts.other += 1,
            }
        }
        let after = if self.counted {
            fs::read_to_string(process.join("io"))
                .ok()
                .and_then(|text| parse_characters(&text))
        } else {
            None
        };
        if start_of(&process) != Some(id.start) {
            return Reading::Gone;
        }
        held.characters = before.zip(after);
        Reading::Taken(held)
    }
}

/// How busy `process` is for choosing the processes to read: its syscalls per second.
pub fn io_rank(process: &Process) -> Option<f64> {
    by_syscall_rate(process)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io_detail;
    use crate::platform::AccessMode;
    use crate::platform::linux::Sampler;
    use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
    use std::os::fd::AsRawFd;

    /// This process, as the sampler names it.
    fn myself() -> Identity {
        Sampler::new()
            .processes()
            .unwrap()
            .into_iter()
            .find(|raw| raw.process.id.pid == std::process::id())
            .expect("the sampler lists this process")
            .process
            .id
    }

    fn unlimited() -> Budget {
        Budget {
            descriptors: usize::MAX,
            files: usize::MAX,
        }
    }

    /// A temporary directory of its own, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("isotop-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Scratch(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn held_by_me(reader: &mut Descriptors, budget: Budget) -> Held {
        match reader.read(myself(), budget) {
            Reading::Taken(held) => held,
            other => panic!("this process is readable: {other:?}"),
        }
    }

    #[test]
    fn io_counters_parse_only_when_both_are_there() {
        let text = "rchar: 99\nwchar: 98\nsyscr: 7\nsyscw: 5\nread_bytes: 4096\nwrite_bytes: 1000\ncancelled_write_bytes: 0\n";
        assert_eq!(
            parse_characters(text),
            Some(Characters {
                read: 99,
                write: 98
            })
        );
        assert_eq!(parse_characters("rchar: 99\nsyscr: 7\n"), None);
        assert_eq!(parse_characters("wchar: 98\n"), None);
        assert_eq!(parse_characters("rchar: x\nwchar: 98\n"), None);
        assert_eq!(parse_characters(""), None);
    }

    #[test]
    fn a_process_is_read_with_its_files_positions_and_other_descriptors() {
        let scratch = Scratch::new("descriptors");
        let path = scratch.0.join("a file\nwith a newline");
        fs::write(&path, vec![9_u8; 10_000]).unwrap();
        let mut reader = Descriptors::new();
        assert!(reader.counts_bytes(), "task I/O accounting is on here");

        let mut reading = fs::File::open(&path).unwrap();
        reading.read_exact(&mut [0_u8; 4096]).unwrap();
        let mut writing = fs::OpenOptions::new().write(true).open(&path).unwrap();
        writing.seek(SeekFrom::Start(1234)).unwrap();
        let mut appending = fs::OpenOptions::new().append(true).open(&path).unwrap();
        appending.write_all(b"tail").unwrap();
        let (mut pipe_read, pipe_write) = std::io::pipe().unwrap();
        let held = held_by_me(&mut reader, unlimited());
        let at = |descriptor: &fs::File| {
            held.files
                .iter()
                .find(|file| file.fd == descriptor.as_raw_fd() as u32)
                .expect("a regular file is listed")
        };
        let read = at(&reading);
        assert_eq!(read.access, AccessMode::Read);
        assert_eq!((read.position, read.size), (4096, 10_004));
        assert!(!read.appending && !read.deleted);
        assert_eq!(
            read.path,
            path.to_str().unwrap(),
            "the path is kept raw, newline and all"
        );
        assert_eq!(
            (at(&writing).access, at(&writing).position),
            (AccessMode::Write, 1234)
        );
        let append = at(&appending);
        assert!(append.appending);
        assert_eq!(append.position, 10_004, "an appended file sits at its end");
        assert!(held.counts.pipes >= 2, "both ends of the pipe: {held:?}");
        assert!(held.counts.sockets + held.counts.anonymous + held.counts.other > 0);
        assert!(held.examined >= held.files.len() + 2);
        assert!(!held.partial);
        let (before, after) = held.characters.expect("the counters are read");
        assert!(after.read >= before.read && after.write >= before.write);
        drop(pipe_write);
        assert_eq!(pipe_read.read(&mut [0_u8; 1]).unwrap(), 0);
    }

    #[test]
    fn a_real_pass_credits_the_positions_that_moved_and_leaves_offset_io_unattributed() {
        use crate::io_detail::{Caps, Flow, IoScan};
        use std::os::unix::fs::FileExt;
        let scratch = Scratch::new("real-pass");
        let path = scratch.0.join("data");
        fs::write(&path, vec![1_u8; 1 << 20]).unwrap();
        let mut reading = fs::File::open(&path).unwrap();
        let mut writing = fs::OpenOptions::new().write(true).open(&path).unwrap();
        let mut both = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let me = myself();
        let mut descriptors = Descriptors::new();
        let mut scan = IoScan::default();
        // One second per reading of the clock: the interval from the start of the first pass to
        // the end of the second is three seconds.
        let mut now = std::time::Instant::now();
        let mut clock = move || {
            now += std::time::Duration::from_secs(1);
            now
        };
        let first = scan.pass_with(&[me], &mut descriptors, Caps::DEFAULT, &mut clock);
        let first = first[&me].known().expect("this process is read");
        let file = |detail: &io_detail::IoDetail| {
            detail
                .files
                .iter()
                .find(|file| file.path == path.to_str().unwrap())
                .cloned()
                .expect("the file is listed")
        };
        assert_eq!(
            file(first).flow,
            Flow::Unmeasured,
            "nothing to compare with"
        );
        assert_eq!(first.unattributed, None);

        // Between the passes: 32 KiB read and 64 KiB written through positions, 4 KiB each way
        // through a read-write descriptor, and 16 KiB read and 8 KiB written at offsets of their
        // own, which no position records.
        reading.read_exact(&mut [0_u8; 32768]).unwrap();
        writing.write_all(&[2_u8; 65536]).unwrap();
        both.read_exact(&mut [0_u8; 4096]).unwrap();
        both.write_all(&[3_u8; 4096]).unwrap();
        reading.read_exact_at(&mut [0_u8; 16384], 500_000).unwrap();
        both.write_all_at(&[4_u8; 8192], 200_000).unwrap();

        let second = scan.pass_with(&[me], &mut descriptors, Caps::DEFAULT, &mut clock);
        let second = second[&me].known().expect("this process is read");
        let Flow::Measured {
            read,
            written,
            either,
        } = file(second).flow
        else {
            panic!("the positions moved: {:?}", file(second));
        };
        let bytes = |rate: f32| (f64::from(rate) * 3.0).round() as u64;
        assert_eq!(
            (bytes(read), bytes(written), bytes(either)),
            (32768, 65536, 8192)
        );
        // The counters grew by at least what the positions and the offsets moved, and the offsets
        // are all that is left, with whatever other threads read and wrote meanwhile.
        let left = second.unattributed.expect("Linux counts the bytes");
        assert!(bytes(left.read) >= 16384, "{left:?}");
        assert!(bytes(left.write) >= 8192, "{left:?}");
        assert_eq!(bytes(left.either), 8192);
    }

    #[test]
    fn a_deleted_file_is_listed_without_the_suffix_and_marked() {
        let scratch = Scratch::new("deleted-descriptor");
        let path = scratch.0.join("rotated.log");
        let file = fs::File::create(&path).unwrap();
        fs::remove_file(&path).unwrap();
        let held = held_by_me(&mut Descriptors::new(), unlimited());
        let listed = held
            .files
            .iter()
            .find(|open| open.fd == file.as_raw_fd() as u32)
            .unwrap();
        assert!(listed.deleted);
        assert_eq!(listed.path, path.to_str().unwrap());
    }

    #[test]
    fn caps_mark_the_reading_partial() {
        let scratch = Scratch::new("descriptor-caps");
        let file = scratch.0.join("held");
        fs::write(&file, b"x").unwrap();
        let open: Vec<fs::File> = (0..12).map(|_| fs::File::open(&file).unwrap()).collect();
        let mut reader = Descriptors::new();
        let all = held_by_me(&mut reader, unlimited());
        assert!(!all.partial);
        assert!(all.files.len() >= open.len());
        // Too many files for one process: the first are kept, and the reading says it is cut.
        let few = held_by_me(
            &mut reader,
            Budget {
                descriptors: usize::MAX,
                files: 3,
            },
        );
        assert_eq!(few.files.len(), 3);
        assert!(few.partial);
        // Too few descriptors for the pass: the walk stops where the budget does.
        let short = held_by_me(
            &mut reader,
            Budget {
                descriptors: 5,
                files: usize::MAX,
            },
        );
        assert_eq!(short.examined, 5);
        assert!(short.partial);
    }

    #[test]
    fn a_process_that_is_gone_or_has_another_start_is_not_read() {
        let mut reader = Descriptors::new();
        let me = myself();
        let wrong_start = Identity {
            start: me.start + 1,
            ..me
        };
        assert_eq!(reader.read(wrong_start, unlimited()), Reading::Gone);
        // Beyond pid_max (4194304 at most), so no process can have it.
        let nobody = Identity {
            pid: 4_194_305 + 10,
            start: 1,
        };
        assert_eq!(reader.read(nobody, unlimited()), Reading::Gone);
    }

    #[test]
    fn a_table_that_cannot_be_listed_is_unreadable_not_empty() {
        // The fd directory of another user's process is closed to us. As root nothing is, so the
        // test makes one that is, where the permission checks apply.
        if is_root() {
            return;
        }
        let scratch = Scratch::new("descriptor-denied");
        let process = scratch.0.join("77");
        fs::create_dir_all(process.join("fd")).unwrap();
        fs::write(
            process.join("stat"),
            b"77 (x) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 5",
        )
        .unwrap();
        let mut fixed = fs::metadata(process.join("fd")).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut fixed, 0o000);
        fs::set_permissions(process.join("fd"), fixed).unwrap();
        let mut reader = Descriptors {
            root: scratch.0.clone(),
            counted: false,
        };
        let id = Identity { pid: 77, start: 5 };
        assert_eq!(reader.read(id, unlimited()), Reading::Unreadable);
    }

    /// Whether this test run is root, for which the permission checks of the kernel do not apply.
    fn is_root() -> bool {
        // SAFETY: geteuid takes no arguments, has no preconditions and cannot fail.
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn only_the_busiest_processes_by_syscall_rate_are_scanned() {
        use crate::model::SyscallIo;
        let busy = |pid: u32, calls: f32| Process {
            syscall_io: Some(SyscallIo {
                reads: calls,
                writes: 0.0,
                ..SyscallIo::default()
            }),
            ..crate::model::testing::process(pid, 1)
        };
        // Thirty processes, the pid being the call rate, so the busiest are the largest pids.
        let mut processes: Vec<Process> = (1..=30).map(|pid| busy(pid, pid as f32)).collect();
        processes.push(Process {
            syscall_io: None,
            ..crate::model::testing::process(31, 1)
        });
        processes.push(busy(32, 0.0));
        let chosen: Vec<u32> = io_detail::targets(&processes)
            .iter()
            .map(|id| id.pid)
            .collect();
        let expected: Vec<u32> = (7..=30).rev().collect();
        assert_eq!(chosen, expected, "24 processes, busiest first");
        assert_eq!(chosen.len(), io_detail::IO_PROCESSES);
        assert!(
            !chosen.contains(&31) && !chosen.contains(&32),
            "a process with no reading, or no calls, is never chosen"
        );
    }

    #[test]
    #[ignore = "prints how long a pass over the 24 processes with the most descriptors takes"]
    fn print_how_long_a_pass_over_the_busiest_descriptors_takes() {
        let mut sampler = Sampler::new();
        let mut processes = sampler.processes().unwrap();
        let count = |pid: u32| {
            fs::read_dir(format!("/proc/{pid}/fd")).map_or(0, std::iter::Iterator::count)
        };
        processes.sort_by_key(|raw| std::cmp::Reverse(count(raw.process.id.pid)));
        let targets: Vec<Identity> = processes
            .iter()
            .take(io_detail::IO_PROCESSES)
            .map(|raw| raw.process.id)
            .collect();
        let mut scan = io_detail::IoScan::default();
        let mut reader = Descriptors::new();
        for round in 0..4 {
            let started = std::time::Instant::now();
            let found = scan.pass(&targets, &mut reader);
            let known = found.values().filter(|d| d.known().is_some()).count();
            println!(
                "pass {round}: {:.2} ms for {} of {} processes",
                started.elapsed().as_secs_f64() * 1000.0,
                known,
                targets.len()
            );
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        // The worst case a pass allows: this process holding 3000 regular files open, on top of
        // the busiest 24, of which the pass examines at most 4096 descriptors.
        let scratch = Scratch::new("pass-time");
        let file = scratch.0.join("held");
        fs::write(&file, b"x").unwrap();
        let held: Vec<fs::File> = (0..3000).map(|_| fs::File::open(&file).unwrap()).collect();
        let me = [myself()];
        for round in 0..3 {
            let started = std::time::Instant::now();
            let found = scan.pass(&me, &mut reader);
            let files = found[&me[0]].known().map_or(0, |d| d.files.len());
            println!(
                "heavy pass {round}: {:.2} ms for {files} files",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        drop(held);
    }
}
