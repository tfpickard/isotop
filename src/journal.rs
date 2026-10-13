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

/// One journal entry as text, with its syslog severity.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub priority: u8,
    pub text: String,
}

impl Line {
    pub fn new(priority: u8, text: impl Into<String>) -> Self {
        Self {
            priority,
            text: text.into(),
        }
    }
}

/// The journal lines that arrived while one sample was the newest, which travel on that
/// `Snapshot` (`Snapshot::journal`) to the view that draws them. `main.rs` attaches what the
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
    let first = (from / DEMO_INTERVAL).floor() as i64 + 1;
    let last = (to / DEMO_INTERVAL).floor() as i64;
    (first.max(0)..=last)
        .map(|k| {
            let hash = hash::golden_gamma_high(k as u64);
            let (priority, template) = LINES[hash as usize % LINES.len()];
            let text = template
                .replace("{n}", &(hash % 9 + 1).to_string())
                .replace("{p}", &(1000 + hash % 60000).to_string());
            Line::new(priority, text)
        })
        .collect()
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
    fn demo_lines_split_across_intervals_match_one_long_interval() {
        let mut pieces = demo(0.0, 1.3);
        pieces.extend(demo(1.3, 4.0));
        assert_eq!(pieces, demo(0.0, 4.0));
        assert_eq!(pieces.len(), (4.0 / DEMO_INTERVAL) as usize);
    }
}
