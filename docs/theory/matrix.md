# Matrix: the system journal in digital rain

**View:** `matrix` · **Code:** `src/matrix.rs`, `src/glyphs.rs`, `src/journal.rs`, `platform/linux/journal.rs`, `platform/macos/journal.rs`

## What you are looking at

This view is the odd one out: it draws **log lines, not processes**.

| On screen | Measured quantity |
|---|---|
| A line of text | one journal entry, newest at the bottom |
| The text's colour | its syslog severity: red for 0 to 3 (emergency to error), amber for 4 (warning), green for 5 and 6 (notice, info), dim teal for 7 (debug) |
| A shower of rain falling onto a line | the line arriving: each of its characters is struck by a falling stream and flashes white as it resolves |
| The steady green rain between lines | decoration (the "cmatrix" look): flickering glyphs with white heads |
| The greeting at start ("Follow the white rabbit.") | decoration, three lines of film quotes with your `$USER` |
| Status strip: latest line, skipped count | the follower's real state: how many lines were dropped because they arrived faster than the screen could show |

Only Linux's `journalctl` and macOS's unified log feed the real thing; `--demo` replays synthetic entries every 0.35 s.

## History

**Green on black** is a fossil of hardware. Early video terminals used monochrome CRTs, and a popular phosphor (the P1 type) glowed green, so the look of "a computer" for a generation was green characters on a black field. When designers wanted to say *machine* in the 1990s, they reached for it.

**Digital rain** is that look, set in motion by one film. *The Matrix* (1999) opens on streams of glyphs falling down a screen. The design is credited to Simon Whiteley, a production designer from England who was working in Sydney. He has said, in later interviews, that the characters came from his wife's Japanese cookbooks, scanned and then altered: half-width katakana, with some mirrored, and some Western digits mixed in. By his account the "code" is, in part, sushi recipes. That account rests on his own recollection and is told in several outlets, and I have not found it independently verified. The sequels and the whole genre of terminal screensavers copied the look. In the terminal world it has been imitated ever since: programs like `cmatrix` fill a text screen with falling green characters, and isotop's rain is in that tradition.

**The thing falling is not the film's.** isotop's glyphs are not katakana. They are printable ASCII from the X11 `misc-fixed` 7 by 14 bitmap font, a public-domain font from the early window-system era, because that is what the program can draw identically on the CPU and the GPU from a table of bits.

**Logs are older than screens.** Eric Allman wrote `syslog` in the 1980s as a side-tool for `sendmail`, a protocol for programs to hand their messages to one daemon with a *facility* (what produced it) and a *severity* (how bad). The eight severity levels from 0 (emergency) to 7 (debug) were standardized in RFC 3164 (2001) and RFC 5424 (2009), and survive in `journald`, whose structured, indexed binary journal arrived with systemd and reached Fedora users with Fedora 17 in 2012. On macOS the equivalent is the unified log of 2016. Both are databases of everything the machine says to itself, and almost nobody reads them until something breaks.

## Background theory

### Severity

A syslog message carries a priority $P = 8F + S$ where $F$ is the facility and $S \in \{0, \dots, 7\}$ the severity:

| $S$ | Name | Colour here |
|---|---|---|
| 0 | emergency | red |
| 1 | alert | red |
| 2 | critical | red |
| 3 | error | red |
| 4 | warning | amber |
| 5 | notice | green |
| 6 | informational | green |
| 7 | debug | teal |

The journal's `PRIORITY` field is $S$. Entries with no priority are treated as informational.

### Bitmap fonts

A bitmap font stores each character as a small grid of on/off pixels. The 7 by 14 cell here has 98 pixels, which fit in a 128-bit integer: bit $r \cdot 7 + c$ is the pixel in row $r$ and column $c$, column 0 at the left. A glyph is therefore `u128`, an entire character in a register, and drawing it is a loop over set bits. 95 printable ASCII glyphs (space to tilde) take 1.5 kB. This has no antialiasing, no hinting and no subpixels, so the CPU rasterizer and the GPU shader draw identical pixels: the pixel grid *is* the font. The scale is an integer, `height / 540` at least 1, so a 4K frame draws each font pixel as a block.

### The decode as signal and noise

Think of the screen as a channel. The text is the signal; the rain is noise at a controlled level. Each character of a new line is hidden until a stream falls through its cell, when it flashes white and relaxes to its colour:

$$\text{colour}(t) = \text{steady} + (\text{white} - \text{steady})\,e^{-(t - t_{\text{struck}})/\tau}, \qquad \tau = 0.35\ \text{s}$$

where steady is 85% of the severity colour. An exponential relaxation is the natural choice: it is smooth, has no free shape parameter beyond $\tau$, and after $10\tau$ (3.5 s) the flash is down to $e^{-10} \approx 5\times10^{-5}$ of its start and the character is settled. If no stream ever strikes a character, it appears anyway 3 s after its line arrives, so a line is never lost.

## How isotop implements it

1. **The follower.** On Linux, `journalctl` runs in export format and a reader thread parses it into `Line { priority, text }`; on macOS, `log show` then `log stream` do the same with a small JSON parser. The follower runs only while the view needs it (and for 30 s afterward). The backlog is bounded at 2,048 lines; further ones are dropped and counted, not buffered forever.
2. **Release.** Lines are not shown the instant they arrive. They queue and are released at a pace $\Delta t = 0.5/(1 + n/3)$ seconds for $n$ waiting: half a second apart when the queue is empty, a sixth of a second when six wait, faster still beyond. A burst after a long gap is limited to one second's worth. If the queue grows past two screens, the oldest are dropped and counted in the status strip as *skipped*. So the view is honest about overload: it prefers losing lines to lying about when they happened.
3. **Wrapping.** A line wider than the grid is broken after the last space that fits (or mid-word), and continuation rows are indented by two. When the frame is resized, the lines on screen are rewrapped and decoded again under a fresh shower.
4. **Showers and ambient rain.** For every non-space character of a new line, a stream is started a random 1 to 11 rows above its row, at 14 to 26 rows/s, with a length of 2 to 6, that stops at the character's row. Ambient streams (about half a stream per text column) run at 8 to 16 rows/s with a length of 8 to 23 and a glowing white head. A rain glyph is a pseudo-random printable character that changes six times a second, out of step with its neighbours (`flicker`). The random numbers are seeded, so a `--time` frame is repeatable.
5. **Layering.** Rain does not draw over resolved text: a cell that holds a visible character is skipped by streams.

## Faithful, stylized, invented

- **Faithful:** the line text, the severity, the order of arrival, the dropped-line count and the status strip's latest line.
- **Stylized:** the pacing of release, the flash and its decay constant, ASCII in place of katakana, the colour for each severity.
- **Invented:** the rain, entirely; the greeting; the premise that a log is a message from a simulation.

## Reading the picture

- A screen of calm green is a healthy machine saying informational things. Red lines are errors and worse, and in a flood they are the story.
- A burst of lines arriving faster than they can be shown makes the *skipped* counter rise. That is itself a signal: something is logging in a loop.
- Amber lines are warnings: the log's way of saying "this will become red".
- No lines at all means the follower sees nothing, which is also information (a quiet machine, or no permission to read the journal; the legend names which).

## Further reading

- RFC 5424, *The Syslog Protocol* (2009), sections on facility and severity.
- `systemd.journal-fields(7)` and `journalctl(1)`, for the export format and fields.
- The X11 `misc-fixed` fonts (the `7x14` face), public domain.
- Whiteley's account of the code, in interviews around the film's anniversaries; treat it as a recollection, not a record.
