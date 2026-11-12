//! Frames through POSIX shared memory (Kitty graphics `t=s`): the names of the objects, writing
//! one, the ring that removes them again, and when to try the medium after it failed.
//!
//! # Why a segment is never reused
//!
//! The Kitty graphics protocol says the terminal unlinks a shared-memory object after it has read
//! it, and isotop sends `q=2` (no response), so the terminal never says when it has finished: there
//! is no read-complete signal. A segment that was written again for the next frame could be read
//! half-new by a terminal that is still decoding the last one (a torn frame), or already be gone
//! when the terminal unlinked it. A ring of segments needs that signal to know which one is free.
//! So each frame gets a new object, and what this module does instead is make that cheap and safe:
//! the name is built on the stack (`Name`, no allocation and no formatting machinery, so the
//! panic and signal paths of `restore` can use it too), the object is created with `O_EXCL`, and
//! a name that already exists is skipped, never unlinked or written (`EXISTING_NAMES`).
//!
//! Note for the record: this was decided from the protocol's text and from isotop's own use of
//! `q=2`; the Ghostty and Kitty source was not read, so what either does with an object it has
//! finished reading (unlink at once, or after a delay) is not checked here. The design does not
//! depend on it: an object the terminal already removed makes the later unlink a harmless ENOENT.

use std::collections::VecDeque;
use std::ffi::CStr;
use std::fmt;
use std::io;
use std::time::{Duration, Instant};

/// macOS limits a shared-memory name, leading slash included, to `PSHMNAMLEN` bytes; longer
/// names fail in `shm_open` with ENAMETOOLONG.
pub const NAME_LIMIT: usize = 31;

/// The room a name takes: its bytes and the NUL that ends it.
const NAME_BYTES: usize = NAME_LIMIT + 1;

/// How many frames' names are kept: the terminal unlinks each object after reading it, and any it
/// never read are removed when the one `NAMES_KEPT` frames newer is written, or when isotop exits.
pub const NAMES_KEPT: usize = 8;

/// How many names after a taken one are tried before the frame goes inline instead.
pub const EXISTING_NAMES: u64 = 4;

/// How many of the newest serials the panic and signal paths unlink (`unlink_recent`). They have
/// no list of what the app made, so they sweep the range it could have left: every name written
/// is within `NAMES_KEPT` of the newest, plus the names skipped over by `EXISTING_NAMES`.
pub const SWEPT_SERIALS: u64 = 64;

/// The serial wraps at twelve digits so a name stays within `NAME_LIMIT` for any pid
/// (`/isotop-` + 10 + `-` + 12 = 31 bytes); only a handful of objects are alive at once, so a
/// wrapped serial never collides.
const SERIAL_MODULUS: u64 = 1_000_000_000_000;

/// The name of one shared-memory object, held on the stack: the bytes, a NUL, and the length.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Name {
    bytes: [u8; NAME_BYTES],
    length: usize,
}

impl Name {
    fn new() -> Self {
        Self {
            bytes: [0; NAME_BYTES],
            length: 0,
        }
    }

    fn push(&mut self, text: &[u8]) {
        let end = (self.length + text.len()).min(NAME_LIMIT);
        let count = end - self.length;
        self.bytes[self.length..end].copy_from_slice(&text[..count]);
        self.length = end;
    }

    /// Appends `number` in decimal; no allocation, so it is safe in a signal handler.
    fn push_number(&mut self, mut number: u64) {
        let mut digits = [0_u8; 20];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (number % 10) as u8;
            number /= 10;
            if number == 0 {
                break;
            }
        }
        self.push(&digits[start..]);
    }

    /// `/isotop-<pid>-<serial>`, the object of one frame.
    pub fn frame(pid: u32, serial: u64) -> Self {
        let mut name = Self::new();
        name.push(b"/isotop-");
        name.push_number(u64::from(pid));
        name.push(b"-");
        name.push_number(serial % SERIAL_MODULUS);
        name
    }

    /// `/isotop-<pid>-probe`, the object the capability probe offers the terminal.
    pub fn probe(pid: u32) -> Self {
        let mut name = Self::new();
        name.push(b"/isotop-");
        name.push_number(u64::from(pid));
        name.push(b"-probe");
        name
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.length]).unwrap_or("")
    }

    fn as_c_str(&self) -> &CStr {
        // The buffer is zero after the length, so the NUL is always there.
        CStr::from_bytes_until_nul(&self.bytes).unwrap_or_default()
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_str())
    }
}

