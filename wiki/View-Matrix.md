# View Matrix

The system log, decoded out of digital rain. The screen is the machine's journal, written horizontally in the colour of each
entry's severity, newest line at the bottom; but no character is there until a falling stream of rain passes over it, and
then it flashes white and settles. Between lines, ambient rain of flickering green glyphs falls through the empty screen. It
is the one view that draws no processes at all: its data are text, and the pictures are a way of making the *arrival* of
that text visible.

![Matrix: journal lines decoding out of digital rain](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/matrix.webp)

![The README image of the same view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/matrix.webp)

## The system being modelled

**The rain.** The "digital rain" is the title-sequence motif of *The Matrix* (1999): columns of glyphs falling down a
black screen, a bright head and a fading green tail, the glyphs changing as they fall. Terminal clones such as `cmatrix` (named in the code comments) made it a screensaver. The
conceit here is the reverse of the film's: the rain is decoration and the *code* it reveals is real text.

**The journal.** A running Linux system writes a stream of structured log entries (systemd-journald, or syslog before it). Each carries a *severity*, defined by RFC 5424 as
eight levels: 0 emergency, 1 alert, 2 critical, 3 error, 4 warning, 5 notice, 6 informational, 7 debug. isotop reads the stream as `journalctl --follow` in *export format*, which carries
fields as `NAME=value` lines, or, for binary or multi-line values, the name, a little-endian 64-bit length and the raw bytes. On macOS the equivalent is the unified log, read as JSON lines from `log show` and `log stream`; it has message types (Fault, Error, Default, Info,
Debug) instead of syslog levels. See [Theory Journal](Theory-Journal) for the follower and its parsers.

## Theory

The view needs very little physics, but three pieces of theory explain its behaviour.

**A queue that speeds up as it fills.** Lines arrive at the rate the system logs (bursty: a boot or an error storm can be hundreds of lines per second; an idle desktop, a few per minute). The log is written
one line at a time, with a gap between lines of

$$
\Delta t(n) = \frac{0.5\ \text{s}}{1 + n/3}, \qquad n = \text{lines waiting},
$$

so the server's rate is $2(1 + n/3)$ lines per second: 2 per second with nothing waiting, 4 with three waiting, 52.7 with 76 waiting. It is a work-conserving server whose speed is proportional to the backlog, which keeps the delay between an event and its appearance roughly bounded
instead of growing without limit. A hard cap bounds the queue at $2\max(\text{rows}, 16)$ lines; beyond that the *oldest* queued lines are dropped and counted, and the status strip says `N skipped`. The channel therefore has a stated loss policy: it
loses the past, never the present.

**Exponential relaxation.** A character struck by rain at time $t_0$ is drawn in the colour $\mathrm{mix}(\text{steady}, \text{head}, e^{-(t - t_0)/\tau})$ with $\tau = 0.35$ s: a first-order relaxation from near-white to the steady colour at 85 % of full intensity. A first-order system is
the simplest thing that makes an event easy to notice and then recedes without a step.

**Bitmap fonts as integers.** Each glyph is a $7 \times 14$ monochrome bitmap packed into the low 98 bits of a `u128`: pixel (row $r$, column $c$) is bit $7r + c$, column 0 at the left. Drawing a glyph is "for each set bit, fill one square of $s \times s$ pixels", with an integer scale $s$.
The font is the public-domain X11 `misc-fixed` $7\times 14$ face (printable ASCII only).

![The bitmap font decoded from src/glyphs.rs: all 95 glyphs and the letter g with its bit numbering](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/matrix-glyph-sheet.svg)

*The sheet is decoded from the Rust source at figure time, not copied. Capitals occupy rows 2 to 11, the lowercase x-height rows 5 to 11, descenders rows 12 and 13; row 0 is never inked. Script: `docs/figures/matrix-glyph-sheet.py`.*

