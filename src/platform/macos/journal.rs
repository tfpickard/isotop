//! The macOS unified log: `log show` for the last two minutes, then `log stream`, both as
//! newline-delimited JSON. The parser is std-only and compiled into Linux test builds too.

use std::io::{self, BufRead};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use crate::journal::Line;
use crate::platform::JournalParser;

/// Name of the log follower, for error messages.
pub const JOURNAL: &str = "log";

/// History first, so the rain begins full, then the live stream. `log show` needs `--info` to
/// include info messages; `log stream` takes `--level`. `exec` leaves `log stream` as the child
/// itself, but while `log show` runs the child is the shell waiting for it, so the follower runs
/// in a process group of its own and `stop_journal` ends the whole group.
const FOLLOW: &str = "/usr/bin/log show --last 2m --style ndjson --info; \
                      exec /usr/bin/log stream --style ndjson --level info";

/// Bytes read of any one line; the rest of a longer line is read and discarded.
const LINE: usize = 64 * 1024;
/// Bytes kept of the message; unified log messages can be many kilobytes long.
const MESSAGE: usize = 4096;
/// Bytes kept of the other string fields and of keys.
const FIELD: usize = 512;

/// syslog severity: 0 emergency to 3 error, 4 warning, 5 notice, 6 info, 7 debug.
const CRITICAL: u8 = 2;
const ERROR: u8 = 3;
const NOTICE: u8 = 5;
const INFO: u8 = 6;
const DEBUG: u8 = 7;

/// Follows the unified log, starting with its most recent entries so the rain begins full. The
/// child leads a new process group, which holds `log show` too.
pub fn journal() -> io::Result<(Child, JournalParser)> {
    let child = Command::new("/bin/sh")
        .args(["-c", FOLLOW])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    Ok((child, read_entry))
}

/// Reads lines until one is a log event and returns it as `source[pid]: message`; `None` at the
/// end of the stream. Lines that are not JSON objects (the "Filtering the log data" banner,
/// blanks) and records without a message (the trailing `finished` record) are skipped.
fn read_entry(reader: &mut dyn BufRead) -> io::Result<Option<Line>> {
    loop {
        let Some(line) = read_line(reader)? else {
            return Ok(None);
        };
        if let Some(entry) = parse_line(&line) {
            return Ok(Some(entry));
        }
    }
}

/// Reads up to the next newline, which is consumed but not returned, keeping at most `LINE`
/// bytes. Returns `None` at the end of the stream.
fn read_line(reader: &mut dyn BufRead) -> io::Result<Option<Vec<u8>>> {
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
        let room = LINE.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..end.min(room)]);
        reader.consume(newline.map_or(end, |at| at + 1));
        if newline.is_some() {
            return Ok(Some(kept));
        }
    }
}

/// The top-level fields of one record that isotop shows.
#[derive(Default)]
struct Fields {
    message: Option<String>,
    kind: Option<String>,
    image: Option<String>,
    subsystem: Option<String>,
    pid: Option<u32>,
}

/// Formats one ndjson record, or `None` when it is not a log event with something to say. A line
/// cut off at `LINE` bytes still yields the fields that were complete before the cut.
fn parse_line(line: &[u8]) -> Option<Line> {
    let fields = scan(line);
    let message = fields.message?;
    if message.trim().is_empty() {
        return None;
    }
    let message = message.replace(['\n', '\r', '\t'], " ");
    let source = fields
        .image
        .as_deref()
        .and_then(basename)
        .or(fields.subsystem.as_deref().filter(|name| !name.is_empty()))
        .unwrap_or("log");
    let text = match fields.pid {
        Some(pid) => format!("{source}[{pid}]: {message}"),
        None => format!("{source}: {message}"),
    };
    Some(Line::new(severity(fields.kind.as_deref()), text))
}

/// The last component of a path, if it has one.
fn basename(path: &str) -> Option<&str> {
    path.rsplit('/').find(|part| !part.is_empty())
}

