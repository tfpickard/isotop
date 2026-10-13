# Theory: Text safety

isotop's input is the process table of a machine on which, by definition, other people's programs are
running, and the names of those programs are chosen by the programs. A process can call itself anything
that fits in fifteen bytes, and give itself any command line. isotop then writes those names onto your
terminal, which is a device that treats certain byte sequences as commands. This page is a small security
chapter: the threat model, what the sanitising function does and why that is sufficient, a verified
demonstration that the hostile text really arrives from the kernel, the gaps in the current design (one of
them an availability bug that I found while checking), and the plan to make the compiler enforce the rule.

## History

A terminal is an interpreter, and the language it interprets is older than most of the programs that print to it. The control functions are standardised as ECMA-48 (the reference below is its fifth edition, 1991) and ISO/IEC 6429, and DEC's VT100 terminals spread the escape-sequence style through the late 1970s (dates from memory, unverified). Xterm and its successors added window titles, colours, reports and, later, graphics. Those functions exist so that a program can control its own terminal; the case of a program that prints other people's text was not what they were introduced for.

The vulnerability class was named in public early in 2003. A message titled "Terminal Emulator Security Issues" was posted to Vulnwatch, Bugtraq and Full-Disclosure on 24 February 2003. Among the issues it described were the escape sequences that make a terminal report its own state back: set the window title with one sequence, ask for it with another, and the terminal types the title onto the command line as if the user had entered it. A carriage return cannot be put in the title, so the user still has to press Enter. Eterm and rxvt were also reported to have a screen-dump sequence that wrote the terminal's contents to a file. CVE entries followed for xterm in XFree86 4.2.0 and earlier (CVE-2003-0063), Eterm 0.9.1 and earlier (CVE-2003-0068) and dtterm (CVE-2003-0064). CVE-2003-0063 was reserved on 4 February 2003 and, depending on the database, published on 3 March 2003 or on 1 September 2004.

The text itself is the second attack surface, because Unicode added ways for a string to display as something other than what it stores. Boucher and Anderson's *Trojan Source* (CVE-2021-42574, published 1 November 2021) used the bidirectional controls of the Unicode Bidirectional Algorithm to make source code read differently to a reviewer and to a compiler, and some compilers and code hosts are reported to have added warnings for bidi controls (a secondary source, unverified here). Confusable characters were catalogued in the Unicode Consortium's Technical Standard #39. A process viewer meets both problems in a weaker form, since the strings it prints are names that anyone may choose.

The design that this page documents is an allow-list: accept only what is known to be harmless, instead of removing what is known to be dangerous. The idea of moving that decision from a comment into the type of the string, planned in section 6, is the one King summarised in 2019 as "parse, don't validate".

## 1. The invariant

`AGENTS.md`, Invariants: "**Process text is untrusted.** Names and command lines are sanitized before they
reach the terminal (`process_text_cannot_inject_terminal_controls`). Keep it that way for any new text."
`docs/VIEW_CONVENTIONS.md`, "Untrusted text": every string from the system is stored raw in the model, and
reaches the terminal only through `terminal::clean_text` (generated Braille sparklines have their own function, `terminal::clean_sparkline`, described in section 3); scene labels also go through `render::label_name`;
text drawn into the frame (`Item::Glyph`) is cleaned before layout; kernel strings that are only compared are
mapped to enums and never displayed raw; and "do not widen `clean_text`".

## 2. Threat model: the text is hostile, and it is cheap to make hostile

### 2.1 Who writes the text

| text | source | who controls it | length |
|---|---|---|---|
| process name | Linux `comm` in `/proc/<pid>/stat`, set by `prctl(PR_SET_NAME)` or `exec` | **the process itself, any unprivileged user** | 15 bytes, any byte but NUL |
| command line | `/proc/<pid>/cmdline` | the process (its `argv`, and `setproctitle`-style rewriting) | unbounded |
| cgroup path and unit name | `/proc/<pid>/cgroup` | whoever may create directories in a delegated cgroup subtree | path components |
| journal lines (Matrix) | `journalctl -o export`; on macOS `log show`/`log stream` | **any local user** via `logger`, `systemd-cat`, `sd_journal_print`, or the stdout of a service | unbounded, binary-safe |
| exit records | taskstats `ac_comm` | the dying process | 15 bytes |
| file names, paths | open-file scans (`/proc/<pid>/fd`) | whoever creates files | up to `PATH_MAX` |
| macOS names | `proc_name`, `KERN_PROCARGS2` | the process | various |

