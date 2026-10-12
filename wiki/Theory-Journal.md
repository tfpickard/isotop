# Theory: The journal

The [Matrix view](View-Matrix) does not draw processes; it draws the machine's log. That makes the journal the one
data source that is *text written by other programs*: untrusted, unbounded, arriving at the log's pace rather than
isotop's, and produced by a follower process that isotop must start, feed from, bound, and stop. This page is the
pipeline from `journalctl` (Linux) or `log` (macOS) to the lines on screen: the formats and their parsers, the
reader thread and its three bounded stages, the linger that keeps the follower alive after you leave the view, the
deterministic demo lines, and where untrusted text is neutralised.

## 1. The sources (theory)

**systemd journal.** `journald` stores log entries as sets of named fields. The ones isotop wants are `MESSAGE`,
`PRIORITY` (a syslog severity, 0 emergency to 7 debug; see RFC 5424), `SYSLOG_IDENTIFIER` (the program's own tag),
`_COMM` (the kernel's `comm` of the sender) and `_PID`. `journalctl --output=export` writes entries in a
byte-exact **export format** designed for lossless transport:

* an entry is a sequence of fields, each ending in a newline, **terminated by an empty line**;
* a text field is `NAME=value\n`;
* a field whose value is binary or contains a newline is `NAME\n`, then the length as a **little-endian 64-bit
  integer**, then exactly that many bytes, then a newline.

Because the binary form carries a length, a `MESSAGE` with embedded newlines, control characters or invalid UTF-8
parses unambiguously; no escaping can be fooled. Without it a log line is a trust problem.

**macOS unified log.** The `log` tool reads the unified logging system. `log show` prints history, `log stream`
follows live; both can emit newline-delimited JSON (`--style ndjson`), one record per line, with fields such as
`eventMessage`, `messageType` (`Fault`, `Error`, `Default`, `Info`, `Debug`), `processImagePath`, `subsystem` and
`processID`. The unified log has no warning level, so nothing is amber there.

## 2. The Linux follower and parser

`platform::journal()` (in `platform/linux/journal.rs`) spawns

```
journalctl --follow --lines=200 --output=export \
           --output-fields=PRIORITY,SYSLOG_IDENTIFIER,_COMM,_PID,MESSAGE
```

with stdin null and stdout and stderr piped. `--lines=200` starts with the 200 most recent entries so the rain begins
full; `--follow` then streams. `read_entry` consumes one entry per call:

* `read_line` reads to the next newline, **keeping at most `FIELD = 4096` bytes** and discarding the rest of an
  overlong line while still advancing the stream, so a megabyte `MESSAGE` cannot grow memory and cannot desynchronise
  the parser.
* A line with `=` is a text field (name before the first `=`). A line without is a binary field's name: read the
  eight length bytes, keep at most 4096 of the value, **read and discard** the remainder with `io::copy` into a sink,
  and read the trailing newline. If the stream ends before `length` bytes, the result is `UnexpectedEof`, an
  error, not a short message.
* Values are decoded lossily (`from_utf8_lossy`); a `PRIORITY` that does not parse as a number reads as 6 (info),
  and is capped at 7.
* Only the five requested fields are kept; others (journalctl always adds cursor and timestamp metadata in export
  mode) are read and ignored.
* An empty line ends the entry. The line shown is `source[pid]: message`, where `source` is `SYSLOG_IDENTIFIER`,
  else `_COMM`, else `"journal"`; `[pid]` is omitted when there is no `_PID`; newlines and tabs in the message become
  spaces.

The size cap is applied slightly differently to the two forms, and a test records it: for a text field it includes
the `MESSAGE=` name, for a binary field only the value.

## 3. The macOS follower and parser

`platform/macos/journal.rs` runs, in `/bin/sh -c` in a **process group of its own**:

```
/usr/bin/log show --last 2m --style ndjson --info; exec /usr/bin/log stream --style ndjson --level info
```

History first (the last two minutes, again so the rain starts full), then the live stream. The shell is the child
while `log show` runs, so killing the child alone would leave `log show` orphaned and running; `stop_journal` therefore
`killpg(SIGKILL)`s the whole group (falling back to `kill` if that fails) and reaps it.

The parser is a small **std-only JSON scanner**, compiled into Linux test builds so its tests run there, not a general
JSON library. It reads one line (at most `LINE = 64 KiB`, the rest discarded), then walks the top-level object, keeping
only `eventMessage` (at most `MESSAGE = 4096` bytes), `messageType`, `processImagePath`, `subsystem` (each at most
`FIELD = 512`) and `processID`. Nested values are skipped whole honouring strings and escapes, so a key buried inside
`backtrace` or inside a string never shadows a top-level one; escapes, surrogate pairs and a lone surrogate or
`\u0000` (which become U+FFFD) are decoded; the first of duplicate keys wins; and parsing **stops at the first
malformed byte keeping what it had**, so a record cut at the length cap still yields its early fields. Banner lines
that are not JSON objects and records with no message (the trailing `finished` record) are skipped.

The displayed source is the basename of `processImagePath`, else `subsystem`, else `"log"`. Severity maps
`Fault` to 2, `Error` to 3, `Default` to 5, `Info` to 6, `Debug` to 7, and anything else (or none) to 5.

## 4. The reader thread and three bounded stages

`Journal::start` (in `journal.rs`) owns the follower and a reader thread:

```mermaid
flowchart LR
    A["follower<br/>(journalctl / log)"] -->|"stdout pipe"| B["reader thread<br/>BufReader + read_entry"]
    B -->|"try_send"| C["sync_channel<br/>BACKLOG = 2048"]
    C -->|"drain() each frame"| D["Rain.pending<br/>(2 screens)"]
    D -->|"one line at a time"| E["rows on screen"]
    B -. "channel full: count" .-> F["Journal.dropped"]
    D -. "trim oldest: skipped += 1" .-> G["Rain.skipped"]
```

1. **Pipe and reader.** The reader thread parses entries from the follower's stdout. It sends each with
   **`try_send`**, never blocking: if the channel is full the line is *dropped and counted* in an `AtomicU64`; if the
   receiver is gone the thread returns. A parse error sends one `isotop: journal read failed: …` line (severity 3) and
   ends. At the end of the stream it reads the follower's stderr and sends each non-blank line as `journalctl: …`
   (severity 3), so a failure to authorise, or a missing binary, shows up in the rain instead of vanishing. A spawn
   failure sends `isotop: cannot run journalctl: …` through the same path and leaves no child or reader.
2. **Channel.** `mpsc::sync_channel(BACKLOG)` holds at most `BACKLOG = 2048` lines. This is the backlog "while the
   view is hidden or paused": the main loop only drains it while the Matrix view is shown and the app is not paused,
   so a flood during a pause fills it and the overflow is counted.
3. **The rain.** Each frame, `feed` moves `drain()`ed lines into `Rain::push`, and adds `Journal::dropped()` (which
   `swap`s the counter back to 0) to `Rain::skipped`. `Rain` keeps at most `max(rows, 16) · 2` pending lines, two
   screens' worth, trimming the *oldest* and counting each as skipped, but not before the first frame has sized the
   grid, so the startup history survives. Lines are written into the screen **one at a time, faster when they
   queue**: the next release is `0.5 / (1 + pending/3)` seconds after the last, and a burst after a gap is limited to
   one second's worth. The legend shows `N queued, M skipped`.

Net: the stream is lossy by design and says so. There is no point where the frame loop can block on the log.

## 5. Lifecycle and the linger

The follower costs a process and a thread, so it runs only while it is wanted:

* At startup, if the first view demands `Source::Journal` and the run is live, `Journal::start` is called before the
  first sample. In `--demo` no follower is started.
* Each frame (live runs), `App::journal_wanted` records when the demand last contained `Source::Journal` and says the
  follower is wanted while that was **less than `JOURNAL_LINGER = 30 s` ago**. Leaving the Matrix therefore does not
  kill the follower: a quick look at another view and back keeps the same stream and its backlog. After 30 s the
  `Journal` value is dropped, and coming back starts a new `journalctl` with `--lines=200` again.
* `Drop for Journal` calls `stop`: `platform::stop_journal` ends the follower (on Linux `kill` and `wait`; on macOS the
  group), then waits up to `READER_GRACE = 500 ms` for the reader to see end-of-stream. A reader still blocked after
  that (some process outside the group holding the pipe) is left to end on its own, on end of stream or when its next
  `try_send` finds no receiver, rather than stall the frame loop.

The demand mechanism that sets all this is in [the background sampler](Theory-Background-Sampler-and-Demand#the-journals-linger).
The journal does not go through `Reads`, so it never appears in `Snapshot::sources`.

## 6. Demo lines

`--demo` has no follower. `journal::demo(from, to)` returns the synthetic lines whose index $k$ satisfies
$k\cdot 0.35 \in (from, to]$ (`DEMO_INTERVAL = 0.35 s`, and $k \ge 0$): a **pure function of time**. Line $k$ takes
`h = golden_gamma_high(k)` $= \lfloor k\cdot\gamma / 2^{32}\rfloor$ truncated (the splitmix64 increment $\gamma$ times
$k$, shifted right 32), picks template `h mod 16` of sixteen, and fills `{n}` with `h mod 9 + 1` and `{p}` with
`1000 + h mod 60000`. The templates look like a real machine, with severities from debug to critical, and three of
them are quotes from the film. A test checks that splitting an interval anywhere yields the same lines as one long
interval, which is what lets a warm-up replay and a live demo agree
([Determinism](Theory-Determinism)).

## 7. Untrusted text

A log message is arbitrary bytes chosen by whoever can write to the journal, including a remote attacker through a
failed-login message. Two layers stop it from reaching the terminal as a control sequence:

* The rain is **drawn with a bitmap font**, not printed. `glyphs::glyph(character)` maps printable ASCII to a
  7×14 bitmap (the public-domain X11 `7x14` font) and **everything else to `?`**: there is no path from a message
  byte to a terminal escape inside the picture.
* The one place the text is *printed* is the status line (the newest line, ` > …`). All status text goes through
  `terminal::clean_text`, which keeps ASCII graphic characters and spaces, replaces every other character with `?`,
  and truncates to the width.

Lengths are bounded on the way in (4096 bytes of a field, 64 KiB of a macOS line) so the parser cannot be made to
allocate more. The general rule and its test (`process_text_cannot_inject_terminal_controls`) are on
[Text safety](Theory-Text-Safety).

## 8. What a viewer can infer

* The rain is a *sample* of the log, not the log: lines can be dropped (the "skipped" count) and a flood is shown
  at the pace the screen can write.
* Colour is the sender's own severity: red for 0 to 3, amber for 4, green for 5 and 6, teal for 7 (nothing amber on
  macOS).
* On Linux a user outside `adm` or `systemd-journal` sees only their own journal; an empty rain may be permissions, not
  silence.
* Anything the follower or parser could not do shows up as an error line, not as a gap.

## Where in the code

* [`journal.rs`](https://github.com/tfpickard/isotop/blob/main/src/journal.rs): `Journal`, `Line`, `BACKLOG`,
  `READER_GRACE`, `demo`.
* [`platform/linux/journal.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/journal.rs):
  `journal`, `read_entry`, `read_line`, `FIELD`.
* [`platform/macos/journal.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/journal.rs): `FOLLOW`,
  `parse_line`, `scan`, `Cursor`, `severity`.
* [`platform/mod.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/mod.rs): `JournalParser`,
  `stop_journal` contract.
* [`main.rs`](https://github.com/tfpickard/isotop/blob/main/src/main.rs): `JOURNAL_LINGER`, `journal_wanted`, `feed`;
  [`matrix.rs`](https://github.com/tfpickard/isotop/blob/main/src/matrix.rs): `Rain`;
  [`glyphs.rs`](https://github.com/tfpickard/isotop/blob/main/src/glyphs.rs): `glyph`;
  [`terminal.rs`](https://github.com/tfpickard/isotop/blob/main/src/terminal.rs): `clean_text`.

## References

* The systemd documentation "Journal Export Formats", and `journalctl(1)` and `systemd.journal-fields(7)`.
* S. Gerhards, "The Syslog Protocol", RFC 5424 (2009): the severity levels.
* Apple's `log(1)` manual page for `log show` and `log stream`.
