//! Where a process that holds pipes is blocked in the kernel, from `/proc/<pid>/wchan`, read on
//! the background thread while `Source::WaitChannel` is demanded. The file holds the name of the
//! kernel function the task sleeps in, or `0`. Only whether that function is the one that writes
//! to or reads from a pipe is kept (`Wait`); the symbol itself never leaves this file.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::platform::Wait;

/// The most processes one `WaitScan::sample` pass reads the wait channel of.
const WAIT_PROCESSES: usize = 256;

/// The function names a task blocked on a pipe sleeps in. Kernels differ: the write and read
/// paths were `pipe_write` and `pipe_read` for a long time, the sleep itself moved into
/// `pipe_wait_readable` and `pipe_wait_writable` (Linux 5.5 and later, to verify on a machine
/// with `CONFIG_KALLSYMS`), and Linux 6.12 renamed the read and write paths `anon_pipe_read` and
/// `anon_pipe_write`. A name not listed reads as `Wait::Other`, which is the safe way to be wrong.
const WRITING: &[&str] = &["pipe_write", "anon_pipe_write", "pipe_wait_writable"];
const READING: &[&str] = &["pipe_read", "anon_pipe_read", "pipe_wait_readable"];

/// What the text of a `wchan` file says: `0` and an empty file are `Unreadable` (the kernel prints
/// `0` for a running task and for one this user may not inspect), the pipe functions are
/// `PipeWrite` and `PipeRead`, anything else `Other`.
pub fn parse(text: &str) -> Wait {
    match text.trim() {
        "" | "0" => Wait::Unreadable,
        name if WRITING.contains(&name) => Wait::PipeWrite,
        name if READING.contains(&name) => Wait::PipeRead,
        _ => Wait::Other,
    }
}

/// The wait channel scan, which resumes after the last pid it read so a machine with more than
/// `WAIT_PROCESSES` holders of pipes is covered over several passes.
pub struct WaitScan {
    root: PathBuf,
    after: u32,
}

impl Default for WaitScan {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/proc"),
            after: 0,
        }
    }
}

impl WaitScan {
    pub fn new() -> Self {
        Self::default()
    }

    /// The wait channel of at most `WAIT_PROCESSES` of `pids`, in pid order from after the last
    /// pid the previous pass read. A pid that has exited is left out and one whose file cannot be
    /// read is `Unreadable`. None when the kernel has no `wchan` at all (it is built without
    /// `CONFIG_KALLSYMS`), which this process finds out by reading its own.
    pub fn sample(&mut self, pids: &[u32]) -> Option<HashMap<u32, Wait>> {
        if matches!(
            fs::read_to_string(self.root.join("self").join("wchan")),
            Err(error) if error.kind() == ErrorKind::NotFound
        ) {
            return None;
        }
        let mut pids = pids.to_vec();
        pids.sort_unstable();
        pids.dedup();
        let start = pids.partition_point(|&pid| pid <= self.after);
        let mut found = HashMap::new();
        for &pid in pids[start..].iter().chain(&pids[..start]) {
            if found.len() == WAIT_PROCESSES {
                break;
            }
            match fs::read_to_string(self.root.join(pid.to_string()).join("wchan")) {
                Ok(text) => {
                    found.insert(pid, parse(&text));
                }
                // Exited since the pipe scan listed it: nothing to say, not even a gap.
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => {
                    found.insert(pid, Wait::Unreadable);
                }
            }
            self.after = pid;
        }
        Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake `/proc` in a directory of its own, removed when dropped.
    struct Proc(PathBuf);

    impl Proc {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("isotop-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("self")).unwrap();
            fs::write(root.join("self").join("wchan"), "0").unwrap();
            Proc(root)
        }

        fn wchan(&self, pid: u32, text: &str) {
            fs::create_dir_all(self.0.join(pid.to_string())).unwrap();
            fs::write(self.0.join(pid.to_string()).join("wchan"), text).unwrap();
        }

        fn scan(&self) -> WaitScan {
            WaitScan {
                root: self.0.clone(),
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
    fn wchan_zero_parses_as_unreadable() {
        for text in ["0", "0\n", "", "  \n"] {
            assert_eq!(parse(text), Wait::Unreadable, "{text:?}");
        }
    }

    #[test]
    fn the_pipe_functions_parse_as_a_blocked_writer_and_a_blocked_reader() {
        for name in ["pipe_write", "anon_pipe_write", "pipe_wait_writable"] {
            assert_eq!(parse(name), Wait::PipeWrite, "{name}");
        }
        for name in ["pipe_read", "anon_pipe_read", "pipe_wait_readable"] {
            assert_eq!(parse(&format!("{name}\n")), Wait::PipeRead, "{name}");
        }
        for name in [
            "do_epoll_wait",
            "futex_wait_queue",
            "wait_for_partner",
            "xpipe_write",
        ] {
            assert_eq!(parse(name), Wait::Other, "{name}");
        }
    }

    #[test]
    fn a_pass_reads_each_pid_and_leaves_out_one_that_has_exited() {
        let proc = Proc::new("wchan-read");
        proc.wchan(5, "pipe_write\n");
        proc.wchan(9, "pipe_read\n");
        proc.wchan(12, "0");
        let found = proc.scan().sample(&[12, 5, 9, 77]).unwrap();
        assert_eq!(
            found,
            HashMap::from([
                (5, Wait::PipeWrite),
                (9, Wait::PipeRead),
                (12, Wait::Unreadable)
            ]),
            "pid 77 has no directory"
        );
    }

    #[test]
    fn a_pass_stops_at_the_cap_and_the_next_one_resumes_after_it() {
        let proc = Proc::new("wchan-cap");
        let pids: Vec<u32> = (1..=WAIT_PROCESSES as u32 + 40).collect();
        for &pid in &pids {
            proc.wchan(pid, "do_poll");
        }
        let mut scan = proc.scan();
        let first = scan.sample(&pids).unwrap();
        assert_eq!(first.len(), WAIT_PROCESSES);
        assert!(first.contains_key(&1) && !first.contains_key(&(WAIT_PROCESSES as u32 + 1)));
        let second = scan.sample(&pids).unwrap();
        assert_eq!(second.len(), WAIT_PROCESSES);
        assert!(
            second.contains_key(&(WAIT_PROCESSES as u32 + 40)) && second.contains_key(&1),
            "the pass wraps round to the pids the first one did not reach"
        );
        assert!(
            (WAIT_PROCESSES as u32 + 1..=WAIT_PROCESSES as u32 + 40)
                .all(|pid| second.contains_key(&pid)),
            "and starts with them"
        );
    }

    #[test]
    fn a_kernel_without_wchan_makes_the_whole_source_missing() {
        let proc = Proc::new("wchan-none");
        fs::remove_file(proc.0.join("self").join("wchan")).unwrap();
        assert_eq!(proc.scan().sample(&[1, 2]), None);
    }
}