All of these are stored **raw** in the model (`Process.name`, `Process.command`, `Process.group`, journal
`Line`s, `Cause`/`ExitRecord` comm strings), "unsanitized like every process name" as the doc comments say.
The sanitising happens at output.

### 2.2 What a terminal does with bytes

A terminal emulator is an interpreter. Its input language (ECMA-48, ISO 6429, and the xterm and Kitty
extensions) has these constructs, each introduced by a control character:

* **CSI** (`ESC [` or the C1 character U+009B): cursor movement, erase, scroll regions, colours (SGR), mode
  changes, and *reports* that make the terminal write text back to the program's standard input.
* **OSC** (`ESC ]` or U+009D): set the window title, set colours, hyperlinks, and on some terminals read or write
  the clipboard.
* **DCS, APC, PM, SOS** (`ESC P`, `ESC _`, `ESC ^`, `ESC X` or their C1 forms): device control strings. **Kitty's
  graphics protocol is APC**: isotop's own frame transfer is `ESC _ G ...`. A name that could contain `ESC _ G`
  could draw an image or delete isotop's.
* **C0 controls**: BEL, backspace, tab, line feed, carriage return, SO/SI (charset switching), and others.
* **Cursor and layout controls**, in any of the above, that let text overwrite text.

The consequences of letting hostile text through are of four kinds:

1. **Spoofing the display.** Cursor movement and erase let a name overwrite labels, popups and the status
   lines drawn around it: show a different process's name, hide a process, or forge a warning. isotop places
   labels at absolute cells over a graphics image, so a stray cursor move lands on top of anything.
2. **Terminal state corruption.** Mode changes (alternate screen, mouse reporting, charset) that persist after
   isotop exits.
3. **Answerback injection.** Report requests (title report, status report, device attributes) make the
   terminal write a response onto stdin, *as if the user had typed it*. isotop runs in raw mode and
   interprets keys: `q` quits, `v` opens a picker, and in a hostile case a crafted response is a keystroke
   injection into the program itself.
4. **Exfiltration and clipboard writes** through OSC sequences, on terminals that allow them.

### 2.3 Beyond ASCII controls: Unicode

Even with every escape removed, Unicode text can lie:

* **C1 controls** U+0080 to U+009F. In a UTF-8 terminal they arrive as two-byte sequences; some emulators still
  honour them as 8-bit controls (U+009B CSI, U+009D OSC, U+0090 DCS, U+009F APC). A filter that removes only
  ESC (U+001B) is not enough.
