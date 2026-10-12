//! The pipes and FIFOs each process holds, read on the background thread while `Source::Pipes` is
//! demanded: anonymous pipes from their `pipe:[inode]` descriptor links, named pipes from a stat
//! of the descriptor, and the direction of each from its fdinfo (`fdinfo.rs`). Linux exposes no
//! count of what is buffered in a pipe another process holds, so `PipeEnd::buffered` is None.

use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::fdinfo::parse_fdinfo;
use super::files::{attributes, device};
use crate::platform::{PipeEnd, PipeId, PipeSample};

/// How much one pass may read. A descriptor is classified by a readlink, plus a stat for a path;
/// only the pipes and FIFOs found cost an fdinfo read, which is the expensive step: an open, a
/// read and a close.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    /// fdinfo reads per pass.
    reads: usize,
    /// Directory entries listed per process; the listing stops there.
    listed: usize,
    /// Directory entries listed per pass. A process is only scanned while a whole `listed` share
    /// remains, so none is judged from a sliver of its table.
    scan: usize,
}

const BOUNDS: Bounds = Bounds {
    reads: 4096,
    listed: 16384,
    scan: 65536,
};

/// A descriptor that is a pipe or FIFO, and the name of its file in the fdinfo directory.
struct Candidate {
    descriptor: std::ffi::OsString,
    id: PipeId,
}

/// What `/proc/<pid>/fd/<n>` is, when it is a pipe: an anonymous pipe reads `pipe:[inode]`, and a
/// named one is a path whose file is of type FIFO. The link is followed to the open file, so a
/// FIFO unlinked while open is still found.
fn classify(link: &Path) -> Option<PipeId> {
    let target = fs::read_link(link).ok()?;
    let target = target.as_os_str().as_bytes();
    if let Some(inode) = target
        .strip_prefix(b"pipe:[")
        .and_then(|rest| rest.strip_suffix(b"]"))
    {
        return std::str::from_utf8(inode)
            .ok()?
            .parse()
            .ok()
            .map(PipeId::Anonymous);
    }
    // A socket, an anonymous inode and a memfd are not paths.
    if !target.starts_with(b"/") || target.starts_with(b"/memfd:") {
        return None;
    }
    let status = attributes(link)?;
    (u32::from(status.stx_mode) & libc::S_IFMT == libc::S_IFIFO).then_some(PipeId::Fifo {
        device: device(&status),
        inode: status.stx_ino,
    })
}

/// The pipes and FIFOs in a descriptor directory such as `/proc/<pid>/fd`, and how many entries
/// were listed. An error when the directory cannot be read: other users' processes without
/// privileges, or NotFound for a process that has exited.
fn candidates(directory: &Path, listed_at_most: usize) -> io::Result<(Vec<Candidate>, usize)> {
    let mut found = Vec::new();
    let mut listed = 0;
    for entry in fs::read_dir(directory)?.flatten() {
        if listed == listed_at_most {
            break;
        }
        listed += 1;
        if let Some(id) = classify(&entry.path()) {
            found.push(Candidate {
                descriptor: entry.file_name(),
                id,
            });
        }
    }
    Ok((found, listed))
}

/// The pipe and FIFO scan across processes, which resumes where the previous pass ran out of
/// budget.
pub struct PipeScan {
    root: PathBuf,
    bounds: Bounds,
    /// The last pid scanned, so the next pass starts after it.
    after: u32,
}

impl Default for PipeScan {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/proc"),
            bounds: BOUNDS,
            after: 0,
        }
    }
}

impl PipeScan {
    pub fn new() -> Self {
        Self::default()
    }

