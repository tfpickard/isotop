//! Test only: a counting allocator and what it measures, the heap allocations of a steady-state
//! frame of every view. The bounds below are a ratchet: they only go down, and a change that
//! raises one says why.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// Where allocations were requested: a call path, as text, to the requests and bytes it made.
type Sites = HashMap<String, [u64; 2]>;

thread_local! {
    /// While a test looks for where allocations come from, the sites it has seen. Recording
    /// allocates, so `RECORDING` keeps the allocator from recording its own requests.
    static SITES: RefCell<Option<Sites>> = const { RefCell::new(None) };
    /// Set while an allocation is being recorded.
    static RECORDING: Cell<bool> = const { Cell::new(false) };
    /// Allocations and bytes requested by this thread so far. A thread's own counts, so tests
    /// that run side by side do not see each other's allocations. Const-initialized and without
    /// a destructor, so the allocator may touch it at any point of a thread's life.
    static TALLY: Cell<[u64; 2]> = const { Cell::new([0, 0]) };
}

/// The system allocator, counting every request (an allocation, a zeroed one and a `realloc`,
/// which may move) and the bytes asked for.
struct Counting;

/// Adds one request of `bytes` to this thread's tally.
fn count(bytes: usize) {
    // A thread that is being torn down has no tally left to add to, and that is fine.
    let _ = TALLY.try_with(|tally| {
        let [requests, total] = tally.get();
        tally.set([requests + 1, total + bytes as u64]);
    });
    if !RECORDING.get() && SITES.try_with(|sites| sites.borrow().is_some()) == Ok(true) {
        record(bytes);
    }
}

/// Notes the call path of the request in progress, unless it is the recording's own.
fn record(bytes: usize) {
    if RECORDING.replace(true) {
        return;
    }
    let path = site();
    let _ = SITES.try_with(|sites| {
        if let Some(sites) = sites.borrow_mut().as_mut() {
            let seen = sites.entry(path).or_default();
            seen[0] += 1;
            seen[1] += bytes as u64;
        }
    });
    RECORDING.set(false);
}

