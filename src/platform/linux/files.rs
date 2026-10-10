//! Open files and file locks, read on the background thread: the regular files each process holds
//! open and those among them that were deleted while still open (from `/proc/<pid>/fd`), and the
//! POSIX, flock and lease locks with the processes blocked on them (from `/proc/locks`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::CString;
use std::fs;
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::platform::{DeletedFile, Files, Locks, UNNAMED};

/// How much one scan may read. Listing a descriptor directory costs about a microsecond per
/// entry warm and several cold; examining a descriptor adds a readlink and a stat.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    /// Descriptors examined per process; past them the open count is estimated.
    examined: usize,
    /// Directory entries listed per process; the listing stops there.
    listed: usize,
    /// Directory entries listed per scan. A process is only scanned while a whole `listed`
    /// share remains, so none is judged from a sliver of its table.
    scan: usize,
}

const BOUNDS: Bounds = Bounds {
    examined: 4096,
    listed: 16384,
    scan: 65536,
};

/// What one descriptor refers to, as far as open files are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Descriptor {
    /// A regular file with at least one name, by device and inode.
    File(u64, u64),
    /// A regular file with no name left.
    Deleted(DeletedFile),
    /// A socket, pipe, device, directory, memfd, anonymous inode or a descriptor that closed.
    Other,
}

/// Type, link count, inode and size of the file behind `link`, from the attributes the kernel
/// already holds. AT_STATX_DONT_SYNC keeps network and FUSE filesystems from asking their server,
/// so a hung mount cannot block the background thread here.
fn attributes(link: &Path) -> Option<libc::statx> {
    let path = CString::new(link.as_os_str().as_bytes()).ok()?;
    let mut status = MaybeUninit::<libc::statx>::uninit();
    let mask = libc::STATX_TYPE | libc::STATX_NLINK | libc::STATX_INO | libc::STATX_SIZE;
    // SAFETY: path is a NUL-terminated string that outlives the call, and status points to
    // writable memory the size of a statx, which the kernel fills when it returns 0.
    let result = unsafe {
        libc::statx(
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::AT_STATX_DONT_SYNC,
            mask,
            status.as_mut_ptr(),
        )
    };
    // SAFETY: a return of 0 means the kernel wrote the whole structure.
    (result == 0).then(|| unsafe { status.assume_init() })
}

/// Classifies `/proc/<pid>/fd/<n>` by its link target and, for paths, by the attributes found
/// through the link (which reaches the open file even after it was deleted).
fn classify(link: &Path) -> Descriptor {
    let Ok(target) = fs::read_link(link) else {
        return Descriptor::Other;
    };
    let target = target.as_os_str().as_bytes();
    // memfd targets look like deleted files ("/memfd:name (deleted)") but were never on disk.
    if !target.starts_with(b"/") || target.starts_with(b"/memfd:") {
        return Descriptor::Other;
    }
    let Some(status) = attributes(link) else {
        return Descriptor::Other;
    };
    if u32::from(status.stx_mode) & libc::S_IFMT != libc::S_IFREG {
        return Descriptor::Other;
    }
    let device = (u64::from(status.stx_dev_major) << 32) | u64::from(status.stx_dev_minor);
    // The suffix alone could be part of a real name; a link count of 0 confirms it.
    if target.ends_with(b" (deleted)") && status.stx_nlink == 0 {
        Descriptor::Deleted(DeletedFile {
            device,
            inode: status.stx_ino,
            size: status.stx_size,
        })
    } else {
        Descriptor::File(device, status.stx_ino)
    }
}

/// The open-file count of a table of `examined + beyond` descriptors of which `examined` were
/// looked at and `files` were distinct regular files: the rest are assumed to hold files in the
/// same proportion. Deleted files are never extrapolated.
fn estimate(files: u32, examined: usize, beyond: usize) -> u32 {
    if examined == 0 {
        return 0;
    }
    let extra = beyond as u64 * files as u64 / examined as u64;
    files.saturating_add(u32::try_from(extra).unwrap_or(u32::MAX))
}

/// Counts examined descriptors, each file once however many descriptors refer to it.
fn tally(descriptors: impl IntoIterator<Item = Descriptor>) -> Files {
    let mut open = HashSet::new();
    let mut deleted = BTreeSet::new();
    for descriptor in descriptors {
        match descriptor {
            Descriptor::File(device, inode) => {
                open.insert((device, inode));
            }
            Descriptor::Deleted(file) => {
                open.insert((file.device, file.inode));
                deleted.insert(file);
            }
            Descriptor::Other => {}
        }
    }
    Files {
        open: u32::try_from(open.len()).unwrap_or(u32::MAX),
        deleted: deleted.into_iter().collect(),
        partial: false,
    }
}

