//! The system journal as a stream of text lines for the matrix view. The platform's log follower
//! runs as a child process whose output a background thread parses.

use std::io::{BufReader, Read};
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use crate::platform;

/// Seconds between synthetic entries in demo mode.
const DEMO_INTERVAL: f64 = 0.35;
/// Entries held while the view is hidden or paused; later ones are dropped and counted.
const BACKLOG: usize = 2048;
/// syslog severity: 0 emergency to 3 error, 4 warning, 5 notice, 6 info, 7 debug.
const ERROR: u8 = 3;

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

pub struct Journal {
    lines: mpsc::Receiver<Line>,
    dropped: Arc<AtomicU64>,
    child: Option<Child>,
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
                };
            }
        };
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let count = Arc::clone(&dropped);
        std::thread::spawn(move || {
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
}

impl Drop for Journal {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            platform::stop_journal(child);
        }
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
            let hash = (k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32;
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
    fn demo_lines_split_across_intervals_match_one_long_interval() {
        let mut pieces = demo(0.0, 1.3);
        pieces.extend(demo(1.3, 4.0));
        assert_eq!(pieces, demo(0.0, 4.0));
        assert_eq!(pieces.len(), (4.0 / DEMO_INTERVAL) as usize);
    }
}
