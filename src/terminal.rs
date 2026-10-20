use std::collections::VecDeque;
use std::ffi::CString;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{cursor, execute, terminal};
use flate2::{Compression, write::ZlibEncoder};

use crate::render::Frame;

mod text;

pub use text::SafeText;
use text::display_width;

pub struct Terminal {
    active: bool,
    image: u32,
    /// The terminal reports mouse positions in pixels (SGR-Pixels, mode 1016).
    pub pixel_mouse: bool,
    /// Frames travel through POSIX shared memory instead of compressed inline data.
    pub shared_memory: bool,
    /// The Kitty keyboard protocol is on (`KEYBOARD_FLAGS` pushed), so Shift+Space arrives apart
    /// from Space. Restoring the terminal pops it.
    pub keyboard: bool,
    serial: u64,
    /// Recently written shared-memory names; the terminal unlinks each after reading it, and
    /// anything it never read is removed once it is old enough or when isotop exits.
    pending: VecDeque<String>,
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
        terminal::enable_raw_mode()?;
        let mut guard = Self {
            active: true,
            image: 100,
            pixel_mouse: false,
            shared_memory: false,
            keyboard: false,
            serial: 0,
            pending: VecDeque::new(),
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
        guard.shared_memory = support.shared_memory && !direct;
        if support.keyboard {
            // Set before the push is written, so a restore that follows a failed write still pops.
            guard.keyboard = true;
            execute!(io::stdout(), PushKeyboardEnhancementFlags(KEYBOARD_FLAGS))?;
        }
        Ok(guard)
    }

    fn transmit_shared(&mut self, frame: &Frame) -> io::Result<String> {
        let name = frame_name(std::process::id(), self.serial);
        self.serial += 1;
        write_shared(&name, &frame.pixels)?;
        self.pending.push_back(name.clone());
        while self.pending.len() > 8 {
            if let Some(old) = self.pending.pop_front() {
                unlink_shared(&old);
            }
        }
        Ok(name)
    }

