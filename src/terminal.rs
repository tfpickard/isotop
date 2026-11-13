use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{cursor, execute, terminal};

use crate::render::Frame;

mod inline;
mod restore;
mod shared;
mod text;

use inline::Inline;
use restore::Restoration;
use shared::{Medium, Name, Segments};
pub use text::SafeText;
use text::display_width;

/// The Kitty image id of the frame drawn first, and of every second one after it. A frame is
/// placed under the id the previous frame does not have, and only then is the previous image
/// deleted, so the screen is never without an image between two frames.
const IMAGE_A: u32 = 100;
/// The id of every other frame; see `IMAGE_A`.
const IMAGE_B: u32 = 101;
/// The id of the capability probe's inline query (`a=q,t=d`); the terminal answers `i=31;OK`.
const PROBE_INLINE: u32 = 31;
/// The id of the probe's shared-memory query (`a=q,t=s`); the terminal answers `i=32;OK`.
const PROBE_SHARED: u32 = 32;

/// Where frames are written: standard output, each write counted by the `Restoration` so that
/// none starts after the terminal was restored (a write after that is dropped as if it had
/// worked). Cloning it is cheap; the encoder thread has its own.
#[derive(Clone)]
struct FrameSink(&'static Restoration);

impl Write for FrameSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.0.frame(|| {
            let mut out = io::stdout().lock();
            out.write_all(bytes)?;
            out.flush()
        });
        written.unwrap_or(Ok(())).map(|()| bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct Terminal {
    active: bool,
    image: u32,
    /// The terminal reports mouse positions in pixels (SGR-Pixels, mode 1016).
    pub pixel_mouse: bool,
    /// The last frame went through POSIX shared memory instead of compressed inline data (and
    /// before the first, whether shared memory is in use at all).
    pub shared_memory: bool,
    /// The Kitty keyboard protocol is on (`KEYBOARD_FLAGS` pushed), so Shift+Space arrives apart
    /// from Space. Restoring the terminal pops it.
    pub keyboard: bool,
    restoration: &'static Restoration,
    sink: FrameSink,
    /// The objects frames were written to; removed as they age out and when the terminal is
    /// dropped. A terminal unlinks each after reading it.
    segments: Segments,
    /// Whether shared memory is used now, or waits after a failure (`shared::Medium`).
    medium: Medium,
    /// Frames sent inline, encoded and written off the frame loop.
    inline: Inline<FrameSink>,
    /// Bytes and frames written through shared memory.
    shared_written: (u64, u64),
    /// A clear of the screen was asked for (`clear`) and has not been written yet because the
    /// encoder was still writing a frame; `present` writes it first once the encoder is idle.
    clear_pending: bool,
}

pub struct Popup {
    pub column: u16,
    pub row: u16,
    pub width: u16,
    /// The lines, sparklines among them (`SafeText::braille`); each is as wide in cells as
    /// `SafeText::width` says, whatever it holds.
    pub lines: Vec<SafeText>,
}

impl Popup {
    pub fn height(&self) -> u16 {
        self.lines.len() as u16 + 2
    }

    pub fn contains(&self, column: u16, row: u16) -> bool {
        (self.column..self.column + self.width).contains(&column)
            && (self.row..self.row + self.height()).contains(&row)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Quiet,
    Bright,
    Tag,
}

pub struct Label {
    pub column: u16,
    pub row: u16,
    pub text: SafeText,
    pub tone: Tone,
}

impl Terminal {
    pub fn enter(force: bool, direct: bool) -> io::Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(io::Error::other(
                "interactive mode needs a terminal; use --output scene.png or --benchmark for headless rendering",
            ));
        }
        let modes = restore::modes(libc::STDIN_FILENO);
        terminal::enable_raw_mode()?;
        // From here a panic or a signal restores the terminal as well as `Drop` does.
        let restoration = restore::shared();
        restoration.arm(
            libc::STDOUT_FILENO,
            libc::STDIN_FILENO,
            modes,
            restore_bytes(false),
            restore_bytes(true),
        );
        restore::install(restoration);
        let sink = FrameSink(restoration);
        let mut guard = Self {
            active: true,
            image: IMAGE_A,
            pixel_mouse: false,
            shared_memory: false,
            keyboard: false,
            restoration,
            sink: sink.clone(),
            segments: Segments::new(std::process::id()),
            medium: Medium::new(false),
            inline: Inline::new(sink),
            shared_written: (0, 0),
            clear_pending: false,
        };
        execute!(
            io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            crossterm::event::EnableMouseCapture
        )?;
        let support = probe()?;
        if !force && !support.graphics {
            return Err(io::Error::other(if std::env::var_os("TMUX").is_some() {
                "tmux does not pass Kitty graphics through; run isotop directly in Ghostty or Kitty (--force-graphics bypasses detection)"
            } else {
                "Kitty graphics support was not detected. Run directly in Ghostty or Kitty; --force-graphics bypasses detection"
            }));
        }
        if support.pixels {
            write!(io::stdout(), "\x1b[?1016h")?;
            guard.pixel_mouse = true;
        }
        guard.medium = Medium::new(support.shared_memory && !direct);
        guard.shared_memory = guard.medium.offered();
        if support.keyboard {
            // Set before the push is written, so a restore that follows a failed write still pops.
            guard.keyboard = true;
            restoration.keyboard_pushed();
            execute!(io::stdout(), PushKeyboardEnhancementFlags(KEYBOARD_FLAGS))?;
        }
        Ok(guard)
    }

    /// Writes the frame's pixels as a new shared-memory object and returns its name.
    fn transmit_shared(&mut self, frame: &Frame) -> io::Result<Name> {
        // The newest serial this can use is told to the restoration before the object exists, so
        // a restore that comes in between still sweeps it.
        self.restoration
            .shared_written(self.segments.next_serial() + shared::EXISTING_NAMES);
        Ok(self.segments.send(&frame.pixels)?.name)
    }

    /// Present builds a frame's bytes, then sends them one of two ways. Through shared memory
    /// when that medium is on (the pixels go in a new object and the terminal is sent its name,
    /// written here and now), otherwise inline: the pixels are copied into a `Job` for the
    /// encoder thread to compress, encode and write, so this call returns at once.
    ///
    /// Returns the bytes the frame takes on the wire: exact for shared memory, and for an inline
    /// frame that of the newest frame the encoder has finished (0 before the first).
    pub fn present(
        &mut self,
        frame: &Frame,
        columns: u16,
        rows: u16,
        labels: &[Label],
        panels: &[Popup],
        text: &[SafeText],
    ) -> io::Result<usize> {
        // A clear that had to wait goes out before any frame; while the encoder is still busy
        // with the old size's frame there is nothing to draw on, and this frame is skipped.
        if !flush_clear(&self.inline, &mut self.clear_pending, &mut self.sink)? {
            return Ok(self.inline.last_wire());
        }
        let previous = self.image;
        self.image = if previous == IMAGE_A {
            IMAGE_B
        } else {
            IMAGE_A
        };
        let mut job = self.inline.blank();
        frame_head(&mut job.head, rows)?;
        // z below INT32_MIN/2 draws the scene under cells with a background colour, so the
        // popup panel covers it while blank cells stay transparent.
        write!(
            job.placement,
            "a=T,f=24,s={},v={},i={},c={},r={},z=-1073741825,C=1,q=2",
            frame.width, frame.height, self.image, columns, rows
        )
        .map_err(io::Error::other)?;
        frame_tail(&mut job.tail, previous, labels, panels, text, columns, rows)?;
        let now = Instant::now();
        if shared_memory_free(&self.medium, &self.inline, now) {
            match self.transmit_shared(frame) {
                Ok(name) => {
                    self.medium.succeeded();
                    self.shared_memory = true;
                    let mut output = std::mem::take(&mut job.head);
                    write!(
                        output,
                        "\x1b_G{},t=s,S={};{}\x1b\\",
                        job.placement,
                        frame.pixels.len(),
                        base64::engine::general_purpose::STANDARD.encode(name.as_str())
                    )?;
                    output.extend_from_slice(&job.tail);
                    let size = output.len();
                    // The encoder was idle when this was chosen (`shared_memory_free`) and only
                    // this thread submits, so no inline frame is half written.
                    self.sink.write_all(&output)?;
                    self.shared_written.0 += size as u64;
                    self.shared_written.1 += 1;
                    job.head = output;
                    self.inline.recycle(job);
                    return Ok(size);
                }
                Err(error) => {
                    // This frame goes inline, and the next try waits (see `Medium::failed`).
                    self.medium.failed(&error, now);
                }
            }
        }
        self.shared_memory = false;
        job.pixels.extend_from_slice(&frame.pixels);
        self.inline.submit(job)?;
        Ok(self.inline.last_wire())
    }

    /// Bytes and frames written so far, by either medium.
    pub fn written(&self) -> (u64, u64) {
        let (bytes, frames) = self.inline.written();
        (
            bytes + self.shared_written.0,
            frames + self.shared_written.1,
        )
    }

    /// Waits for the frame being written inline, and drops the one waiting. Whether it finished
    /// within the limit does not matter to the caller, which only reads the totals afterwards.
    pub fn settle(&self) {
        let _ = self.inline.settle();
    }

    /// Clears the screen and deletes every image, after a resize. When the encoder is still
    /// writing a frame past its limit nothing is written (the frame's remaining pieces would land
    /// after the clear); the request is kept and `present` performs it once the encoder is idle.
    pub fn clear(&mut self) -> io::Result<()> {
        self.clear_pending = true;
        flush_clear(&self.inline, &mut self.clear_pending, &mut self.sink).map(|_| ())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.active {
            self.inline.shutdown();
            // Only the path that claims the restoration restores: a panic hook or a signal that
            // got there first has already done it, and this finds nothing left.
            self.restoration.restore();
            let _ = terminal::disable_raw_mode();
            // No flush of standard output: every frame is flushed as it is written, the restore
            // writes with write(2), and an encoder thread stuck in a write holds its lock.
            self.segments.release();
            self.active = false;
        }
    }
}

/// The bytes of `Terminal::clear`: delete every image, clear the screen, home the cursor.
const CLEAR: &[u8] = b"\x1b_Ga=d,d=A,q=2\x1b\\\x1b[2J\x1b[H";

/// Writes the clear that `pending` asks for, through `sink`, once `inline` is idle, and says
/// whether nothing is pending now. If the encoder is still writing a frame after its limit the
/// clear is not written and stays pending, so no direct write lands inside a frame's pieces.
fn flush_clear<S: Write + Send + Clone + 'static>(
    inline: &Inline<S>,
    pending: &mut bool,
    sink: &mut impl Write,
) -> io::Result<bool> {
    if !*pending {
        return Ok(true);
    }
    match inline.direct(|| sink.write_all(CLEAR)) {
        None => Ok(false),
        Some(written) => {
            // Written or failed, the request is over: a failure is the caller's to handle.
            *pending = false;
            written.map(|()| true)
        }
    }
}

