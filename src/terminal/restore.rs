//! Putting the terminal back when the program ends any way but by returning: a panic, or a
//! SIGINT, SIGTERM or SIGHUP. `Drop` alone cannot do it: a panic on another thread, or a build
//! with `panic = "abort"`, never runs it, and a signal that kills the process runs nothing at all.
//!
//! One `Restoration` (`shared()`) is armed right after raw mode is on. It holds what a restore
//! needs, all of it made beforehand so that restoring allocates nothing and takes no lock: the
//! terminal's modes from before raw mode, the exact bytes `restore_screen` writes (once for a run
//! that pushed the Kitty keyboard flags and once for one that did not), the pid, and the thread
//! that owns the terminal. Every path that wants to restore (the `Drop` of `Terminal`, the panic
//! hook, a signal handler) calls `claim`, an atomic compare-and-swap from armed to restoring, and
//! only the one that wins restores, so a restore never runs twice: not when `Drop` follows the
//! hook, not on a second panic, not when a signal lands in the middle of one.
//!
//! Restoring is `write(2)` of the precomputed bytes, `tcsetattr(2)` with the saved modes and
//! `shm_unlink(3)` of the objects the run may have left, in that order; all three are
//! async-signal-safe, which is what lets a signal handler do them.
//!
//! No frame may reach the terminal after the restore, or it would draw over the main screen.
//! Every write of a frame goes through `Restoration::frame`, which counts the write before it
//! checks that the terminal is still armed; the restore leaves armed first and then waits up to
//! `WRITER_WAIT` for the writes already counted, so a write that began before the restore has
//! ended before it, and a write that begins after is refused.
//!
//! The panic hook restores, then calls the hook it replaced (the default one prints the message,
//! so on the main screen), but only for a panic on the thread that owns the terminal, or in a
//! build that aborts on panic. A panic on any other thread of a build that unwinds leaves the
//! terminal alone: the sampler, journal and events threads may die and the app carries on (see
//! `Snapshot::sampler`), and the normal `Drop` restores later.

use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering};

use super::shared;

/// Nothing is armed: before `arm`.
const IDLE: u8 = 0;
/// The terminal is in raw mode on the alternate screen and nobody has claimed it.
const ARMED: u8 = 1;
/// A restore has been claimed and is running.
const RESTORING: u8 = 2;
/// A restore ran to its end.
const RESTORED: u8 = 3;

/// How long a restore waits for the frame writes already under way, in milliseconds (one
/// millisecond a turn, so it needs no clock).
pub const WRITER_WAIT: u32 = 200;

/// How many times a write to a full, non-blocking terminal is retried, a millisecond apart.
const WRITE_RETRIES: u32 = 200;

/// What a restore needs, made when the terminal was armed and never changed after.
struct Record {
    /// Where the bytes go: standard output.
    output: libc::c_int,
    /// Whose modes `termios` are: standard input.
    input: libc::c_int,
    /// The modes from before raw mode; None when they could not be read.
    termios: Option<libc::termios>,
    /// What `restore_screen` writes when the keyboard flags were not pushed.
    plain: Vec<u8>,
    /// What it writes when they were.
    with_keyboard: Vec<u8>,
    pid: u32,
}

/// The state shared by every path that can end the run; see the module docs.
pub struct Restoration {
    state: AtomicU8,
    /// Frame writes under way.
    writers: AtomicUsize,
    /// The Kitty keyboard flags were pushed, so the restore pops them.
    keyboard: AtomicBool,
    /// Some shared-memory object may have been written.
    shared_used: AtomicBool,
    /// The newest serial of a shared-memory object that was written.
    serial: AtomicU64,
    /// The thread that armed it (`pthread_self`), the one whose panic restores.
    owner: AtomicUsize,
    /// The `Record`, which is leaked on purpose: a restore may run in a signal handler at any
    /// time, so nothing frees it, and a process arms once.
    record: AtomicPtr<Record>,
}