/// syslog severity of a unified log `messageType`.
fn severity(kind: Option<&str>) -> u8 {
    let Some(kind) = kind else {
        return NOTICE;
    };
    [
        ("Fault", CRITICAL),
        ("Error", ERROR),
        ("Default", NOTICE),
        ("Info", INFO),
        ("Debug", DEBUG),
    ]
    .into_iter()
    .find(|(name, _)| kind.eq_ignore_ascii_case(name))
    .map_or(NOTICE, |(_, severity)| severity)
}

/// Collects the wanted top-level keys of the JSON object on a line. Nested values are skipped
/// whole, so a key inside `backtrace` or inside a string never shadows a top-level one. Stops at
/// the first thing that is not valid JSON, keeping what it has; the first of duplicate keys wins.
fn scan(line: &[u8]) -> Fields {
    let mut fields = Fields::default();
    let mut cursor = Cursor { bytes: line, at: 0 };
    let _ = cursor.object(&mut fields);
    fields
}

/// A position in one line of JSON. Every method returns `None` at malformed or cut-off input.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn blanks(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.peek()? == byte).then(|| self.at += 1)
    }

    fn object(&mut self, fields: &mut Fields) -> Option<()> {
        self.blanks();
        self.expect(b'{')?;
        loop {
            self.blanks();
            match self.peek()? {
                b'}' => return Some(()),
                b',' => {
                    self.at += 1;
                    continue;
                }
                b'"' => {}
                _ => return None,
            }
            let key = self.string(FIELD)?;
            self.blanks();
            self.expect(b':')?;
            self.blanks();
            match key.as_slice() {
                b"eventMessage" => keep(&mut fields.message, self.text(MESSAGE)?),
                b"messageType" => keep(&mut fields.kind, self.text(FIELD)?),
                b"processImagePath" => keep(&mut fields.image, self.text(FIELD)?),
                b"subsystem" => keep(&mut fields.subsystem, self.text(FIELD)?),
                b"processID" => {
                    let number = self.value()?;
                    if fields.pid.is_none() {
                        fields.pid = number.and_then(|digits| {
                            std::str::from_utf8(digits).ok()?.parse::<u32>().ok()
                        });
                    }
                }
                _ => self.skip()?,
            }
        }
    }

    /// A string value, or `Some(None)` after skipping a value of any other type.
    fn text(&mut self, limit: usize) -> Option<Option<String>> {
        if self.peek()? == b'"' {
            let bytes = self.string(limit)?;
            Some(Some(String::from_utf8_lossy(&bytes).into_owned()))
        } else {
            self.skip()?;
            Some(None)
        }
    }

    /// A bare scalar (number, `true`, `false`, `null`) as its text, or `Some(None)` after
    /// skipping a string, object or array. A scalar cut off by the end of the line is `None`.
    fn value(&mut self) -> Option<Option<&[u8]>> {
        if matches!(self.peek()?, b'"' | b'{' | b'[') {
            self.skip()?;
            return Some(None);
        }
        let start = self.at;
        self.at += self.bytes[start..]
            .iter()
            .take_while(|&&b| !b.is_ascii_whitespace() && !matches!(b, b',' | b'}' | b']'))
            .count();
        // A scalar must be followed by something, or it may have been cut short.
        (self.at > start && self.at < self.bytes.len()).then(|| Some(&self.bytes[start..self.at]))
    }

    /// Skips one value of any type, honouring strings and their escapes at any depth.
    fn skip(&mut self) -> Option<()> {
        match self.peek()? {
            b'"' => self.skip_string(),
            b'{' | b'[' => {
                let mut depth = 0_usize;
                loop {
                    match self.peek()? {
                        b'"' => {
                            self.skip_string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => depth -= 1,
                        _ => {}
                    }
                    self.at += 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
            }
            _ => self.value().map(|_| ()),
        }
    }

    fn skip_string(&mut self) -> Option<()> {
        self.expect(b'"')?;
        loop {
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(());
                }
                // The escaped byte is never a closing quote; `\uXXXX` digits are plain bytes.
                b'\\' => self.at += 2,
                _ => self.at += 1,
            }
        }
    }

    /// Decodes a string, keeping at most `limit` bytes of it. Raw bytes pass through (the caller
    /// decodes UTF-8); escapes are decoded, a surrogate pair into one character and a lone
    /// surrogate or `\u0000` into U+FFFD.
    fn string(&mut self, limit: usize) -> Option<Vec<u8>> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            let byte = self.peek()?;
            self.at += 1;
            let decoded = match byte {
                b'"' => return Some(out),
                b'\\' => {
                    let escape = self.peek()?;
                    self.at += 1;
                    match escape {
                        b'"' | b'\\' | b'/' => escape as char,
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode()?,
                        _ => return None,
                    }
                }
                _ => {
                    if out.len() < limit {
                        out.push(byte);
                    }
                    continue;
                }
            };
            if out.len() < limit {
                let mut buffer = [0; 4];
                out.extend_from_slice(decoded.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }

    /// The character of a `\uXXXX` escape whose `\u` is consumed, reading a following low
    /// surrogate escape when the first is a high one.
    fn unicode(&mut self) -> Option<char> {
        let first = self.hex4()?;
        match first {
            0xd800..=0xdbff => {
                let saved = self.at;
                if self.bytes.get(self.at..self.at + 2) == Some(b"\\u") {
                    self.at += 2;
                    if let Some(second @ 0xdc00..=0xdfff) = self.hex4() {
                        let scalar = 0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00);
                        return Some(char::from_u32(scalar).unwrap_or('\u{fffd}'));
                    }
                }
                // Not followed by a low surrogate: leave whatever follows to be read normally.
                self.at = saved;
                Some('\u{fffd}')
            }
            0 | 0xdc00..=0xdfff => Some('\u{fffd}'),
            _ => Some(char::from_u32(first).unwrap_or('\u{fffd}')),
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.bytes.get(self.at..self.at + 4)?;
        let mut value = 0;
        for &digit in digits {
            value = value * 16 + (digit as char).to_digit(16)?;
        }
        self.at += 4;
        Some(value)
    }
}