/// Creates the object `name`, which must not exist (`O_EXCL`), and fills it with `data`. The
/// object is exactly `data.len()` bytes. A failure after the creation removes it again.
pub fn create(name: &Name, data: &[u8]) -> io::Result<()> {
    // SAFETY: the name is NUL-terminated and lives through the call; O_EXCL guarantees a fresh
    // object private to this user.
    let fd = unsafe {
        libc::shm_open(
            name.as_c_str().as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is the descriptor just returned by shm_open and is owned here.
    let mut file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    let written = fill(&mut file, data);
    if written.is_err() {
        unlink(name);
    }
    written
}

/// Linux tmpfs accepts write(), which copies a whole frame in one call instead of faulting in a
/// fresh mapping page by page.
#[cfg(target_os = "linux")]
fn fill(file: &mut std::fs::File, data: &[u8]) -> io::Result<()> {
    io::Write::write_all(file, data)
}

/// Other systems only allow shared memory to be sized and mapped.
#[cfg(not(target_os = "linux"))]
fn fill(file: &mut std::fs::File, data: &[u8]) -> io::Result<()> {
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
/// a second unlink finds nothing (ENOENT); that and every other failure is ignored. Safe in a
/// signal handler: `shm_unlink` is async-signal-safe and nothing here allocates.
pub fn unlink(name: &Name) {
    // SAFETY: the name is NUL-terminated and outlives the call; the result is deliberately unused.
    unsafe { libc::shm_unlink(name.as_c_str().as_ptr()) };
}

/// Unlinks the objects the app can have left behind when it dies: the newest `SWEPT_SERIALS`
/// serials up to `newest`, and the probe's object. It has no list of what was made, so a name in
/// the range that was never made (or was skipped for already existing) is a harmless ENOENT, or,
/// if some dead process with this pid left one, removes that leftover. Safe in a signal handler.
pub fn unlink_recent(pid: u32, newest: Option<u64>) {
    if let Some(newest) = newest {
        for serial in newest.saturating_sub(SWEPT_SERIALS - 1)..=newest {
            unlink(&Name::frame(pid, serial));
        }
    }
    unlink(&Name::probe(pid));
}

/// What a frame needs of the segments: the object that holds it and the serial it was made under
/// (a name that already existed was skipped, so it can be later than the one asked for).
pub struct Segment {
    pub name: Name,
    pub serial: u64,
}

/// The shared-memory objects of one run: where the next serial is, and the names written lately,
/// which are removed when they fall out of the ring and when it is dropped.
pub struct Segments {
    pid: u32,
    serial: u64,
    ring: VecDeque<Name>,
}

impl Segments {
    pub fn new(pid: u32) -> Self {
        Self {
            pid,
            serial: 0,
            ring: VecDeque::new(),
        }
    }

    /// The serial the next object is tried under first.
    pub fn next_serial(&self) -> u64 {
        self.serial
    }

    /// Writes `data` as a new object and keeps its name. A name that exists is skipped, up to
    /// `EXISTING_NAMES` times; the object found there is somebody else's and is left alone.
    pub fn send(&mut self, data: &[u8]) -> io::Result<Segment> {
        let mut taken = None;
        for skipped in 0..=EXISTING_NAMES {
            let serial = self.serial + skipped;
            let name = Name::frame(self.pid, serial);
            match create(&name, data) {
                Ok(()) => {
                    taken = Some(Segment { name, serial });
                    break;
                }
                Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {}
                Err(error) => return Err(error),
            }
        }
        let Some(segment) = taken else {
            return Err(io::Error::from_raw_os_error(libc::EEXIST));
        };
        self.serial = segment.serial + 1;
        self.ring.push_back(segment.name);
        while self.ring.len() > NAMES_KEPT {
            if let Some(old) = self.ring.pop_front() {
                unlink(&old);
            }
        }
        Ok(segment)
    }

    /// Removes every object still in the ring.
    pub fn release(&mut self) {
        for name in self.ring.drain(..) {
            unlink(&name);
        }
    }
}

impl Drop for Segments {
    fn drop(&mut self) {
        self.release();
    }
}

/// The least wait before shared memory is tried again after a failure.
pub const FIRST_DELAY: Duration = Duration::from_secs(1);

/// The longest wait between two attempts.
pub const LONGEST_DELAY: Duration = Duration::from_secs(60);

/// Whether an error from creating a shared-memory object can only come again: the system has no
/// shared memory (ENOSYS), or no such object (ENOTSUP, EOPNOTSUPP), or the name can never be
/// accepted (ENAMETOOLONG). Anything else is transient: a full tmpfs (ENOSPC), permissions that
/// change (EACCES), too many descriptors (EMFILE, ENFILE), memory (ENOMEM), a name that was taken
/// (EEXIST) or a signal (EINTR).
pub fn is_permanent(error: &io::Error) -> bool {
    // Compared one by one: ENOTSUP and EOPNOTSUPP are the same number on some systems.
    error.raw_os_error().is_some_and(|code| {
        code == libc::ENOSYS
            || code == libc::ENOTSUP
            || code == libc::EOPNOTSUPP
            || code == libc::ENAMETOOLONG
    })
}

/// Whether frames may go through shared memory now: it was offered, has not failed for good, and
/// the wait after the last transient failure is over.
pub struct Medium {
    offered: bool,
    failures: u32,
    retry_at: Option<Instant>,
}

impl Medium {
    /// `offered` is false when the terminal did not answer the probe's shared-memory query or
    /// `--direct` was given; the medium is then never used.
    pub fn new(offered: bool) -> Self {
        Self {
            offered,
            failures: 0,
            retry_at: None,
        }
    }

    /// Whether shared memory is still a medium at all (not off for good).
    pub fn offered(&self) -> bool {
        self.offered
    }

    /// Whether a frame may be tried through shared memory at `now`.
    pub fn ready(&self, now: Instant) -> bool {
        self.offered && self.retry_at.is_none_or(|at| now >= at)
    }

    /// A frame went through: the next failure waits `FIRST_DELAY` again.
    pub fn succeeded(&mut self) {
        self.failures = 0;
        self.retry_at = None;
    }

    /// A frame failed at `now`. A permanent error turns the medium off for good; any other waits
    /// one second, then two, four and so on, never more than `LONGEST_DELAY`.
    pub fn failed(&mut self, error: &io::Error, now: Instant) {
        if is_permanent(error) {
            self.offered = false;
            return;
        }
        self.failures = self.failures.saturating_add(1);
        self.retry_at = Some(now + delay(self.failures));
    }
}

/// The wait after the `failures`th failure in a row.
pub fn delay(failures: u32) -> Duration {
    let doublings = 1_u32
        .checked_shl(failures.saturating_sub(1))
        .unwrap_or(u32::MAX);
    FIRST_DELAY.saturating_mul(doublings).min(LONGEST_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn frame_names_fit_the_macos_shared_memory_limit() {
        for (pid, serial) in [(0, 0), (99_999, 1 << 20), (u32::MAX, u64::MAX)] {
            let name = Name::frame(pid, serial);
            assert!(name.as_str().len() <= NAME_LIMIT, "{name} is too long");
            assert!(name.as_str().starts_with("/isotop-"));
        }
        assert_eq!(Name::frame(7, 3).as_str(), "/isotop-7-3");
        assert_eq!(
            Name::frame(u32::MAX, 999_999_999_999).as_str().len(),
            NAME_LIMIT,
            "the longest name should sit exactly at the limit"
        );
        assert_eq!(Name::probe(u32::MAX).as_str(), "/isotop-4294967295-probe");
        assert!(Name::probe(u32::MAX).as_str().len() <= NAME_LIMIT);
        assert_eq!(Name::frame(0, 0).as_str(), "/isotop-0-0");
    }

    /// Reads a shared-memory object back through shm_open and mmap, as a terminal does.
    fn read_shared(name: &Name, length: usize) -> Option<Vec<u8>> {
        // SAFETY: the name is NUL-terminated; the descriptor is closed below.
        let fd = unsafe { libc::shm_open(name.as_c_str().as_ptr(), libc::O_RDONLY, 0) };
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

    /// Each test names its objects after a pid of its own that no process has (above the
    /// kernel's pid limit), so tests in parallel never meet and never touch a real object.
    fn test_pid(test: u32) -> u32 {
        4_200_000_000 + test
    }

    #[test]
    fn shared_memory_frames_round_trip_and_are_cleaned_up() {
        let name = Name::frame(test_pid(1), 700_000_001);
        let pixels: Vec<u8> = (0..300_u32).map(|i| (i * 7) as u8).collect();
        create(&name, &pixels).unwrap();
        assert_eq!(read_shared(&name, pixels.len()).unwrap(), pixels);
        #[cfg(target_os = "linux")]
        {
            let file = std::path::Path::new("/dev/shm").join(&name.as_str()[1..]);
            assert_eq!(std::fs::read(&file).unwrap(), pixels);
            unlink(&name);
            assert!(!file.exists());
        }
        #[cfg(not(target_os = "linux"))]
        unlink(&name);
        assert!(read_shared(&name, 1).is_none());
    }

    #[test]
    fn unlinking_a_frame_the_terminal_already_removed_is_harmless() {
        let name = Name::frame(test_pid(2), 700_000_002);
        create(&name, &[1, 2, 3]).unwrap();
        unlink(&name);
        unlink(&name);
        unlink(&Name::frame(test_pid(3), 700_000_003));
    }

    #[test]
    fn a_name_that_exists_is_skipped_and_left_alone() {
        let pid = test_pid(4);
        let first = 700_000_010;
        let taken = [Name::frame(pid, first), Name::frame(pid, first + 1)];
        for name in &taken {
            create(name, b"theirs").unwrap();
        }
        let mut segments = Segments::new(pid);
        segments.serial = first;
        let segment = segments.send(b"mine").unwrap();
        assert_eq!(segment.serial, first + 2, "two names were taken");
        assert_eq!(read_shared(&segment.name, 4).unwrap(), b"mine");
        for name in &taken {
            assert_eq!(read_shared(name, 6).unwrap(), b"theirs", "not touched");
        }
        assert_eq!(segments.serial, first + 3, "the next frame starts after it");
        drop(segments);
        assert!(read_shared(&segment.name, 1).is_none(), "ours is removed");
        for name in &taken {
            assert!(read_shared(name, 1).is_some(), "theirs is still there");
            unlink(name);
        }
    }

    #[test]
    fn a_run_of_taken_names_ends_in_a_transient_error() {
        let pid = test_pid(5);
        let first = 700_000_020;
        let taken: Vec<Name> = (0..=EXISTING_NAMES)
            .map(|skipped| Name::frame(pid, first + skipped))
            .collect();
        for name in &taken {
            create(name, b"theirs").unwrap();
        }
        let mut segments = Segments::new(pid);
        segments.serial = first;
        let error = segments.send(b"mine").err().expect("every name is taken");
        assert_eq!(error.raw_os_error(), Some(libc::EEXIST));
        assert!(!is_permanent(&error), "a taken name can clear");
        assert!(
            read_shared(&Name::frame(pid, first + EXISTING_NAMES + 1), 1).is_none(),
            "no name past the limit was tried"
        );
        for name in &taken {
            assert_eq!(read_shared(name, 6).unwrap(), b"theirs");
            unlink(name);
        }
    }

    #[test]
    fn the_ring_keeps_the_newest_names_and_unlinks_the_rest() {
        let pid = test_pid(6);
        let mut segments = Segments::new(pid);
        segments.serial = 700_000_100;
        let made: Vec<Name> = (0..NAMES_KEPT + 3)
            .map(|_| segments.send(b"frame").unwrap().name)
            .collect();
        for (index, name) in made.iter().enumerate() {
            assert_eq!(
                read_shared(name, 5).is_some(),
                index >= 3,
                "{name}: only the newest {NAMES_KEPT} remain"
            );
        }
        segments.release();
        for name in &made {
            assert!(read_shared(name, 1).is_none());
        }
    }

    #[test]
    fn errors_that_can_only_come_again_are_told_from_those_that_may_clear() {
        for permanent in [libc::ENOSYS, libc::ENOTSUP, libc::ENAMETOOLONG] {
            assert!(is_permanent(&io::Error::from_raw_os_error(permanent)));
        }
        for transient in [
            libc::ENOSPC,
            libc::EACCES,
            libc::EEXIST,
            libc::EMFILE,
            libc::ENFILE,
            libc::ENOMEM,
            libc::EINTR,
        ] {
            assert!(!is_permanent(&io::Error::from_raw_os_error(transient)));
        }
        assert!(!is_permanent(&io::Error::other("no code")));
    }

    #[test]
    fn shared_memory_is_retried_after_a_transient_failure() {
        let start = Instant::now();
        let mut medium = Medium::new(true);
        assert!(medium.ready(start), "the first frame tries it");
        medium.failed(&io::Error::from_raw_os_error(libc::ENOSPC), start);
        assert!(!medium.ready(start), "the frame after goes inline");
        assert!(!medium.ready(start + Duration::from_millis(999)));
        assert!(
            medium.ready(start + Duration::from_secs(1)),
            "and it is tried again"
        );
        // Failing again doubles the wait.
        let later = start + Duration::from_secs(1);
        medium.failed(&io::Error::from_raw_os_error(libc::EACCES), later);
        assert!(!medium.ready(later + Duration::from_millis(1999)));
        assert!(medium.ready(later + Duration::from_secs(2)));
        // A frame that goes through resets the wait to the first.
        medium.succeeded();
        medium.failed(&io::Error::from_raw_os_error(libc::ENOSPC), later);
        assert!(medium.ready(later + Duration::from_secs(1)));
        assert!(medium.offered());
    }

    #[test]
    fn the_wait_doubles_and_stops_at_sixty_seconds() {
        let seconds: Vec<u64> = (1..=10).map(|failures| delay(failures).as_secs()).collect();
        assert_eq!(seconds, [1, 2, 4, 8, 16, 32, 60, 60, 60, 60]);
        assert_eq!(delay(0), FIRST_DELAY);
        assert_eq!(
            delay(u32::MAX),
            LONGEST_DELAY,
            "no overflow however long it fails"
        );
    }

    #[test]
    fn a_permanent_error_or_a_refusal_keeps_shared_memory_off_for_good() {
        let now = Instant::now();
        let mut medium = Medium::new(true);
        medium.failed(&io::Error::from_raw_os_error(libc::ENOSYS), now);
        assert!(!medium.offered());
        assert!(!medium.ready(now + Duration::from_secs(3600)));
        // The terminal did not offer it, or `--direct`: never ready.
        let medium = Medium::new(false);
        assert!(!medium.ready(now + Duration::from_secs(3600)));
    }

    #[test]
    fn unlink_recent_removes_the_newest_serials_and_the_probe() {
        let pid = test_pid(7);
        let newest = 700_000_203;
        let names: Vec<Name> = (700_000_200..=newest)
            .map(|serial| Name::frame(pid, serial))
            .collect();
        for name in &names {
            create(name, b"x").unwrap();
        }
        create(&Name::probe(pid), b"x").unwrap();
        unlink_recent(pid, Some(newest));
        for name in names.iter().chain([&Name::probe(pid)]) {
            assert!(read_shared(name, 1).is_none(), "{name} is gone");
        }
        // Nothing was ever written: only the probe is swept, and nothing fails.
        unlink_recent(pid, None);
    }

    #[test]
    fn a_name_is_the_c_string_the_kernel_is_given() {
        let name = Name::frame(12, 34);
        assert_eq!(
            name.as_c_str(),
            CString::new("/isotop-12-34").unwrap().as_c_str()
        );
    }
}
