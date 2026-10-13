//! The system journal as a stream of text lines for the matrix view. The platform's log follower
//! runs as a child process whose output a background thread parses.

use std::io::{BufReader, Read};
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::platform;
use crate::simulation::hash;

/// Seconds between synthetic entries in demo mode.
const DEMO_INTERVAL: f64 = 0.35;
/// Entries held while the view is hidden or paused; later ones are dropped and counted.
const BACKLOG: usize = 2048;
/// syslog severity: 0 emergency to 3 error, 4 warning, 5 notice, 6 info, 7 debug.
const ERROR: u8 = 3;
/// How long stopping waits for the reader thread to see the end of the follower's output.
const READER_GRACE: Duration = Duration::from_millis(500);
/// Lines one sample keeps in its `Delivery`: the follower's backlog, which is as many as a frame
/// can drain. A longer flood loses its oldest lines, counted as dropped.
pub(crate) const KEPT: usize = BACKLOG;

/// systemd's verdict on a finished job, the `JOB_RESULT` field of the journal entry that reports
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobResult {
    Done,
    Canceled,
    Timeout,
    Failed,
    Dependency,
    /// The job was not run because a condition did not hold: neither a success nor a failure.
    Skipped,
}

impl JobResult {
    /// The result a `JOB_RESULT` value names; None for one this reader does not know.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "done" => Self::Done,
            "canceled" => Self::Canceled,
            "timeout" => Self::Timeout,
            "failed" => Self::Failed,
            "dependency" => Self::Dependency,
            "skipped" => Self::Skipped,
            _ => return None,
        })
    }

    /// Whether the job did not finish what it was asked to.
    pub fn failed(self) -> bool {
        matches!(
            self,
            Self::Failed | Self::Timeout | Self::Canceled | Self::Dependency
        )
    }
}

/// One journal entry as text, with its syslog severity and, where the platform reports one, the
/// result of the job it announces.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub priority: u8,
    pub text: String,
    /// `JOB_RESULT`: None for an entry that reports no job, and on macOS, whose unified log has no
    /// such field.
    pub job: Option<JobResult>,
}

impl Line {
    pub fn new(priority: u8, text: impl Into<String>) -> Self {
        Self {
            priority,
            text: text.into(),
            job: None,
        }
    }

    /// The same line announcing a job that ended as `job`.
    pub fn with_job(self, job: Option<JobResult>) -> Self {
        Self { job, ..self }
    }
}

/// The journal lines that arrived while one sample was the newest, which travel on that
/// `Snapshot` (`Snapshot::journal`) to the view that draws them. the app attaches what the
/// follower (or the demo) produced each frame to the newest sample; a view takes in the lines it
/// has not seen yet (`unseen`), so drawing the same sample twice, or an older one after a rewind,
/// adds nothing. Empty on a sample nothing was attached to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delivery {
    /// The lines attached to the sample, oldest first.
    pub lines: Vec<Line>,
    /// How many lines had arrived in all once the last of these did. The first of `lines` is
    /// number `through - lines.len()`.
    pub through: u64,
    /// How many lines had been dropped in all by then: the follower's backlog was full, or a
    /// sample held more than it keeps.
    pub dropped: u64,
}

/// What has been handed over since the follower started, which numbers the lines of a `Delivery`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    lines: u64,
    dropped: u64,
}

impl Delivery {
    /// Attaches `lines` that just arrived and the `dropped` count the follower reported, counting
    /// both in `totals`.
    pub fn add(&mut self, totals: &mut Totals, lines: Vec<Line>, dropped: u64) {
        totals.lines += lines.len() as u64;
        totals.dropped += dropped;
        self.lines.extend(lines);
        let excess = self.lines.len().saturating_sub(KEPT);
        self.lines.drain(..excess);
        totals.dropped += excess as u64;
        self.through = totals.lines;
        self.dropped = totals.dropped;
    }