impl Restoration {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
            writers: AtomicUsize::new(0),
            keyboard: AtomicBool::new(false),
            shared_used: AtomicBool::new(false),
            serial: AtomicU64::new(0),
            owner: AtomicUsize::new(0),
            record: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// Arms the restoration for the calling thread: `output` is where `plain` or
    /// `with_keyboard` are written, and `termios` are the modes of `input` to put back.
    pub fn arm(
        &self,
        output: libc::c_int,
        input: libc::c_int,
        termios: Option<libc::termios>,
        plain: Vec<u8>,
        with_keyboard: Vec<u8>,
    ) {
        let record = Box::leak(Box::new(Record {
            output,
            input,
            termios,
            plain,
            with_keyboard,
            pid: std::process::id(),
        }));
        self.record.store(record, Ordering::Release);
        self.keyboard.store(false, Ordering::Relaxed);
        self.shared_used.store(false, Ordering::Relaxed);
        self.serial.store(0, Ordering::Relaxed);
        self.owner.store(current_thread(), Ordering::Relaxed);
        self.state.store(ARMED, Ordering::SeqCst);
    }

    /// The Kitty keyboard flags are about to be pushed: a restore from now on pops them. Call it
    /// before the push is written, so a restore after a failed write still pops.
    pub fn keyboard_pushed(&self) {
        self.keyboard.store(true, Ordering::SeqCst);
    }

    /// A shared-memory object with `serial` was written (or is about to be).
    pub fn shared_written(&self, serial: u64) {
        self.serial.fetch_max(serial, Ordering::SeqCst);
        self.shared_used.store(true, Ordering::SeqCst);
    }

    /// Whether the calling thread is the one that armed the restoration.
    pub fn owned_by_current_thread(&self) -> bool {
        self.owner.load(Ordering::Relaxed) == current_thread()
    }

    /// Runs `write` as one frame write, and returns None, without running it, once the terminal
    /// is no longer armed (a restore was claimed). The count is taken before the check, and the
    /// restore clears the flag before it looks at the count, so the two cannot both miss.
    pub fn frame<T>(&self, write: impl FnOnce() -> T) -> Option<T> {
        self.writers.fetch_add(1, Ordering::SeqCst);
        let armed = self.state.load(Ordering::SeqCst) == ARMED;
        let result = armed.then(write);
        self.writers.fetch_sub(1, Ordering::SeqCst);
        result
    }