/// The first two frames of this crate on the stack, innermost first, each with its file and line.
fn site() -> String {
    let trace = std::backtrace::Backtrace::force_capture().to_string();
    let mut path = Vec::new();
    let mut lines = trace.lines().peekable();
    while let Some(line) = lines.next() {
        let name = line
            .trim_start()
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == ':')
            .trim();
        let name = name.trim_start_matches('<');
        if name.starts_with("isotop::") && !name.starts_with("isotop::allocations") {
            let place = lines.peek().map_or("", |next| next.trim());
            path.push(format!("{name} {}", place.trim_start_matches("at ")));
            if path.len() == 2 {
                break;
            }
        }
    }
    path.join("  <-  ")
}

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract, with the
// same layout and pointer it was given; the counting only touches a thread-local `Cell`, which
// does not allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: the caller's contract for `alloc` is the one `System::alloc` needs.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: the caller's contract for `alloc_zeroed` is the one `System` needs.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        // SAFETY: the caller's contract for `realloc` is the one `System::realloc` needs.
        unsafe { System.realloc(pointer, layout, new_size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract for `dealloc` is the one `System::dealloc` needs.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// What a stretch of work requested from the allocator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    /// Calls to `alloc`, `alloc_zeroed` and `realloc`.
    pub(crate) requests: u64,
    /// Bytes those calls asked for (a `realloc` counts its new size).
    pub(crate) bytes: u64,
}

/// This thread's allocations so far.
fn now() -> Tally {
    let [requests, bytes] = TALLY.with(Cell::get);
    Tally { requests, bytes }
}

/// Runs `work` and returns where on the stack its allocations were requested, the most bytes
/// first, with the number of requests and the bytes of each path.
fn sites_of<R>(work: impl FnOnce() -> R) -> (R, Vec<(String, [u64; 2])>) {
    SITES.with(|sites| *sites.borrow_mut() = Some(Sites::new()));
    let result = work();
    let sites = SITES
        .with(|sites| sites.borrow_mut().take())
        .unwrap_or_default();
    let mut sites: Vec<_> = sites.into_iter().collect();
    sites.sort_by_key(|(path, [_, bytes])| (std::cmp::Reverse(*bytes), path.clone()));
    (result, sites)
}

/// Runs `work` and returns what it requested from the allocator on this thread.
pub(crate) fn counted<R>(work: impl FnOnce() -> R) -> (R, Tally) {
    let before = now();
    let result = work();
    let after = now();
    (
        result,
        Tally {
            requests: after.requests - before.requests,
            bytes: after.bytes - before.bytes,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;
    use crate::demo_snapshot;
    use crate::headless::warm_up;
    use crate::views::View;

    /// Width of the frames measured: small enough to be quick, wide enough that a view's own
    /// layout has room.
    const WIDTH: &str = "480";
    /// Frames drawn before counting starts: the buffers have reached their size by then.
    const SETTLE: u32 = 6;
    /// Frames counted.
    const FRAMES: u32 = 8;

    /// The app `isotop --demo --time 12 --renderer cpu --width 480` draws for `view` with
    /// `processes` synthetic processes, warmed up as a headless run is.
    fn warmed(view: View, processes: u32) -> (App, Options) {
        let options = options(&[
            "--demo",
            "--time",
            "12",
            "--renderer",
            "cpu",
            "--width",
            WIDTH,
            "--benchmark",
            "1",
            "--processes",
            &processes.to_string(),
            "--view",
            view.name(),
        ]);
        let mut app = App::new(
            view,
            demo_snapshot(
                view.info(),
                options.time - options.sample_seconds(),
                options.time,
                options.processes as usize,
            ),
            all_available(),
        );
        warm_up(&mut app, &options, None);
        (app, options)
    }

    /// Draws and rasterizes one frame of the benchmark loop, recycling the last one.
    fn step(app: &mut App, options: &Options, frame: Frame, index: u32) -> Frame {
        app.animation = options.time as f32 + index as f32 / options.fps as f32;
        app.scene.spare = frame.release();
        let mut frame = app.draw(
            options.width,
            options.headless_height(),
            options.limit as usize,
        );
        app.rasterize(&mut frame);
        frame
    }

    /// What one steady-state frame of `view` requests, on average over `FRAMES` frames (rounded
    /// up), after `SETTLE` frames that are not counted.
    fn steady_state(view: View, processes: u32) -> Tally {
        let (mut app, options) = warmed(view, processes);
        let mut frame = app.render(
            options.width,
            options.headless_height(),
            options.limit as usize,
        );
        for index in 0..SETTLE {
            frame = step(&mut app, &options, frame, index);
        }
        let (_, tally) = counted(|| {
            for index in 0..FRAMES {
                frame = step(&mut app, &options, frame, SETTLE + index);
            }
        });
        let frames = u64::from(FRAMES);
        Tally {
            requests: tally.requests.div_ceil(frames),
            bytes: tally.bytes.div_ceil(frames),
        }
    }

    /// Every view's steady-state allocations, in registry order, each on its own thread.
    fn every_view(processes: u32) -> Vec<(&'static str, Tally)> {
        std::thread::scope(|scope| {
            let runs: Vec<_> = View::all()
                .map(|view| scope.spawn(move || (view.name(), steady_state(view, processes))))
                .collect();
            runs.into_iter()
                .map(|run| run.join().expect("the view renders"))
                .collect()
        })
    }

    /// Processes in the demo of the table below: the default of `--processes`.
    const PROCESSES: u32 = 192;

    /// The ratchet: the most heap allocations (and bytes) a steady-state frame of each view may
    /// request at 192 processes, 480 pixels wide. Lower a bound when a change brings a view
    /// down; never raise one without saying why in the PR.
    const BOUNDS: &[(&str, u64, u64)] = &[
        ("city", 10, 14045),
        ("orbit", 263, 146783),
        ("ripple", 118, 81628),
        ("flow", 278, 1095519),
        ("cores", 650, 50358),
        ("cells", 2057, 140397),
        ("strata", 342, 65482),
        ("globe", 402, 65845),
        ("reef", 828, 79764),
        ("matrix", 10, 8002),
        ("coop", 2729, 219858),
        ("ptolemy", 30, 18970),
        ("tesseract", 4208, 709868),
        ("necropolis", 328, 222049),
        ("cortex", 77, 58306),
        ("horizon", 251, 194537),
        ("murmuration", 22, 114508),
        ("arbor", 85, 32663),
        ("hr", 52, 37750),
        ("hive", 541, 70932),
        ("switchboard", 604, 46486),
        ("ouija", 83, 17910),
        ("anthill", 230, 164572),
        ("plumbing", 723, 90970),
        ("armorial", 151, 237106),
        ("lights", 266, 52336),
        ("die", 150, 43500),
    ];

    #[test]
    fn a_frame_does_not_copy_the_processes_it_draws() {
        // Copying every process costs four strings each, which at a thousand processes is four
        // thousand allocations a frame; a view that only draws them asks for a handful.
        for name in ["city", "matrix", "ptolemy"] {
            let view = View::all()
                .find(|view| view.name() == name)
                .expect("a registered view");
            let tally = steady_state(view, 1000);
            assert!(
                tally.requests < 100,
                "{name} at 1000 processes asked for {} allocations a frame",
                tally.requests
            );
        }
    }

    #[test]
    fn the_counter_counts_what_a_block_allocates() {
        let (kept, tally) = counted(|| vec![0_u8; 4096]);
        assert!(tally.requests >= 1 && tally.bytes >= 4096);
        drop(kept);
        let ((), idle) = counted(|| {});
        assert_eq!(idle, Tally::default());
    }

    /// Prints the table of `BOUNDS`: run with
    /// `cargo test --release print_allocation_table -- --ignored --nocapture`.
    #[test]
    #[ignore = "prints a table; run on demand"]
    fn print_allocation_table() {
        let processes = std::env::var("ALLOCATION_PROCESSES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(PROCESSES);
        println!("view requests/frame bytes/frame ({processes} processes)");
        for (name, tally) in every_view(processes) {
            println!("{name} {} {}", tally.requests, tally.bytes);
        }
    }

    /// Prints where the steady-state frames of the view named by `ALLOCATION_VIEW` allocate:
    /// `ALLOCATION_VIEW=flow cargo test print_allocation_sites -- --ignored --nocapture`.
    #[test]
    #[ignore = "prints a table; run on demand"]
    fn print_allocation_sites() {
        let name = std::env::var("ALLOCATION_VIEW").unwrap_or_else(|_| "city".to_owned());
        let view = View::all()
            .find(|view| view.name() == name)
            .expect("ALLOCATION_VIEW names a view");
        let (mut app, options) = warmed(view, PROCESSES);
        let mut frame = app.render(
            options.width,
            options.headless_height(),
            options.limit as usize,
        );
        for index in 0..SETTLE {
            frame = step(&mut app, &options, frame, index);
        }
        let (_, sites) = sites_of(|| {
            for index in 0..FRAMES {
                frame = step(&mut app, &options, frame, SETTLE + index);
            }
        });
        println!("requests bytes (over {FRAMES} frames) site");
        for (path, [requests, bytes]) in sites.iter().take(25) {
            println!("{requests} {bytes} {path}");
        }
    }

    #[test]
    fn every_view_stays_within_its_allocation_bound() {
        let measured = every_view(PROCESSES);
        let mut over = Vec::new();
        for (name, tally) in &measured {
            match BOUNDS.iter().find(|(bounded, ..)| bounded == name) {
                None => over.push(format!("{name} has no bound; measured {tally:?}")),
                Some(&(_, requests, bytes)) => {
                    if tally.requests > requests || tally.bytes > bytes {
                        over.push(format!(
                            "{name}: {tally:?} is over its bound of {requests} requests and {bytes} bytes"
                        ));
                    }
                }
            }
        }
        assert!(over.is_empty(), "{}", over.join("\n"));
    }
}
