//! The interactive run: the terminal, and the loop that takes input, samples, renders and
//! presents a frame each turn.

use std::error::Error;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};

use crate::app::{App, Layout, typed};
use crate::cli::Options;
use crate::demo_snapshot;
use crate::journal::Journal;
use crate::model::{Collector, Demand};
use crate::platform::Source;
use crate::render::Frame;
use crate::terminal::{Popup, SafeText, Terminal};

/// How far behind the clock the journal feed starts, in seconds: after a pause or a slow frame
/// it catches up over this much time instead of the whole gap.
const FEED_LAG: f64 = 2.0;

type Outcome<T> = Result<T, Box<dyn Error>>;

/// What the frame just presented leaves for the input that follows it: the popups, which mouse
/// events must not fall through, and whether mouse reports arrive in pixels.
struct Shown {
    panels: Vec<Popup>,
    pixels: bool,
}

/// The state of one interactive run.
struct Live<'a> {
    options: &'a Options,
    app: App,
    collector: Collector,
    /// The journal follower, while the shown view or a recent one reads the journal.
    journal: Option<Journal>,
    terminal: Terminal,
    layout: Layout,
    /// The idle time before the tour starts; None when `--tour` is 0.
    tour_after: Option<Duration>,
    origin: Instant,
    last_sample: Instant,
    last_frame: Instant,
    /// The demo's clock: the time of its latest sample.
    demo_time: f64,
    /// Seconds since `origin` up to which the journal's lines have been fed.
    fed: f64,
    /// The optional sources the collector was last told to read.
    demanded: Demand,
    /// Bytes the last frame took on the wire.
    wire: usize,
    frames: u64,
    /// A failure to make on purpose after the third frame (`ISOTOP_FORCE_PANIC`), once.
    forced: Option<Forced>,
}

/// Which panic `ISOTOP_FORCE_PANIC` asks for. A hidden switch for the smoke test, which checks
/// that the terminal is restored after each.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Forced {
    /// A panic on the main thread, which owns the terminal.
    Main,
    /// A panic on a thread of its own, which the app must survive.
    Thread,
}

/// The frame after which a forced panic happens.
const FORCED_AFTER: u64 = 3;

impl Forced {
    fn from_env() -> Option<Self> {
        match std::env::var("ISOTOP_FORCE_PANIC").ok()?.as_str() {
            "main" => Some(Self::Main),
            "thread" => Some(Self::Thread),
            _ => None,
        }
    }

    fn happen(self) {
        match self {
            Self::Main => panic!("forced panic on the main thread (ISOTOP_FORCE_PANIC=main)"),
            Self::Thread => {
                // The result is the handle of a thread that has only a panic to run.
                let _ = std::thread::Builder::new()
                    .name("isotop-forced-panic".to_owned())
                    .spawn(|| panic!("forced panic on a thread (ISOTOP_FORCE_PANIC=thread)"));
            }
        }
    }
}

/// Runs the interactive loop until the user quits or `--duration` passes. With `--duration` it
/// reports the presentation throughput on standard error once the terminal is restored.
pub(crate) fn run(
    app: App,
    collector: Collector,
    journal: Option<Journal>,
    options: &Options,
) -> Outcome<()> {
    let mut live = Live::enter(options, app, collector, journal)?;
    live.run()?;
    // The frame being written inline is finished before the totals are read.
    live.terminal.settle();
    let (frames, origin) = (live.frames, live.origin);
    let (total_wire, written) = live.terminal.written();
    drop(live);
    if options.duration.is_some() {
        let seconds = origin.elapsed().as_secs_f64();
        eprintln!(
            "isotop: {frames} frames in {seconds:.2}s | {:.1} presented fps | {:.2} MiB/s inline graphics (display latency not measured) | {:.1} written fps",
            frames as f64 / seconds,
            total_wire as f64 / 1048576.0 / seconds,
            written as f64 / seconds
        );
    }
    Ok(())
}

