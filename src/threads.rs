//! Per-thread sampling: what a platform reads of each thread (`ThreadReader`), how a pass walks
//! the processes within its caps (`ThreadScan`), the CPU tracker it keeps between passes
//! (`DeltaTracker`), and the threads the demo synthesizes (`demo_list`). The background thread
//! runs a pass only while a shown view demands `Source::Threads`.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Instant;

use crate::model::{Identity, Measured, Process, top_n};
use crate::simulation::Rng;

/// Threads one pass lists at most, over all processes.
pub const THREADS_PER_PASS: usize = 8192;

/// Threads one process lists at most.
pub const THREADS_PER_PROCESS: usize = 1024;

/// What one pass may read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub per_pass: usize,
    pub per_process: usize,
}

impl Caps {
    pub const DEFAULT: Caps = Caps {
        per_pass: THREADS_PER_PASS,
        per_process: THREADS_PER_PROCESS,
    };
}

/// One thread of a process.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    /// The thread id: the pid for the leader on Linux, the low 32 bits of `start` on macOS.
    pub tid: u32,
    /// Identifies the thread within its `tid`, which the system may reuse: the start time in
    /// ticks since boot on Linux, the unique 64-bit thread id on macOS, which is never reused
    /// within a boot.
    pub start: u64,
    /// The name the thread gave itself, raw. It is untrusted text, so every display path
    /// makes it a `terminal::SafeText`.
    pub name: String,
    /// Percent of one core over the last background interval; None on its first observation.
    pub cpu: Option<f32>,
    /// The scheduler state as a Linux letter (R, S, D, T, Z), `'?'` where the platform's state
    /// maps to none.
    pub state: char,
    /// The main thread: the one whose id is the pid on Linux, the oldest on macOS.
    pub leader: bool,
}

/// The threads of one process as the last pass read them.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadList {
    /// Leader first. The `Arc` keeps clones of a `Process` (the history ring's) cheap.
    pub threads: Arc<[Thread]>,
    /// False when a cap cut the list short, or the pass ran out of budget before the process:
    /// the true count stays `Process::threads`, and a consumer samples in proportion to it
    /// rather than inventing threads.
    pub complete: bool,
}

/// A thread as the platform read it, before its CPU is derived.
#[derive(Clone, Debug, PartialEq)]
pub struct RawThread {
    pub tid: u32,
    pub start: u64,
    pub name: String,
    /// Cumulative CPU time in the platform's units (`ThreadReader::ticks_per_second` per
    /// second), user and system together.
    pub ticks: u64,
    pub state: char,
    pub leader: bool,
}

/// A process whose threads a pass reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub id: Identity,
    /// The process's name, which names a thread that has none (macOS).
    pub name: String,
}

/// What listing one process's threads came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Listing {
    /// The process has exited, or its pid now names another process.
    Gone,
    /// The platform refused, normally for another user's process.
    Unreadable,
    /// At most the requested number of threads, leader first. `complete` is false when the
    /// process has more.
    Found {
        threads: Vec<RawThread>,
        complete: bool,
    },
}

/// Reads the threads of one process; each platform provides one as `platform::Threads`.
pub trait ThreadReader {
    /// The unit of `RawThread::ticks`.
    fn ticks_per_second(&self) -> f64;

    /// Lists at most `limit` threads of `target`, leader first. A thread that exits while it is
    /// read is left out.
    fn list(&mut self, target: &Target, limit: usize) -> Listing;
}

/// A counter's growth between two observations of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delta {
    pub count: u64,
    pub seconds: f64,
}

/// Turns the cumulative counters of many things, one reading per pass, into growth since the
/// previous pass. A key observed for the first time, or in a pass that follows one that missed
/// it, has no delta, and neither has a counter that went backwards. Keys the last pass did not
/// observe are forgotten when it ends. Keep the key as specific as the thing: a key that can be
/// reused for another thing (a tid) pairs with what tells them apart (its start).
#[derive(Debug)]
pub struct DeltaTracker<Key> {
    previous: HashMap<Key, (u64, Instant)>,
    current: HashMap<Key, (u64, Instant)>,
}

impl<Key> Default for DeltaTracker<Key> {
    fn default() -> Self {
        DeltaTracker {
            previous: HashMap::new(),
            current: HashMap::new(),
        }
    }
}