    /// Takes the right to restore: true for exactly one caller, and only while armed.
    pub fn claim(&self) -> bool {
        self.state
            .compare_exchange(ARMED, RESTORING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Restores the terminal unless another path has, and says whether this call did.
    pub fn restore(&self) -> bool {
        if !self.claim() {
            return false;
        }
        self.restore_claimed();
        true
    }

    /// What a signal handler does: restores if it can claim, reports that a restore is under way
    /// elsewhere (the handler then returns and lets it finish), or that there is nothing to do.
    pub fn restore_for_signal(&self) -> Handling {
        if self.claim() {
            self.restore_claimed();
            Handling::Restored
        } else if self.state.load(Ordering::SeqCst) == RESTORING {
            Handling::Busy
        } else {
            Handling::NothingArmed
        }
    }

    /// The restore itself, after `claim` won. Allocates nothing and takes no lock.
    fn restore_claimed(&self) {
        let record = self.record.load(Ordering::Acquire);
        // SAFETY: `arm` stores a pointer from `Box::leak` before it sets the state to ARMED, and
        // `claim` only succeeds from ARMED (a sequentially consistent exchange after that store),
        // so the pointer is non-null, valid for the rest of the program (nothing frees it) and
        // never written again.
        let record = unsafe { &*record };
        // Frames already being written finish first; the flag is down, so no new one starts.
        let mut waited = 0;
        while self.writers.load(Ordering::SeqCst) > 0 && waited < WRITER_WAIT {
            pause();
            waited += 1;
        }
        let bytes = if self.keyboard.load(Ordering::SeqCst) {
            &record.with_keyboard
        } else {
            &record.plain
        };
        write_all(record.output, bytes);
        if let Some(termios) = &record.termios {
            // SAFETY: termios is a fully initialised value read by tcgetattr; the descriptor is
            // only used by this call, whose result is deliberately unused.
            unsafe { libc::tcsetattr(record.input, libc::TCSANOW, termios) };
        }
        let newest = self
            .shared_used
            .load(Ordering::SeqCst)
            .then(|| self.serial.load(Ordering::SeqCst));
        shared::unlink_recent(record.pid, newest);
        self.state.store(RESTORED, Ordering::SeqCst);
    }
}

impl Default for Restoration {
    fn default() -> Self {
        Self::new()
    }
}

/// What `Restoration::restore_for_signal` found.
#[derive(Debug, PartialEq, Eq)]
pub enum Handling {
    /// This call restored the terminal.
    Restored,
    /// Another path is restoring right now.
    Busy,
    /// There is nothing to restore: never armed, or already restored.
    NothingArmed,
}

/// The calling thread, as a number that is the same on every call and differs between threads.
fn current_thread() -> usize {
    // SAFETY: pthread_self has no preconditions and cannot fail.
    unsafe { libc::pthread_self() as usize }
}

/// Sleeps a millisecond; `nanosleep` is async-signal-safe.
fn pause() {
    let time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 1_000_000,
    };
    // SAFETY: time is a valid timespec and the remaining-time pointer may be null.
    unsafe { libc::nanosleep(&time, std::ptr::null_mut()) };
}

/// Writes all of `bytes` to `descriptor` with `write(2)` alone, giving up on a hard error.
fn write_all(descriptor: libc::c_int, bytes: &[u8]) {
    let mut done = 0;
    let mut retries = 0;
    while done < bytes.len() {
        let rest = &bytes[done..];
        // SAFETY: rest is a live slice, valid for reads of its length.
        let count = unsafe { libc::write(descriptor, rest.as_ptr().cast(), rest.len()) };
        if count > 0 {
            done += count as usize;
            continue;
        }
        let error = std::io::Error::last_os_error().raw_os_error();
        let transient = count < 0 && matches!(error, Some(libc::EINTR | libc::EAGAIN));
        if !transient || retries >= WRITE_RETRIES {
            return;
        }
        retries += 1;
        if error == Some(libc::EAGAIN) {
            pause();
        }
    }
}

/// The modes of `descriptor`, to put back when raw mode ends; None when it is not a terminal.
pub fn modes(descriptor: libc::c_int) -> Option<libc::termios> {
    // SAFETY: an all-zero termios is a valid value to pass to tcgetattr, which fills it.
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: termios is valid for writes for the call.
    (unsafe { libc::tcgetattr(descriptor, &mut termios) } == 0).then_some(termios)
}

static SHARED: Restoration = Restoration::new();

/// The restoration of this process, which the hook and the signal handlers work on.
pub fn shared() -> &'static Restoration {
    &SHARED
}

/// Whether a panic with these circumstances restores the terminal, and does it: a panic on the
/// thread that owns the terminal always does; a panic anywhere else does only when panics abort
/// (`unwinds` false), because then nothing runs `Drop` afterwards. Returns whether this call
/// restored (false when another path already had, or when it does not apply).
pub fn on_panic(restoration: &Restoration, unwinds: bool) -> bool {
    if restoration.owned_by_current_thread() || !unwinds {
        restoration.restore()
    } else {
        false
    }
}

/// Installs the panic hook and the signal handlers, once. The hook runs `on_panic` and then the
/// hook it replaced.
pub fn install(restoration: &'static Restoration) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            on_panic(restoration, cfg!(panic = "unwind"));
            previous(info);
        }));
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            handle(signal);
        }
    });
}

