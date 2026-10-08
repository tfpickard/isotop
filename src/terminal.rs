use std::collections::VecDeque;
use std::ffi::CString;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use crossterm::{cursor, execute, terminal};
use flate2::{Compression, write::ZlibEncoder};

use crate::render::Frame;

pub struct Terminal {
    active: bool,
    image: u32,
    /// The terminal reports mouse positions in pixels (SGR-Pixels, mode 1016).
    pub pixel_mouse: bool,
    /// Frames travel through POSIX shared memory instead of compressed inline data.
    pub shared_memory: bool,
    serial: u64,
    /// Recently written shared-memory names; the terminal unlinks each after reading it, and
    /// anything it never read is removed once it is old enough or when isotop exits.
    pending: VecDeque<String>,
}

pub struct Popup {
    pub column: u16,
    pub row: u16,
    pub width: u16,
    pub lines: Vec<String>,
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
    pub text: String,
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
        Ok(guard)
    }

    fn transmit_shared(&mut self, frame: &Frame) -> io::Result<String> {
        let name = format!("/isotop-{}-{}", std::process::id(), self.serial);
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
        text: &[String],
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
                clean_text(&label.text, room)
            )?;
        }
        for panel in panels {
            draw_popup(&mut output, panel)?;
        }
        for (index, line) in text.iter().enumerate() {
            write!(
                output,
                "\x1b[{};1H\x1b[2K\x1b[38;2;180;199;218m{}\x1b[0m",
                rows as usize + index + 1,
                clean_text(line, columns as usize)
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
            let _ = write!(
                io::stdout(),
                "\x1b[?2026l\x1b[?1016l\x1b_Ga=d,d=A,q=2\x1b\\"
            );
            let _ = execute!(
                io::stdout(),
                crossterm::event::DisableMouseCapture,
                cursor::Show,
                terminal::LeaveAlternateScreen
            );
            let _ = terminal::disable_raw_mode();
            let _ = io::stdout().flush();
            for name in self.pending.drain(..) {
                unlink_shared(&name);
            }
            self.active = false;
        }
    }
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
    // SAFETY: the object is data.len() bytes long and mapped shared for writing; the mapping is
    // released before returning and never escapes this block.
    unsafe {
        let address = libc::mmap(
            std::ptr::null_mut(),
            data.len(),
            libc::PROT_WRITE,
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

/// Removes a shared-memory object; one the terminal already consumed is simply gone.
fn unlink_shared(name: &str) {
    if let Ok(path) = CString::new(name) {
        // SAFETY: path is NUL-terminated; failure (typically ENOENT) needs no handling.
        unsafe { libc::shm_unlink(path.as_ptr()) };
    }
}

struct Support {
    graphics: bool,
    pixels: bool,
    shared_memory: bool,
}

/// Asks for Kitty graphics (inline and via shared memory) and SGR-Pixels mouse support, then
/// waits for the primary device attributes reply. Every terminal answers DA1 and answers in
/// order, so its arrival means any capability reply has already come; tmux and plain terminals
/// fail fast instead of timing out.
fn probe() -> io::Result<Support> {
    let name = format!("/isotop-{}-probe", std::process::id());
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
    write!(out, "\x1b[?1016$p\x1b[c")?;
    out.flush()?;
    drop(out);
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut response = Vec::new();
    while Instant::now() < deadline && !primary_attributes(&response) {
        let mut fd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: fd points to one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut fd, 1, 50) };
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if ready == 0 {
            continue;
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
        graphics: text.contains("i=31;OK"),
        pixels: text.contains("\x1b[?1016;1$y") || text.contains("\x1b[?1016;2$y"),
        shared_memory: text.contains("i=32;OK"),
    })
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

fn draw_popup(output: &mut Vec<u8>, popup: &Popup) -> io::Result<()> {
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
        write!(
            output,
            "\x1b[{};{column}H{border}│ {style}{:<inner$}{border} │",
            popup.row + 2 + index as u16,
            clean_text(line, inner)
        )?;
    }
    write!(
        output,
        "\x1b[{};{column}H{border}╰{rule}╯\x1b[0m",
        popup.row + 2 + popup.lines.len() as u16
    )
}

pub fn clean_text(text: &str, width: usize) -> String {
    // Process names and command lines are untrusted terminal input. ASCII also keeps cell widths exact.
    text.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .take(width)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_text_cannot_inject_terminal_controls() {
        assert_eq!(clean_text("bad\x1b[2J\nname", 80), "bad?[2J?name");
        assert_eq!(clean_text("abcdef", 3), "abc");
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
    fn shared_memory_frames_round_trip_and_are_cleaned_up() {
        let name = format!("/isotop-test-{}", std::process::id());
        let pixels: Vec<u8> = (0..300_u32).map(|i| (i * 7) as u8).collect();
        write_shared(&name, &pixels).unwrap();
        let file = std::path::Path::new("/dev/shm").join(&name[1..]);
        assert_eq!(std::fs::read(&file).unwrap(), pixels);
        unlink_shared(&name);
        assert!(!file.exists());
    }
}
