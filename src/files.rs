//! Open files and file locks, read on the background thread: the regular files each process holds
//! open and those among them that were deleted while still open (from `/proc/<pid>/fd`), and the
//! POSIX, flock and lease locks with the processes blocked on them (from `/proc/locks`).

use std::collections::HashMap;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Descriptors examined (one readlink each, and one stat for those naming a path) per process and
/// per scan. A process with more descriptors than its share is counted from the directory
/// listing past that point; see `estimate`.
const PER_PROCESS: usize = 4096;
const PER_SCAN: usize = 65536;

/// The regular files one process holds open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Files {
    pub open: u32,
    /// Files whose last link is gone while this process still holds them open: their disk space
    /// is not returned until they are closed. Counted only among examined descriptors.
    pub deleted: u32,
    pub deleted_bytes: u64,
}

/// What one descriptor refers to, as far as open files are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Descriptor {
    /// A regular file with at least one name.
    File,
    /// A regular file with no name left, and its size in bytes.
    Deleted(u64),
    /// A socket, pipe, device, directory, memfd, anonymous inode or a descriptor that closed.
    Other,
}

/// Classifies `/proc/<pid>/fd/<n>` by its link target and, for paths, by a stat through the link
/// (which reaches the open file even after it was deleted).
fn classify(link: &Path) -> Descriptor {
    let Ok(target) = fs::read_link(link) else {
        return Descriptor::Other;
    };
    let target = target.as_os_str().as_bytes();
    // memfd targets look like deleted files ("/memfd:name (deleted)") but were never on disk.
    if !target.starts_with(b"/") || target.starts_with(b"/memfd:") {
        return Descriptor::Other;
    }
    let Ok(metadata) = fs::metadata(link) else {
        return Descriptor::Other;
    };
    if !metadata.is_file() {
        Descriptor::Other
    } else if target.ends_with(b" (deleted)") && metadata.nlink() == 0 {
        // The suffix alone could be part of a real name; a link count of 0 confirms it.
        Descriptor::Deleted(metadata.len())
    } else {
        Descriptor::File
    }
}

/// The open-file count of a table of `examined + beyond` descriptors of which `examined` were
/// looked at and `files` were regular files: the rest are assumed to hold files in the same
/// proportion. This is the only approximation; deleted files are never extrapolated.
fn estimate(files: u32, examined: usize, beyond: usize) -> u32 {
    if examined == 0 {
        return 0;
    }
    let extra = beyond as u64 * files as u64 / examined as u64;
    files.saturating_add(u32::try_from(extra).unwrap_or(u32::MAX))
}

/// The open files listed in a descriptor directory such as `/proc/<pid>/fd`, examining at most
/// `budget` descriptors, and how many it examined. None when the directory cannot be read: other
/// users' processes without privileges, or a process that has exited.
fn scan_table(directory: &Path, budget: usize) -> Option<(Files, usize)> {
    let entries = fs::read_dir(directory).ok()?;
    let mut files = Files::default();
    let (mut examined, mut beyond) = (0, 0);
    for entry in entries.flatten() {
        if examined == budget {
            beyond += 1;
            continue;
        }
        examined += 1;
        match classify(&entry.path()) {
            Descriptor::File => files.open += 1,
            Descriptor::Deleted(size) => {
                files.open += 1;
                files.deleted += 1;
                files.deleted_bytes = files.deleted_bytes.saturating_add(size);
            }
            Descriptor::Other => {}
        }
    }
    files.open = estimate(files.open, examined, beyond);
    Some((files, examined))
}

/// The open-file scan across processes, which resumes where the previous scan ran out of budget.
#[derive(Default)]
pub struct FileScan {
    /// The last pid examined, so the next scan starts after it.
    after: u32,
    /// The last result for each pid: None for an unreadable table.
    known: HashMap<u32, Option<Files>>,
}