**Information-theoretic aside.** Text is the opposite of what the other views draw. The other 13 views compress a process table (floating-point rates, counts, states) into geometry; this one passes symbols through a bottleneck. At 1920 x 1080 a glyph cell is 14 x 28 pixels, so the screen holds $137 \times 38 = 5206$ characters, a few dozen log lines;
a busy journal produces more than that every second. The rain is therefore not a log reader: it is a *rate* display with a sample of the content, and the skipped counter is the measured loss.

## Mapping

Realness contract: every element is either a log entry or declared decorative.

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| A row of text | One journal entry (`journalctl`; `log` on macOS) | `source[pid]: message`, newlines and tabs replaced by spaces; wrapped at the last space that fits, continuation rows indented 2 |
| Row colour | The entry's syslog priority (or the unified-log message type) | 0 to 3 red, 4 amber, 5 and 6 green, 7 teal; steady colour = 0.85 of full |
| Source name | `SYSLOG_IDENTIFIER`, else `_COMM`, else `journal` | As text |
| `[pid]` | `_PID` | As text; omitted if absent |
| White flash on a character | The rain passing over it | $e^{-t/0.35\,\mathrm{s}}$ from head colour (210, 255, 220) to steady |
| A shower | A newly written line | One stream per non-space character, starting 1 to 11 rows above its row, 14 to 26 rows per second, length 2 to 6; *the stream ends at its row* |
| Unveil | Time since the line was written | A character no stream reached appears after 3 s |
| Ambient rain | Nothing: **decorative** | One stream per two columns (always `columns / 2`), 8 to 16 rows per second, length 8 to 23, never over text |
| Rain glyph | Nothing: **decorative** | ASCII `!` to `~`, each cell changing about 6 times a second, out of phase between columns |
| Greeting | Nothing: **decorative** | "Wake up, {user}...", "The Matrix has you...", "Follow the white rabbit." (`$USER`, else "Neo"), queued first |
| Status strip, legend | Queue and skip counts | `N queued`, `, M skipped` |
| Status strip, inspector line | The newest line | `> ` followed by the line, through the terminal text sanitizer |

## How isotop realises it

**Reading.** `journal::Journal::start` spawns the platform follower (`platform::journal()`): on Linux
`journalctl --follow --lines=200 --output=export --output-fields=PRIORITY,SYSLOG_IDENTIFIER,_COMM,_PID,MESSAGE`, with no shell, stdin closed, stdout and stderr piped. A reader thread parses with `read_entry`, keeping at most 4096 bytes per field (the rest is read and discarded so the stream stays in sync), and sends lines
through a bounded channel of `BACKLOG = 2048`; when it is full the reader counts the loss. A priority that does not parse is 6 (info), clamped to 7. The follower runs while the shown view declares `Source::Journal` and for `JOURNAL_LINGER = 30` s after (`main.rs`), so a glance at another view does not restart it; coming back starts it again with the last 200 entries. Without the
`adm` or `systemd-journal` group, journalctl shows only your own journal. On macOS the follower is a fixed `/bin/sh -c` command in its own process group (`log show --last 2m --style ndjson --info; exec log stream --style ndjson --level info`), parsed by a std-only JSON scanner that keeps top-level keys only, at most 4096 bytes of message; Fault and Error are red, Default and Info green, Debug teal, and nothing is amber.
`--demo` starts no follower: `journal::demo(from, to)` synthesises one line per 0.35 s of demo time from 16 templates chosen by a hash of the line counter, so it is deterministic in time.