/// The open files listed in a descriptor directory such as `/proc/<pid>/fd`, and how many
/// entries were listed. An error when the directory cannot be read: other users' processes
/// without privileges, or NotFound for a process that has exited.
fn scan_table(directory: &Path, bounds: Bounds) -> io::Result<(Files, usize)> {
    let entries = fs::read_dir(directory)?;
    let mut examined = Vec::new();
    let mut listed = 0;
    let mut truncated = false;
    for entry in entries.flatten() {
        if listed == bounds.listed {
            truncated = true;
            break;
        }
        listed += 1;
        if examined.len() < bounds.examined {
            examined.push(classify(&entry.path()));
        }
    }
    let count = examined.len();
    let mut files = tally(examined);
    files.open = estimate(files.open, count, listed - count);
    files.partial = truncated || listed > count;
    Ok((files, listed))
}

/// The open-file scan across processes, which resumes where the previous scan ran out of budget.
pub struct FileScan {
    root: PathBuf,
    bounds: Bounds,
    /// The last pid scanned, so the next scan starts after it.
    after: u32,
    /// The last result for each pid: None for an unreadable table.
    known: HashMap<u32, Option<Files>>,
}

impl Default for FileScan {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/proc"),
            bounds: BOUNDS,
            after: 0,
            known: HashMap::new(),
        }
    }
}

impl FileScan {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open files per pid; None for a table that could not be read. A pid the scan has not
    /// reached yet, this time or ever, is absent unless it keeps a previous result (a pid
    /// reused within those few seconds briefly shows its predecessor's files); so is one that
    /// exited during the scan.
    pub fn sample(&mut self) -> HashMap<u32, Option<Files>> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            self.known.clear();
            return HashMap::new();
        };
        let mut pids: Vec<u32> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect();
        pids.sort_unstable();
        let start = pids.partition_point(|&pid| pid <= self.after);
        let mut budget = self.bounds.scan;
        let mut found = HashMap::with_capacity(pids.len());
        for &pid in pids[start..].iter().chain(&pids[..start]) {
            if budget < self.bounds.listed {
                break;
            }
            let table = self.root.join(pid.to_string()).join("fd");
            self.after = pid;
            match scan_table(&table, self.bounds) {
                Ok((files, listed)) => {
                    budget -= listed;
                    found.insert(pid, Some(files));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => {
                    found.insert(pid, None);
                }
            }
        }
        for pid in pids {
            if let Some(previous) = self.known.get(&pid) {
                found.entry(pid).or_insert_with(|| previous.clone());
            }
        }
        self.known = found.clone();
        found
    }
}