impl<Key: Hash + Eq> DeltaTracker<Key> {
    /// Records `count` for `key` at `now` and returns its growth since the previous pass.
    pub fn observe(&mut self, key: Key, count: u64, now: Instant) -> Option<Delta> {
        let delta = self.previous.get(&key).and_then(|&(before, at)| {
            let seconds = now.checked_duration_since(at)?.as_secs_f64();
            (seconds > 0.0).then_some(())?;
            Some(Delta {
                count: count.checked_sub(before)?,
                seconds,
            })
        });
        self.current.insert(key, (count, now));
        delta
    }

    /// Ends a pass: what it observed is what the next compares with, and everything else is
    /// forgotten.
    pub fn end_pass(&mut self) {
        self.previous = std::mem::take(&mut self.current);
    }

    /// Forgets everything, so the next pass starts again at no delta.
    pub fn clear(&mut self) {
        self.previous.clear();
        self.current.clear();
    }
}

/// The pass state kept on the background thread between cycles.
#[derive(Default)]
pub struct ThreadScan {
    /// Per-thread CPU time, keyed by `(tid, start)`.
    tracker: DeltaTracker<(u32, u64)>,
}

impl ThreadScan {
    /// Reads the threads of `targets` within `Caps::DEFAULT`, in the order given.
    pub fn pass(
        &mut self,
        targets: &[Target],
        reader: &mut impl ThreadReader,
    ) -> HashMap<Identity, Measured<ThreadList>> {
        self.pass_with(targets, reader, Caps::DEFAULT, Instant::now)
    }

    /// A pass with its own caps and clock. `targets` come in the order the caller wants them
    /// read, which the pass does not change: the frame thread decides the order, so the pass
    /// never sorts. It stops reading when `caps.per_pass` threads are listed: the process it
    /// stops in is cut short, and every process after it has an empty, incomplete list. A
    /// process that is gone is absent from the result.
    pub fn pass_with(
        &mut self,
        targets: &[Target],
        reader: &mut impl ThreadReader,
        caps: Caps,
        mut clock: impl FnMut() -> Instant,
    ) -> HashMap<Identity, Measured<ThreadList>> {
        let ticks_per_second = reader.ticks_per_second();
        let mut remaining = caps.per_pass;
        let mut found = HashMap::with_capacity(targets.len());
        for target in targets {
            let measured = if remaining == 0 {
                Measured::Known(ThreadList {
                    threads: Arc::from([]),
                    complete: false,
                })
            } else {
                match reader.list(target, remaining.min(caps.per_process)) {
                    Listing::Gone => continue,
                    Listing::Unreadable => Measured::Unreadable,
                    Listing::Found { threads, complete } => {
                        remaining = remaining.saturating_sub(threads.len());
                        let now = clock();
                        Measured::Known(ThreadList {
                            threads: threads
                                .into_iter()
                                .map(|raw| self.thread(raw, ticks_per_second, now))
                                .collect(),
                            complete,
                        })
                    }
                }
            };
            found.insert(target.id, measured);
        }
        self.tracker.end_pass();
        found
    }

    /// Forgets every thread, for a cycle that read none: the next pass starts again at no CPU
    /// readings rather than averaging over the gap.
    pub fn forget(&mut self) {
        self.tracker.clear();
    }

    fn thread(&mut self, raw: RawThread, ticks_per_second: f64, now: Instant) -> Thread {
        let cpu = self
            .tracker
            .observe((raw.tid, raw.start), raw.ticks, now)
            .map(|delta| (delta.count as f64 / ticks_per_second / delta.seconds * 100.0) as f32);
        Thread {
            tid: raw.tid,
            start: raw.start,
            name: raw.name,
            cpu,
            state: raw.state,
            leader: raw.leader,
        }
    }
}

/// The processes to read the threads of, busiest first: CPU descending, ties by `Identity`.
pub fn targets(processes: &[Process]) -> Vec<Target> {
    top_n(processes, processes.len(), |process| f64::from(process.cpu))
        .into_iter()
        .map(|process| Target {
            id: process.id,
            name: process.name.clone(),
        })
        .collect()
}

/// Thread ids of the demo's other threads start here, above every demo pid, and stay apart per
/// process: `DEMO_TID_BASE + pid * DEMO_TID_STRIDE + index`.
const DEMO_TID_BASE: u32 = 1 << 20;
const DEMO_TID_STRIDE: u32 = THREADS_PER_PROCESS as u32;

/// Seeds the draws that shape a demo process's threads, apart from other uses of its identity.
const DEMO_THREAD_SALT: u64 = 0x7b3a_91d4_c05e;