/// Whether the next frame may go through shared memory: the medium is on and no inline frame is
/// still being written (a frame that was going inline must be finished first, so one medium's
/// bytes never come inside the other's). When it is not, the frame goes inline.
fn shared_memory_free<S: Write + Send + Clone + 'static>(
    medium: &Medium,
    inline: &Inline<S>,
    now: Instant,
) -> bool {
    medium.ready(now) && inline.settle()
}

/// ST, the string terminator: the first thing a restore writes, see `restore_screen`.
const STRING_TERMINATOR: &[u8] = b"\x1b\\";

/// The bytes `restore_screen` writes, made ahead of time for the panic hook and the signal
/// handlers, which may neither allocate nor format.
fn restore_bytes(keyboard: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    // Writing to a Vec cannot fail.
    let _ = restore_screen(&mut bytes, keyboard);
    bytes
}

/// What comes before the image: starts the synchronized update, clears every row and homes the
/// cursor. The rows are cleared on each frame so that text that was drawn over the last one does
/// not outlive it.
fn frame_head(output: &mut Vec<u8>, rows: u16) -> io::Result<()> {
    write!(output, "\x1b[?2026h")?;
    for row in 1..=rows {
        write!(output, "\x1b[{row};1H\x1b[2K")?;
    }
    write!(output, "\x1b[1;1H")
}