    /// The pipe and FIFO descriptors of the processes this pass reached, in pid order from where
    /// the last pass stopped. None when `/proc` cannot be listed.
    ///
    /// The pass reads at most `Bounds::reads` fdinfo files. It stops before a process whose pipes
    /// would exceed what is left and marks the result partial, and the next pass starts with that
    /// process, which then gets the whole budget even if it holds more pipes than that: only its
    /// first descriptors are read, and the pass is partial again. Every pass therefore reaches at
    /// least one new process.
    pub fn sample(&mut self) -> Option<PipeSample> {
        let mut pids: Vec<u32> = fs::read_dir(&self.root)
            .ok()?
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect();
        pids.sort_unstable();
        let start = pids.partition_point(|&pid| pid <= self.after);
        let (mut listing, mut reads) = (self.bounds.scan, self.bounds.reads);
        let mut sample = PipeSample::default();
        let mut first = true;
        for &pid in pids[start..].iter().chain(&pids[..start]) {
            if listing < self.bounds.listed {
                sample.partial = true;
                break;
            }
            let process = self.root.join(pid.to_string());
            // Another user's process, or one that has exited: nothing to read, and no ends
            // recorded, so an earlier reading of the pid is kept.
            let Ok((mut found, listed)) = candidates(&process.join("fd"), self.bounds.listed)
            else {
                continue;
            };
            listing -= listed;
            if found.len() > reads {
                if !first {
                    sample.partial = true;
                    break;
                }
                found.truncate(reads);
                sample.partial = true;
            }
            reads -= found.len();
            self.after = pid;
            first = false;
            let ends = found
                .into_iter()
                .filter_map(|candidate| {
                    let text =
                        fs::read_to_string(process.join("fdinfo").join(candidate.descriptor))
                            .ok()?;
                    Some(PipeEnd {
                        id: candidate.id,
                        access: parse_fdinfo(&text)?.access()?,
                        buffered: None,
                    })
                })
                .collect();
            sample.ends.insert(pid, ends);
        }
        Some(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::AccessMode;
    use std::collections::HashMap;
    use std::os::unix::fs::symlink;

    /// A fake `/proc` in a directory of its own, removed when dropped.
    struct Proc(PathBuf);

    impl Proc {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("isotop-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Proc(root)
        }

        /// Gives `pid` a descriptor `fd` that links to `target`, with the fdinfo `flags`.
        fn descriptor(&self, pid: u32, fd: u32, target: &str, flags: &str) {
            let process = self.0.join(pid.to_string());
            fs::create_dir_all(process.join("fd")).unwrap();
            fs::create_dir_all(process.join("fdinfo")).unwrap();
            symlink(target, process.join("fd").join(fd.to_string())).unwrap();
            fs::write(
                process.join("fdinfo").join(fd.to_string()),
                format!("pos:\t0\nflags:\t{flags}\nmnt_id:\t15\n"),
            )
            .unwrap();
        }

        fn scan(&self, reads: usize) -> PipeScan {
            PipeScan {
                root: self.0.clone(),
                bounds: Bounds { reads, ..BOUNDS },
                after: 0,
            }
        }
    }

    impl Drop for Proc {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_scan_reads_pipe_ends_with_their_direction() {
        let proc = Proc::new("pipes-direction");
        proc.descriptor(10, 1, "pipe:[501]", "01");
        proc.descriptor(11, 0, "pipe:[501]", "02000000");
        proc.descriptor(11, 3, "socket:[77]", "02");
        proc.descriptor(11, 4, "anon_inode:[eventfd]", "02");
        let sample = proc.scan(BOUNDS.reads).sample().unwrap();
        let end = |access| PipeEnd {
            id: PipeId::Anonymous(501),
            access,
            buffered: None,
        };
        assert_eq!(
            sample.ends,
            HashMap::from([
                (10, vec![end(AccessMode::Write)]),
                (11, vec![end(AccessMode::Read)]),
            ])
        );
        assert!(!sample.partial);
    }

    #[test]
    fn a_named_pipe_is_found_by_the_type_of_its_file() {
        // A real FIFO, reached through a link as /proc/<pid>/fd/<n> reaches an open file.
        let proc = Proc::new("pipes-fifo");
        let fifo = proc.0.join("queue");
        let path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: path is a NUL-terminated string that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let regular = proc.0.join("plain");
        fs::write(&regular, b"").unwrap();
        proc.descriptor(20, 5, fifo.to_str().unwrap(), "02");
        proc.descriptor(20, 6, regular.to_str().unwrap(), "02");
        let sample = proc.scan(BOUNDS.reads).sample().unwrap();
        let status = attributes(&fifo).unwrap();
        assert_eq!(
            sample.ends[&20],
            vec![PipeEnd {
                id: PipeId::Fifo {
                    device: device(&status),
                    inode: status.stx_ino,
                },
                access: AccessMode::Both,
                buffered: None,
            }],
            "the regular file is not a pipe"
        );
    }

    #[test]
    fn fdinfo_reads_stop_at_the_cap_and_mark_the_result_partial() {
        let proc = Proc::new("pipes-cap");
        for pid in [10, 11, 12] {
            proc.descriptor(pid, 0, &format!("pipe:[{pid}]"), "00");
            proc.descriptor(pid, 1, &format!("pipe:[{pid}]"), "01");
        }
        // Room for three of the six reads: the pass takes the whole of one process, stops before
        // the next, which does not fit, and says so.
        let mut scan = proc.scan(3);
        let first = scan.sample().unwrap();
        assert_eq!(first.ends.keys().copied().collect::<Vec<_>>(), [10]);
        assert!(first.partial);
        // The next pass resumes after it.
        let second = scan.sample().unwrap();
        assert_eq!(second.ends.keys().copied().collect::<Vec<_>>(), [11]);
        assert!(second.partial);
        // Room for all six: every process, and nothing is missing.
        let all = proc.scan(6).sample().unwrap();
        assert_eq!(all.ends.len(), 3);
        assert!(!all.partial);
        // One process with more pipes than the cap still gets its turn, truncated.
        let big = Proc::new("pipes-cap-big");
        for fd in 0..5 {
            big.descriptor(30, fd, &format!("pipe:[{fd}]"), "00");
        }
        let sample = big.scan(2).sample().unwrap();
        assert_eq!(sample.ends[&30].len(), 2);
        assert!(sample.partial);
    }
}
