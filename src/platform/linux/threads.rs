//! The threads of a process from /proc/<pid>/task/<tid>/stat, which `Source::Threads` demands.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::PathBuf;

use super::{split_stat, timing};
use crate::threads::{Listing, RawThread, Target, ThreadReader};

/// Reads the threads of one process after another, with the buffers it reuses between files.
pub struct Threads {
    root: PathBuf,
    /// USER_HZ, the unit of the CPU times in the stat files.
    hz: f64,
    path: String,
    bytes: Vec<u8>,
}

impl Threads {
    pub fn new() -> Self {
        Self::at("/proc")
    }

    fn at(root: &str) -> Self {
        Threads {
            root: PathBuf::from(root),
            // SAFETY: sysconf takes no pointers and has no preconditions; for a selector it
            // does not know it returns -1, which max(1) clamps. _SC_CLK_TCK is known to every
            // supported Linux.
            hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64,
            path: String::new(),
            bytes: Vec::new(),
        }
    }

    /// The thread ids of `pid`, ascending, with the leader (the id that is the pid) first.
    fn task_ids(&self, pid: u32) -> io::Result<Vec<u32>> {
        let mut ids: Vec<u32> = fs::read_dir(self.root.join(pid.to_string()).join("task"))?
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect();
        ids.sort_unstable();
        if let Some(leader) = ids.iter().position(|&id| id == pid) {
            ids[..=leader].rotate_right(1);
        }
        Ok(ids)
    }

    /// The text of one thread's stat file, in `self.bytes`.
    fn read_stat(&mut self, pid: u32, tid: u32) -> io::Result<()> {
        self.path.clear();
        let _ = write!(self.path, "{pid}/task/{tid}/stat");
        self.bytes.clear();
        File::open(self.root.join(&self.path))?.read_to_end(&mut self.bytes)?;
        Ok(())
    }
}

/// What a thread's stat file says.
#[derive(Debug, PartialEq)]
struct TaskStat {
    name: String,
    state: char,
    /// utime plus stime, in clock ticks.
    ticks: u64,
    /// starttime, in clock ticks after boot.
    start: u64,
}

/// Parses a stat file, which has the same fields as a process's. The name is whatever the
/// thread set (`comm`), raw.
fn parse_task_stat(text: &str) -> Option<TaskStat> {
    let (name, fields) = split_stat(text)?;
    let timing = timing(&fields)?;
    Some(TaskStat {
        name: name.to_owned(),
        state: timing.state,
        ticks: timing.ticks,
        start: timing.start,
    })
}

impl ThreadReader for Threads {
    fn ticks_per_second(&self) -> f64 {
        self.hz
    }

    /// Another user's threads are readable unless /proc hides them (hidepid), which shows as
    /// `Unreadable`. A pid that now names another process is not noticed here: the collector
    /// joins the result to the identity the target had, which the new process does not share.
    fn list(&mut self, target: &Target, limit: usize) -> Listing {
        let pid = target.id.pid;
        let ids = match self.task_ids(pid) {
            Ok(ids) => ids,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Listing::Unreadable;
            }
            Err(_) => return Listing::Gone,
        };
        let complete = ids.len() <= limit;
        let mut threads = Vec::with_capacity(ids.len().min(limit));
        for &tid in ids.iter().take(limit) {
            match self.read_stat(pid, tid) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    return Listing::Unreadable;
                }
                // The thread exited after the directory was read.
                Err(_) => continue,
            }
            let Some(task) = parse_task_stat(&String::from_utf8_lossy(&self.bytes)) else {
                continue;
            };
            threads.push(RawThread {
                tid,
                start: task.start,
                name: task.name,
                ticks: task.ticks,
                state: task.state,
                leader: tid == pid,
            });
        }
        // A live process always has its leader, so none read means it exited meanwhile.
        if threads.is_empty() {
            return Listing::Gone;
        }
        Listing::Found { threads, complete }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Identity;
    use std::sync::{Arc, Barrier};

    fn stat(name: &str, edit: impl FnOnce(&mut Vec<String>)) -> String {
        let mut fields: Vec<String> = vec!["0".into(); 22];
        fields[0] = "R".into();
        edit(&mut fields);
        format!("73 ({name}) {}", fields.join(" "))
    }

    #[test]
    fn task_stat_parses_names_with_spaces_and_parentheses() {
        let text = stat("a tricky ) (name", |fields| {
            fields[11] = "123".into();
            fields[12] = "7".into();
            fields[19] = "999".into();
        });
        assert_eq!(
            parse_task_stat(&text),
            Some(TaskStat {
                name: "a tricky ) (name".into(),
                state: 'R',
                ticks: 130,
                start: 999,
            })
        );
        let raw = stat("w\u{1b}[2J\n", |_| {});
        assert_eq!(parse_task_stat(&raw).unwrap().name, "w\u{1b}[2J\n");
        assert_eq!(parse_task_stat("73 (short) S 1"), None);
        assert_eq!(parse_task_stat("73 no parentheses"), None);
    }

    /// This process, with the start time its leader's stat file gives.
    fn myself() -> Target {
        let pid = std::process::id();
        let text = fs::read_to_string(format!("/proc/{pid}/task/{pid}/stat")).unwrap();
        Target {
            id: Identity {
                pid,
                start: parse_task_stat(&text).unwrap().start,
            },
            name: "isotop".into(),
        }
    }

    #[test]
    fn the_leader_is_the_thread_whose_id_is_the_pid() {
        // Three threads that stay, named, until the end of the test.
        let ready = Arc::new(Barrier::new(4));
        let release = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..3)
            .map(|index| {
                let (ready, release) = (Arc::clone(&ready), Arc::clone(&release));
                std::thread::Builder::new()
                    .name(format!("probe-{index}"))
                    .spawn(move || {
                        ready.wait();
                        release.wait();
                    })
                    .unwrap()
            })
            .collect();
        ready.wait();
        let target = myself();
        let mut reader = Threads::new();
        let Listing::Found { threads, complete } = reader.list(&target, 1024) else {
            panic!("this process's threads are readable");
        };
        assert!(complete);
        assert!(threads.len() >= 4, "{threads:?}");
        assert_eq!(threads.iter().filter(|thread| thread.leader).count(), 1);
        assert!(threads[0].leader && threads[0].tid == target.id.pid);
        assert_eq!(threads[0].start, target.id.start);
        let names: Vec<&str> = threads.iter().map(|thread| thread.name.as_str()).collect();
        for index in 0..3 {
            assert!(
                names.contains(&format!("probe-{index}").as_str()),
                "{names:?}"
            );
        }
        assert!(
            threads
                .iter()
                .all(|thread| thread.state.is_ascii_alphabetic())
        );
        // A smaller limit keeps the leader and says the list is cut.
        let Listing::Found { threads, complete } = reader.list(&target, 2) else {
            panic!("readable");
        };
        assert!(!complete);
        assert_eq!(threads.len(), 2);
        assert!(threads[0].leader);
        release.wait();
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn an_exited_process_is_gone() {
        let mut target = myself();
        // Beyond pid_max (4194304 at most), so no process can have it.
        target.id.pid = 4_194_305 + 10;
        assert_eq!(Threads::new().list(&target, 1024), Listing::Gone);
    }
}