/// Handles `signal` by restoring and then dying of it, unless it was ignored when the program
/// started (a `nohup` run keeps ignoring SIGHUP).
fn handle(signal: libc::c_int) {
    // SAFETY: an all-zero sigaction is a valid value to pass to sigaction as the out parameter.
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: a null new action only queries; current is valid for writes.
    let queried = unsafe { libc::sigaction(signal, std::ptr::null(), &mut current) };
    if queried != 0 || current.sa_sigaction == libc::SIG_IGN {
        return;
    }
    // SAFETY: an all-zero sigaction is valid; the fields set below are the ones sigaction reads.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: action.sa_mask is a valid sigset_t to initialise, and the handler it names is an
    // `extern "C"` function that lives for the whole program.
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

/// The handler: restore, then give the signal back its default action and raise it, so the
/// process dies of it as it would have. It runs in a signal context, so it only calls
/// async-signal-safe functions (see `Restoration::restore_claimed`); a signal that finds a
/// restore under way elsewhere returns, and that restore finishes.
extern "C" fn on_signal(signal: libc::c_int) {
    if shared().restore_for_signal() == Handling::Busy {
        return;
    }
    // SAFETY: signal and raise are async-signal-safe; with the default action in place the signal
    // (blocked while this handler runs) ends the process as soon as the handler returns.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::shared::{Name, create, unlink};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// A pipe that stands in for the terminal: what is written lands in `read`.
    struct Pipe {
        read: OwnedFd,
        write: OwnedFd,
    }

    impl Pipe {
        fn new() -> Self {
            let mut ends = [0; 2];
            // SAFETY: ends has room for the two descriptors pipe writes.
            assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
            // SAFETY: both descriptors were just returned by pipe and are owned here.
            let (read, write) =
                unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
            // Non-blocking, so the test reads what is there and never waits for more.
            // SAFETY: the descriptor is valid; F_SETFL with O_NONBLOCK only changes its flags.
            unsafe { libc::fcntl(read.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
            Self { read, write }
        }

        fn written(&self) -> Vec<u8> {
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                // SAFETY: the buffer is valid for writes of its length.
                let count = unsafe {
                    libc::read(
                        self.read.as_raw_fd(),
                        buffer.as_mut_ptr().cast(),
                        buffer.len(),
                    )
                };
                if count <= 0 {
                    return bytes;
                }
                bytes.extend_from_slice(&buffer[..count as usize]);
            }
        }
    }

    /// A restoration armed on the calling thread, writing to `pipe`, with no modes to put back.
    fn armed(pipe: &Pipe) -> Restoration {
        let restoration = Restoration::new();
        restoration.arm(
            pipe.write.as_raw_fd(),
            -1,
            None,
            b"PLAIN".to_vec(),
            b"KEYBOARD+PLAIN".to_vec(),
        );
        restoration
    }

    #[test]
    fn restoring_writes_the_precomputed_bytes_once() {
        let pipe = Pipe::new();
        let restoration = armed(&pipe);
        assert!(restoration.restore());
        assert_eq!(pipe.written(), b"PLAIN");
        assert!(!restoration.restore(), "the second path finds it done");
        assert!(pipe.written().is_empty());
    }

    #[test]
    fn the_keyboard_flags_are_popped_only_when_they_were_pushed() {
        let pipe = Pipe::new();
        let pushed = armed(&pipe);
        pushed.keyboard_pushed();
        assert!(pushed.restore());
        assert_eq!(pipe.written(), b"KEYBOARD+PLAIN");
    }

    #[test]
    fn restore_once_however_many_paths_reach_it() {
        let pipe = Pipe::new();
        let restoration = Arc::new(armed(&pipe));
        let winners: usize = (0..8)
            .map(|_| {
                let restoration = Arc::clone(&restoration);
                thread::spawn(move || usize::from(restoration.restore()))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .sum();
        assert_eq!(winners, 1);
        assert_eq!(pipe.written(), b"PLAIN", "written once, not eight times");
        assert_eq!(restoration.restore_for_signal(), Handling::NothingArmed);
    }

    #[test]
    fn a_signal_during_a_restore_returns_and_after_one_dies() {
        let restoration = Restoration::new();
        assert_eq!(
            restoration.restore_for_signal(),
            Handling::NothingArmed,
            "never armed"
        );
        let pipe = Pipe::new();
        let restoration = armed(&pipe);
        // Another path has claimed it and is partway through.
        assert!(restoration.claim());
        assert_eq!(restoration.restore_for_signal(), Handling::Busy);
        assert!(pipe.written().is_empty(), "the signal wrote nothing");
        restoration.restore_claimed();
        assert_eq!(restoration.restore_for_signal(), Handling::NothingArmed);
    }

    #[test]
    fn a_signal_that_finds_the_terminal_armed_restores_it() {
        let pipe = Pipe::new();
        let restoration = armed(&pipe);
        assert_eq!(restoration.restore_for_signal(), Handling::Restored);
        assert_eq!(pipe.written(), b"PLAIN");
    }

    #[test]
    fn no_frame_is_written_after_restore() {
        let pipe = Pipe::new();
        let restoration = armed(&pipe);
        let before = restoration.frame(|| "written");
        assert_eq!(before, Some("written"));
        restoration.restore();
        let mut ran = false;
        let after = restoration.frame(|| ran = true);
        assert_eq!(after, None);
        assert!(!ran, "the write was refused, not run");
    }

    #[test]
    fn restore_waits_for_a_writer() {
        let pipe = Pipe::new();
        let restoration = Arc::new(armed(&pipe));
        let (started, started_receiver) = mpsc::channel();
        let (release, release_receiver) = mpsc::channel::<()>();
        let write = write_pipe_handle(&pipe);
        let writer = {
            let restoration = Arc::clone(&restoration);
            thread::spawn(move || {
                restoration.frame(|| {
                    started.send(()).unwrap();
                    release_receiver.recv().unwrap();
                    write_all(write, b"FRAME");
                })
            })
        };
        started_receiver.recv().unwrap();
        let restorer = {
            let restoration = Arc::clone(&restoration);
            thread::spawn(move || restoration.restore())
        };
        // The restore has claimed and is waiting; its bytes are not written yet.
        thread::sleep(Duration::from_millis(50));
        assert!(
            pipe.written().is_empty(),
            "restore wrote before the frame ended"
        );
        release.send(()).unwrap();
        assert!(writer.join().unwrap().is_some());
        assert!(restorer.join().unwrap());
        assert_eq!(pipe.written(), b"FRAMEPLAIN", "the frame, then the restore");
    }

    /// The descriptor `pipe` writes to, for a closure that cannot borrow the pipe.
    fn write_pipe_handle(pipe: &Pipe) -> libc::c_int {
        pipe.write.as_raw_fd()
    }

    #[test]
    fn a_restore_does_not_wait_for_ever_on_a_stuck_writer() {
        let pipe = Pipe::new();
        let restoration = Arc::new(armed(&pipe));
        let (started, started_receiver) = mpsc::channel();
        let (release, release_receiver) = mpsc::channel::<()>();
        let writer = {
            let restoration = Arc::clone(&restoration);
            thread::spawn(move || {
                restoration.frame(|| {
                    started.send(()).unwrap();
                    release_receiver.recv().unwrap();
                })
            })
        };
        started_receiver.recv().unwrap();
        let begun = std::time::Instant::now();
        assert!(restoration.restore());
        assert!(begun.elapsed() < Duration::from_secs(5));
        assert_eq!(pipe.written(), b"PLAIN", "restored after the wait ran out");
        release.send(()).unwrap();
        writer.join().unwrap();
    }

    #[test]
    fn a_panic_on_the_owning_thread_restores_whether_or_not_panics_unwind() {
        for unwinds in [true, false] {
            let pipe = Pipe::new();
            let restoration = armed(&pipe);
            assert!(on_panic(&restoration, unwinds), "unwinds = {unwinds}");
            assert_eq!(pipe.written(), b"PLAIN");
            assert!(
                !on_panic(&restoration, unwinds),
                "a second panic finds it done"
            );
            assert!(pipe.written().is_empty());
        }
    }

    #[test]
    fn a_panic_on_another_thread_leaves_the_terminal_to_the_app_when_panics_unwind() {
        let pipe = Pipe::new();
        let restoration = Arc::new(armed(&pipe));
        let other = {
            let restoration = Arc::clone(&restoration);
            thread::spawn(move || on_panic(&restoration, true))
        };
        assert!(!other.join().unwrap(), "the hook only chains");
        assert!(pipe.written().is_empty(), "nothing was written");
        // The terminal is still armed, so the normal restore at quit does its work.
        assert!(restoration.restore());
        assert_eq!(pipe.written(), b"PLAIN");
    }

    #[test]
    fn any_panic_restores_once_when_panics_abort() {
        let pipe = Pipe::new();
        let restoration = Arc::new(armed(&pipe));
        let other = {
            let restoration = Arc::clone(&restoration);
            thread::spawn(move || on_panic(&restoration, false))
        };
        assert!(other.join().unwrap(), "a panic on any thread restores");
        assert_eq!(pipe.written(), b"PLAIN");
        assert!(
            !on_panic(&restoration, false),
            "the owner's panic finds it done"
        );
        assert!(!restoration.restore(), "and so does Drop");
        assert!(pipe.written().is_empty());
    }

    #[test]
    fn shared_memory_unlinked_on_panic() {
        // A pid no process has, so no other test or run meets these names.
        let pid = 4_200_000_100;
        let pipe = Pipe::new();
        let restoration = Restoration::new();
        restoration.arm(pipe.write.as_raw_fd(), -1, None, Vec::new(), Vec::new());
        // The restoration names objects after the real pid; arm it for the fake one instead.
        let record = restoration.record.load(Ordering::Acquire);
        // SAFETY: the record was just made by arm and nothing else holds it; the test is the
        // only thread, so writing the pid does not race.
        unsafe { (*record).pid = pid };
        let names = [
            Name::frame(pid, 5),
            Name::frame(pid, 6),
            Name::frame(pid, 7),
            Name::probe(pid),
        ];
        for name in &names {
            create(name, b"frame").unwrap();
        }
        restoration.shared_written(7);
        assert!(on_panic(&restoration, true));
        for name in &names {
            assert!(create(name, b"x").is_ok(), "{name} was unlinked");
            unlink(name);
        }
    }

    #[test]
    fn restoring_puts_back_the_modes_of_the_terminal() {
        let mut master = 0;
        let mut slave = 0;
        // SAFETY: both out pointers are valid; the name, termios and window size may be null.
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0, "a pseudo-terminal opens");
        let before = modes(slave).expect("a pty has modes");
        // SAFETY: a copy of a valid termios, changed by cfmakeraw and applied to our own pty.
        let raw = unsafe {
            let mut raw = before;
            libc::cfmakeraw(&mut raw);
            assert_eq!(libc::tcsetattr(slave, libc::TCSANOW, &raw), 0);
            raw
        };
        let pipe = Pipe::new();
        let restoration = Restoration::new();
        restoration.arm(
            pipe.write.as_raw_fd(),
            slave,
            Some(before),
            Vec::new(),
            Vec::new(),
        );
        assert_ne!(raw.c_lflag, before.c_lflag, "raw mode changed something");
        assert!(restoration.restore());
        let after = modes(slave).unwrap();
        assert_eq!(after.c_lflag, before.c_lflag);
        assert_eq!(after.c_iflag, before.c_iflag);
        assert_eq!(after.c_oflag, before.c_oflag);
        // SAFETY: both descriptors were opened above and are not used again.
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
    }

    #[test]
    fn a_thread_is_told_from_the_one_that_armed() {
        let restoration = Restoration::new();
        let seen = AtomicBool::new(true);
        let pipe = Pipe::new();
        restoration.arm(pipe.write.as_raw_fd(), -1, None, Vec::new(), Vec::new());
        assert!(restoration.owned_by_current_thread());
        thread::scope(|scope| {
            scope.spawn(|| seen.store(restoration.owned_by_current_thread(), Ordering::SeqCst));
        });
        assert!(!seen.load(Ordering::SeqCst));
    }
}