/// Stores the first value seen for a field.
fn keep(slot: &mut Option<String>, value: Option<String>) {
    if slot.is_none() {
        *slot = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Option<Line> {
        parse_line(line.as_bytes())
    }

    /// Linux runs the follower's shell too, though `log` is missing there, so the gates can see
    /// that it leads a process group of its own, the group `stop_journal` ends on macOS.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_follower_leads_a_process_group_of_its_own() {
        let (mut child, _) = journal().unwrap();
        let pid = child.id();
        // Until it is reaped, even a shell that has exited keeps its stat: "pid (comm) state
        // ppid pgrp ...", where comm may hold spaces and parentheses.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let after = &stat[stat.rfind(')').unwrap() + 1..];
        let group: u32 = after.split_whitespace().nth(2).unwrap().parse().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(group, pid, "{stat}");
    }

    const TRUSTD: &str = r#"{"traceID":1824493551157252,"eventMessage":"ocsp responder: (null) did not include status of requested cert","eventType":"logEvent","source":null,"formatString":"ocsp responder: %@ did not include status of requested cert","activityIdentifier":8049628,"subsystem":"com.apple.securityd","category":"ocsp","threadID":13337131,"senderImageUUID":"AD8A1343-96A6-3E87-8CFA-14FE13754B06","backtrace":{"frames":[{"imageOffset":256208,"imageUUID":"AD8A1343-96A6-3E87-8CFA-14FE13754B06"}]},"bootUUID":"","processImagePath":"/usr/libexec/trustd","timestamp":"2021-04-08 22:28:22.002306-0500","senderImagePath":"/usr/libexec/trustd","machTimestamp":51793593259945,"messageType":"Default","processImageUUID":"AD8A1343-96A6-3E87-8CFA-14FE13754B06","processID":565,"senderProgramCounter":256208,"parentActivityIdentifier":0,"timezoneName":""}"#;

    #[test]
    fn a_real_log_event_becomes_source_pid_and_message() {
        assert_eq!(
            parse(TRUSTD),
            Some(Line::new(
                NOTICE,
                "trustd[565]: ocsp responder: (null) did not include status of requested cert"
            ))
        );
    }

    #[test]
    fn message_types_map_to_syslog_severities() {
        let line = |kind: &str| {
            parse(&format!(
                r#"{{"messageType":"{kind}","eventMessage":"m","processID":1}}"#
            ))
            .unwrap()
            .priority
        };
        assert_eq!(line("Fault"), CRITICAL);
        assert_eq!(line("Error"), ERROR);
        assert_eq!(line("Default"), NOTICE);
        assert_eq!(line("Info"), INFO);
        assert_eq!(line("Debug"), DEBUG);
        assert_eq!(line("Activity"), NOTICE);
        assert_eq!(line(""), NOTICE);
        assert_eq!(parse(r#"{"eventMessage":"m"}"#).unwrap().priority, NOTICE);
        assert_eq!(
            parse(r#"{"messageType":7,"eventMessage":"m"}"#)
                .unwrap()
                .priority,
            NOTICE
        );
    }

    #[test]
    fn the_source_falls_back_to_the_subsystem_then_to_log() {
        assert_eq!(
            parse(r#"{"eventMessage":"hi","subsystem":"com.apple.net","processID":3}"#),
            Some(Line::new(NOTICE, "com.apple.net[3]: hi"))
        );
        assert_eq!(
            parse(r#"{"eventMessage":"hi","processImagePath":"","subsystem":""}"#),
            Some(Line::new(NOTICE, "log: hi"))
        );
        assert_eq!(
            parse(r#"{"eventMessage":"hi","processImagePath":"/Applications/Safari.app/"}"#),
            Some(Line::new(NOTICE, "Safari.app: hi"))
        );
    }

    #[test]
    fn escapes_are_decoded() {
        let line =
            parse(r#"{"eventMessage":"q\" b\\ s\/ \b\f\n\r\t eé \u001b[2J nul\u0000 ☃"}"#).unwrap();
        // Newline, return and tab become spaces; the escape character is left for the
        // terminal layer's own sanitising, and a NUL becomes U+FFFD.
        assert_eq!(
            line.text,
            "log: q\" b\\ s/ \u{8}\u{c}    e\u{e9} \u{1b}[2J nul\u{fffd} \u{2603}"
        );
    }

    #[test]
    fn surrogate_pairs_combine_and_lone_surrogates_are_replaced() {
        let text = |json: &str| parse(json).unwrap().text;
        assert_eq!(text(r#"{"eventMessage":"😀!"}"#), "log: \u{1f600}!");
        assert_eq!(text(r#"{"eventMessage":"a\ud83dB"}"#), "log: a\u{fffd}B");
        assert_eq!(text(r#"{"eventMessage":"\ude00"}"#), "log: \u{fffd}");
        // A high surrogate followed by another high one: both are lone.
        assert_eq!(
            text(r#"{"eventMessage":"\ud83d\ud83d"}"#),
            "log: \u{fffd}\u{fffd}"
        );
        // A high surrogate followed by an ordinary escape keeps that escape.
        assert_eq!(text(r#"{"eventMessage":"\ud83dA"}"#), "log: \u{fffd}A");
        // Raw UTF-8 passes through.
        assert_eq!(
            text("{\"eventMessage\":\"caf\u{e9} \u{1f600}\"}"),
            "log: caf\u{e9} \u{1f600}"
        );
    }

    #[test]
    fn malformed_escapes_drop_the_line() {
        assert_eq!(parse(r#"{"eventMessage":"bad \x escape"}"#), None);
        assert_eq!(parse(r#"{"eventMessage":"short \u12"}"#), None);
        assert_eq!(parse(r#"{"eventMessage":"hex \u12zz"}"#), None);
    }

    #[test]
    fn only_top_level_keys_count() {
        let line = parse(
            r#"{"backtrace":{"frames":[{"eventMessage":"nested","processID":1,"x":"}]}"}]},"source":{"processImagePath":"/no/nested"},"eventMessage":"top","processID":2}"#,
        )
        .unwrap();
        assert_eq!(line.text, "log[2]: top");
    }

    #[test]
    fn key_names_inside_strings_are_not_keys() {
        let line = parse(
            r#"{"formatString":"\"eventMessage\":\"fake\",\"processID\":99","eventMessage":"real \"processID\":98 \\","processID":7}"#,
        )
        .unwrap();
        assert_eq!(line.text, "log[7]: real \"processID\":98 \\");
    }

    #[test]
    fn the_first_of_duplicate_keys_wins_and_key_order_does_not_matter() {
        assert_eq!(
            parse(r#"{"processID":4,"eventMessage":"one","eventMessage":"two","processID":5}"#)
                .unwrap()
                .text,
            "log[4]: one"
        );
    }

    #[test]
    fn process_ids_that_are_not_plain_integers_are_ignored() {
        for pid in ["-1", "1.5", "1e3", "null", "\"7\"", "99999999999"] {
            let json = format!(r#"{{"processID":{pid},"eventMessage":"m"}}"#);
            assert_eq!(parse(&json).unwrap().text, "log: m", "{pid}");
        }
        assert_eq!(
            parse(r#"{"processID":0,"eventMessage":"m"}"#).unwrap().text,
            "log[0]: m"
        );
    }

    #[test]
    fn whitespace_around_tokens_is_allowed() {
        assert_eq!(
            parse(
                "  { \"eventMessage\" : \"m\" ,\r\n \"processID\" : 12 , \"x\" : [ 1 , { } ] }\r"
            )
            .unwrap()
            .text,
            "log[12]: m"
        );
    }

    #[test]
    fn records_that_are_not_log_events_are_skipped() {
        assert_eq!(
            parse(r#"{"finished":1,"eventCount":123,"timestamp":"2021"}"#),
            None
        );
        assert_eq!(parse(r#"{"eventMessage":null,"processID":1}"#), None);
        assert_eq!(parse(r#"{"eventMessage":12}"#), None);
        assert_eq!(parse(r#"{"eventMessage":"  "}"#), None);
        assert_eq!(parse(r#"{"eventMessage":""}"#), None);
        assert_eq!(parse(r#"{}"#), None);
        assert_eq!(parse(r#"["eventMessage"]"#), None);
        assert_eq!(parse(r#"Filtering the log data using "x""#), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
    }

    #[test]
    fn truncated_lines_never_panic_and_give_only_complete_fields() {
        for end in 0..TRUSTD.len() {
            let _ = parse_line(&TRUSTD.as_bytes()[..end]);
        }
        let cut = |end: usize| parse_line(&TRUSTD.as_bytes()[..end]);
        let message_end = TRUSTD.find(r#","eventType""#).unwrap();
        // Cut inside the message: nothing to show.
        assert_eq!(cut(message_end - 1), None);
        // Cut after it but before the process id: the message alone.
        let line = cut(message_end + 3).unwrap();
        assert!(line.text.starts_with("log: ocsp responder"));
        // Cut in the middle of the process id's digits: the number is not trusted.
        let pid_end = TRUSTD.find(r#""processID":565"#).unwrap() + r#""processID":56"#.len();
        assert!(!cut(pid_end).unwrap().text.contains('['));
        // Cut right after it, before the comma: also not trusted, it may have been longer.
        assert!(!cut(pid_end + 1).unwrap().text.contains('['));
        assert!(cut(pid_end + 2).unwrap().text.contains("[565]"));
    }

    #[test]
    fn garbage_never_panics() {
        for junk in [
            "{",
            "{\"",
            "{\"a",
            "{\"a\"",
            "{\"a\":",
            "{\"a\":\"",
            "{\"a\":[",
            "{\"a\":[\"\\",
            "{\"a\":{\"b\":[}",
            "{\"a\":}",
            "{\"a\":]}",
            "{\"a\" \"b\"}",
            "{]}",
            "{\"a\":{}}}}}",
            "{\"a\":[]]]]",
            "{\"eventMessage\":\"\\ud83d\\u",
            "{\"eventMessage\":\"\\",
            "\u{0}\u{1}\u{2}",
        ] {
            let _ = parse(junk);
        }
        let _ = parse_line(&[0xff, 0xfe, b'{', 0x80]);
        let _ = parse_line(b"{\"eventMessage\":\"\xff\xfe\"}");
    }

    #[test]
    fn invalid_utf8_in_a_message_is_replaced() {
        let line = parse_line(b"{\"eventMessage\":\"a\xffb\"}").unwrap();
        assert_eq!(line.text, "log: a\u{fffd}b");
    }

    #[test]
    fn long_messages_are_cut_at_a_fixed_size() {
        let json = format!(
            r#"{{"eventMessage":"{}","processID":3}}"#,
            "y".repeat(MESSAGE * 3)
        );
        let line = parse(&json).unwrap();
        assert_eq!(line.text.len(), "log[3]: ".len() + MESSAGE);
    }

    #[test]
    fn the_stream_skips_banners_and_blanks_and_ends_cleanly() {
        let mut data = String::from("Filtering the log data using \"composedMessage ~= x\"\n\n");
        data.push_str(TRUSTD);
        data.push('\n');
        data.push_str("\r\n");
        data.push_str(r#"{"eventMessage":"second","messageType":"Error","processID":2,"processImagePath":"/bin/sh"}"#);
        data.push('\n');
        data.push_str(r#"{"finished":1}"#);
        data.push('\n');
        // No final newline: a partial last line is still parsed if complete.
        data.push_str(r#"{"eventMessage":"last"}"#);
        let mut reader = io::BufReader::with_capacity(16, data.as_bytes());
        let first = read_entry(&mut reader).unwrap().unwrap();
        assert!(first.text.starts_with("trustd[565]: ocsp"));
        assert_eq!(
            read_entry(&mut reader).unwrap(),
            Some(Line::new(ERROR, "sh[2]: second"))
        );
        assert_eq!(
            read_entry(&mut reader).unwrap(),
            Some(Line::new(NOTICE, "log: last"))
        );
        assert_eq!(read_entry(&mut reader).unwrap(), None);
        assert_eq!(read_entry(&mut reader).unwrap(), None);
    }

    #[test]
    fn oversized_lines_are_cut_and_the_stream_stays_in_step() {
        let huge = "z".repeat(LINE * 3);
        let mut data = format!("{{\"eventMessage\":\"{huge}\"}}\n");
        // The message is complete before the cut, so it survives.
        data.push_str(&format!(
            "{{\"eventMessage\":\"early\",\"pad\":\"{huge}\"}}\n"
        ));
        data.push_str("{\"eventMessage\":\"after\"}\n");
        let mut reader = io::BufReader::with_capacity(512, data.as_bytes());
        assert_eq!(
            read_entry(&mut reader).unwrap(),
            Some(Line::new(NOTICE, "log: early"))
        );
        assert_eq!(
            read_entry(&mut reader).unwrap(),
            Some(Line::new(NOTICE, "log: after"))
        );
        assert_eq!(read_entry(&mut reader).unwrap(), None);
    }

    #[test]
    fn the_follower_command_shows_history_before_streaming() {
        let show = FOLLOW
            .find("log show --last 2m --style ndjson --info")
            .unwrap();
        let stream = FOLLOW
            .find("exec /usr/bin/log stream --style ndjson --level info")
            .unwrap();
        assert!(show < stream);
    }
}