    /// Lets go of the lines once the sample is no longer the newest, the only one that is ever
    /// fed: a view takes in a sample's lines when it draws it live, and the next frame draws the
    /// next sample, so an older delivery is not read again. The numbering stays, so `unseen`
    /// finds nothing in it, and the history holds the lines of one sample at most.
    pub fn retire(&mut self) {
        self.lines = Vec::new();
    }

    /// The lines of the delivery after the first `taken` that ever arrived.
    pub fn unseen(&self, taken: u64) -> &[Line] {
        let first = self.through - self.lines.len() as u64;
        let skip = usize::try_from(taken.saturating_sub(first)).unwrap_or(usize::MAX);
        self.lines.get(skip..).unwrap_or(&[])
    }
}

pub struct Journal {
    lines: mpsc::Receiver<Line>,
    dropped: Arc<AtomicU64>,
    child: Option<Child>,
    reader: Option<JoinHandle<()>>,
}

impl Journal {
    /// Follows the journal, starting with its most recent entries so the rain begins full.
    pub fn start() -> Self {
        let (sender, lines) = mpsc::sync_channel(BACKLOG);
        let dropped = Arc::new(AtomicU64::new(0));
        let (mut child, read_entry) = match platform::journal() {
            Ok(spawned) => spawned,
            Err(error) => {
                let _ = sender.try_send(Line::new(
                    ERROR,
                    format!("isotop: cannot run {}: {error}", platform::JOURNAL),
                ));
                return Self {
                    lines,
                    dropped,
                    child: None,
                    reader: None,
                };
            }
        };
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let count = Arc::clone(&dropped);
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_entry(&mut reader) {
                    Ok(Some(line)) => match sender.try_send(line) {
                        Ok(()) => {}
                        Err(mpsc::TrySendError::Full(_)) => {
                            count.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => return,
                    },
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.try_send(Line::new(
                            ERROR,
                            format!("isotop: journal read failed: {error}"),
                        ));
                        return;
                    }
                }
            }
            let mut complaint = String::new();
            let _ = stderr.read_to_string(&mut complaint);
            for line in complaint.lines().filter(|line| !line.trim().is_empty()) {
                let _ = sender.try_send(Line::new(ERROR, format!("{}: {line}", platform::JOURNAL)));
            }
        });
        Self {
            lines,
            dropped,
            child: Some(child),
            reader: Some(reader),
        }
    }

    /// Lines that arrived since the last call.
    pub fn drain(&self) -> Vec<Line> {
        self.lines.try_iter().collect()
    }

    /// Lines dropped since the last call because the backlog was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.swap(0, Ordering::Relaxed)
    }

    /// Stops the follower, which closes its output, and lets the reader thread finish on the end
    /// of the stream. Returns whether the reader has ended. One still blocked after
    /// `READER_GRACE` (some process outside the follower's group holding the pipe open) is left
    /// to end on its own rather than stall the frame loop: it exits at the end of the stream or
    /// when its next line finds no receiver.
    fn stop(&mut self) -> bool {
        if let Some(child) = &mut self.child {
            platform::stop_journal(child);
        }
        self.child = None;
        let Some(reader) = self.reader.take() else {
            return true;
        };
        let deadline = Instant::now() + READER_GRACE;
        while !reader.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        reader.is_finished() && reader.join().is_ok()
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Synthetic journal lines that arrive in `(from, to]` seconds of demo time, deterministic in time.
pub fn demo(from: f64, to: f64) -> Vec<Line> {
    let first = (from / DEMO_INTERVAL).floor() as i64 + 1;
    let last = (to / DEMO_INTERVAL).floor() as i64;
    (first.max(0)..=last).map(demo_line).collect()
}

/// The `counter`-th line of the demo journal, which arrives at `counter * DEMO_INTERVAL`.
fn demo_line(counter: i64) -> Line {
    const LINES: [(u8, &str); 16] = [
        (
            6,
            "systemd[1]: Started session-{n}.scope - Session {n} of User neo.",
        ),
        (
            6,
            "sshd[{p}]: Accepted publickey for neo from 10.0.{n}.7 port {p} ssh2",
        ),
        (
            6,
            "kernel: usb 3-{n}: new high-speed USB device number {n} using xhci_hcd",
        ),
        (
            5,
            "NetworkManager[{p}]: <info> device (wlan0): state change: activated",
        ),
        (
            6,
            "systemd[1]: Starting logrotate.service - Rotate log files...",
        ),
        (
            6,
            "systemd[1]: logrotate.service: Deactivated successfully.",
        ),
        (
            6,
            "CRON[{p}]: (root) CMD (command -v debian-sa1 > /dev/null && debian-sa1 1 1)",
        ),
        (
            4,
            "kernel: [UFW BLOCK] IN=wlan0 SRC=10.0.{n}.1 DST=224.0.0.251 PROTO=UDP",
        ),
        (
            7,
            "dockerd[{p}]: time=\"now\" level=debug msg=\"ignoring event\" container={p}",
        ),
        (
            4,
            "gnome-shell[{p}]: Window manager warning: last_user_time is ahead",
        ),
        (
            5,
            "systemd-resolved[{p}]: Using degraded feature set UDP for DNS server",
        ),
        (
            5,
            "sudo[{p}]: neo : TTY=pts/{n} ; PWD=/home/neo ; USER=root ; COMMAND=/usr/bin/apt",
        ),
        (
            3,
            "kernel: nvme0: I/O {p} QID {n} timeout, completion polled",
        ),
        (2, "smith[{p}]: Never send a human to do a machine's job."),
        (6, "systemd-journald[{p}]: Journal started"),
        (6, "morpheus[{p}]: Follow the white rabbit."),
    ];
    let hash = hash::golden_gamma_high(counter as u64);
    let (priority, template) = LINES[hash as usize % LINES.len()];
    let text = template
        .replace("{n}", &(hash % 9 + 1).to_string())
        .replace("{p}", &(1000 + hash % 60000).to_string());
    Line::new(priority, text)
}

/// Seconds between the job lines of `demo_jobs`.
const JOB_INTERVAL: f64 = 7.0;

/// Synthetic lines that report a job, arriving in `(from, to]` seconds of demo time: every seven
/// seconds, alternately systemd finishing a backup (priority 6, `JobResult::Done`) and failing to
/// start a mount (priority 3, `JobResult::Failed`), the first of them at seven seconds.
pub fn demo_jobs(from: f64, to: f64) -> Vec<Line> {
    let first = (from / JOB_INTERVAL).floor() as i64 + 1;
    let last = (to / JOB_INTERVAL).floor() as i64;
    (first.max(1)..=last).map(demo_job).collect()
}

/// The `counter`-th job line, which arrives at `counter * JOB_INTERVAL`.
fn demo_job(counter: i64) -> Line {
    if counter % 2 == 1 {
        Line::new(6, "systemd[1]: Finished backup.service - Nightly backup.")
            .with_job(Some(JobResult::Done))
    } else {
        Line::new(
            3,
            "systemd[1]: Failed to start mount-nas.service - NAS mount.",
        )
        .with_job(Some(JobResult::Failed))
    }
}

/// Of the demo journal's lines, the one in this many that `demo_spirits` keeps: a line takes
/// the board about ten seconds to spell, so the full stream would only ever flood it.
const SPIRIT_STRIDE: i64 = 25;
/// Which of each `SPIRIT_STRIDE` lines `demo_spirits` keeps: the fifth, so the second of them, at
/// ten and a half seconds, is the demo's disk error.
const SPIRIT_OFFSET: i64 = 5;

/// What the demo feeds a view that spells the journal one letter at a time: the job lines of
/// `demo_jobs` and every 25th line of `demo`, in the order they arrive. `demo` itself is
/// unchanged, so the views that read it see what they always did.
pub fn demo_spirits(from: f64, to: f64) -> Vec<Line> {
    let first = (from / DEMO_INTERVAL).floor() as i64 + 1;
    let last = (to / DEMO_INTERVAL).floor() as i64;
    let mut arrivals: Vec<(f64, Line)> = (first.max(0)..=last)
        .filter(|counter| counter % SPIRIT_STRIDE == SPIRIT_OFFSET)
        .map(|counter| (counter as f64 * DEMO_INTERVAL, demo_line(counter)))
        .collect();
    let first = (from / JOB_INTERVAL).floor() as i64 + 1;
    let last = (to / JOB_INTERVAL).floor() as i64;
    arrivals.extend(
        (first.max(1)..=last).map(|counter| (counter as f64 * JOB_INTERVAL, demo_job(counter))),
    );
    arrivals.sort_by(|a, b| a.0.total_cmp(&b.0));
    arrivals.into_iter().map(|(_, line)| line).collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retired_delivery_keeps_its_numbering_and_offers_no_lines() {
        let mut totals = Totals::default();
        let mut delivery = Delivery::default();
        delivery.add(&mut totals, vec![Line::new(6, "a"), Line::new(6, "b")], 3);
        assert_eq!(delivery.unseen(0).len(), 2);
        delivery.retire();
        assert!(delivery.lines.is_empty());
        assert_eq!((delivery.through, delivery.dropped), (2, 3));
        assert!(delivery.unseen(0).is_empty() && delivery.unseen(5).is_empty());
    }

    /// macOS, where the follower is a shell and `log` in a group of their own: the reader sees
    /// the end of the stream only once every process holding the pipe is gone.
    #[cfg(target_os = "macos")]
    #[test]
    fn stopping_the_journal_ends_its_reader() {
        let mut journal = Journal::start();
        assert!(journal.stop(), "the reader outlived its follower");
        assert!(journal.child.is_none() && journal.reader.is_none());
    }

    fn lines(first: usize, count: usize) -> Vec<Line> {
        (first..first + count)
            .map(|k| Line::new(6, format!("line {k}")))
            .collect()
    }

    #[test]
    fn a_delivery_numbers_its_lines_so_a_reader_takes_each_once() {
        let mut totals = Totals::default();
        let mut delivery = Delivery::default();
        delivery.add(&mut totals, lines(0, 3), 0);
        assert_eq!(delivery.unseen(0), lines(0, 3));
        assert_eq!(delivery.unseen(3), []);
        delivery.add(&mut totals, lines(3, 2), 4);
        assert_eq!((delivery.through, delivery.dropped), (5, 4));
        assert_eq!(
            delivery.unseen(3),
            lines(3, 2),
            "only the lines after the third"
        );
        // A later sample starts empty and its numbers continue from the totals.
        let mut next = Delivery::default();
        next.add(&mut totals, lines(5, 1), 0);
        assert_eq!(next.unseen(5), lines(5, 1));
        assert_eq!(
            next.unseen(0),
            lines(5, 1),
            "a reader that missed some takes what is there"
        );
        assert_eq!(Delivery::default().unseen(7), []);
    }

    #[test]
    fn a_sample_keeps_the_newest_lines_of_a_flood_and_counts_the_rest_as_dropped() {
        let mut totals = Totals::default();
        let mut delivery = Delivery::default();
        for batch in 0..3 {
            delivery.add(&mut totals, lines(batch * KEPT, KEPT), 1);
        }
        assert_eq!(delivery.lines.len(), KEPT);
        assert_eq!(delivery.through as usize, 3 * KEPT);
        assert_eq!(delivery.unseen(0), lines(2 * KEPT, KEPT));
        assert_eq!(
            delivery.dropped as usize,
            3 + 2 * KEPT,
            "three reported, two samples' worth cut"
        );
    }

    #[test]
    fn the_existing_journal_demo_text_is_unchanged() {
        let lines = demo(0.0, 70.0);
        // Fingerprints taken before the job lines were added: the matrix view's demo is as it was.
        assert_eq!(lines.len(), 200);
        let joined: String = lines
            .iter()
            .map(|line| format!("{} {}\n", line.priority, line.text))
            .collect();
        assert_eq!(hash::fnv1a_64(&joined), 17_593_658_054_892_929_715);
        assert_eq!(
            lines[..4],
            [
                Line::new(
                    4,
                    "gnome-shell[36769]: Window manager warning: last_user_time is ahead"
                ),
                Line::new(
                    6,
                    "kernel: usb 3-9: new high-speed USB device number 9 using xhci_hcd"
                ),
                Line::new(3, "kernel: nvme0: I/O 1012 QID 7 timeout, completion polled"),
                Line::new(6, "systemd[1]: logrotate.service: Deactivated successfully."),
            ]
        );
        assert!(lines.iter().all(|line| line.job.is_none()));
    }

    #[test]
    fn demo_jobs_alternate_a_finished_backup_and_a_failed_mount_every_seven_seconds() {
        assert!(demo_jobs(0.0, 6.99).is_empty());
        let jobs = demo_jobs(0.0, 30.0);
        assert_eq!(jobs.len(), 4, "at 7, 14, 21 and 28 seconds");
        assert_eq!(
            jobs[0],
            Line::new(6, "systemd[1]: Finished backup.service - Nightly backup.")
                .with_job(Some(JobResult::Done))
        );
        assert_eq!(
            jobs[1],
            Line::new(
                3,
                "systemd[1]: Failed to start mount-nas.service - NAS mount."
            )
            .with_job(Some(JobResult::Failed))
        );
        assert_eq!((jobs[2].priority, jobs[3].priority), (6, 3));
        let mut pieces = demo_jobs(-5.0, 9.0);
        pieces.extend(demo_jobs(9.0, 30.0));
        assert_eq!(
            pieces, jobs,
            "splitting the interval loses and repeats nothing"
        );
        assert_eq!(demo_jobs(7.0, 7.0), []);
        assert_eq!(
            demo_jobs(6.9, 7.0).len(),
            1,
            "an interval is open at its start"
        );
    }

    #[test]
    fn the_spirits_demo_merges_jobs_and_a_thin_share_of_the_journal_in_arrival_order() {
        let lines = demo_spirits(0.0, 30.0);
        // Ordinary lines at 1.75, 10.5, 19.25 and 28 seconds; jobs at 7, 14, 21 and 28.
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[0], demo_line(5));
        assert_eq!(lines[1].job, Some(JobResult::Done), "the backup at 7 s");
        assert_eq!(lines[2], demo_line(30), "the disk error at 10.5 s");
        assert!(lines[2].text.starts_with("kernel: nvme0: I/O "));
        assert_eq!(lines[2].priority, 3);
        assert_eq!(lines[3].job, Some(JobResult::Failed), "the mount at 14 s");
        let mut pieces = demo_spirits(0.0, 13.3);
        pieces.extend(demo_spirits(13.3, 30.0));
        assert_eq!(pieces, lines);
    }

    #[test]
    fn job_results_parse_and_failures_are_the_unhappy_ones() {
        assert_eq!(JobResult::parse("done\n"), Some(JobResult::Done));
        assert_eq!(JobResult::parse("dependency"), Some(JobResult::Dependency));
        assert_eq!(JobResult::parse("assert"), None);
        assert_eq!(JobResult::parse(""), None);
        let failed: Vec<bool> = [
            "done",
            "failed",
            "timeout",
            "canceled",
            "dependency",
            "skipped",
        ]
        .iter()
        .map(|value| JobResult::parse(value).expect("known").failed())
        .collect();
        assert_eq!(failed, [false, true, true, true, true, false]);
    }

    #[test]
    fn demo_lines_split_across_intervals_match_one_long_interval() {
        let mut pieces = demo(0.0, 1.3);
        pieces.extend(demo(1.3, 4.0));
        assert_eq!(pieces, demo(0.0, 4.0));
        assert_eq!(pieces.len(), (4.0 / DEMO_INTERVAL) as usize);
    }
}