/// Names the demo's non-leader threads take; `{}` is the thread's index.
const DEMO_THREAD_NAMES: [&str; 7] = [
    "worker-{}",
    "gc",
    "io",
    "timer",
    "net-{}",
    "pool-{}",
    "flush",
];

/// The threads of a demo process at demo `time` (`DemoOptions::threads`): as many as
/// `Process::threads`, at most `THREADS_PER_PROCESS`, with the leader's tid the pid and the
/// others from a range above every demo pid. Their ids, names and start times depend only on
/// the process's identity, so a thread keeps its identity from sample to sample; only their
/// CPU moves. Each takes a share of the process's CPU that swells and ebbs on its own period,
/// the shares summing to `Process::cpu`, so a base process whose CPU exceeds 100% per thread
/// (`DemoOptions::exact_cpu_time` off) has threads above 100% too.
pub fn demo_list(process: &Process, time: f64) -> ThreadList {
    let count = (process.threads as usize).min(THREADS_PER_PROCESS);
    let mut random = Rng::for_identity(process.id, DEMO_THREAD_SALT);
    let mut drawn = Vec::with_capacity(count);
    for _ in 0..count {
        let (base, offset, period) = (random.unit(), random.unit(), random.unit());
        let pick = random.next_u64() as usize % DEMO_THREAD_NAMES.len();
        drawn.push((base, offset, period, pick));
    }
    // The leader does the most work on average; the others swell and ebb on their own periods.
    let weights: Vec<f64> = drawn
        .iter()
        .enumerate()
        .map(|(index, &(base, offset, period, _))| {
            let weight = f64::from(base * base) + if index == 0 { 0.6 } else { 0.05 };
            let swell = 0.5
                + 0.5
                    * (time * (0.3 + f64::from(period))
                        + f64::from(offset) * std::f64::consts::TAU)
                        .sin();
            weight * (0.25 + 0.75 * swell)
        })
        .collect();
    let total: f64 = weights.iter().sum();
    let threads = drawn
        .iter()
        .zip(&weights)
        .enumerate()
        .map(|(index, (&(_, offset, _, pick), &weight))| {
            let cpu = (f64::from(process.cpu) * weight / total) as f32;
            let leader = index == 0;
            Thread {
                tid: if leader {
                    process.id.pid
                } else {
                    DEMO_TID_BASE
                        .wrapping_add(process.id.pid.wrapping_mul(DEMO_TID_STRIDE))
                        .wrapping_add(index as u32)
                },
                start: if leader {
                    process.id.start
                } else {
                    process.id.start + 1 + (offset * 6000.0) as u64
                },
                name: if leader {
                    process.name.clone()
                } else {
                    DEMO_THREAD_NAMES[pick].replace("{}", &index.to_string())
                },
                cpu: Some(cpu),
                state: match process.state {
                    state @ ('Z' | 'T') => state,
                    'D' if leader => 'D',
                    _ if cpu > 5.0 => 'R',
                    _ => 'S',
                },
                leader,
            }
        })
        .collect();
    ThreadList {
        threads,
        complete: process.threads as usize <= THREADS_PER_PROCESS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DemoOptions, demo_with, testing};
    use std::time::Duration;

    fn raw(tid: u32, start: u64, ticks: u64) -> RawThread {
        RawThread {
            tid,
            start,
            name: format!("t{tid}"),
            ticks,
            state: 'S',
            leader: false,
        }
    }

    fn target(pid: u32) -> Target {
        Target {
            id: Identity { pid, start: 1 },
            name: format!("p{pid}"),
        }
    }

    /// Reads the threads it was given, per pid, leader first, and counts how it was asked.
    struct Fake {
        threads: HashMap<u32, Vec<RawThread>>,
        asked: Vec<(u32, usize)>,
    }

    impl Fake {
        fn new(threads: impl IntoIterator<Item = (u32, Vec<RawThread>)>) -> Self {
            Fake {
                threads: threads.into_iter().collect(),
                asked: Vec::new(),
            }
        }
    }

    impl ThreadReader for Fake {
        fn ticks_per_second(&self) -> f64 {
            100.0
        }

        fn list(&mut self, target: &Target, limit: usize) -> Listing {
            self.asked.push((target.id.pid, limit));
            match self.threads.get(&target.id.pid) {
                None => Listing::Gone,
                Some(all) => Listing::Found {
                    threads: all.iter().take(limit).cloned().collect(),
                    complete: all.len() <= limit,
                },
            }
        }
    }

    /// A clock that moves `step` per call, from a fixed start.
    fn clock(step: Duration) -> impl FnMut() -> Instant {
        let mut now = Instant::now();
        move || {
            now += step;
            now
        }
    }

    fn cpu_of(found: &HashMap<Identity, Measured<ThreadList>>, pid: u32, tid: u32) -> Option<f32> {
        let list = found[&target(pid).id].known().unwrap();
        list.threads.iter().find(|t| t.tid == tid).unwrap().cpu
    }

    #[test]
    fn thread_cpu_comes_from_tick_deltas_and_reused_tids_start_over() {
        let mut scan = ThreadScan::default();
        let targets = [target(10)];
        // Each pass reads the thread one second after the last one (a clock tick per process).
        let mut time = clock(Duration::from_secs(1));
        let mut read = |ticks: &[(u32, u64, u64)]| {
            let mut fake = Fake::new([(
                10,
                ticks
                    .iter()
                    .map(|&(tid, start, t)| raw(tid, start, t))
                    .collect(),
            )]);
            scan.pass_with(&targets, &mut fake, Caps::DEFAULT, &mut time)
        };
        // The first observation of a thread has no CPU, not zero.
        let first = read(&[(10, 5, 1000), (11, 6, 200)]);
        assert_eq!(cpu_of(&first, 10, 10), None);
        // 150 ticks at 100 per second over one second is one and a half cores.
        let second = read(&[(10, 5, 1150), (11, 6, 200)]);
        assert_eq!(cpu_of(&second, 10, 10), Some(150.0));
        assert_eq!(cpu_of(&second, 10, 11), Some(0.0));
        // Tid 11 now belongs to another thread, which starts again at no reading; a counter
        // that went backwards has none either.
        let third = read(&[(10, 5, 1100), (11, 99, 5000)]);
        assert_eq!(cpu_of(&third, 10, 10), None);
        assert_eq!(cpu_of(&third, 10, 11), None);
        // A thread that missed a pass is forgotten, so it starts over once it is back.
        read(&[(10, 5, 1200)]);
        let back = read(&[(10, 5, 1300), (11, 99, 5100)]);
        assert_eq!(cpu_of(&back, 10, 10), Some(100.0));
        assert_eq!(cpu_of(&back, 10, 11), None);
    }

    #[test]
    fn a_pass_with_no_time_between_readings_has_no_cpu() {
        let mut tracker = DeltaTracker::default();
        let now = Instant::now();
        assert_eq!(tracker.observe(1, 10, now), None);
        tracker.end_pass();
        assert_eq!(tracker.observe(1, 20, now), None, "no time has passed");
        tracker.end_pass();
        let later = now + Duration::from_secs(2);
        assert_eq!(
            tracker.observe(1, 50, later),
            Some(Delta {
                count: 30,
                seconds: 2.0
            })
        );
    }

    #[test]
    fn the_thread_caps_bound_a_pass_and_mark_lists_incomplete() {
        let many = |count: u32, first: u32| -> Vec<RawThread> {
            (0..count).map(|k| raw(first + k, 1, 0)).collect()
        };
        // Pid 1 has more threads than a process may list; 2 to 8 take 1,000 each, which brings
        // the pass to 1024 + 7 * 1000 = 8024; 9 gets the 168 that remain; 10 is not reached.
        let mut all = vec![(1, many(5000, 1_000_000))];
        for pid in 2..=8 {
            all.push((pid, many(1000, pid * 10_000)));
        }
        all.push((9, many(500, 90_000)));
        all.push((10, many(3, 100_000)));
        let mut fake = Fake::new(all);
        // Pid 99 is gone, which the pass finds out as it comes to it.
        let mut targets: Vec<Target> = (1..=10).map(target).collect();
        targets.insert(2, target(99));
        let found = ThreadScan::default().pass(&targets, &mut fake);
        let list = |pid: u32| found[&target(pid).id].known().unwrap().clone();
        assert_eq!(list(1).threads.len(), THREADS_PER_PROCESS);
        assert!(!list(1).complete, "cut by the per-process cap");
        assert!(list(8).complete);
        assert_eq!(list(8).threads.len(), 1000);
        assert_eq!(list(9).threads.len(), THREADS_PER_PASS - 1024 - 7 * 1000);
        assert!(!list(9).complete, "cut by what the pass had left");
        assert!(list(10).threads.is_empty() && !list(10).complete);
        assert!(!found.contains_key(&target(99).id), "gone");
        let listed: usize = found
            .values()
            .map(|measured| measured.known().unwrap().threads.len())
            .sum();
        assert_eq!(listed, THREADS_PER_PASS);
        assert!(
            fake.asked
                .iter()
                .all(|&(_, limit)| limit <= THREADS_PER_PROCESS),
            "no process is asked for more than its cap"
        );
        assert!(
            !fake.asked.iter().any(|&(pid, _)| pid == 10),
            "once the budget is spent nothing more is read"
        );
    }

    #[test]
    fn an_unreadable_process_is_unreadable_and_costs_no_budget() {
        struct Refuses;
        impl ThreadReader for Refuses {
            fn ticks_per_second(&self) -> f64 {
                100.0
            }
            fn list(&mut self, _: &Target, _: usize) -> Listing {
                Listing::Unreadable
            }
        }
        let found = ThreadScan::default().pass(&[target(3)], &mut Refuses);
        assert_eq!(found[&target(3).id], Measured::Unreadable);
    }

    #[test]
    fn targets_are_the_busiest_first_and_ties_go_to_the_smaller_identity() {
        let process = |pid: u32, cpu: f32| Process {
            cpu,
            ..testing::process(pid, 1)
        };
        let processes = [
            process(5, 10.0),
            process(3, 50.0),
            process(4, 10.0),
            process(9, 0.0),
        ];
        let order: Vec<u32> = targets(&processes).iter().map(|t| t.id.pid).collect();
        assert_eq!(order, [3, 4, 5, 9]);
    }

    #[test]
    fn demo_threads_match_the_thread_count_and_process_cpu() {
        let options = DemoOptions {
            churn: true,
            threads: true,
            ..DemoOptions::NONE
        };
        for time in [0.0, 12.0, 47.5] {
            let snapshot = demo_with(time, 128, options);
            let mut tids = std::collections::HashSet::new();
            let mut pids = std::collections::HashSet::new();
            for process in &snapshot.processes {
                let list = process
                    .thread_list
                    .known()
                    .expect("every process has threads");
                assert_eq!(list.threads.len(), process.threads as usize);
                assert!(list.complete);
                assert_eq!(list.threads.iter().filter(|t| t.leader).count(), 1);
                let leader = &list.threads[0];
                assert!(leader.leader && leader.tid == process.id.pid);
                let sum: f32 = list.threads.iter().map(|t| t.cpu.unwrap()).sum();
                assert!(
                    (sum - process.cpu).abs() <= 1e-3 * process.cpu.max(1.0),
                    "{} threads of pid {} sum to {sum}, not {}",
                    list.threads.len(),
                    process.id.pid,
                    process.cpu
                );
                pids.insert(process.id.pid);
                for thread in list.threads.iter() {
                    assert!(tids.insert(thread.tid), "tid {} is not unique", thread.tid);
                }
            }
            assert!(
                tids.iter()
                    .filter(|tid| !pids.contains(tid))
                    .all(|&tid| tid >= DEMO_TID_BASE),
                "non-leader tids stay above the pids"
            );
        }
    }

    #[test]
    fn demo_threads_keep_their_identity_while_their_cpu_moves() {
        let options = DemoOptions {
            threads: true,
            ..DemoOptions::NONE
        };
        let at = |time: f64| {
            demo_with(time, 64, options)
                .processes
                .into_iter()
                .find(|p| p.threads > 4)
                .unwrap()
        };
        let (before, after) = (at(3.0), at(9.0));
        let keys = |p: &Process| -> Vec<(u32, u64, String)> {
            p.thread_list
                .known()
                .unwrap()
                .threads
                .iter()
                .map(|t| (t.tid, t.start, t.name.clone()))
                .collect()
        };
        assert_eq!(keys(&before), keys(&after));
        assert_ne!(
            before.thread_list.known().unwrap().threads[1].cpu,
            after.thread_list.known().unwrap().threads[1].cpu
        );
    }

    #[test]
    fn demo_threads_cover_every_thread_of_a_big_process_up_to_the_cap() {
        let mut process = testing::process(7, 1);
        process.cpu = 200.0;
        process.threads = 3000;
        let list = demo_list(&process, 5.0);
        assert_eq!(list.threads.len(), THREADS_PER_PROCESS);
        assert!(!list.complete);
        let unique: std::collections::HashSet<u32> = list.threads.iter().map(|t| t.tid).collect();
        assert_eq!(unique.len(), THREADS_PER_PROCESS);
    }
}
