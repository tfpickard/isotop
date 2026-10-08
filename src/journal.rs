//! The systemd journal as a stream of text lines for the matrix view. `journalctl -f` runs on a
//! background thread in export format, so binary messages and multi-line fields parse exactly.

use std::io::{self, BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

/// Seconds between synthetic entries in demo mode.
const DEMO_INTERVAL: f64 = 0.35;
/// syslog severity: 0 emergency to 3 error, 4 warning, 5 notice, 6 info, 7 debug.
const INFO: u8 = 6;
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
    child: Option<Child>,
}

impl Journal {
    /// Follows the journal, starting with its most recent entries so the rain begins full.
    pub fn start() -> Self {
        let (sender, lines) = mpsc::channel();
        let spawned = Command::new("journalctl")
            .args([
                "--follow",
                "--lines=200",
                "--output=export",
                "--output-fields=PRIORITY,SYSLOG_IDENTIFIER,_COMM,_PID,MESSAGE",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                let _ = sender.send(Line::new(
                    ERROR,
                    format!("isotop: cannot run journalctl: {error}"),
                ));
                return Self { lines, child: None };
            }
        };
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_entry(&mut reader) {
                    Ok(Some(line)) => {
                        if sender.send(line).is_err() {
                            return;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Line::new(
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
                let _ = sender.send(Line::new(ERROR, format!("journalctl: {line}")));
            }
        });
        Self {
            lines,
            child: Some(child),
        }
    }

    /// Lines that arrived since the last call.
    pub fn drain(&self) -> Vec<Line> {
        self.lines.try_iter().collect()
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Reads one entry of `journalctl --output=export` and formats it as `source[pid]: message`.
/// Returns `None` at the end of the stream. A field is either `NAME=value` on one line or, when
/// the value is binary or spans lines, `NAME`, a little-endian 64-bit length, the bytes, and a
/// newline.
fn read_entry(reader: &mut impl BufRead) -> io::Result<Option<Line>> {
    let mut priority = INFO;
    let mut identifier = None;
    let mut command = None;
    let mut pid = None;
    let mut message = None;
    let mut started = false;
    loop {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(None);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.is_empty() {
            if started {
                break;
            }
            continue;
        }
        started = true;
        let (name, value) = match line.iter().position(|&b| b == b'=') {
            Some(at) => (line[..at].to_vec(), line[at + 1..].to_vec()),
            None => {
                let mut length = [0; 8];
                reader.read_exact(&mut length)?;
                let length = u64::from_le_bytes(length);
                let mut value = Vec::new();
                reader.by_ref().take(length).read_to_end(&mut value)?;
                if value.len() as u64 != length {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                let mut newline = [0; 1];
                reader.read_exact(&mut newline)?;
                (line, value)
            }
        };
        let value = String::from_utf8_lossy(&value).into_owned();
        match name.as_slice() {
            b"PRIORITY" => priority = value.trim().parse().unwrap_or(INFO).min(7),
            b"SYSLOG_IDENTIFIER" => identifier = Some(value),
            b"_COMM" => command = Some(value),
            b"_PID" => pid = Some(value),
            b"MESSAGE" => message = Some(value),
            _ => {}
        }
    }
    let source = identifier.or(command).unwrap_or_else(|| "journal".into());
    let message = message.unwrap_or_default().replace(['\n', '\t'], " ");
    let text = match pid {
        Some(pid) => format!("{source}[{pid}]: {message}"),
        None => format!("{source}: {message}"),
    };
    Ok(Some(Line::new(priority, text)))
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
    fn export_entries_parse_text_and_binary_fields() {
        let mut data =
            b"__CURSOR=s=1\nPRIORITY=3\nSYSLOG_IDENTIFIER=sshd\n_PID=42\nMESSAGE=hello\n\n"
                .to_vec();
        data.extend_from_slice(b"_COMM=bash\nMESSAGE\n");
        data.extend_from_slice(&10_u64.to_le_bytes());
        data.extend_from_slice(b"two\nlines\x01\n\n");
        data.extend_from_slice(b"PRIORITY=junk\nMESSAGE=no source\n\n");
        let mut reader = &data[..];
        assert_eq!(
            read_entry(&mut reader).unwrap(),
            Some(Line::new(3, "sshd[42]: hello"))
        );
        assert_eq!(
            read_entry(&mut reader).unwrap(),
            Some(Line::new(INFO, "bash: two lines\u{1}"))
        );
        assert_eq!(
            read_entry(&mut reader).unwrap(),
            Some(Line::new(INFO, "journal: no source"))
        );
        assert_eq!(read_entry(&mut reader).unwrap(), None);
    }

    #[test]
    fn a_truncated_binary_field_is_an_error() {
        let mut data = b"MESSAGE\n".to_vec();
        data.extend_from_slice(&100_u64.to_le_bytes());
        data.extend_from_slice(b"short");
        assert!(read_entry(&mut &data[..]).is_err());
    }

    #[test]
    fn demo_lines_split_across_intervals_match_one_long_interval() {
        let mut pieces = demo(0.0, 1.3);
        pieces.extend(demo(1.3, 4.0));
        assert_eq!(pieces, demo(0.0, 4.0));
        assert_eq!(pieces.len(), (4.0 / DEMO_INTERVAL) as usize);
    }
}