/// What comes after the image: the previous image's deletion, the labels, the popups, the status
/// rows and the end of the synchronized update.
fn frame_tail(
    output: &mut Vec<u8>,
    previous: u32,
    labels: &[Label],
    panels: &[Popup],
    text: &[SafeText],
    columns: u16,
    rows: u16,
) -> io::Result<()> {
    write!(output, "\x1b_Ga=d,d=I,i={previous},q=2\x1b\\")?;
    for label in labels {
        draw_label(output, label, columns)?;
    }
    for panel in panels {
        draw_popup(output, panel)?;
    }
    for (index, line) in text.iter().enumerate() {
        write!(
            output,
            "\x1b[{};1H\x1b[2K\x1b[38;2;180;199;218m{}\x1b[0m",
            rows as usize + index + 1,
            line.cut(columns as usize)
        )?;
    }
    output.extend_from_slice(b"\x1b[?2026l");
    Ok(())
}

/// The keyboard flags pushed when the terminal answers the Kitty keyboard query. Shift+Space is
/// text, which the protocol sends as plain text, apart from Space or not at all, unless every
/// key is reported as an escape code (`REPORT_ALL_KEYS_AS_ESCAPE_CODES`). That in turn sends
/// Shift+q as `q` with Shift held, so `REPORT_ALTERNATE_KEYS` supplies the shifted key (`Q`, `?`,
/// `*`) that crossterm substitutes.
const KEYBOARD_FLAGS: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS)
        .union(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES);

/// Writes everything that undoes `Terminal::enter` except raw mode. It begins with a string
/// terminator (`ESC \`): a restore can land inside a graphics command that a cut-off frame write
/// left open (`ESC _G...;base64`, which only `ESC \` ends, and a terminal that does not end it on
/// the `ESC` of the next sequence would swallow all of what follows). Outside a command the
/// terminator is ignored. Then the keyboard flags: the main and the alternate screen keep
/// separate stacks of them, and the push was made on the alternate screen.
///
/// Every step is tried whatever became of the ones before it, so that one failed write cannot
/// leave the cursor hidden or the terminal on the alternate screen; the first error is returned.
///
/// `Restoration` writes the result of this function (made beforehand by `restore_bytes`) when a
/// panic or a signal ends the run, and when `Terminal` is dropped.
fn restore_screen(out: &mut impl Write, keyboard: bool) -> io::Result<()> {
    let mut first = Ok(());
    let mut note = |step: io::Result<()>| {
        if first.is_ok() {
            first = step;
        }
    };
    note(out.write_all(STRING_TERMINATOR));
    if keyboard {
        note(execute!(out, PopKeyboardEnhancementFlags));
    }
    note(write!(out, "\x1b[?2026l\x1b[?1016l\x1b_Ga=d,d=A,q=2\x1b\\"));
    note(execute!(
        out,
        crossterm::event::DisableMouseCapture,
        cursor::Show,
        terminal::LeaveAlternateScreen
    ));
    first
}

struct Support {
    /// The terminal answered the Kitty keyboard protocol query.
    keyboard: bool,
    graphics: bool,
    pixels: bool,
    shared_memory: bool,
}