impl<'a> Live<'a> {
    /// Takes over the terminal and sizes the scene to it.
    fn enter(
        options: &'a Options,
        mut app: App,
        collector: Collector,
        journal: Option<Journal>,
    ) -> Outcome<Self> {
        let terminal = Terminal::enter(options.force_graphics, options.direct)?;
        app.scene.intro = true;
        app.keyboard = terminal.keyboard;
        // The GPU draws a frame while the next is recorded, which a headless run never does.
        let _ = app.renderer.set_pipelined(!options.no_gpu_pipeline);
        let layout = Layout::measure(options.width, app.panel_shown())?;
        let now = Instant::now();
        Ok(Self {
            options,
            demanded: app.demand(),
            app,
            collector,
            journal,
            terminal,
            layout,
            tour_after: (options.tour > 0).then(|| Duration::from_secs(u64::from(options.tour))),
            origin: now,
            last_sample: now,
            last_frame: now,
            demo_time: 0.0,
            fed: 0.0,
            wire: 0,
            frames: 0,
            forced: Forced::from_env(),
        })
    }

    /// One turn per frame: sample, render, present, then input until the next frame is due.
    fn run(&mut self) -> Outcome<()> {
        let frame_interval = Duration::from_secs_f64(1.0 / f64::from(self.options.fps));
        let mut quit = false;
        while !quit {
            let frame_start = Instant::now();
            let elapsed = self.last_frame.elapsed().as_secs_f32();
            self.last_frame = Instant::now();
            self.sample(elapsed)?;
            self.follow_journal();
            // The first frame the GPU is given has nothing before it to present.
            let Some((mut frame, render_ms)) = self.render(elapsed, frame_start) else {
                continue;
            };
            let shown = self.present(&mut frame, render_ms)?;
            quit = self.input(frame_start + frame_interval, &frame, &shown)?;
            quit |= self.finish(frame);
            // A view that deferred the quit has played its farewell in the frame just drawn.
            quit |= self.app.farewell_over();
        }
        // The frame the GPU still holds is finished before the terminal is given back.
        let _ = self.app.renderer.flush();
        Ok(())
    }

    /// Advances the animation clock, unless paused, and takes in a new sample when one is due.
    fn sample(&mut self, elapsed: f32) -> Outcome<()> {
        if self.app.history.is_paused() {
            return Ok(());
        }
        self.app.animation += elapsed;
        let interval = Duration::from_millis(u64::from(self.options.sample_ms));
        if self.last_sample.elapsed() < interval {
            return Ok(());
        }
        let snapshot = if self.options.demo {
            let previous = self.demo_time;
            self.demo_time = self.origin.elapsed().as_secs_f64();
            demo_snapshot(
                self.app.row(),
                previous,
                self.demo_time,
                self.options.processes as usize,
            )
        } else {
            self.collector.sample()?
        };
        self.app
            .take_sample(snapshot, self.options.history as usize);
        self.last_sample = Instant::now();
        Ok(())
    }

    /// Starts or stops the journal follower as the shown view needs it, and feeds the newest
    /// sample the lines that arrived since the last frame.
    fn follow_journal(&mut self) {
        if !self.options.demo {
            if !self.app.journal_wanted(Instant::now()) {
                // Dropping it stops the follower and its reader.
                self.journal = None;
            } else if self.journal.is_none() {
                self.journal = Some(Journal::start());
            }
        }
        if self.app.demand().contains(Source::Journal) {
            let now = self.origin.elapsed().as_secs_f64();
            // Paused, the view keeps what it shows; the journal backlog is bounded meanwhile.
            if !self.app.history.is_paused() {
                self.fed = f64::max(self.fed, now - FEED_LAG);
                self.app
                    .feed_journal(self.journal.as_ref(), self.options.demo, self.fed, now);
                self.fed = now;
            }
        }
    }

