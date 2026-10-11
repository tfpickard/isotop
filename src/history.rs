//! One shared history of every process's CPU and memory, recorded on each sample whichever view is
//! shown, so a view that reads the past has it the moment it opens. Pure state: no drawing.

use std::collections::VecDeque;

use crate::model::{Identity, Snapshot};

/// Samples kept. At one sample per second that is four minutes; at `--sample-ms` below 1000 it is
/// less, and a consumer asks for a time window, never a count.
pub const CAPACITY: usize = 240;
/// How many samples of one process a view asks `History::series` for, and how many seconds a
/// headless run replays before the captured frame.
pub const SERIES: usize = 60;

/// What one sample says about one process: the raw readings, not the scene's display smoothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub cpu: f32,
    pub memory: u64,
}

/// The readings of one snapshot, sorted by `Identity`.
#[derive(Debug)]
pub struct Sample {
    /// `Snapshot::elapsed` of the snapshot this came from.
    pub elapsed: f64,
    /// How many samples the history took in before this one: an absolute count that keeps
    /// rising as the ring turns over, so a view can number samples across windows.
    #[cfg_attr(not(test), expect(dead_code, reason = "first read by ptolemy (#40)"))]
    pub sequence: u64,
    readings: Vec<(Identity, Reading)>,
}

/// A process that appeared in a sample, or was gone from it, against the sample before.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(test), expect(dead_code, reason = "first read by ptolemy (#40)"))]
pub struct Change {
    pub id: Identity,
    /// `elapsed` of the first sample it was in, for a birth, or the first it was missing from,
    /// for an exit.
    pub elapsed: f64,
    /// A birth when true, an exit when false.
    pub born: bool,
    /// Its first reading for a birth, its last for an exit.
    pub reading: Reading,
}

impl Sample {
    pub fn reading(&self, id: Identity) -> Option<Reading> {
        self.readings
            .binary_search_by_key(&id, |&(key, _)| key)
            .ok()
            .map(|index| self.readings[index].1)
    }

    pub fn cpu(&self, id: Identity) -> Option<f32> {
        self.reading(id).map(|reading| reading.cpu)
    }
}

/// A ring of the last `CAPACITY` samples, oldest first, with strictly increasing `elapsed`.
#[derive(Debug, Default)]
pub struct History {
    samples: VecDeque<Sample>,
    /// Scene time at which the newest sample arrived, so a view that scrolls can move smoothly
    /// between samples.
    arrived: f32,
    /// Samples taken in so far, including those the ring has dropped.
    recorded: u64,
}

impl History {
    /// Keeps a sample unless it is not newer than the newest one, so the scene and a replay may
    /// both offer the same snapshot. `time` is the scene time it arrived at.
    pub fn record(&mut self, snapshot: &Snapshot, time: f32) {
        if self.samples.back().is_some_and(|newest| {
            snapshot.elapsed.partial_cmp(&newest.elapsed) != Some(std::cmp::Ordering::Greater)
        }) {
            return;
        }
        let mut readings: Vec<(Identity, Reading)> = snapshot
            .processes
            .iter()
            .map(|process| {
                (
                    process.id,
                    Reading {
                        cpu: process.cpu,
                        memory: process.memory,
                    },
                )
            })
            .collect();
        readings.sort_by_key(|&(id, _)| id);
        self.samples.push_back(Sample {
            elapsed: snapshot.elapsed,
            sequence: self.recorded,
            readings,
        });
        self.recorded += 1;
        self.arrived = time;
        while self.samples.len() > CAPACITY {
            self.samples.pop_front();
        }
    }

    /// Scene time at which the newest sample arrived.
    pub fn arrived(&self) -> f32 {
        self.arrived
    }

    /// `elapsed` of the newest sample.
    pub fn newest(&self) -> Option<f64> {
        self.samples.back().map(|sample| sample.elapsed)
    }

    /// The samples whose `elapsed` lies in `until - seconds..=until`, oldest first. `until` is
    /// the shown snapshot's `elapsed`, so pause and rewind show the past they were taken in.
    pub fn window(&self, until: f64, seconds: f64) -> impl Iterator<Item = &Sample> {
        let from = until - seconds;
        let start = self.samples.partition_point(|sample| sample.elapsed < from);
        let end = self
            .samples
            .partition_point(|sample| sample.elapsed <= until);
        self.samples.range(start..end.max(start))
    }