/// Asks for Kitty graphics (inline and via shared memory), the Kitty keyboard protocol and SGR-Pixels mouse support, then
/// waits for the primary device attributes reply. Every terminal answers DA1 and answers in
/// order, so its arrival means any capability reply has already come; tmux and plain terminals
/// fail fast instead of timing out.
fn probe() -> io::Result<Support> {
    let name = Name::probe(std::process::id());
    let shared = shared::create(&name, &[0, 0, 0]).is_ok();
    let mut out = io::stdout().lock();
    write!(
        out,
        "\x1b_Gi={PROBE_INLINE},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"
    )?;
    if shared {
        write!(
            out,
            "\x1b_Gi={PROBE_SHARED},s=1,v=1,a=q,t=s,f=24,S=3;{}\x1b\\",
            base64::engine::general_purpose::STANDARD.encode(name.as_str())
        )?;
    }
    write!(out, "\x1b[?u\x1b[?1016$p\x1b[c")?;
    out.flush()?;
    drop(out);
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut response = Vec::new();
    while Instant::now() < deadline && !primary_attributes(&response) {
        match wait_for_input(50)? {
            Input::Timeout => continue,
            Input::Closed => break,
            Input::Ready => {}
        }
        let mut bytes = [0_u8; 512];
        // Read the fd directly: a buffered stdin would swallow input crossterm needs later.
        // SAFETY: the buffer is valid for writes of its full length.
        let count =
            unsafe { libc::read(libc::STDIN_FILENO, bytes.as_mut_ptr().cast(), bytes.len()) };
        if count <= 0 {
            break;
        }
        response.extend_from_slice(&bytes[..count as usize]);
    }
    let text = String::from_utf8_lossy(&response);
    shared::unlink(&name);
    Ok(Support {
        keyboard: keyboard_reply(&text),
        graphics: text.contains(&format!("i={PROBE_INLINE};OK")),
        pixels: text.contains("\x1b[?1016;1$y") || text.contains("\x1b[?1016;2$y"),
        shared_memory: text.contains(&format!("i={PROBE_SHARED};OK")),
    })
}

/// Whether `text` holds the reply to the keyboard query, `CSI ? flags u`; the device attributes
/// reply, `CSI ? ... c`, and mode reports, `CSI ? ... $ y`, do not match.
fn keyboard_reply(text: &str) -> bool {
    text.split("\x1b[?").skip(1).any(|rest| {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0 && rest[digits..].starts_with('u')
    })
}

/// What `wait_for_input` found on standard input.
#[derive(Debug, PartialEq, Eq)]
enum Input {
    /// Bytes (or end of file) can be read without blocking.
    Ready,
    Timeout,
    /// The descriptor is invalid or hung up with nothing left to read.
    Closed,
}

/// Waits up to `milliseconds` for standard input to become readable.
fn wait_for_input(milliseconds: i32) -> io::Result<Input> {
    // poll(2) does not support devices on macOS (see BUGS in its man page), so a terminal is
    // waited on with select(2) there.
    #[cfg(target_os = "macos")]
    if io::stdin().is_terminal() {
        return select_input(milliseconds);
    }
    poll_input(milliseconds)
}

fn poll_input(milliseconds: i32) -> io::Result<Input> {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: fd points to one valid pollfd for the duration of the call.
    let ready = unsafe { libc::poll(&mut fd, 1, milliseconds) };
    if ready < 0 {
        return Err(io::Error::last_os_error());
    }
    // Only POLLIN may lead to a read; POLLNVAL, POLLERR and a POLLHUP with no data pending
    // would otherwise reach a read that can block or fail forever.
    Ok(if ready == 0 {
        Input::Timeout
    } else if fd.revents & libc::POLLIN != 0 {
        Input::Ready
    } else {
        Input::Closed
    })
}

#[cfg(target_os = "macos")]
fn select_input(milliseconds: i32) -> io::Result<Input> {
    let mut timeout = libc::timeval {
        tv_sec: (milliseconds / 1000).into(),
        tv_usec: (milliseconds % 1000) * 1000,
    };
    // SAFETY: an all-zero fd_set is a valid empty set.
    let mut readable: libc::fd_set = unsafe { std::mem::zeroed() };
    // SAFETY: readable is an initialised fd_set; FD_ZERO and FD_SET only write inside it, and
    // STDIN_FILENO is far below FD_SETSIZE.
    unsafe {
        libc::FD_ZERO(&mut readable);
        libc::FD_SET(libc::STDIN_FILENO, &mut readable);
    }
    // SAFETY: readable and timeout are valid for the call; the write and error sets are null.
    let ready = unsafe {
        libc::select(
            libc::STDIN_FILENO + 1,
            &mut readable,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut timeout,
        )
    };
    if ready < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: readable was initialised above and is only read here.
    Ok(
        if ready > 0 && unsafe { libc::FD_ISSET(libc::STDIN_FILENO, &readable) } {
            Input::Ready
        } else {
            Input::Timeout
        },
    )
}

/// True once the buffer holds a DA1 reply, `ESC [ ? <digits and semicolons> c`.
fn primary_attributes(response: &[u8]) -> bool {
    (0..response.len()).any(|start| {
        response[start..].starts_with(b"\x1b[?") && {
            let rest = &response[start + 3..];
            let digits = rest
                .iter()
                .take_while(|b| b.is_ascii_digit() || **b == b';')
                .count();
            rest.get(digits) == Some(&b'c')
        }
    })
}