/// Parses `/proc/locks`. Each lock is one line, such as
/// `1: POSIX  ADVISORY  WRITE 1234 08:01:131 0 EOF`, followed by the requests it blocks, marked
/// with `->` (indented further for a request blocked behind another waiter). Every waiter is
/// mapped to the holder of the lock line above it. The pid is the fourth field after the marks
/// for every kind (POSIX, FLOCK, OFDLCK, LEASE, DELEG).
fn parse_locks(text: &str) -> Locks {
    let mut locks = Locks::default();
    let mut holder = UNNAMED;
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
            if let Some(waiter) = pid {
                locks.blocked.entry(waiter).or_insert(holder);
            }
            continue;
        }
        holder = pid.unwrap_or(UNNAMED);
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

    const ALL: Bounds = Bounds {
        examined: usize::MAX,
        listed: usize::MAX,
        scan: usize::MAX,
    };

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
6: -> FLOCK  ADVISORY  WRITE 1700 00:1a:78 0 EOF
garbage
";
        let locks = parse_locks(text);
        assert_eq!(locks.held, HashMap::from([(1200, 2), (1500, 1), (1600, 1)]));
        // The nested waiter is blocked behind 1300 but waits for the lock 1200 holds. Waiters on
        // an OFD lock or on a holder in another pid namespace are blocked all the same, on a
        // holder that cannot be named.
        assert_eq!(
            locks.blocked,
            HashMap::from([
                (1300, 1200),
                (1301, 1200),
                (1400, UNNAMED),
                (1501, 1500),
                (1700, UNNAMED)
            ])
        );
        assert_eq!(locks.unattributed, 1, "the OFD lock has no pid");
        assert_eq!(parse_locks(""), Locks::default());
    }

    #[test]
    fn a_file_held_through_several_descriptors_counts_once() {
        let rotated = DeletedFile {
            device: 2049,
            inode: 77,
            size: 1 << 30,
        };
        let files = tally([
            Descriptor::File(2049, 10),
            Descriptor::File(2049, 10),
            Descriptor::File(2050, 10),
            Descriptor::Deleted(rotated),
            Descriptor::Deleted(rotated),
            Descriptor::Other,
        ]);
        assert_eq!(files.open, 3);
        assert_eq!(files.deleted, vec![rotated]);
        assert_eq!(files.deleted_bytes(), 1 << 30);
    }

    #[test]
    fn a_deleted_file_held_open_is_counted_once_with_its_size() {
        let directory = std::env::temp_dir();
        let path = directory.join(format!("isotop-deleted-{}", std::process::id()));
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(&[7_u8; 12345]).unwrap();
        file.flush().unwrap();
        let link = format!("/proc/self/fd/{}", file.as_raw_fd());
        assert!(matches!(classify(Path::new(&link)), Descriptor::File(_, _)));
        let scan = || scan_table(Path::new("/proc/self/fd"), ALL).unwrap().0;
        // Other tests in this process open and close files concurrently, so the table's totals
        // are compared until one pair of scans brackets only this change.
        let mut seen = false;
        let mut before = scan();
        fs::remove_file(&path).unwrap();
        // A second descriptor on the same file, as `cmd >log 2>&1` leaves.
        let mut twin = file.try_clone().unwrap();
        let Descriptor::Deleted(deleted) = classify(Path::new(&link)) else {
            panic!("not deleted");
        };
        assert_eq!(deleted.size, 12345);
        for _ in 0..20 {
            let after = scan();
            if after.deleted.len() == before.deleted.len() + 1
                && after.deleted_bytes() == before.deleted_bytes() + 12345
                && after.deleted.contains(&deleted)
            {
                seen = true;
                break;
            }
            // Retake the baseline without the file, then remove it again.
            drop((file, twin));
            before = scan();
            file = fs::File::create(&path).unwrap();
            file.write_all(&[7_u8; 12345]).unwrap();
            fs::remove_file(&path).unwrap();
            twin = file.try_clone().unwrap();
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
        let bounds = |examined, listed| Bounds {
            examined,
            listed,
            scan: usize::MAX,
        };
        let table = Path::new("/proc/self/fd");
        let (files, listed) = scan_table(table, bounds(1, usize::MAX)).unwrap();
        assert!(listed >= 3, "stdin, stdout and stderr at least");
        assert!(files.partial);
        assert!(files.deleted.len() <= 1);
        // The listing itself stops at its bound, and the result says it is partial.
        let (files, listed) = scan_table(table, bounds(1, 2)).unwrap();
        assert_eq!(listed, 2);
        assert!(files.partial);
        assert!(!scan_table(table, ALL).unwrap().0.partial);
        let missing = scan_table(Path::new("/proc/self/no-such-table"), ALL).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_scan_stops_rather_than_judge_a_table_from_a_sliver_and_resumes_there() {
        let root = std::env::temp_dir().join(format!("isotop-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let file = root.join("held");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, b"x").unwrap();
        for pid in [10, 11, 12] {
            let table = root.join(format!("{pid}/fd"));
            fs::create_dir_all(&table).unwrap();
            for descriptor in 0..3 {
                std::os::unix::fs::symlink(&file, table.join(descriptor.to_string())).unwrap();
            }
        }
        // A process that exited has no table; one whose table cannot be read is unreadable.
        fs::create_dir_all(root.join("13")).unwrap();
        fs::create_dir_all(root.join("14")).unwrap();
        fs::write(root.join("14/fd"), b"").unwrap();
        let mut scan = FileScan {
            root: root.clone(),
            bounds: Bounds {
                examined: 3,
                listed: 3,
                scan: 8,
            },
            ..FileScan::default()
        };
        let first = scan.sample();
        // 10 and 11 list 6 entries; the 2 left are less than a whole share, so 12 waits.
        let expected = Files {
            open: 1,
            deleted: Vec::new(),
            partial: false,
        };
        assert_eq!(first.get(&10), Some(&Some(expected.clone())));
        assert_eq!(first.get(&11), Some(&Some(expected.clone())));
        assert_eq!(first.get(&12), None, "not reached yet");
        let second = scan.sample();
        assert_eq!(second.get(&12), Some(&Some(expected.clone())));
        assert_eq!(second.get(&13), None, "exited");
        assert_eq!(second.get(&14), Some(&None), "unreadable");
        assert_eq!(second.get(&10), Some(&Some(expected)), "kept from before");
        fs::remove_dir_all(&root).unwrap();
    }
}