**Writing.** `Rain::push` queues a line (trimming the queue once the grid size is known, so the startup history survives to fill the first screen). Every frame, `advance` releases lines whose time has come, `release = max(release, time - 1) + interval` (so a slow frame rate still keeps pace and a burst after a gap is at most one second's worth), `write`s each into the log
(rows that would scroll straight off are skipped), and starts one shower stream per non-space character. A `Row` stores, per character, the time rain last struck it (negative infinity if never).

**Rain.** A `Stream` has a fractional head row, a speed, a length and, for a shower, a target row at which it stops and drains away. Each frame the head advances by `speed * dt`; every row it crosses in that column stamps `struck = time` on the row's character, if there is one. Ambient streams are topped up to `columns * 0.5`.
A character is drawn only once struck or after the 3 s unveil. Rain glyphs are drawn only on cells not occupied by revealed text. A head is bright (210, 255, 220) with a soft glow for ambient streams; the tail fades linearly to 60 % green. The glyph in a rain cell is `'!' + hash(column, row, tick) % 94` with `tick = floor(6 t + 0.37 column)`.

**Layout.** `Rain::layout`: scale $s = \max(1, \lfloor \text{height}/540\rfloor)$, columns $= \max(8, \lfloor\text{width}/(7s)\rfloor)$, rows $= \max(2, \lfloor\text{height}/(14 s)\rfloor)$. A change of size re-wraps what is on screen; it decodes again under a fresh shower.

**Pause and time jumps.** If the clock jumps (pausing switches clocks), `rebase` moves every stored time by the jump so nothing is struck or unveiled by it; paused, the view keeps what it shows and the backlog is bounded. The main loop never feeds more than two seconds of demo time at once.

**Determinism and parity.** The rain's randomness is `simulation::Rng` with a fixed seed, and glyph flicker is a hash of (column, row, tick), so a demo frame at a fixed `--time` is repeatable. Glyphs are display-list items (`Item::Glyph`) that both rasterizers draw, with the same constants from `render::shared`.

**Text safety.** Log text is attacker-influenced (anything can write to a journal). It never reaches the terminal as text: it is drawn as pixels by `glyphs::glyph`, which maps everything outside printable ASCII (space and `!` to `~`) to `?`. The one text path, the inspector line `> latest`,
goes through `terminal::clean_text`, which keeps ASCII graphic characters and spaces and replaces everything else with `?` (a test shows `"bad\x1b[2J\nname"` becoming `bad?[2J?name`). See [Theory Text Safety](Theory-Text-Safety).

**Row.** `Backdrop::Void`, `haze: false`, `sources: Journal`, the harness contract `draws_processes: false`, and a headless warm-up of 161 frames at 0.05 s with an 8 s lead so the first screenshot has a full screen. There is no picking and the camera and process controls do nothing.

## Reading the picture

What I see in a demo render. The top-left of the black screen holds the log: first the greeting ("Wake up, Neo...", "The Matrix has you...", "Follow the white rabbit."), then entries such as `sshd[39161]: Accepted publickey for neo ...` in green, an amber `kernel: [UFW BLOCK] ...` warning, and in red
`smith[28837]: Never send a human to do a machine's job.` Below them a line is half-decoded, with scattered glyphs (`o  us 99 : Fo l tew  a`): that is the instant between a shower's strike and the unveil. The rest of the screen is thin vertical streams, a bright head and a dim trail in fading green, with the faintest glow around ambient heads. The log fills from the top; once the screen is full, new lines come in at the bottom and the rest scroll up.

* **Measured:** the entries, in the order the follower delivered them, their severity colour, the queue length and the skipped count. The *arrival* (a shower decoding a line within a second or so) is the real event.
* **Derived:** only the layout (wrapping, rows).
* **Decorative:** all ambient rain, the glyphs in it, the greeting, the flash colours, the exact flicker. The *amount* of ambient rain is constant: it does **not** reflect log volume. Volume is shown by how fast lines are written and by the queued and skipped counts.
* **What it cannot tell you:** the order of events inside a second when several arrive together (they are written in delivery order, at most a few frames apart), anything outside printable ASCII (it prints `?`), anything beyond 4096 bytes of a field, entries of other users without the right group, and anything dropped (counted, not shown). The timestamp of an entry is not drawn.
* **Time:** the text appears when the follower delivered the line plus the queue delay, not when the event happened.

## Try it

```sh
./target/release/isotop --demo --view matrix --width 1600 --output /tmp/matrix.png --renderer cpu
./target/release/isotop --view matrix            # live journal; key 0 jumps to it, Tab cycles
./target/release/isotop --list-views             # confirms matrix is supported on this platform
python3 docs/figures/matrix-rain-timing.py       # the clocks above, from the constants of src/matrix.rs
```

![Timing of the rain: flash, release rate, shower](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/matrix-rain-timing.svg)

*Left: the colour channels of an information-level character after a strike. Middle: release rate against queue length. Right: time for a shower to reach a character, median 0.30 s and nothing later than 0.78 s. Script: `docs/figures/matrix-rain-timing.py`.*

## Where in the code

* `src/matrix.rs`: `Rain` (`push`, `advance`, `write`, `draw`, `trim`, `rebase`, `layout`), `Stream`, `Row`, `wrap`, `flicker`, `level`; constants `UNVEIL`, `FLASH`, `STEADY`, `AMBIENT`, `SHOWER`, `DENSITY`, `INDENT`, `RAIN`, `HEAD`.
* `src/glyphs.rs`: `GLYPHS` (95 `u128`s), `glyph`, `WIDTH`, `HEIGHT`.
* `src/journal.rs`: `Journal`, `Line`, `demo`, `BACKLOG`. `src/platform/linux/journal.rs`: `journal`, `read_entry`, `read_line`. `src/platform/macos/journal.rs`: `journal`, `parse_line`, `severity`.
* `src/main.rs`: `JOURNAL_LINGER`, `feed`, the follower start and stop; `src/views.rs`: the `matrix` row, `matrix_legend`, `matrix_line`; `src/terminal.rs`: `clean_text`; `src/simulation.rs`: `hash::lattice_flicker`.
* Tests in `matrix.rs`: `a_line_is_decoded_horizontally_in_its_level_colour`, `characters_stay_hidden_until_rain_passes_over_them`, `a_flood_is_capped_and_counted`, `a_slow_frame_releases_every_overdue_line`, `flicker_glyphs_stay_what_they_were`.
* The matrix is a built-in arm of `Scene::render` (`BuiltIn::Matrix`); there is no plugin system.

## Variations and open questions

1. **Rain from the log rate.** Make the ambient density or speed a function of the lines per second, so volume is visible as weather. This would turn decoration into a measurement.
2. **Columns by unit.** Put each systemd unit in its own column band, so one noisy service rains in one place.
3. **A real glyph set.** The film's rain is half-width katakana mirrored; the 7 x 14 ASCII font cannot. A second font table would do, at the cost of a licensing decision.
4. **Severity as speed.** Errors fall faster, debug lines more slowly.
5. **Unicode.** `glyph()` prints `?` for everything non-ASCII, including accented names and CJK messages; a wider font or a fallback box glyph would show that something was there.
6. **A rate-distortion question.** Given a fixed screen of 5 206 cells and a log of $r$ lines per second, which dropped-line policy maximises what a human reads: drop oldest (as now), drop lowest severity first, or sample uniformly?

## References

* R. Gerhards, "The Syslog Protocol", RFC 5424 (2009): the severity levels.
* The systemd documentation, "Journal Export Formats" and `journalctl(1)` (`--output=export`, `--output-fields`).
* The X11 `misc-fixed` bitmap fonts, the `7x14` face (public domain, as stated in `src/glyphs.rs`).
* L. Wachowski and L. Wachowski, *The Matrix* (1999). The rain's glyph design is credited to Simon Whiteley (unverified here).
* C. E. Shannon, "A mathematical theory of communication", *Bell System Technical Journal* 27 (1948), 379-423 and 623-656.
* [Theory Journal](Theory-Journal), [Theory Text Safety](Theory-Text-Safety), [Theory Background Sampler and Demand](Theory-Background-Sampler-and-Demand).
