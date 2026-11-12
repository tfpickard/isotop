//! Frames sent inline (Kitty graphics `t=d`): zlib, then base64, then the write, on a worker
//! thread so the frame loop never waits for them.
//!
//! `Terminal::present` builds a `Job` (the bytes before the image, its placement, a copy of the
//! pixels, the bytes after) and hands it to `Inline::submit`, which puts it in a mailbox of one
//! slot. A job still waiting there when a newer one arrives is replaced (the terminal only ever
//! needs the newest frame), so the loop never queues work behind a slow terminal. The
//! `isotop-encoder` thread, started on the first job, takes the job out, encodes it and writes it
//! through its sink; frames are written in the order they were submitted, and a job that is
//! finished goes back to be reused (at most `SPARE_JOBS` are kept), together with the worker's own
//! buffers, which live as long as it does.
//!
//! Three things need the worker to be out of the way. `Inline::settle` drops the job that is
//! waiting and waits for the one being written, and runs before a frame goes through shared
//! memory or the screen is cleared, so one medium's bytes never interleave with the other's.
//! `Inline::shutdown` settles (up to `SETTLE_LIMIT`), stops the worker and joins it (up to
//! `JOIN_LIMIT`), so a terminal that stopped reading cannot hold the exit up. A write that fails
//! is reported by the next `submit`. A worker that panics clears its busy flag, stores an error
//! and wakes the waiters (`Guard`), so every later `submit` fails with it and nothing waits for ever.
//!
//! If the thread cannot be started, the frame is encoded and written on the caller's thread.
//! Either way the bytes of a frame are those the loop used to write itself (`legacy`, in the
//! tests, is a copy of that code, and the two are compared).

use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine as _;
use flate2::{Compression, write::ZlibEncoder};

/// The base64 text of one graphics command: the protocol allows at most 4096 bytes in a chunk.
const CHUNK: usize = 4096;

/// A frame is written in pieces of this many bytes, each its own write to the sink, so a restore
/// waits for one piece and not for a whole frame (see `Restoration::frame`).
const PIECE: usize = 64 * 1024;

/// How many finished jobs are kept to be reused.
pub const SPARE_JOBS: usize = 3;

/// How long `shutdown` waits for the frame being written.
pub const SETTLE_LIMIT: Duration = Duration::from_secs(2);

/// How long `shutdown` waits for the worker thread to end.
pub const JOIN_LIMIT: Duration = Duration::from_millis(500);

/// The message of the error every `submit` returns after the worker panicked.
pub const PANICKED: &str = "the encoder thread panicked";

/// One frame to send: everything the writer needs, so the frame loop keeps none of it.
#[derive(Default)]
pub struct Job {
    /// Everything before the image: the synchronized update, the cleared rows, the cursor home.
    pub head: Vec<u8>,
    /// The control keys of the first graphics command (`a=T,f=24,s=..`), without `t=` and `m=`.
    pub placement: String,
    /// The frame's pixels, RGB, a copy.
    pub pixels: Vec<u8>,
    /// Everything after the image: the old image's deletion, labels, popups and status rows, and
    /// the end of the synchronized update.
    pub tail: Vec<u8>,
}

impl Job {
    fn clear(&mut self) {
        self.head.clear();
        self.placement.clear();
        self.pixels.clear();
        self.tail.clear();
    }
}

/// Turns jobs into bytes and writes them: the part of the encoder that holds the buffers it
/// reuses. It runs on the worker thread, or on the caller's when no thread could be started.
struct Worker<S> {
    sink: S,
    compressed: Vec<u8>,
    encoded: String,
    wire: Vec<u8>,
}

impl<S: Write> Worker<S> {
    fn new(sink: S) -> Self {
        Self {
            sink,
            compressed: Vec::new(),
            encoded: String::new(),
            wire: Vec::new(),
        }
    }

