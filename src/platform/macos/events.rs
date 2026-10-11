//! Exec and exit events of the sampled processes from kqueue (`EVFILT_PROC`), read on the
//! `isotop-events` thread while `Source::LifeEvents` is demanded.
//!
//! kqueue watches one pid at a time, any process of the same user, child or not. It gives exact
//! exec and exit times; a fork names no child, so births come from sample diffing, and the exit
//! status (`NOTE_EXITSTATUS`) is reported for children only, so every exit's cause is unknown.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex};

use crate::model::Identity;
use crate::platform::{Cause, LifeQueue, LifeSource, Observation, Observed, Source, Sources};

/// How long the thread waits for an event before it watches newly sampled processes and checks
/// whether it is still wanted, in nanoseconds.
const TIMEOUT_NS: libc::c_long = 200_000_000;

/// CLOCK_MONOTONIC in nanoseconds.
pub fn monotonic_ns() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is a valid, writable timespec for the duration of the call.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return 0;
    }
    (time.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.tv_nsec as u64)
}

/// Opens a kqueue and starts the `isotop-events` thread, which watches every process in
/// `identities` (the latest sample's) for exec and exit; None, with no thread, when no kqueue
/// can be opened.
pub fn open_events(
    queue: Arc<LifeQueue>,
    identities: Arc<Mutex<HashMap<u32, Identity>>>,
) -> Option<Sources> {
    // SAFETY: kqueue() has no preconditions; the result is checked below.
    let fd = unsafe { libc::kqueue() };
    if fd < 0 {
        return None;
    }
    // SAFETY: fd is a freshly created descriptor that nothing else owns.
    let kqueue = unsafe { OwnedFd::from_raw_fd(fd) };
    std::thread::Builder::new()
        .name("isotop-events".into())
        .spawn(move || run(&queue, &identities, &kqueue))
        .ok()?;
    Some(Source::Kqueue.into())
}

/// Watches the sampled processes until nothing but the thread holds `queue`.
fn run(queue: &Arc<LifeQueue>, identities: &Mutex<HashMap<u32, Identity>>, kqueue: &OwnedFd) {
    // Each pid as it was watched. An exited one stays until the samples drop it, so it is not
    // watched again.
    let mut watched: HashMap<u32, Identity> = HashMap::new();
    // SAFETY: kevent holds integers and a pointer, for which all-zero bits are valid.
    let mut events: [libc::kevent; 64] = unsafe { std::mem::zeroed() };
    let timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: TIMEOUT_NS,
    };
    while Arc::strong_count(queue) > 1 {
        let sampled = identities.lock().map(|map| map.clone()).unwrap_or_default();
        watched.retain(|pid, _| sampled.contains_key(pid));
        for (&pid, &id) in &sampled {
            if watched.get(&pid) != Some(&id) {
                watch(kqueue.as_raw_fd(), pid);
                watched.insert(pid, id);
            }
        }
        // SAFETY: events is valid for writes of the length passed, timeout for reads, and no
        // changes are passed.
        let count = unsafe {
            libc::kevent(
                kqueue.as_raw_fd(),
                std::ptr::null(),
                0,
                events.as_mut_ptr(),
                events.len() as libc::c_int,
                &timeout,
            )
        };
        let at = monotonic_ns();
        for event in events.iter().take(count.max(0) as usize) {
            if event.filter != libc::EVFILT_PROC || event.flags & libc::EV_ERROR != 0 {
                continue;
            }
            let pid = event.ident as u32;
            let start = watched.get(&pid).map(|id| id.start);
            if event.fflags & libc::NOTE_EXEC != 0 {
                queue.push(Observed {
                    at,
                    source: LifeSource::Kqueue,
                    what: Observation::Exec { pid, start },
                });
            }
            if event.fflags & libc::NOTE_EXIT != 0 {
                queue.push(Observed {
                    at,
                    source: LifeSource::Kqueue,
                    what: Observation::Exit {
                        pid,
                        start,
                        cause: Cause::Unknown,
                    },
                });
            }
        }
    }
}

/// Asks for `pid`'s exec and exit. A process that is gone, or that belongs to another user, is
/// refused, and then it is simply not watched.
fn watch(kqueue: RawFd, pid: u32) {
    let change = libc::kevent {
        ident: pid as libc::uintptr_t,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_CLEAR,
        fflags: libc::NOTE_EXIT | libc::NOTE_EXEC,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: change is valid for reads for the call; no event list is passed, so nothing is
    // written.
    unsafe {
        libc::kevent(
            kqueue,
            &change,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn kqueue_reports_exec_and_exit_of_a_same_user_non_child() {
        // The subshell is the shell's child; once the shell has exited, launchd adopts it, so it
        // is no child of this process when it execs and exits.
        let mut shell = Command::new("sh")
            .args(["-c", "(sleep 1; exec /bin/echo) & echo $!"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("sh runs");
        let mut output = BufReader::new(shell.stdout.take().expect("piped"));
        let mut line = String::new();
        output.read_line(&mut line).expect("the shell prints a pid");
        let pid: u32 = line.trim().parse().expect("a pid");
        shell.wait().expect("the shell exits");
        let queue = Arc::new(LifeQueue::default());
        let identities = Arc::new(Mutex::new(HashMap::from([(
            pid,
            Identity { pid, start: 7 },
        )])));
        assert_eq!(
            open_events(Arc::clone(&queue), identities),
            Some(Source::Kqueue.into())
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = Vec::new();
        while Instant::now() < deadline
            && !seen
                .iter()
                .any(|observed: &Observed| matches!(observed.what, Observation::Exit { .. }))
        {
            seen.extend(queue.drain().0);
            std::thread::sleep(Duration::from_millis(20));
        }
        let what: Vec<&Observation> = seen
            .iter()
            .filter(|observed| observed.source == LifeSource::Kqueue)
            .map(|observed| &observed.what)
            .collect();
        assert_eq!(
            what,
            [
                &Observation::Exec {
                    pid,
                    start: Some(7)
                },
                &Observation::Exit {
                    pid,
                    start: Some(7),
                    cause: Cause::Unknown
                },
            ]
        );
        assert!(seen[0].at < seen[1].at);
        drop(output);
    }
}