    pub fn present(
        &mut self,
        frame: &Frame,
        columns: u16,
        rows: u16,
        labels: &[Label],
        panels: &[Popup],
        text: &[SafeText],
    ) -> io::Result<usize> {
        let previous = self.image;
        self.image = if self.image == 100 { 101 } else { 100 };
        let mut output = Vec::with_capacity(8192);
        write!(output, "\x1b[?2026h")?;
        for row in 1..=rows {
            write!(output, "\x1b[{row};1H\x1b[2K")?;
        }
        write!(output, "\x1b[1;1H")?;
        // z below INT32_MIN/2 draws the scene under cells with a background colour, so the
        // popup panel covers it while blank cells stay transparent.
        let placement = format!(
            "a=T,f=24,s={},v={},i={},c={},r={},z=-1073741825,C=1,q=2",
            frame.width, frame.height, self.image, columns, rows
        );
        let shared = if self.shared_memory {
            match self.transmit_shared(frame) {
                Ok(name) => Some(name),
                Err(_) => {
                    self.shared_memory = false;
                    None
                }
            }
        } else {
            None
        };
        if let Some(name) = shared {
            write!(
                output,
                "\x1b_G{placement},t=s,S={};{}\x1b\\",
                frame.pixels.len(),
                base64::engine::general_purpose::STANDARD.encode(name)
            )?;
        } else {
            let mut compressor = ZlibEncoder::new(
                Vec::with_capacity(frame.pixels.len() / 4),
                Compression::fast(),
            );
            compressor.write_all(&frame.pixels)?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(compressor.finish()?);
            output.reserve(encoded.len() + 1024);
            let chunks = encoded.as_bytes().chunks(4096);
            let count = chunks.len();
            for (index, chunk) in chunks.enumerate() {
                let more = usize::from(index + 1 < count);
                if index == 0 {
                    write!(output, "\x1b_G{placement},t=d,o=z,m={more};")?;
                } else {
                    write!(output, "\x1b_Gm={more},q=2;")?;
                }
                output.extend_from_slice(chunk);
                output.extend_from_slice(b"\x1b\\");
            }
        }
        write!(output, "\x1b_Ga=d,d=I,i={previous},q=2\x1b\\")?;
        for label in labels {
            draw_label(&mut output, label, columns)?;
        }
        for panel in panels {
            draw_popup(&mut output, panel)?;
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
        let size = output.len();
        let mut out = io::stdout().lock();
        out.write_all(&output)?;
        out.flush()?;
        Ok(size)
    }

    pub fn clear(&mut self) -> io::Result<()> {
        let mut out = io::stdout().lock();
        write!(out, "\x1b_Ga=d,d=A,q=2\x1b\\\x1b[2J\x1b[H")?;
        out.flush()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.active {
            let _ = restore_screen(&mut io::stdout(), self.keyboard);
            let _ = terminal::disable_raw_mode();
            let _ = io::stdout().flush();
            for name in self.pending.drain(..) {
                unlink_shared(&name);
            }
            self.active = false;
        }
    }
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

/// Writes everything that undoes `Terminal::enter` except raw mode. The keyboard flags go first:
/// the main and the alternate screen keep separate stacks of them, and the push was made on the
/// alternate screen.
///
/// Every step is tried whatever became of the ones before it, so that one failed write cannot
/// leave the cursor hidden or the terminal on the alternate screen; the first error is returned.
fn restore_screen(out: &mut impl Write, keyboard: bool) -> io::Result<()> {
    let mut first = Ok(());
    let mut note = |step: io::Result<()>| {
        if first.is_ok() {
            first = step;
        }
    };
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

/// macOS limits a shared-memory name, leading slash included, to `PSHMNAMLEN` bytes; longer
/// names fail in `shm_open` with ENAMETOOLONG.
const SHARED_NAME_LIMIT: usize = 31;

/// Name of the shared-memory object for one frame. The serial wraps at twelve digits so the
/// name stays within `SHARED_NAME_LIMIT` for any pid (`/isotop-` + 10 + `-` + 12 = 31 bytes);
/// only a handful of frames are alive at once, so a wrapped serial never collides.
fn frame_name(pid: u32, serial: u64) -> String {
    format!("/isotop-{pid}-{}", serial % 1_000_000_000_000)
}

/// Writes pixels into a new POSIX shared-memory object of exactly their size.
fn write_shared(name: &str, data: &[u8]) -> io::Result<()> {
    let path = CString::new(name).map_err(io::Error::other)?;
    // SAFETY: path is NUL-terminated; O_EXCL guarantees a fresh object private to this user.
    let fd = unsafe {
        libc::shm_open(
            path.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is the descriptor just returned by shm_open and is owned here.
    let mut file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    let written = fill_shared(&mut file, data);
    if written.is_err() {
        unlink_shared(name);
    }
    written
}

/// Linux tmpfs accepts write(), which copies a whole frame in one call instead of faulting in a
/// fresh mapping page by page.
#[cfg(target_os = "linux")]
fn fill_shared(file: &mut std::fs::File, data: &[u8]) -> io::Result<()> {
    file.write_all(data)
}

/// Other systems only allow shared memory to be sized and mapped.
#[cfg(not(target_os = "linux"))]
fn fill_shared(file: &mut std::fs::File, data: &[u8]) -> io::Result<()> {
    file.set_len(data.len() as u64)?;
    // SAFETY: the object was sized to data.len() bytes just above and is mapped shared for
    // reading and writing (some systems refuse a write-only mapping); the mapping is released
    // before returning and never escapes this block.
    unsafe {
        let address = libc::mmap(
            std::ptr::null_mut(),
            data.len(),
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            std::os::fd::AsRawFd::as_raw_fd(file),
            0,
        );
        if address == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), address.cast(), data.len());
        libc::munmap(address, data.len());
    }
    Ok(())
}

/// Removes a shared-memory object. The terminal unlinks each frame itself after reading it, so
/// a second unlink finds nothing (ENOENT); that and every other failure is ignored.
fn unlink_shared(name: &str) {
    if let Ok(path) = CString::new(name) {
        // SAFETY: path is NUL-terminated and outlives the call; the result is deliberately unused.
        unsafe { libc::shm_unlink(path.as_ptr()) };
    }
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
    let name = format!("/isotop-{}-probe", std::process::id());
    debug_assert!(name.len() <= SHARED_NAME_LIMIT);
    let shared = write_shared(&name, &[0, 0, 0]).is_ok();
    let mut out = io::stdout().lock();
    write!(out, "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\")?;
    if shared {
        write!(
            out,
            "\x1b_Gi=32,s=1,v=1,a=q,t=s,f=24,S=3;{}\x1b\\",
            base64::engine::general_purpose::STANDARD.encode(&name)
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
    unlink_shared(&name);
    Ok(Support {
        keyboard: keyboard_reply(&text),
        graphics: text.contains("i=31;OK"),
        pixels: text.contains("\x1b[?1016;1$y") || text.contains("\x1b[?1016;2$y"),
        shared_memory: text.contains("i=32;OK"),
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

    #[test]
    fn frame_names_fit_the_macos_shared_memory_limit() {
        for (pid, serial) in [(0, 0), (99_999, 1 << 20), (u32::MAX, u64::MAX)] {
            let name = frame_name(pid, serial);
            assert!(name.len() <= SHARED_NAME_LIMIT, "{name} is too long");
            assert!(name.starts_with("/isotop-"));
        }
        assert_eq!(frame_name(7, 3), "/isotop-7-3");
        assert_eq!(
            frame_name(u32::MAX, 999_999_999_999).len(),
            SHARED_NAME_LIMIT,
            "the longest name should sit exactly at the limit"
        );
    }

    /// Reads a shared-memory object back through shm_open and mmap, as a terminal does.
    fn read_shared(name: &str, length: usize) -> Option<Vec<u8>> {
        let path = CString::new(name).unwrap();
        // SAFETY: path is NUL-terminated; the descriptor is closed below.
        let fd = unsafe { libc::shm_open(path.as_ptr(), libc::O_RDONLY, 0) };
        if fd < 0 {
            return None;
        }
        // SAFETY: the object holds at least length bytes (the caller wrote that many); the
        // mapping is copied out and released before the descriptor is closed.
        let bytes = unsafe {
            let address = libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd,
                0,
            );
            assert_ne!(address, libc::MAP_FAILED);
            let bytes = std::slice::from_raw_parts(address.cast::<u8>(), length).to_vec();
            libc::munmap(address, length);
            bytes
        };
        // SAFETY: fd was opened above and is not used again.
        unsafe { libc::close(fd) };
        Some(bytes)
    }

    #[test]
    fn shared_memory_frames_round_trip_and_are_cleaned_up() {
        let name = format!("/isotop-test-{}", std::process::id());
        let pixels: Vec<u8> = (0..300_u32).map(|i| (i * 7) as u8).collect();
        write_shared(&name, &pixels).unwrap();
        assert_eq!(read_shared(&name, pixels.len()).unwrap(), pixels);
        #[cfg(target_os = "linux")]
        {
            let file = std::path::Path::new("/dev/shm").join(&name[1..]);
            assert_eq!(std::fs::read(&file).unwrap(), pixels);
            unlink_shared(&name);
            assert!(!file.exists());
        }
        #[cfg(not(target_os = "linux"))]
        unlink_shared(&name);
        assert!(read_shared(&name, 1).is_none());
    }

    #[test]
    fn unlinking_a_frame_the_terminal_already_removed_is_harmless() {
        let name = format!("/isotop-test-twice-{}", std::process::id());
        write_shared(&name, &[1, 2, 3]).unwrap();
        unlink_shared(&name);
        unlink_shared(&name);
        unlink_shared("/isotop-test-never-created");
    }
}