    /// Fills `wire` with the bytes of `job`.
    fn encode(&mut self, job: &Job) -> io::Result<()> {
        let mut compressor =
            ZlibEncoder::new(std::mem::take(&mut self.compressed), Compression::fast());
        compressor.get_mut().clear();
        compressor.write_all(&job.pixels)?;
        self.compressed = compressor.finish()?;
        self.encoded.clear();
        base64::engine::general_purpose::STANDARD
            .encode_string(&self.compressed, &mut self.encoded);
        self.wire.clear();
        self.wire.reserve(
            job.head.len()
                + job.tail.len()
                + self.encoded.len()
                + self.encoded.len() / CHUNK * 16
                + 1024,
        );
        self.wire.extend_from_slice(&job.head);
        let chunks = self.encoded.as_bytes().chunks(CHUNK);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            let more = usize::from(index + 1 < count);
            if index == 0 {
                write!(self.wire, "\x1b_G{},t=d,o=z,m={more};", job.placement)?;
            } else {
                write!(self.wire, "\x1b_Gm={more},q=2;")?;
            }
            self.wire.extend_from_slice(chunk);
            self.wire.extend_from_slice(b"\x1b\\");
        }
        self.wire.extend_from_slice(&job.tail);
        Ok(())
    }

    /// Encodes and writes `job`; returns the bytes written.
    fn write(&mut self, job: &Job) -> io::Result<usize> {
        self.encode(job)?;
        for piece in self.wire.chunks(PIECE) {
            self.sink.write_all(piece)?;
        }
        self.sink.flush()?;
        Ok(self.wire.len())
    }
}

/// What the caller and the worker share, behind one lock.
#[derive(Default)]
struct State {
    /// The newest job not yet taken.
    waiting: Option<Job>,
    /// The worker has a job out and is writing it.
    writing: bool,
    /// Told to stop.
    closed: bool,
    /// The worker panicked.
    panicked: bool,
    /// The failure of the last write, until a `submit` reports it.
    error: Option<io::Error>,
    /// Finished jobs, cleared, to be filled again.
    spare: Vec<Job>,
    /// Bytes of the last frame written.
    last_wire: usize,
    /// Bytes and frames written in all.
    written: u64,
    frames: u64,
    /// Waiting jobs a newer one replaced.
    replaced: u64,
}

impl State {
    fn recycle(&mut self, mut job: Job) {
        job.clear();
        if self.spare.len() < SPARE_JOBS {
            self.spare.push(job);
        }
    }
}

struct Mailbox {
    state: Mutex<State>,
    changed: Condvar,
}

impl Mailbox {
    /// The state, whether or not a thread panicked holding the lock.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Ends a worker that panicked in a state the rest of the program can see: not busy, with an
/// error stored and everyone woken.
struct Guard(Arc<Mailbox>);

impl Drop for Guard {
    fn drop(&mut self) {
        if thread::panicking() {
            let mut state = self.0.lock();
            state.writing = false;
            state.panicked = true;
            state.waiting = None;
            state.error = Some(io::Error::other(PANICKED));
            self.0.changed.notify_all();
        }
    }
}

/// The worker's loop: take the newest job, encode and write it, until told to stop.
fn run<S: Write>(mailbox: &Arc<Mailbox>, mut worker: Worker<S>) {
    let _guard = Guard(Arc::clone(mailbox));
    loop {
        let job = {
            let mut state = mailbox.lock();
            loop {
                if state.closed {
                    return;
                }
                if let Some(job) = state.waiting.take() {
                    state.writing = true;
                    break job;
                }
                state = mailbox.wait(state);
            }
        };
        let outcome = worker.write(&job);
        let mut state = mailbox.lock();
        state.writing = false;
        match outcome {
            Ok(bytes) => {
                state.last_wire = bytes;
                state.written += bytes as u64;
                state.frames += 1;
            }
            Err(error) => state.error = Some(error),
        }
        state.recycle(job);
        mailbox.changed.notify_all();
    }
}

/// Where the encoding happens.
enum Encoding<S> {
    /// Nothing sent yet: the thread starts with the first job.
    Idle(S),
    Thread(JoinHandle<()>),
    /// The thread could not be started: encode on the caller.
    Local(Box<Worker<S>>),
    Stopped,
}

/// The inline encoder: see the module docs.
pub struct Inline<S: Write + Send + Clone + 'static> {
    mailbox: Arc<Mailbox>,
    engine: Encoding<S>,
    settle_limit: Duration,
    join_limit: Duration,
}