impl FileScan {
    /// Open files per pid; None for a table that could not be read. A pid that the scan budget
    /// did not reach this time keeps its previous result while it exists (a pid reused within
    /// those few seconds briefly shows its predecessor's files).
    pub fn sample(&mut self) -> HashMap<u32, Option<Files>> {
        let Ok(entries) = fs::read_dir("/proc") else {
            self.known.clear();
            return HashMap::new();
        };
        let mut pids: Vec<u32> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect();
        pids.sort_unstable();
        let start = pids.partition_point(|&pid| pid <= self.after);
        let mut budget = PER_SCAN;
        let mut found = HashMap::with_capacity(pids.len());
        for &pid in pids[start..].iter().chain(&pids[..start]) {
            if budget == 0 {
                break;
            }
            let table = format!("/proc/{pid}/fd");
            let result = scan_table(Path::new(&table), PER_PROCESS.min(budget));
            budget -= result.map_or(0, |(_, examined)| examined);
            found.insert(pid, result.map(|(files, _)| files));
            self.after = pid;
        }
        for pid in pids {
            if let Some(&previous) = self.known.get(&pid) {
                found.entry(pid).or_insert(previous);
            }
        }
        self.known = found.clone();
        found
    }
}

/// File locks from `/proc/locks`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Locks {
    /// Locks and leases held, per pid.
    pub held: HashMap<u32, u32>,
    /// Pids blocked on a lock, with the pid holding the lock that blocks them.
    pub blocked: HashMap<u32, u32>,
    /// Open file description locks: they belong to an open file, not a process, and report pid
    /// -1, so no process can be named as their holder.
    pub unattributed: u32,
}

/// Parses `/proc/locks`. Each lock is one line, such as
/// `1: POSIX  ADVISORY  WRITE 1234 08:01:131 0 EOF`, followed by the requests it blocks, marked
/// with `->` (indented further for a request blocked behind another waiter). Every waiter is
/// mapped to the holder of the lock line above it. The pid is the fourth field after the marks
/// for every kind (POSIX, FLOCK, OFDLCK, LEASE, DELEG).
pub fn parse_locks(text: &str) -> Locks {
    let mut locks = Locks::default();
    let mut holder: Option<u32> = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace().skip(1).peekable();
        let mut waiting = false;
        while fields.next_if_eq(&"->").is_some() {
            waiting = true;
        }
        let Some(number) = fields.nth(3).and_then(|pid| pid.parse::<i64>().ok()) else {
            continue;
        };
        // OFD locks report -1; 0 marks a holder outside this pid namespace.
        let pid = u32::try_from(number).ok().filter(|&pid| pid > 0);
        if waiting {
            if let (Some(waiter), Some(holder)) = (pid, holder) {
                locks.blocked.entry(waiter).or_insert(holder);
            }
            continue;
        }
        holder = pid;
        match pid {
            Some(pid) => *locks.held.entry(pid).or_default() += 1,
            None if number == -1 => locks.unattributed += 1,
            None => {}
        }
    }
    locks
}