/// Writes `label` over the image, its text cut to the `columns` left of it.
pub(crate) fn draw_label(output: &mut Vec<u8>, label: &Label, columns: u16) -> io::Result<()> {
    let style = match label.tone {
        Tone::Quiet => "\x1b[38;2;150;172;198m",
        Tone::Bright => "\x1b[1;38;2;214;226;240m",
        Tone::Tag => "\x1b[38;2;255;226;170;48;2;12;19;31m",
    };
    let room = columns.saturating_sub(label.column) as usize;
    write!(
        output,
        "\x1b[{};{}H{style}{}\x1b[0m",
        label.row + 1,
        label.column + 1,
        label.text.cut(room)
    )
}

pub(crate) fn draw_popup(output: &mut Vec<u8>, popup: &Popup) -> io::Result<()> {
    let border = "\x1b[22;38;2;92;122;158;48;2;12;19;31m";
    let inner = popup.width.saturating_sub(4) as usize;
    let rule = "─".repeat(popup.width.saturating_sub(2) as usize);
    let column = popup.column + 1;
    write!(output, "\x1b[{};{column}H{border}╭{rule}╮", popup.row + 1)?;
    for (index, line) in popup.lines.iter().enumerate() {
        let style = if index == 0 {
            "\x1b[1;38;2;241;206;136m"
        } else {
            "\x1b[38;2;206;220;236m"
        };
        let text = line.cut(inner);
        // Padded by cells: `{:<inner$}` would count characters.
        let padding = " ".repeat(inner.saturating_sub(display_width(text)));
        write!(
            output,
            "\x1b[{};{column}H{border}│ {style}{text}{padding}{border} │",
            popup.row + 2 + index as u16,
        )?;
    }
    write!(
        output,
        "\x1b[{};{column}H{border}╰{rule}╯\x1b[0m",
        popup.row + 2 + popup.lines.len() as u16
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, mpsc};

    #[test]
    fn process_text_cannot_inject_terminal_controls() {
        assert_eq!(SafeText::new("bad\x1b[2J\nname"), "bad?[2J?name");
        assert_eq!(SafeText::new("abcdef").cut(3), "abc");
        let label = Label {
            column: 0,
            row: 0,
            text: SafeText::new("\x1b]0;title\x07\u{9b}2J\u{202e}rtl"),
            tone: Tone::Quiet,
        };
        let mut output = Vec::new();
        draw_label(&mut output, &label, 80).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.ends_with("?]0;title??2J?rtl\x1b[0m"), "{text:?}");
        assert_eq!(text.matches('\x1b').count(), 3, "{text:?}");
    }

    /// The text of what one popup line writes between its two borders.
    fn drawn(line: SafeText, width: u16) -> String {
        let popup = Popup {
            column: 0,
            row: 0,
            width,
            lines: vec![line],
        };
        let mut output = Vec::new();
        draw_popup(&mut output, &popup).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn popup_lines_are_cut_and_padded_by_cells() {
        let text = drawn(SafeText::new("abcdefghij"), 10);
        assert!(text.contains("mabcdef\x1b"), "{text:?}");
        let text = drawn(SafeText::new("ab"), 10);
        assert!(
            text.contains("ab    \x1b"),
            "padded to the six inner cells: {text:?}"
        );
        // A sparkline is one cell a dot, however many bytes a dot takes.
        let text = drawn(SafeText::braille("⣿⣿⣿⣿⣿⣿⣿⣿"), 10);
        assert!(text.contains("m⣿⣿⣿⣿⣿⣿\x1b"), "{text:?}");
        let text = drawn(SafeText::braille("⣿⣿"), 10);
        assert!(text.contains("⣿⣿    \x1b"), "{text:?}");
        // The same dots in an ordinary line are cleaned away.
        let text = drawn(SafeText::new("⣿⣿"), 10);
        assert!(!text.contains('⣿'), "{text:?}");
    }

    #[test]
    fn a_thread_name_with_control_bytes_is_cleaned_for_display() {
        let thread = crate::threads::Thread {
            tid: 7,
            start: 1,
            name: "w\x1b[2J\x07\n\u{202e}x".into(),
            cpu: None,
            state: 'S',
            leader: false,
        };
        assert!(thread.name.contains('\x1b'), "the name is stored raw");
        let label = Label {
            column: 0,
            row: 0,
            text: SafeText::new(&format!("thread {}", thread.name)),
            tone: Tone::Tag,
        };
        let mut output = Vec::new();
        draw_label(&mut output, &label, 80).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.ends_with("thread w?[2J???x\x1b[0m"), "{text:?}");
        assert_eq!(
            text.matches('\x1b').count(),
            3,
            "only the cursor move, the style and the reset"
        );
    }

    #[test]
    fn a_label_is_cut_to_the_room_left_of_it() {
        let label = Label {
            column: 76,
            row: 0,
            text: SafeText::new("abcdefgh"),
            tone: Tone::Quiet,
        };
        let mut output = Vec::new();
        draw_label(&mut output, &label, 80).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.ends_with("abcd\x1b[0m"), "{text:?}");
    }

    #[test]
    fn popup_height_counts_the_border_and_every_line() {
        let popup = Popup {
            column: 2,
            row: 3,
            width: 10,
            lines: vec![SafeText::new("a"), SafeText::new("b")],
        };
        assert_eq!(popup.height(), 4);
        assert!(popup.contains(2, 3) && popup.contains(11, 6));
        assert!(!popup.contains(12, 3) && !popup.contains(2, 7));
    }

    #[test]
    fn device_attributes_end_the_probe_but_mode_reports_do_not() {
        assert!(!primary_attributes(b"\x1b_Gi=31;OK\x1b\\\x1b[?1016;2$y"));
        assert!(primary_attributes(
            b"\x1b_Gi=31;OK\x1b\\\x1b[?1016;2$y\x1b[?62;22;52c"
        ));
        assert!(primary_attributes(b"\x1b[?1;2c"));
    }

    #[test]
    fn the_keyboard_query_reply_is_told_from_the_other_replies() {
        assert!(keyboard_reply("\x1b[?0u\x1b[?62;22c"));
        assert!(keyboard_reply(
            "\x1b_Gi=31;OK\x1b\\\x1b[?1016;2$y\x1b[?15u\x1b[?1;2c"
        ));
        assert!(!keyboard_reply(
            "\x1b_Gi=31;OK\x1b\\\x1b[?1016;2$y\x1b[?62;22c"
        ));
        assert!(!keyboard_reply("\x1b[?u"));
    }

    #[test]
    fn keyboard_enhancement_is_popped_on_restore() {
        let mut out = Vec::new();
        restore_screen(&mut out, true).unwrap();
        let text = String::from_utf8(out).unwrap();
        let pop = text.find("\x1b[<1u").expect("the flags are popped");
        let leave = text
            .find("\x1b[?1049l")
            .expect("the alternate screen is left");
        assert!(pop < leave, "the stack to pop is the alternate screen's");
        let mut push = Vec::new();
        execute!(&mut push, PushKeyboardEnhancementFlags(KEYBOARD_FLAGS)).unwrap();
        assert_eq!(push, b"\x1b[>13u", "disambiguate, alternate keys, all keys");
        let mut out = Vec::new();
        restore_screen(&mut out, false).unwrap();
        assert!(!String::from_utf8(out).unwrap().contains("\x1b[<"));
    }

    /// A writer whose first write fails and whose later ones land in `written`.
    struct FailsOnce {
        failed: bool,
        written: Vec<u8>,
    }

    impl Write for FailsOnce {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if !self.failed {
                self.failed = true;
                return Err(io::Error::other("the first write fails"));
            }
            self.written.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_failed_write_does_not_leave_the_alternate_screen_in_place() {
        let mut out = FailsOnce {
            failed: false,
            written: Vec::new(),
        };
        assert!(
            restore_screen(&mut out, true).is_err(),
            "the error is reported"
        );
        let text = String::from_utf8(out.written).unwrap();
        assert!(
            text.contains("\x1b[?1049l"),
            "but the screen is left: {text:?}"
        );
        assert!(text.contains("\x1b[?25h"), "and the cursor shown");
    }

    /// What a strict terminal does with bytes, as far as these tests need: control sequences end
    /// at their final byte, an ESC aborts one, and a graphics command (an APC) swallows
    /// everything, an ESC that does not begin `ESC \` included, until `ESC \`. A real terminal
    /// may end the command sooner; this is the worst case the restore must survive.
    #[derive(Default)]
    struct Parser {
        state: Mode,
        pending: Vec<u8>,
        /// Every control sequence that ran, without its introducer: `?1049l`, `<1u`.
        sequences: Vec<String>,
        /// Every graphics command that ended, without its introducer.
        commands: Vec<Vec<u8>>,
    }

    #[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
    enum Mode {
        #[default]
        Ground,
        Escape,
        Sequence,
        Command,
        CommandEscape,
    }

    impl Parser {
        fn feed(&mut self, bytes: &[u8]) {
            for &byte in bytes {
                self.state = match (self.state, byte) {
                    (Mode::Ground | Mode::Escape, 0x1b) => Mode::Escape,
                    (Mode::Ground, _) => Mode::Ground,
                    (Mode::Escape, b'_') => {
                        self.pending.clear();
                        Mode::Command
                    }
                    (Mode::Escape, b'[') => {
                        self.pending.clear();
                        Mode::Sequence
                    }
                    (Mode::Escape, _) => Mode::Ground,
                    (Mode::Sequence, 0x1b) => Mode::Escape,
                    (Mode::Sequence, 0x40..=0x7e) => {
                        self.pending.push(byte);
                        self.sequences
                            .push(String::from_utf8_lossy(&self.pending).into_owned());
                        Mode::Ground
                    }
                    (Mode::Sequence, _) => {
                        self.pending.push(byte);
                        Mode::Sequence
                    }
                    (Mode::Command, 0x1b) => Mode::CommandEscape,
                    (Mode::Command, _) => {
                        self.pending.push(byte);
                        Mode::Command
                    }
                    // An ESC after an ESC is the start of a terminator again.
                    (Mode::CommandEscape, 0x1b) => Mode::CommandEscape,
                    (Mode::CommandEscape, b'\\') => {
                        self.commands.push(std::mem::take(&mut self.pending));
                        Mode::Ground
                    }
                    (Mode::CommandEscape, _) => {
                        self.pending.push(0x1b);
                        self.pending.push(byte);
                        Mode::Command
                    }
                };
            }
        }

        fn ran(&self, sequence: &str) -> bool {
            self.sequences.iter().any(|ran| ran == sequence)
        }
    }

    /// Every sequence of `restore_screen` that a terminal has to act on.
    const RESTORED: [&str; 4] = ["?2026l", "?1016l", "?1049l", "?25h"];

    #[test]
    fn the_restore_bytes_begin_with_a_string_terminator() {
        for keyboard in [false, true] {
            assert!(
                restore_bytes(keyboard).starts_with(b"\x1b\\"),
                "keyboard = {keyboard}"
            );
        }
    }

    #[test]
    fn a_restore_ends_a_graphics_command_a_cut_off_frame_left_open() {
        let open = b"\x1b_Gm=1,q=2;QUFBQUFBQUFBQUFB".as_slice();
        // The model is strict, so without the terminator the first sequences are swallowed, up
        // to the `ESC \` of the restore's own graphics command: the keyboard pop and the end of
        // the synchronized update would be lost, and the terminal left frozen in it.
        let mut without = Parser::default();
        without.feed(open);
        without.feed(&restore_bytes(true)[STRING_TERMINATOR.len()..]);
        assert!(!without.ran("<1u") && !without.ran("?2026l") && !without.ran("?1016l"));
        // With it, the command ends and every sequence is parsed.
        for keyboard in [false, true] {
            let mut parser = Parser::default();
            parser.feed(open);
            assert_eq!(parser.state, Mode::Command);
            parser.feed(&restore_bytes(keyboard));
            assert_eq!(parser.state, Mode::Ground, "keyboard = {keyboard}");
            assert_eq!(parser.commands.len(), 2, "the open command, then a=d");
            for sequence in RESTORED {
                assert!(
                    parser.ran(sequence),
                    "{sequence} ran, keyboard = {keyboard}"
                );
            }
            assert_eq!(parser.ran("<1u"), keyboard);
        }
    }

    #[test]
    fn a_restore_lands_cleanly_wherever_a_write_was_cut() {
        // Inside a control sequence, just after an ESC, inside a command and inside its ESC.
        for cut in [
            b"\x1b[38;2;18".as_slice(),
            b"\x1b",
            b"\x1b_Gm=1;QUJD",
            b"\x1b_Gm=1;QUJD\x1b",
            b"\x1b[?2026h\x1b[1;1H",
            b"",
        ] {
            let mut parser = Parser::default();
            parser.feed(cut);
            parser.feed(&restore_bytes(true));
            assert_eq!(parser.state, Mode::Ground, "after {cut:?}");
            for sequence in RESTORED {
                assert!(parser.ran(sequence), "{sequence} ran after {cut:?}");
            }
        }
    }

    /// How the sink of `Cut` ends the frame.
    #[derive(Clone, Copy)]
    enum Cutting {
        /// The restore comes between two pieces, from another thread.
        Between,
        /// The restore comes inside a piece, from the thread that is writing it (a signal
        /// handler on the encoder thread): half the piece is out, and the rest never is.
        Inside,
    }

    /// A frame sink like `FrameSink`, on a restoration of its own, that restores at its
    /// `after`-th write.
    #[derive(Clone)]
    struct Cut {
        restoration: &'static Restoration,
        written: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        after: usize,
        cutting: Cutting,
        /// What the terminal had been sent when the restore began.
        before_restore: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    }

    impl Write for Cut {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let number = self
                .writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            let restore_here = number == self.after;
            let inside = restore_here && matches!(self.cutting, Cutting::Inside);
            self.restoration.frame(|| {
                let end = if inside { bytes.len() / 2 } else { bytes.len() };
                self.written
                    .lock()
                    .unwrap()
                    .extend_from_slice(&bytes[..end]);
                if inside {
                    *self.before_restore.lock().unwrap() =
                        Some(self.written.lock().unwrap().clone());
                    assert!(self.restoration.restore());
                }
            });
            if restore_here && !inside {
                *self.before_restore.lock().unwrap() = Some(self.written.lock().unwrap().clone());
                assert!(self.restoration.restore());
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_frame_cut_off_by_a_restore_leaves_a_terminal_that_is_restored() {
        let mut state = 0x2545_f491_u32;
        let mut pixels = vec![0_u8; 260_000];
        for byte in &mut pixels {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (state >> 24) as u8;
        }
        for cutting in [Cutting::Between, Cutting::Inside] {
            for after in 1..=5 {
                let mut ends = [0; 2];
                // SAFETY: ends has room for the two descriptors pipe writes.
                assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
                let restoration: &'static Restoration = Box::leak(Box::new(Restoration::new()));
                restoration.arm(ends[1], -1, None, restore_bytes(false), restore_bytes(true));
                let sink = Cut {
                    restoration,
                    written: Default::default(),
                    writes: Default::default(),
                    after,
                    cutting,
                    before_restore: Default::default(),
                };
                let mut inline = Inline::without_thread(sink.clone());
                let mut job = inline.blank();
                job.head.extend_from_slice(b"\x1b[?2026h\x1b[1;1H");
                job.placement = "a=T,f=24,s=2,v=2,i=100,c=10,r=5,q=2".to_owned();
                job.pixels = pixels.clone();
                job.tail
                    .extend_from_slice(b"\x1b_Ga=d,d=I,i=101,q=2\x1b\\text\x1b[?2026l");
                inline.submit(job).unwrap();
                let mut restored = [0_u8; 256];
                // SAFETY: the buffer is valid for writes of its length; the descriptor is open.
                let count = unsafe { libc::read(ends[0], restored.as_mut_ptr().cast(), 256) };
                // SAFETY: both descriptors were opened above and are not used again.
                unsafe {
                    libc::close(ends[0]);
                    libc::close(ends[1]);
                }
                assert!(count > 0, "the restore wrote its bytes");
                let sent = sink.written.lock().unwrap().clone();
                let at_restore = sink.before_restore.lock().unwrap().clone().unwrap();
                assert_eq!(sent, at_restore, "nothing was written after the restore");
                let mut parser = Parser::default();
                parser.feed(&sent);
                if matches!(cutting, Cutting::Between) {
                    assert_eq!(
                        parser.state,
                        Mode::Ground,
                        "a restore between two pieces finds the terminal outside any command"
                    );
                }
                parser.feed(&restored[..count as usize]);
                assert_eq!(parser.state, Mode::Ground, "after {after} writes");
                for sequence in RESTORED {
                    assert!(parser.ran(sequence), "{sequence}: {after} writes");
                }
            }
        }
    }

    /// A sink for the encoder that keeps what it writes and holds every write until it is let
    /// through, telling when one starts.
    #[derive(Clone)]
    struct Gate {
        written: Arc<Mutex<Vec<u8>>>,
        started: mpsc::Sender<()>,
        pass: Arc<Mutex<mpsc::Receiver<()>>>,
    }

    impl Write for Gate {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let _ = self.started.send(());
            // A dropped release means the test is over: let the write go.
            let _ = self.pass.lock().unwrap().recv();
            self.written.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// An encoder whose first frame is held in its sink.
    struct Stalled {
        inline: Inline<Gate>,
        /// What the sink has written so far.
        frame: Arc<Mutex<Vec<u8>>>,
        /// Lets the held write go.
        release: mpsc::Sender<()>,
    }

    /// An encoder holding its first frame in the sink, with short limits so a settle that cannot
    /// finish gives up at once.
    fn stalled() -> Stalled {
        let (started, started_receiver) = mpsc::channel();
        let (release, pass) = mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = Gate {
            written: Arc::clone(&written),
            started,
            pass: Arc::new(Mutex::new(pass)),
        };
        let mut inline =
            Inline::new(sink).with_limits(Duration::from_millis(60), Duration::from_millis(60));
        let mut job = inline.blank();
        job.head.extend_from_slice(b"HEAD");
        job.placement = "a=T,f=24,s=1,v=1,i=100,q=2".to_owned();
        job.pixels.extend_from_slice(&[1, 2, 3]);
        job.tail.extend_from_slice(b"TAIL");
        inline.submit(job).unwrap();
        started_receiver.recv().unwrap();
        Stalled {
            inline,
            frame: written,
            release,
        }
    }

    /// Waits until the encoder has finished the frame it was writing.
    fn until_written(inline: &Inline<Gate>) {
        let begun = Instant::now();
        while inline.written().1 == 0 {
            assert!(begun.elapsed() < Duration::from_secs(10), "never written");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_clear_waits_for_a_frame_the_encoder_cannot_finish_and_is_written_after_it() {
        let Stalled {
            inline,
            frame,
            release,
        } = stalled();
        let mut screen = Vec::new();
        let mut pending = true;
        // The settle runs out while the worker holds the frame: nothing goes to the screen.
        assert!(!flush_clear(&inline, &mut pending, &mut screen).unwrap());
        assert!(pending, "the request is kept");
        assert!(screen.is_empty(), "no clear before the frame is done");
        assert!(frame.lock().unwrap().is_empty(), "the frame is still held");
        // Asking again changes nothing while it is held.
        assert!(!flush_clear(&inline, &mut pending, &mut screen).unwrap());
        assert!(screen.is_empty());
        release.send(()).unwrap();
        until_written(&inline);
        assert!(flush_clear(&inline, &mut pending, &mut screen).unwrap());
        assert!(!pending, "performed once the worker is idle");
        assert_eq!(screen, CLEAR);
        let frame = frame.lock().unwrap().clone();
        assert!(
            frame.starts_with(b"HEAD") && frame.ends_with(b"TAIL"),
            "the whole frame went out, and before the clear"
        );
        // Nothing pending: nothing written.
        screen.clear();
        assert!(flush_clear(&inline, &mut pending, &mut screen).unwrap());
        assert!(screen.is_empty());
    }

    #[test]
    fn a_frame_does_not_go_through_shared_memory_while_an_inline_one_is_being_written() {
        let Stalled {
            inline, release, ..
        } = stalled();
        let medium = Medium::new(true);
        let now = Instant::now();
        assert!(medium.ready(now));
        assert!(
            !shared_memory_free(&medium, &inline, now),
            "the inline frame is half written: this one goes inline too"
        );
        release.send(()).unwrap();
        until_written(&inline);
        assert!(shared_memory_free(&medium, &inline, now));
        assert!(!shared_memory_free(&Medium::new(false), &inline, now));
    }

    #[test]
    fn a_clear_that_fails_to_write_is_reported_once_and_not_kept() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let inline = Inline::without_thread(Vec::<u8>::new());
        let mut pending = true;
        let error = flush_clear(&inline, &mut pending, &mut Broken).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert!(!pending);
    }
}
