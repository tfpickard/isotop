//! The systemd journal: `journalctl -f` in export format, so binary messages and multi-line
//! fields parse exactly.

use std::io::{self, BufRead, Read};
use std::process::{Child, Command, Stdio};

use crate::journal::Line;
use crate::platform::JournalParser;

/// Bytes kept of any one field; journal messages can be megabytes long.
const FIELD: usize = 4096;
/// syslog severity: 0 emergency to 3 error, 4 warning, 5 notice, 6 info, 7 debug.
const INFO: u8 = 6;

/// Follows the journal, starting with its most recent entries so the rain begins full.
pub fn journal() -> io::Result<(Child, JournalParser)> {
    let child = Command::new("journalctl")
        .args([
            "--follow",
            "--lines=200",
            "--output=export",
            "--output-fields=PRIORITY,SYSLOG_IDENTIFIER,_COMM,_PID,MESSAGE",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    Ok((child, parse))
}

/// `read_entry` over a type-erased reader, as `JournalParser` needs.
fn parse(mut reader: &mut dyn BufRead) -> io::Result<Option<Line>> {
    read_entry(&mut reader)
}

/// Reads one entry of `journalctl --output=export` and formats it as `source[pid]: message`.
/// Returns `None` at the end of the stream. A field is either `NAME=value` on one line or, when
/// the value is binary or spans lines, `NAME`, a little-endian 64-bit length, the bytes, and a
/// newline. Each field keeps at most `FIELD` bytes; the rest is read and discarded.
fn read_entry(reader: &mut impl BufRead) -> io::Result<Option<Line>> {
    let mut priority = INFO;
    let mut identifier = None;
    let mut command = None;
    let mut pid = None;
    let mut message = None;
    let mut started = false;
    loop {
        let Some(line) = read_line(reader)? else {
            return Ok(None);
        };
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
                let kept = length.min(FIELD as u64);
                let mut value = Vec::new();
                reader.by_ref().take(kept).read_to_end(&mut value)?;
                let skipped = io::copy(&mut reader.by_ref().take(length - kept), &mut io::sink())?;
                if value.len() as u64 + skipped != length {
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

/// Reads up to the next newline, which is consumed but not returned, keeping at most `FIELD`
/// bytes. Returns `None` at the end of the stream.
fn read_line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut kept = Vec::new();
    let mut read_any = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(read_any.then_some(kept));
        }
        read_any = true;
        let newline = buffer.iter().position(|&b| b == b'\n');
        let end = newline.unwrap_or(buffer.len());
        let room = FIELD.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..end.min(room)]);
        reader.consume(newline.map_or(end, |at| at + 1));
        if newline.is_some() {
            return Ok(Some(kept));
        }
    }
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
    fn oversized_fields_are_cut_short_and_the_stream_stays_in_step() {
        let huge = "y".repeat(FIELD * 50);
        let mut data = format!("MESSAGE={huge}\n\n").into_bytes();
        data.extend_from_slice(b"MESSAGE\n");
        data.extend_from_slice(&((FIELD * 50) as u64).to_le_bytes());
        data.extend_from_slice(huge.as_bytes());
        data.extend_from_slice(b"\n\nMESSAGE=after\n\n");
        let mut reader = io::BufReader::with_capacity(512, &data[..]);
        // A text field's cap includes its `MESSAGE=` name; a binary field's covers the value only.
        for kept in [FIELD - "MESSAGE=".len(), FIELD] {
            let line = read_entry(&mut reader).unwrap().unwrap();
            assert_eq!(line.text.len(), "journal: ".len() + kept);
        }
        assert_eq!(
            read_entry(&mut reader).unwrap(),
            Some(Line::new(INFO, "journal: after"))
        );
    }

    #[test]
    fn a_truncated_binary_field_is_an_error() {
        let mut data = b"MESSAGE\n".to_vec();
        data.extend_from_slice(&100_u64.to_le_bytes());
        data.extend_from_slice(b"short");
        assert!(read_entry(&mut &data[..]).is_err());
    }
}