* **Bidirectional controls** (U+202A to U+202E, U+2066 to U+2069, U+200E, U+200F, U+061C). The Unicode bidirectional
  algorithm (UAX #9) reorders text for display; an explicit override can make `cod\u{202E}exe.` display
  as a different name than it stores. This is the mechanism of *Trojan Source* (Boucher and Anderson 2021,
  CVE-2021-42574), there for source code; in a process viewer it lets one process's name display as another's.
* **Zero-width and format characters** (U+200B, U+200D, U+2060, U+FEFF, U+00AD soft hyphen, variation
  selectors): two different processes can display as the same name, and a name can be invisible.
* **Width.** The number of terminal cells a character occupies is not its character count: East Asian Wide and
  Fullwidth characters take two (UAX #11), combining marks and zero-width joiners take zero, emoji
  sequences are several scalars in one or two cells, and the tables differ between terminals and between
  versions of one terminal. isotop lays out text on a cell grid and draws popups as boxes of exact width;
  one wrong width and a box edge lands in the wrong column, or a label shifts the others.
* **Confusables** (UTS #39): Cyrillic "a" (U+0430) looks like Latin "a". `ssh` and `ѕsh` differ in the first
  letter and not on screen.
* **Invalid UTF-8.** A name is bytes; "text" is a convention. A decoder either fails, replaces (U+FFFD), or
  passes bytes through.

## 3. What `clean_text` does, exactly

```rust
pub fn clean_text(text: &str, width: usize) -> String {
    // Process names and command lines are untrusted terminal input. ASCII also keeps cell widths exact.
    text.chars()
        .map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '?' })
        .take(width)
        .collect()
}
```

1. Iterate the **Unicode scalar values** (`char`) of the string.
2. Keep a scalar if it is printable ASCII: `is_ascii_graphic()` is U+0021 to U+007E, plus the space U+0020.
3. Replace every other scalar by `'?'`: all C0 controls (U+0000 to U+001F, including ESC, BEL, TAB, LF, CR),
   DEL (U+007F), **all** C1 controls, and **all non-ASCII text, printable or not**.
4. Take at most `width` of the resulting characters.

Four properties follow, and I checked the first three exhaustively by compiling a verbatim copy of the function
and running it over every one of the 1 112 064 Unicode scalar values:

* **P1, the output is printable ASCII.** For every input scalar the output is exactly one byte in
  $[0\mathrm{x}20,0\mathrm{x}7E]$. 95 scalars pass through unchanged (the printable ASCII set); the other
  1 111 969 become `?`. No control byte, no ESC, no C1 introducer, no bidi or zero-width character, and no
  non-ASCII byte can appear in the output.
* **P2, one character in, one character out, then truncation.** The map is length-preserving in characters
  (a four-byte emoji becomes one `?`; a base letter plus a combining accent becomes two), so
  `chars().count()` of the output is `min(input chars, width)`. Because of P1 each output character is one
  byte and one terminal cell. **Display width is exact, not estimated.**
* **P3, idempotent.** `clean_text(clean_text(s, w), w) == clean_text(s, w)`.
* **P4, no partial characters.** `take(width)` truncates after mapping, on character boundaries of an
  ASCII string, so it cannot split a multi-byte sequence into invalid UTF-8.

**Why P1 is sufficient against the threats of section 2.** Every terminal control function is introduced by an
ESC, a C0 control, or a C1 control (as one byte in a legacy encoding or as U+0080 to U+009F in UTF-8). None of
those can occur in the output. The residue of a sequence is harmless text: the test string
`"bad\x1b[2J\nname"` becomes `"bad?[2J?name"`, a literal `[2J` with no ESC to give it meaning. Unicode threats
(bidi, zero-width, confusables, width) vanish together, because **there is no non-ASCII in the output**:
`ѕsh` displays as `?sh`, which reveals the lookalike where the original hid it. The cost is the price of
exactness, and it is paid in readability (section 5).

Where it is applied, in `terminal.rs`: `Terminal::present` runs every status-line string through
`clean_text(line, columns)`; `draw_label` through `clean_text(&label.text, room)`; `draw_popup` through
`clean_text(line, inner)`. These are the three places where text enters the escape stream; inside `draw_popup`, a line whose index is in `popup.braille` goes through `clean_sparkline` instead. The Matrix view's text is
drawn as pixels by the 7 by 14 bitmap font in `glyphs.rs`, whose `glyph(c)` has its own allow-list ("anything
outside printable ASCII is drawn as `?`") and cannot emit a control by construction.

`render::label_name` shortens names for scene labels *before* cleaning: it drops Apple's `com.apple.`
prefix (macOS) and, if more than `LABEL_NAME_CELLS = 15` characters remain, keeps the first 13 and appends an
ASCII `..` ("the ellipsis is ASCII because `terminal::clean_text` turns every other character into `?`").
It counts characters, not bytes (`labels_count_characters_not_bytes`), which is correct *only because* the
sanitiser later makes characters and cells coincide.

### 3.0 The second function: `clean_sparkline`

The inspector card of a process may carry generated braille lines (`stage.sparklines`: Cortex's membrane trace, for
one). They cannot go through `clean_text`, which would turn every dot into `?`. `clean_sparkline(text, width)`
keeps the Braille Patterns block (U+2800 to U+28FF) and the space and maps every other scalar to `?`, so a
sparkline can carry nothing but dots the view generated. It leaves `clean_text` alone: "Do not widen `clean_text`"
in `docs/VIEW_CONVENTIONS.md`, and `process_names_still_lose_braille_in_clean_text` pins that a process name with
braille in it still comes out as `??name`. The output is not ASCII, so P1 does not hold for it, but the allowed set
(256 braille cells and the space) contains no control, no bidi control and no zero-width character, and every
member occupies one cell. Only `Stage::sparklines` reaches it, and its doc comment says that text from the system
never belongs there. The test is `sparkline_sanitizer_passes_braille_but_not_controls_or_other_text`.

### 3.1 The tests

| test | file | asserts |
|---|---|---|
| `process_text_cannot_inject_terminal_controls` | `terminal.rs` | `clean_text("bad\x1b[2J\nname", 80) == "bad?[2J?name"`, and `clean_text("abcdef", 3) == "abc"` |
| `thing_card_text_cannot_inject_terminal_controls` | `main.rs` | a pickable "lantern" with a card title ending `\x1b]0;flare\x07` and a line `wick \x1b[2J\x07lit`: the bytes `draw_label` and `draw_popup` produce contain `lantern 100?]0;flare?` and `wick ?[2J?lit`, and contain none of `\x1b]0;`, `\x1b[2J`, `\x07` |
| `epitaphs_are_sanitized` | `necropolis.rs` | a process named `evil\u{1b}]0;pwned\u{7}name` has a grave card whose popup output contains `evil?]0;pwned?name-77` and no `\x1b]0;` or BEL |
| `sparkline_sanitizer_passes_braille_but_not_controls_or_other_text` | `terminal.rs` | `clean_sparkline("⣿\x1b[2J\n⣿", 80) == "⣿?????⣿"`, `"a\u{27ff}\u{2900}\u{202e}"` becomes `????`, and the cut is in cells |
| `cut_labels_survive_terminal_text_cleaning_unchanged` | `render.rs` | `label_name` output passes through `clean_text` unchanged (no `?`) and is at most 15 cells |

The first is the sentinel the guides name; the second and third are end-to-end (hostile text through the
view, the overlay, and the byte writer). None is a property test over all inputs; section 5 suggests one.

## 4. The hostile text is real: a demonstration

Any process can set its own name. On this machine (Linux 6.18), a Python process sets its `comm` to a string
holding ESC, a bracket sequence, a newline, a right parenthesis, BEL, an accented letter and the bidi
override U+202E, then reads what the kernel prints:

```python
import ctypes
libc = ctypes.CDLL(None)
name = b"a\x1b[2J\nb)\x07c\xc3\xa9\xe2\x80\xae"          # 15 bytes
buf = ctypes.create_string_buffer(name, 16)
libc.prctl(15, buf, 0, 0, 0)                                  # PR_SET_NAME
print(open("/proc/self/stat", "rb").read()[:80])
print(open("/proc/self/comm", "rb").read())
print(open("/proc/self/status", "rb").read().split(b"\n")[0])
```

```text
b'9049 (a\x1b[2J\nb)\x07c\xc3\xa9\xe2\x80\xae) R 9048 9049 9048 0 -1 4194304 1065 0 0 0 2 0 0 0 20 0 1 0'
b'a\x1b[2J\nb)\x07c\xc3\xa9\xe2\x80\xae\n'
b'Name:\ta\x1b[2J\\nb)\x07c\xc3\xa9\xe2\x80\xae'
```

* `/proc/<pid>/stat` carries the name **unescaped**: ESC, newline, BEL, a `)` and bidi override all arrive.
  The file isotop parses is the one with the raw bytes.
* `/proc/<pid>/status`, in this test, escaped only the newline (as `\n`); ESC, BEL and the bidi override
  arrived raw. An implementation that read `status` instead would still need a sanitiser.
* The `)` inside the name does not let the process forge fields: `parse_stat` finds the name between the first
  `(` and the **last** `)` of the line (`text.rfind(')')`: "comm can contain both whitespace and closing
  parentheses; only the last ')' delimits it"), and the real fields after it contain no parenthesis.
* The name is 15 bytes, the kernel's limit, but the command line has no such limit.

`cmdline` is decoded with `String::from_utf8_lossy(..).replace('\0', " ").trim()`: NUL separators become spaces,
invalid sequences become U+FFFD, and the surrounding whitespace is trimmed. The control characters survive
into the model and are removed at output.

## 5. Gaps, verified

### 5.1 The choke point is a convention, not a guarantee

Every label, popup and status line passes through `clean_text` "because three functions happen to call it".
`Label.text`, `Popup.lines` and the status rows are plain `String`s; a new overlay path that writes a `String`
straight to the output would compile. Issue [#81](https://github.com/tfpickard/isotop/issues/81) says it
in those words and plans the type (section 6).

### 5.2 Byte widths in `place_lines`

`place_panel(lines, ...)` in `main.rs` calls `place_lines`, which computes the popup width from the **byte**
length (`line.len()`) of the *uncleaned* lines, except for the lines flagged as braille, whose width is
`chars().count()` (a braille cell is three bytes and one cell). The border and padding are then drawn with the
cleaned, one-character-per-cell text. For ASCII the two agree; for non-ASCII text in an unflagged line the popup is
wider than needed (a two-byte character counts 2 and draws as one cell; the box is only too wide, never too
narrow, since bytes are at least characters). Labels and hover tags use `chars().count()`, which is right because
`clean_text` is character-for-character. #81 lists the discrepancy and adds `SafeText::width()`.

### 5.3 Legibility: every non-ASCII name is `?`

A process named in Cyrillic, Greek, Chinese or with accents is displayed as a row of question marks, each
scalar one `?` (a decomposed `é` is `e?`). This is the safe, simple, and sometimes unhelpful choice; #81 keeps
it and defers "whether to allow printable Unicode" as "a separate decision that needs width tables".

### 5.4 An availability bug found on the way: a hostile or merely long name hides a process (Linux)

The Linux sampler reads `/proc/<pid>/stat` with `fs::read_to_string`, which fails on invalid UTF-8, and skips
the process when it fails:

```rust
let Ok(stat) = fs::read_to_string(path.join("stat")) else { continue; };
```

`comm` is bytes, not text. Two cases produce invalid UTF-8:

* **Hostile**: a process sets its `comm` to a byte string such as `\xff`.
* **Benign**: a program with a long non-ASCII name. The kernel truncates `comm` to 15 bytes at a byte
  boundary, which can fall in the middle of a multi-byte character.

I reproduced the benign case: a child set its name to the first 15 bytes of "приложение_длинное"
(Cyrillic letters are two bytes each, 15 is odd, so the cut falls inside a letter); its `stat` begins
`b'9479 (\xd0\xbf\xd1\x80\xd0\xb8\xd0\xbb\xd0\xbe\xd0\xb6\xd0\xb5\xd0) S 9478 ...'`, which Python reports as
`invalid continuation byte` at position 20, and a Rust program calling `fs::read_to_string` on that very file
printed `error: stream did not contain valid UTF-8`. With that error the sampler's `continue` drops the
process from the snapshot, so **it does not appear in any view**. The macOS path decodes with
`String::from_utf8_lossy` (`platform/macos/mod.rs`, `logic.rs`) and is not affected, nor is the Linux
`cmdline` read, which is lossy too. The fix is to read `stat` as bytes and decode lossily, and to treat the
replaced name like any other raw text. This is a bug in the *ingestion* of text rather than its display; this
page documents it and does not change the code.

### 5.5 Smaller observations

* **Truncation is by character count, not display width**: correct after mapping to ASCII, wrong for any
  future widening.
* **No property test.** The sentinels test two strings and two end-to-end cases. An exhaustive test over all
  scalar values of the kind I ran (section 3) is a few lines and pins P1.
* **Other sinks.** I found no path by which process-derived text reaches `println!` or `eprintln!`: the
  standard-output messages print counts, view names and file paths from the command line, and the one
  error message path (`isotop: {error}`) runs after the terminal has been restored. This is an absence of
  hits in a search of `src/`, not a proof.
* **Terminal replies** are read once at start-up (`probe`), before any process text exists, with
  `String::from_utf8_lossy` and substring tests; later input arrives through `crossterm`.

## 6. Planned: `SafeText` (issue #81)

> **Planned, not yet implemented.** Specified in
> [issue #81](https://github.com/tfpickard/isotop/issues/81), "A22: Enforce sanitized terminal text by type
> (SafeText) and measure widths in characters". `terminal.rs` has no `SafeText` today.

The design: `pub struct SafeText(String)` in `terminal.rs`, constructible only by `SafeText::new(&str)`, which
applies `clean_text`, and by `SafeText::trusted(&'static str)` for literals. `Label.text`, `Popup.lines` and the
status rows take `SafeText`; `present` stops sanitising "because it cannot receive unsanitized text".
`SafeText::width()` returns the display width and every layout uses it, which fixes 5.2. A `compile_fail`
doctest shows that `Label { text: String::from(..) }` does not compile. Acceptance: `grep -n 'clean_text' src/`
shows it called only inside `SafeText::new`, and no frame changes (the golden hashes hold). It keeps the
ASCII-only output.

The idea is the standard one of making the invalid state unrepresentable (a *parse, don't validate* newtype):
the property "this string has been through the sanitiser" moves from a comment to the type system. The one
residual hole is `trusted`, which must stay `'static`.

### 6.1 If Unicode is ever allowed

Allowing printable Unicode needs a stricter filter and a width model, because P1 would no longer hold:

* **Allow-list by general category**: letters, marks, numbers, punctuation, symbols, and the space
  separator; remove `Cc` (controls), `Cf` (format: bidi, zero-width, joiners, soft hyphen, tag characters),
  `Zl`, `Zp`, `Co`, `Cs`, `Cn`. That removes the injection and bidi classes in one rule but not width.
* **Width**: East Asian Width (UAX #11) for wide and fullwidth, zero for combining marks (`Mn`, `Me`), and
  grapheme clustering (UAX #29) for emoji sequences. These tables are versioned with Unicode, and terminals
  disagree on emoji and ambiguous-width characters; the wrong width misaligns every box. The project's "no
  new dependency unless it replaces substantial, error-prone code" rule applies: this is such code, and an
  in-tree table is the alternative.
* **Visible escapes**: a different policy renders disallowed scalars as `\u{202e}` instead of `?`, which keeps the
  evidence and costs width.
* **Bidi**: either strip the explicit controls (above) or wrap each field in isolates (FSI to PDI) so that
  one field's direction cannot leak into its neighbours.

## 7. Try it

```sh
cargo test process_text_cannot_inject_terminal_controls
cargo test thing_card_text_cannot_inject_terminal_controls
cargo test epitaphs_are_sanitized
cargo test sparkline_sanitizer_passes_braille
cargo test cut_labels_survive_terminal_text_cleaning_unchanged

# the hostile name: run the script of section 4 with `import time; time.sleep(120)` appended, in the
# background, then look for it in a live run
./target/release/isotop --view city      # the process is labelled with a ? for each unsafe character
```

In a headless run the names appear only in the PNG (as bitmap text via `glyph`, also `?` for non-ASCII).

## 8. Where in the code

* [`src/terminal.rs`](https://github.com/tfpickard/isotop/blob/main/src/terminal.rs): `clean_text`,
  `clean_sparkline`, `Terminal::present`, `draw_label`, `draw_popup`, `Label`, `Popup`.
* [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs): `label_name`,
  `without_apple_prefix`, `LABEL_NAME_CELLS`; `Card` (its text "is untrusted"), `Stage::sparklines`.
* [`src/glyphs.rs`](https://github.com/tfpickard/isotop/blob/main/src/glyphs.rs): `glyph`.
* [`src/main.rs`](https://github.com/tfpickard/isotop/blob/main/src/main.rs): `place_panel` and `place_lines`, the label and
  tag placement, `thing_card_text_cannot_inject_terminal_controls`.
* [`src/platform/linux/mod.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/mod.rs):
  `parse_stat` (`rfind(')')`), `Texts::read` (lossy `cmdline`), `Sampler::processes` (the
  `read_to_string` and `continue`).
* [`src/platform/linux/journal.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/journal.rs)
  and [`src/platform/macos/journal.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/journal.rs):
  decoding of journal text.

## 9. Variations and open questions

* **Fix the ingestion hole** of section 5.4 with a byte read and `from_utf8_lossy`, and add a test with a
  fixture holding a truncated multi-byte `comm`.
* **An exhaustive test**: for every scalar value, `clean_text` output is one printable-ASCII byte.
* **Escape instead of replace** for non-ASCII, so a Cyrillic name is `\u{43f}\u{440}...` rather than
  `???????`: legible to a person who needs to tell two names apart, at three to eight times the width.
* **Detect and flag** a name that contained a control character: tint its label or add a marker; today the
  attack is neutralised but invisible to the viewer.
* **Typed input**: a `RawText` type for model strings with `Display` unimplemented, so that `format!("{name}")`
  of a raw name fails to compile and only `.safe()` is allowed.

## 10. References

* ECMA-48, 5th edition (1991), *Control Functions for Coded Character Sets*; ISO/IEC 6429.
* The Unicode Consortium. *The Unicode Standard*; Unicode Standard Annex #9 (Bidirectional Algorithm), #11
  (East Asian Width), #29 (Text Segmentation); Unicode Technical Standard #39 (Security Mechanisms).
* Boucher, N. and Anderson, R. 2021. "Trojan Source: invisible vulnerabilities." (CVE-2021-42574.) Published
  at USENIX Security 2023.
* The Kitty terminal graphics protocol specification (APC `G` commands).
* `man 2 prctl` (`PR_SET_NAME`), `man 5 proc` (`/proc/<pid>/stat`, `comm`, `status`).
* King, A. 2019. "Parse, don't validate." (Essay on making invalid states unrepresentable.)