/// The locks in `/proc/locks`, which every user may read; None when it is missing.
pub fn locks() -> Option<Locks> {
    fs::read_to_string("/proc/locks")
        .ok()
        .map(|text| parse_locks(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    #[test]
    fn proc_locks_map_blocked_waiters_to_their_holder_and_skip_ofd_locks() {
        let text = "\
1: POSIX  ADVISORY  WRITE 1200 08:01:1311 0 EOF
1: -> POSIX  ADVISORY  WRITE 1300 08:01:1311 0 EOF
1:  -> POSIX  ADVISORY  WRITE 1301 08:01:1311 0 EOF
2: FLOCK  ADVISORY  WRITE 1200 00:1a:77 0 EOF
3: OFDLCK ADVISORY  READ  -1 00:06:9999 0 EOF
3: -> FLOCK  ADVISORY  WRITE 1400 00:06:9999 0 EOF
4: LEASE  ACTIVE    READ  1500 fe:00:42 0 EOF
4: -> LEASE  BREAKER  READ  1501 fe:00:42 0 EOF
5: DELEG  ACTIVE    READ  1600 fe:00:43 0 EOF
6: FLOCK  ADVISORY  WRITE 0 00:1a:78 0 EOF
garbage
";
        let locks = parse_locks(text);
        assert_eq!(locks.held, HashMap::from([(1200, 2), (1500, 1), (1600, 1)]));
        // The nested waiter is blocked behind 1300 but waits for the lock 1200 holds.
        assert_eq!(
            locks.blocked,
            HashMap::from([(1300, 1200), (1301, 1200), (1501, 1500)])
        );
        assert_eq!(locks.unattributed, 1, "the OFD lock has no pid");
        assert!(
            !locks.blocked.contains_key(&1400),
            "a waiter on an OFD lock has no holder to name"
        );
        assert_eq!(parse_locks(""), Locks::default());
    }

    #[test]
    fn a_deleted_file_held_open_is_counted_with_its_size() {
        let directory = std::env::temp_dir();
        let path = directory.join(format!("isotop-deleted-{}", std::process::id()));
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(&[7_u8; 12345]).unwrap();
        file.flush().unwrap();
        let link = format!("/proc/self/fd/{}", file.as_raw_fd());
        assert_eq!(classify(Path::new(&link)), Descriptor::File);
        // Other tests in this process open and close files concurrently, so the table's totals
        // are compared until one pair of scans brackets only this change.
        let mut seen = false;
        let mut before = scan_table(Path::new("/proc/self/fd"), usize::MAX)
            .unwrap()
            .0;
        fs::remove_file(&path).unwrap();
        assert_eq!(classify(Path::new(&link)), Descriptor::Deleted(12345));
        for _ in 0..20 {
            let after = scan_table(Path::new("/proc/self/fd"), usize::MAX)
                .unwrap()
                .0;
            if after.deleted == before.deleted + 1
                && after.deleted_bytes == before.deleted_bytes + 12345
            {
                seen = true;
                break;
            }
            // Retake the baseline without the file, then remove it again.
            drop(file);
            before = scan_table(Path::new("/proc/self/fd"), usize::MAX)
                .unwrap()
                .0;
            file = fs::File::create(&path).unwrap();
            file.write_all(&[7_u8; 12345]).unwrap();
            fs::remove_file(&path).unwrap();
        }
        assert!(seen, "one more deleted file of 12345 bytes");
    }

    #[test]
    fn a_memfd_is_neither_an_open_file_nor_a_deleted_one() {
        // SAFETY: the name is a valid NUL-terminated string and the flags are plain.
        let raw = unsafe { libc::memfd_create(c"isotop-test".as_ptr(), 0) };
        assert!(raw >= 0, "memfd_create failed");
        // SAFETY: raw is a descriptor just returned by memfd_create and owned only here.
        let memfd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut file = fs::File::from(memfd);
        file.write_all(&[1_u8; 4096]).unwrap();
        let link = format!("/proc/self/fd/{}", file.as_raw_fd());
        let target = fs::read_link(&link).unwrap();
        assert!(target.as_os_str().as_bytes().starts_with(b"/memfd:"));
        assert_eq!(classify(Path::new(&link)), Descriptor::Other);
    }

    #[test]
    fn devices_and_directories_are_not_open_files() {
        let null = fs::File::open("/dev/null").unwrap();
        let root = fs::File::open("/").unwrap();
        for file in [&null, &root] {
            let link = format!("/proc/self/fd/{}", file.as_raw_fd());
            assert_eq!(classify(Path::new(&link)), Descriptor::Other);
        }
    }

    #[test]
    fn a_table_past_the_bound_is_estimated_and_never_invents_deleted_files() {
        assert_eq!(estimate(30, 100, 0), 30);
        // 30 of the first 100 descriptors were files, so 30 % of the 900 beyond are counted.
        assert_eq!(estimate(30, 100, 900), 300);
        assert_eq!(estimate(0, 0, 50), 0);
        let (files, examined) = scan_table(Path::new("/proc/self/fd"), 1).unwrap();
        assert_eq!(examined, 1);
        assert!(files.deleted <= 1);
        assert!(scan_table(Path::new("/proc/self/no-such-table"), 10).is_none());
    }
}