    /// The last `count` samples at or before `until`, oldest first; fewer when the history is
    /// shorter.
    #[cfg_attr(not(test), expect(dead_code, reason = "first read by ptolemy (#40)"))]
    pub fn recent(&self, until: f64, count: usize) -> impl ExactSizeIterator<Item = &Sample> {
        let end = self
            .samples
            .partition_point(|sample| sample.elapsed <= until);
        self.samples.range(end.saturating_sub(count)..end)
    }

    /// The last `count` samples at or before `until`, oldest first: the process's reading in
    /// each, or `None` where it was absent. Fewer than `count` entries when the history is
    /// shorter.
    #[cfg_attr(not(test), expect(dead_code, reason = "first read by ptolemy (#40)"))]
    pub fn series(&self, id: Identity, until: f64, count: usize) -> Vec<Option<Reading>> {
        self.recent(until, count)
            .map(|sample| sample.reading(id))
            .collect()
    }

    /// The births and exits of the samples whose `elapsed` lies in `until - seconds..=until`,
    /// each found against the sample before it, oldest first and by `Identity` within a sample.
    /// The oldest sample the history holds has nothing to compare with, so it reports none: a
    /// history that starts with a running machine does not count it as born.
    #[cfg_attr(not(test), expect(dead_code, reason = "first read by ptolemy (#40)"))]
    pub fn changes(&self, until: f64, seconds: f64) -> Vec<Change> {
        let from = until - seconds;
        let start = self.samples.partition_point(|sample| sample.elapsed < from);
        let end = self
            .samples
            .partition_point(|sample| sample.elapsed <= until);
        let mut changes = Vec::new();
        for index in start.max(1)..end {
            let (before, after) = (&self.samples[index - 1], &self.samples[index]);
            let elapsed = after.elapsed;
            let (mut old, mut new) = (
                before.readings.iter().peekable(),
                after.readings.iter().peekable(),
            );
            loop {
                let change = match (old.peek(), new.peek()) {
                    (None, None) => break,
                    (Some(&&(id, reading)), None) => {
                        old.next();
                        (id, false, reading)
                    }
                    (None, Some(&&(id, reading))) => {
                        new.next();
                        (id, true, reading)
                    }
                    (Some(&&(gone, last)), Some(&&(came, first))) => match gone.cmp(&came) {
                        std::cmp::Ordering::Less => {
                            old.next();
                            (gone, false, last)
                        }
                        std::cmp::Ordering::Greater => {
                            new.next();
                            (came, true, first)
                        }
                        std::cmp::Ordering::Equal => {
                            old.next();
                            new.next();
                            continue;
                        }
                    },
                };
                let (id, born, reading) = change;
                changes.push(Change {
                    id,
                    elapsed,
                    born,
                    reading,
                });
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Process, Snapshot, testing::process};

    fn snapshot(elapsed: f64, processes: Vec<Process>) -> Snapshot {
        Snapshot {
            elapsed,
            processes,
            ..Snapshot::default()
        }
    }

    fn busy(pid: u32, cpu: f32, memory: u64) -> Process {
        Process {
            cpu,
            memory,
            ..process(pid, 1)
        }
    }

    fn elapsed(history: &History) -> Vec<f64> {
        history
            .samples
            .iter()
            .map(|sample| sample.elapsed)
            .collect()
    }

    #[test]
    fn history_ignores_a_sample_that_is_not_newer() {
        let mut history = History::default();
        history.record(&snapshot(5.0, vec![busy(2, 10.0, 1)]), 1.0);
        history.record(&snapshot(5.0, vec![busy(2, 99.0, 1)]), 2.0);
        history.record(&snapshot(4.0, vec![busy(2, 98.0, 1)]), 3.0);
        history.record(&snapshot(f64::NAN, vec![busy(2, 97.0, 1)]), 4.0);
        assert_eq!(elapsed(&history), [5.0]);
        assert_eq!(history.arrived(), 1.0);
        history.record(&snapshot(6.0, vec![]), 5.0);
        assert_eq!(elapsed(&history), [5.0, 6.0]);
        assert_eq!(history.arrived(), 5.0);
    }

    #[test]
    fn history_keeps_at_most_its_capacity() {
        let mut history = History::default();
        for second in 0..CAPACITY + 10 {
            history.record(&snapshot(second as f64, vec![busy(2, 1.0, 1)]), 0.0);
        }
        assert_eq!(history.samples.len(), CAPACITY);
        assert_eq!(history.samples.front().map(|s| s.elapsed), Some(10.0));
        assert_eq!(history.newest(), Some((CAPACITY + 9) as f64));
    }

    #[test]
    fn a_window_selects_samples_by_elapsed_time_not_by_count() {
        let mut history = History::default();
        // Four samples a second for twenty seconds.
        for tick in 0..80 {
            history.record(&snapshot(tick as f64 * 0.25, vec![]), 0.0);
        }
        let picked: Vec<f64> = history.window(19.75, 2.0).map(|s| s.elapsed).collect();
        assert_eq!(picked.len(), 9);
        assert_eq!(picked.first(), Some(&17.75));
        assert_eq!(picked.last(), Some(&19.75));
        // Looking back from the middle of the history ignores the newer samples.
        let past: Vec<f64> = history.window(10.0, 1.0).map(|s| s.elapsed).collect();
        assert_eq!(past, [9.0, 9.25, 9.5, 9.75, 10.0]);
        assert_eq!(history.window(-1.0, 5.0).count(), 0);
    }

    #[test]
    fn a_series_marks_samples_where_the_process_was_absent() {
        let mut history = History::default();
        let wanted = process(7, 1).id;
        history.record(&snapshot(1.0, vec![busy(2, 5.0, 10)]), 0.0);
        history.record(
            &snapshot(2.0, vec![busy(7, 20.0, 700), busy(2, 5.0, 10)]),
            0.0,
        );
        history.record(
            &snapshot(3.0, vec![busy(2, 5.0, 10), busy(7, 40.0, 800)]),
            0.0,
        );
        history.record(&snapshot(4.0, vec![busy(2, 5.0, 10)]), 0.0);
        let series = history.series(wanted, 4.0, 4);
        assert_eq!(
            series,
            [
                None,
                Some(Reading {
                    cpu: 20.0,
                    memory: 700
                }),
                Some(Reading {
                    cpu: 40.0,
                    memory: 800
                }),
                None
            ]
        );
        // Asking for more than there is returns what there is.
        assert_eq!(history.series(wanted, 4.0, SERIES).len(), 4);
    }

    #[test]
    fn changes_list_births_and_exits_against_the_sample_before() {
        let mut history = History::default();
        let id = |pid| process(pid, 1).id;
        history.record(
            &snapshot(1.0, vec![busy(2, 1.0, 10), busy(3, 1.0, 30)]),
            0.0,
        );
        history.record(
            &snapshot(2.0, vec![busy(2, 1.0, 11), busy(3, 1.0, 31)]),
            0.0,
        );
        history.record(
            &snapshot(3.0, vec![busy(3, 1.0, 32), busy(4, 2.0, 40)]),
            0.0,
        );
        history.record(&snapshot(4.0, vec![busy(4, 2.0, 41)]), 0.0);
        let change = |pid, elapsed, born, memory, cpu| Change {
            id: id(pid),
            elapsed,
            born,
            reading: Reading { cpu, memory },
        };
        assert_eq!(
            history.changes(4.0, 10.0),
            [
                change(2, 3.0, false, 11, 1.0),
                change(4, 3.0, true, 40, 2.0),
                change(3, 4.0, false, 32, 1.0),
            ],
            "the first sample has nothing to compare with, so 2 and 3 are not born"
        );
        assert_eq!(history.changes(4.0, 0.5), [change(3, 4.0, false, 32, 1.0)]);
        assert_eq!(history.changes(2.0, 10.0), []);
        let sequences: Vec<u64> = history.recent(4.0, 2).map(|s| s.sequence).collect();
        assert_eq!(sequences, [2, 3]);
    }

    #[test]
    fn sequences_keep_counting_when_the_ring_turns_over() {
        let mut history = History::default();
        for second in 0..CAPACITY + 5 {
            history.record(&snapshot(second as f64, vec![]), 0.0);
        }
        let oldest = history
            .recent(f64::INFINITY, CAPACITY)
            .next()
            .map(|s| s.sequence);
        assert_eq!(oldest, Some(5));
        assert_eq!(history.recent(f64::INFINITY, 1).len(), 1);
    }

    #[test]
    fn a_series_ends_at_the_shown_snapshot_when_paused() {
        let mut history = History::default();
        let wanted = process(7, 1).id;
        for second in 1..=10 {
            let cpu = second as f32;
            history.record(&snapshot(second as f64, vec![busy(7, cpu, 1)]), 0.0);
        }
        let series = history.series(wanted, 6.0, 3);
        let cpu: Vec<f32> = series.iter().map(|r| r.expect("present").cpu).collect();
        assert_eq!(cpu, [4.0, 5.0, 6.0]);
    }
}