    /// Moves the camera and the tour on, records the frame and gives it to the rasterizer, and
    /// returns the frame to present: this one, or with the GPU running a frame behind the one
    /// before it, which the GPU drew while this one was recorded. Also returns how long the turn
    /// has taken so far, in milliseconds.
    fn render(&mut self, elapsed: f32, frame_start: Instant) -> Option<(Frame, f32)> {
        self.app.steer(elapsed, self.tour_after);
        let frame = self.app.draw(
            self.layout.width,
            self.layout.height,
            self.options.limit as usize,
        );
        let frame = self.app.rasterize_ahead(frame)?;
        Some((frame, frame_start.elapsed().as_secs_f32() * 1000.0))
    }

    /// Puts the text over the frame and sends it to the terminal.
    fn present(&mut self, frame: &mut Frame, render_ms: f32) -> Outcome<Shown> {
        let pixels = self.terminal.pixel_mouse && self.layout.cell.is_some();
        let radius = self.layout.pick_radius(pixels);
        self.app.pointer.hover_over(frame, radius);
        let (labels, panels) = self.app.overlay(frame, &self.layout, radius);
        let transport = if self.terminal.shared_memory {
            "shared memory".to_owned()
        } else {
            format!("{:.1} KiB/frame inline", self.wire as f32 / 1024.0)
        };
        let text: Vec<SafeText> = if self.app.panel_shown() {
            self.app.status_rows(
                render_ms,
                &transport,
                self.options.fps,
                self.options.limit as usize,
            )
        } else {
            Vec::new()
        };
        self.wire = self.terminal.present(
            frame,
            self.layout.columns,
            self.layout.rows,
            &labels,
            &panels,
            &text,
        )?;
        self.frames += 1;
        if self.frames == FORCED_AFTER
            && let Some(forced) = self.forced
        {
            forced.happen();
        }
        Ok(Shown { panels, pixels })
    }

    /// Handles the input that arrives until `deadline`, and returns whether the user quit.
    fn input(&mut self, deadline: Instant, frame: &Frame, shown: &Shown) -> Outcome<bool> {
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            if !event::poll(wait)? {
                return Ok(false);
            }
            // Past the deadline, poll(0) keeps draining queued input; motion floods must not delay keys.
            if self.handle(event::read()?, frame, shown)? {
                return Ok(true);
            }
        }
    }

    /// Acts on one event, and returns whether it quits.
    fn handle(&mut self, event: Event, frame: &Frame, shown: &Shown) -> Outcome<bool> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let panel = self.app.panel_shown();
                let (code, modifiers) = typed(&key);
                let quit = self
                    .app
                    .key(code, modifiers, self.layout.width as f32 * 0.04);
                if self.app.panel_shown() != panel {
                    self.resize()?;
                }
                Ok(quit)
            }
            Event::Resize(_, _) => {
                self.resize()?;
                Ok(false)
            }
            Event::Mouse(mouse) => {
                self.app.touch();
                self.app
                    .mouse(mouse, &self.layout, frame, &shown.panels, shown.pixels);
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// Measures the terminal again and clears it, after a resize or when the panel came or went.
    fn resize(&mut self) -> Outcome<()> {
        // A frame still in flight has the old size, and is not shown.
        if let Some(frame) = self.app.renderer.flush() {
            self.app.scene.spare = frame.release();
        }
        self.layout = Layout::measure(self.options.width, self.app.panel_shown())?;
        self.terminal.clear()?;
        Ok(())
    }

    /// Closes the turn: tells the collector what the shown view now reads, keeps the frame's
    /// buffers for the next one and returns whether `--duration` is up.
    fn finish(&mut self, frame: Frame) -> bool {
        if self.app.demand() != self.demanded {
            self.demanded = self.app.demand();
            self.collector.demand(self.demanded);
        }
        self.app.scene.spare = frame.release();
        self.options
            .duration
            .is_some_and(|duration| self.origin.elapsed().as_secs_f64() >= duration)
    }
}