impl<S: Write + Send + Clone + 'static> Inline<S> {
    /// An encoder that writes through `sink`; the thread starts with the first frame.
    pub fn new(sink: S) -> Self {
        Self::with_engine(Encoding::Idle(sink))
    }

    /// One that encodes on the caller's thread, as `new` does when the thread cannot start.
    #[cfg(test)]
    pub fn without_thread(sink: S) -> Self {
        Self::with_engine(Encoding::Local(Box::new(Worker::new(sink))))
    }

    fn with_engine(engine: Encoding<S>) -> Self {
        Self {
            mailbox: Arc::new(Mailbox {
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
            }),
            engine,
            settle_limit: SETTLE_LIMIT,
            join_limit: JOIN_LIMIT,
        }
    }

    /// Shorter limits, for tests that stall the writer on purpose.
    #[cfg(test)]
    fn with_limits(mut self, settle: Duration, join: Duration) -> Self {
        self.settle_limit = settle;
        self.join_limit = join;
        self
    }

    /// An empty job, with the buffers of an earlier one when there is one to reuse.
    pub fn blank(&self) -> Job {
        self.mailbox.lock().spare.pop().unwrap_or_default()
    }

    /// Hands back a job that was not submitted (its frame went another way).
    pub fn recycle(&self, job: Job) {
        self.mailbox.lock().recycle(job);
    }

    /// Queues `job` as the next frame, replacing one still waiting. The failure of an earlier
    /// write is returned instead, once; after a panic of the worker every call fails.
    pub fn submit(&mut self, job: Job) -> io::Result<()> {
        if let Encoding::Idle(_) = self.engine {
            self.start();
        }
        {
            let mut state = self.mailbox.lock();
            if state.panicked {
                return Err(io::Error::other(PANICKED));
            }
            if let Some(error) = state.error.take() {
                state.recycle(job);
                return Err(error);
            }
            if state.closed {
                return Err(io::Error::other("the encoder was shut down"));
            }
        }
        if let Encoding::Local(worker) = &mut self.engine {
            let outcome = worker.write(&job);
            let mut state = self.mailbox.lock();
            if let Ok(bytes) = &outcome {
                state.last_wire = *bytes;
                state.written += *bytes as u64;
                state.frames += 1;
            }
            state.recycle(job);
            return outcome.map(|_| ());
        }
        let mut state = self.mailbox.lock();
        if let Some(older) = state.waiting.replace(job) {
            state.replaced += 1;
            state.recycle(older);
        }
        self.mailbox.changed.notify_all();
        Ok(())
    }

    /// Starts the worker thread, or falls back to encoding on the caller's.
    fn start(&mut self) {
        let Encoding::Idle(sink) = std::mem::replace(&mut self.engine, Encoding::Stopped) else {
            return;
        };
        let for_thread = sink.clone();
        let mailbox = Arc::clone(&self.mailbox);
        let spawned = thread::Builder::new()
            .name("isotop-encoder".to_owned())
            .spawn(move || run(&mailbox, Worker::new(for_thread)));
        self.engine = match spawned {
            Ok(handle) => Encoding::Thread(handle),
            Err(_) => Encoding::Local(Box::new(Worker::new(sink))),
        };
    }

    /// Drops the frame that is waiting and waits, up to the limit, for the one being written.
    /// Returns whether nothing is being written any more.
    pub fn settle(&self) -> bool {
        let deadline = Instant::now() + self.settle_limit;
        let mut state = self.mailbox.lock();
        if let Some(job) = state.waiting.take() {
            state.recycle(job);
        }
        while state.writing {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            state = self
                .mailbox
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Settles, stops the worker and joins it, each for a limited time; a worker still stuck in
    /// a write after that is left behind and ends with the process.
    pub fn shutdown(&mut self) {
        self.settle();
        {
            let mut state = self.mailbox.lock();
            state.closed = true;
            state.waiting = None;
            self.mailbox.changed.notify_all();
        }
        if let Encoding::Thread(handle) = std::mem::replace(&mut self.engine, Encoding::Stopped) {
            let deadline = Instant::now() + self.join_limit;
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        } else {
            self.engine = Encoding::Stopped;
        }
    }

    /// Bytes of the last frame written, 0 before the first.
    pub fn last_wire(&self) -> usize {
        self.mailbox.lock().last_wire
    }

    /// Bytes and frames written so far.
    pub fn written(&self) -> (u64, u64) {
        let state = self.mailbox.lock();
        (state.written, state.frames)
    }
}

impl<S: Write + Send + Clone + 'static> Drop for Inline<S> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// A sink that keeps what is written, shared between clones.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<u8>>>);

    impl Recorder {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Write for Recorder {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// The code the frame loop used to run, copied from before the encoder moved off it: all the
    /// bytes of one inline frame.
    fn legacy(head: &[u8], placement: &str, pixels: &[u8], tail: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(8192);
        output.extend_from_slice(head);
        let mut compressor =
            ZlibEncoder::new(Vec::with_capacity(pixels.len() / 4), Compression::fast());
        compressor.write_all(pixels).unwrap();
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(compressor.finish().unwrap());
        output.reserve(encoded.len() + 1024);
        let chunks = encoded.as_bytes().chunks(4096);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            let more = usize::from(index + 1 < count);
            if index == 0 {
                write!(output, "\x1b_G{placement},t=d,o=z,m={more};").unwrap();
            } else {
                write!(output, "\x1b_Gm={more},q=2;").unwrap();
            }
            output.extend_from_slice(chunk);
            output.extend_from_slice(b"\x1b\\");
        }
        output.extend_from_slice(tail);
        output
    }

    fn job(head: &str, pixels: &[u8], tail: &str) -> Job {
        Job {
            head: head.as_bytes().to_vec(),
            placement: "a=T,f=24,s=2,v=2,i=100,c=10,r=5,z=-1073741825,C=1,q=2".to_owned(),
            pixels: pixels.to_vec(),
            tail: tail.as_bytes().to_vec(),
        }
    }

    /// Pseudo-random bytes that do not compress.
    fn noise(length: usize) -> Vec<u8> {
        let mut stream = crate::simulation::XorShift32::new(12_345);
        (0..length).map(|_| (stream.unit() * 256.0) as u8).collect()
    }

    #[test]
    fn inline_encoding_is_byte_identical_to_the_previous_path() {
        let frames: Vec<(&str, Vec<u8>, &str)> = vec![
            (
                "\x1b[?2026h\x1b[1;1H",
                vec![10, 20, 30, 40, 50, 60],
                "\x1b_Ga=d,d=I,i=101,q=2\x1b\\\x1b[?2026l",
            ),
            ("H2", noise(300_000), "T2"),
            ("", Vec::new(), ""),
            ("H4", vec![7], "T4"),
            ("H5", vec![200; 1 << 20], "T5"),
            ("H6", vec![200; 1 << 20], "T6"),
            ("\x1b[?2026h", noise(12_000), "\x1b[?2026l"),
        ];
        let recorder = Recorder::default();
        // One worker for all seven, so a buffer reused from a big frame must not leak into a
        // small one.
        let mut worker = Worker::new(recorder.clone());
        let mut single = 0;
        let mut multiple = 0;
        for (head, pixels, tail) in &frames {
            let sent = job(head, pixels, tail);
            let expected = legacy(head.as_bytes(), &sent.placement, pixels, tail.as_bytes());
            let before = recorder.bytes().len();
            let written = worker.write(&sent).unwrap();
            let bytes = recorder.bytes();
            assert_eq!(written, expected.len());
            assert_eq!(&bytes[before..], &expected[..], "the bytes of {head:?}");
            if !expected.windows(5).any(|window| window == b"\x1b_Gm=") {
                single += 1;
            } else {
                multiple += 1;
            }
        }
        assert!(
            single >= 3 && multiple >= 2,
            "{single} single and {multiple} multi-chunk frames"
        );
        // And through the thread, in order.
        let recorder = Recorder::default();
        let mut inline = Inline::new(recorder.clone());
        let mut expected = Vec::new();
        for (index, (head, pixels, tail)) in frames.iter().enumerate() {
            let sent = job(head, pixels, tail);
            expected.extend(legacy(
                head.as_bytes(),
                &sent.placement,
                pixels,
                tail.as_bytes(),
            ));
            inline.submit(sent).unwrap();
            until_accounted(&inline, index as u64 + 1);
        }
        assert_eq!(recorder.bytes(), expected);
        inline.shutdown();
    }

    #[test]
    fn encoding_falls_back_to_the_caller_when_the_thread_cannot_start() {
        let recorder = Recorder::default();
        let mut inline = Inline::without_thread(recorder.clone());
        let sent = job("H", &noise(50_000), "T");
        let expected = legacy(b"H", &sent.placement, &sent.pixels, b"T");
        inline.submit(sent).unwrap();
        assert_eq!(recorder.bytes(), expected, "written before submit returned");
        assert_eq!(inline.last_wire(), expected.len());
        assert_eq!(inline.written(), (expected.len() as u64, 1));
    }

    /// A sink that holds every write until it is let through, and tells when one starts.
    #[derive(Clone)]
    struct Gate {
        record: Recorder,
        started: mpsc::Sender<()>,
        pass: Arc<Mutex<mpsc::Receiver<()>>>,
    }

    fn gate() -> (Gate, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started, started_receiver) = mpsc::channel();
        let (release, pass) = mpsc::channel();
        (
            Gate {
                record: Recorder::default(),
                started,
                pass: Arc::new(Mutex::new(pass)),
            },
            started_receiver,
            release,
        )
    }

    impl Write for Gate {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let _ = self.started.send(());
            // A dropped release means the test is over: let the write go.
            let _ = self.pass.lock().unwrap().recv();
            self.record.write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn marked(mark: &str) -> Job {
        job(mark, &[1, 2, 3], "")
    }

    /// The numbers after the `#` of each frame in `bytes`, in order.
    fn marks(bytes: &[u8]) -> Vec<u32> {
        String::from_utf8_lossy(bytes)
            .split('#')
            .skip(1)
            .map(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().unwrap()
            })
            .collect()
    }

    /// Waits until the worker has written or been spared `count` frames.
    fn until_accounted<S: Write + Send + Clone + 'static>(inline: &Inline<S>, count: u64) {
        let begun = Instant::now();
        loop {
            let state = inline.mailbox.lock();
            if state.frames + state.replaced >= count && !state.writing {
                return;
            }
            drop(state);
            assert!(
                begun.elapsed() < Duration::from_secs(10),
                "the worker stalled"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_newer_waiting_frame_replaces_an_older_one() {
        let (sink, started, release) = gate();
        let record = sink.record.clone();
        let mut inline = Inline::new(sink);
        inline.submit(marked("#1")).unwrap();
        started.recv().unwrap(); // frame 1 is being written
        inline.submit(marked("#2")).unwrap();
        inline.submit(marked("#3")).unwrap(); // replaces #2
        inline.submit(marked("#4")).unwrap(); // replaces #3
        for _ in 0..4 {
            let _ = release.send(());
        }
        until_accounted(&inline, 4);
        assert_eq!(marks(&record.bytes()), [1, 4], "2 and 3 were never written");
        assert_eq!(inline.mailbox.lock().replaced, 2);
    }

    #[test]
    fn frames_are_written_in_the_order_they_were_submitted() {
        let recorder = Recorder::default();
        let mut inline = Inline::new(recorder.clone());
        for index in 0..40 {
            inline.submit(marked(&format!("#{index}"))).unwrap();
            if index % 7 == 0 {
                thread::sleep(Duration::from_millis(1));
            }
        }
        until_accounted(&inline, 40);
        let written = marks(&recorder.bytes());
        assert!(
            written.windows(2).all(|pair| pair[0] < pair[1]),
            "{written:?}"
        );
        assert_eq!(
            written.last(),
            Some(&39),
            "the newest frame is never the one dropped"
        );
        assert_eq!(written.len() as u64 + inline.mailbox.lock().replaced, 40);
    }

    #[test]
    fn settle_drops_the_waiting_frame_and_waits_for_the_one_being_written() {
        let (sink, started, release) = gate();
        let record = sink.record.clone();
        let mut inline = Inline::new(sink);
        inline.submit(marked("#1")).unwrap();
        started.recv().unwrap();
        inline.submit(marked("#2")).unwrap();
        let let_go = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            let _ = release.send(());
            release
        });
        let begun = Instant::now();
        assert!(inline.settle(), "settled once the write ended");
        assert!(
            begun.elapsed() >= Duration::from_millis(80),
            "it waited for the write"
        );
        let _release = let_go.join().unwrap();
        assert_eq!(marks(&record.bytes()), [1], "the waiting frame was dropped");
        assert!(inline.mailbox.lock().waiting.is_none());
    }

    #[test]
    fn a_stalled_write_does_not_hold_up_shutdown_past_its_limits() {
        let (sink, started, release) = gate();
        let mut inline =
            Inline::new(sink).with_limits(Duration::from_millis(100), Duration::from_millis(50));
        inline.submit(marked("#1")).unwrap();
        started.recv().unwrap();
        let begun = Instant::now();
        inline.shutdown();
        assert!(
            begun.elapsed() < Duration::from_secs(2),
            "{:?}",
            begun.elapsed()
        );
        // Let the stuck write go so the thread ends with the test.
        let _ = release.send(());
    }

    #[test]
    fn spent_buffers_come_back_and_at_most_three_are_kept() {
        let recorder = Recorder::default();
        let mut inline = Inline::new(recorder.clone());
        for index in 0..8 {
            let mut sent = inline.blank();
            sent.pixels.extend_from_slice(&noise(4096));
            inline.submit(sent).unwrap();
            until_accounted(&inline, index + 1);
        }
        {
            let state = inline.mailbox.lock();
            assert!(
                state.spare.len() <= SPARE_JOBS,
                "{} kept",
                state.spare.len()
            );
            assert!(!state.spare.is_empty(), "spent jobs were returned");
        }
        let reused = inline.blank();
        assert!(reused.pixels.is_empty(), "a returned job is cleared");
        assert!(reused.pixels.capacity() >= 4096, "but keeps its room");
        for _ in 0..10 {
            inline.recycle(Job::default());
        }
        assert!(inline.mailbox.lock().spare.len() <= SPARE_JOBS);
    }

    /// A sink that fails.
    #[derive(Clone)]
    struct Fails;

    impl Write for Fails {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the terminal went away",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_failed_write_is_reported_by_the_next_present() {
        let mut inline = Inline::new(Fails);
        inline.submit(marked("#1")).unwrap();
        let begun = Instant::now();
        while inline.mailbox.lock().error.is_none() {
            assert!(
                begun.elapsed() < Duration::from_secs(5),
                "the failure was never recorded"
            );
            thread::sleep(Duration::from_millis(2));
        }
        assert!(inline.settle());
        let error = inline.submit(marked("#2")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert!(error.to_string().contains("went away"));
        // Reported once.
        assert!(inline.submit(marked("#3")).is_ok());
    }

    /// A sink that panics when written to.
    #[derive(Clone)]
    struct Panics;

    impl Write for Panics {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            panic!("the sink panics on purpose");
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_panic_on_the_worker_is_reported_as_an_error_and_hangs_nothing() {
        let mut inline = Inline::new(Panics);
        inline.submit(marked("#1")).unwrap();
        // Settling waits for a worker that was writing when it panicked: it must come back.
        let begun = Instant::now();
        while !inline.mailbox.lock().panicked {
            assert!(
                begun.elapsed() < Duration::from_secs(5),
                "the panic was never recorded"
            );
            thread::sleep(Duration::from_millis(2));
        }
        assert!(inline.settle(), "nothing is being written");
        let error = inline.submit(marked("#2")).unwrap_err();
        assert_eq!(error.to_string(), PANICKED);
        let again = inline.submit(marked("#3")).unwrap_err();
        assert_eq!(again.to_string(), PANICKED, "and every frame after it");
        let begun = Instant::now();
        inline.shutdown();
        assert!(begun.elapsed() < Duration::from_secs(2));
    }
}
