mod app;
mod arbor;
mod blackbody;
mod cells;
mod coop;
mod cores;
mod cortex;
mod geo;
mod geometry;
mod globe;
mod glyphs;
mod gpu;
#[cfg(test)]
mod harness;
mod history;
mod horizon;
mod journal;
mod layout;
mod matrix;
mod medium;
mod model;
mod murmuration;
mod necropolis;
mod neurons;
mod palette;
mod picker;
mod places;
mod platform;
mod ptolemy;
mod reef;
mod render;
mod simulation;
mod status;
mod stellar;
mod strata;
mod terminal;
mod tesseract;
mod threads;
mod views;

use std::collections::VecDeque;
use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use clap::builder::{PossibleValuesParser, TypedValueParser};
use crossterm::event::{self, Event, KeyEventKind};

use app::{App, Layout, feed, typed};
use journal::Journal;
use model::{Collector, Demand, Snapshot};
use platform::Source;
use terminal::Terminal;
use views::{Phase, REGISTRY, View, ViewInfo, Warmup, When};

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Renderer {
    Auto,
    Gpu,
    Cpu,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Power {
    /// Integrated GPU where there is one: keeps a laptop's discrete GPU asleep
    Low,
    /// Discrete GPU (for example NVIDIA) where there is one
    High,
}

#[derive(Parser)]
#[command(
    version,
    about = "A living process city and orbital observatory inside your terminal"
)]
struct Options {
    /// Use a deterministic synthetic workload instead of /proc
    #[arg(long)]
    demo: bool,
    /// Initial visualization (city, or the first of --views)
    #[arg(long, value_parser = view_parser())]
    view: Option<View>,
    /// The views Tab and Shift+Tab cycle, as a comma-separated list such as reef,city: a name
    /// repeated or not supported on this platform is dropped. Without it, every supported view
    #[arg(long, value_delimiter = ',', value_parser = view_parser())]
    views: Vec<View>,
    /// Print the views this platform supports, one per line, and exit
    #[arg(long)]
    list_views: bool,
    /// Print startup notes to stderr, such as each view this platform does not support
    #[arg(long)]
    verbose: bool,
    /// Number of synthetic processes
    #[arg(long, default_value_t = 192, value_parser = clap::value_parser!(u32).range(1..=10000))]
    processes: u32,
    /// Maximum processes admitted into a scene
    #[arg(long, default_value_t = 1024, value_parser = clap::value_parser!(u32).range(1..=10000))]
    limit: u32,
    /// Maximum internal rendering width (height follows terminal aspect ratio)
    #[arg(long, default_value_t = 1920, value_parser = clap::value_parser!(u32).range(160..=2560))]
    width: u32,
    /// Target animation frame rate
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=60))]
    fps: u32,
    /// Process sampling interval in milliseconds
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(100..=10000))]
    sample_ms: u32,
    /// Number of snapshots retained for pause/rewind
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(2..=3600))]
    history: u32,
    /// Seconds without input before the guided tour starts (0 disables it)
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(0..=3600))]
    tour: u32,
    /// Render a single PNG and exit (no terminal required)
    #[arg(long)]
    output: Option<PathBuf>,
    /// Synthetic time for PNG output and benchmark
    #[arg(long, default_value_t = 12.0)]
    time: f64,
    /// Render this many frames headlessly and report renderer performance
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10000))]
    benchmark: Option<u32>,
    /// Bypass the terminal graphics capability query
    #[arg(long)]
    force_graphics: bool,
    /// Send frames inline (zlib + base64) even when the terminal can read shared memory
    #[arg(long)]
    direct: bool,
    /// Rasterizer: the GPU when one is available, falling back to the CPU
    #[arg(long, value_enum, default_value_t = Renderer::Auto)]
    renderer: Renderer,
    /// Which GPU to prefer on machines with more than one
    #[arg(long, value_enum, default_value_t = Power::High)]
    gpu_power: Power,
    /// Exit after this many seconds and report end-to-end presentation throughput
    #[arg(long)]
    duration: Option<f64>,
    /// City-level GeoIP database (MaxMind format, such as DB-IP Lite or GeoLite2 City) for the
    /// globe; found in ~/.local/share/isotop and /usr/share/GeoIP when not given
    #[arg(long)]
    geoip: Option<PathBuf>,
    /// This machine's location as LAT,LON for the globe; defaults to the system time zone's city
    #[arg(long, value_parser = parse_home, allow_hyphen_values = true)]
    home: Option<(f32, f32)>,
}

impl Options {
    /// Height of a headless frame: 16:9 at the requested width.
    fn headless_height(&self) -> u32 {
        self.width * 9 / 16
    }

    /// Whether the run draws without a terminal: `--output` or `--benchmark`.
    fn headless(&self) -> bool {
        self.output.is_some() || self.benchmark.is_some()
    }

    /// `--sample-ms` in seconds: how far back the sample before the first one is.
    fn sample_seconds(&self) -> f64 {
        f64::from(self.sample_ms) / 1000.0
    }
}

/// Parses `--view` from the registry's names, so its help lists them and any other name is a
/// usage error.
fn view_parser() -> impl TypedValueParser<Value = View> {
    PossibleValuesParser::new(REGISTRY.iter().map(|row| row.name))
        .map(|name| View::named(&name).expect("the parser accepts only the registry's names"))
}

fn parse_home(text: &str) -> Result<(f32, f32), String> {
    let (latitude, longitude) = text
        .split_once(',')
        .ok_or("expected LAT,LON, for example 40.7,-74.0")?;
    let latitude: f32 = latitude
        .trim()
        .parse()
        .map_err(|_| "latitude is not a number")?;
    let longitude: f32 = longitude
        .trim()
        .parse()
        .map_err(|_| "longitude is not a number")?;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err("latitude must be within ±90 and longitude within ±180".into());
    }
    Ok((latitude, longitude))
}

/// What `run` does after the startup that comes before the terminal is touched.
#[derive(Debug, PartialEq, Eq)]
enum Preflight {
    /// Exit at once with this status.
    Exit(i32),
    /// Run, starting on this view.
    Start(View),
}

/// The startup before the terminal is touched. With `--verbose` it names each view this platform
/// does not support on `err`; `--list-views` lists the supported ones on `out`. A headless run
/// of an unsupported `--view` fails with status 1. An interactive one, and any run without
/// `--view` whose default (city) is unsupported, starts on the first supported view instead (the
/// first of `--views` when given). `--views` with no supported name fails with status 1. `available` holds each registry row's answer.
fn preflight(
    options: &Options,
    available: &[bool],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Preflight {
    // Writes to the standard streams only fail when they are closed; there is no one to tell.
    let supported = |view: View| available.get(view.index()) == Some(&true);
    if options.verbose {
        for view in View::all().filter(|&view| !supported(view)) {
            let _ = writeln!(
                err,
                "skipping view {}: not supported on this platform",
                view.name()
            );
        }
    }
    if options.list_views {
        for view in View::all().filter(|&view| supported(view)) {
            let _ = writeln!(out, "{}", view.name());
        }
        return Preflight::Exit(0);
    }
    let playlist = picker::playlist(&options.views, available);
    if !options.views.is_empty() && playlist.is_empty() {
        let _ = writeln!(err, "isotop: no available view in --views");
        return Preflight::Exit(1);
    }
    let requested = options.view.unwrap_or_else(|| {
        if options.views.is_empty() {
            View::named("city").expect("the registry has the city view")
        } else {
            playlist[0]
        }
    });
    if supported(requested) {
        return Preflight::Start(requested);
    }
    if options.headless() && options.view.is_some() {
        let _ = writeln!(
            err,
            "isotop: view {} is not supported on this platform",
            requested.name()
        );
        return Preflight::Exit(1);
    }
    match playlist.first() {
        Some(&view) => Preflight::Start(view),
        None => {
            let _ = writeln!(err, "isotop: no view is supported on this platform");
            Preflight::Exit(1)
        }
    }
}

/// Runs isotop and returns the exit status for `main`.
#[expect(clippy::too_many_lines, reason = "split in #79 (A20)")]
fn run(options: Options) -> Result<i32, Box<dyn Error>> {
    let available = views::availability();
    let view = match preflight(&options, &available, &mut io::stdout(), &mut io::stderr()) {
        Preflight::Exit(status) => return Ok(status),
        Preflight::Start(view) => view,
    };
    if !options.time.is_finite() || options.time < 0.0 {
        return Err("--time must be finite and nonnegative".into());
    }
    if options.duration.is_some_and(|v| !v.is_finite() || v <= 0.0) {
        return Err("--duration must be finite and positive".into());
    }
    let demand = Demand::from(view.info().sources);
    let mut collector = Collector::new(options.geoip.clone(), options.home);
    collector.demand(demand);
    let mut journal = (demand.contains(Source::Journal) && !options.demo).then(Journal::start);
    // The demo's clock: the time of its latest sample.
    let mut demo_time = if options.headless() {
        options.time
    } else {
        0.0
    };
    let mut snapshot = if options.demo {
        demo_snapshot(
            view.info(),
            demo_time - options.sample_seconds(),
            demo_time,
            options.processes as usize,
        )
    } else {
        collector.sample()?
    };
    if !options.demo && options.headless() {
        std::thread::sleep(Duration::from_millis(options.sample_ms as u64));
        snapshot = collector.sample()?;
    }
    let playlist = picker::playlist(&options.views, &available);
    let mut app = App::new(view, snapshot, available);
    app.playlist = playlist;
    app.choose_renderer(options.renderer, options.gpu_power)?;
    let limit = options.limit as usize;
    if options.headless() {
        let height = options.headless_height();
        warm_up(&mut app, &options, journal.as_ref());
        app.animation = options.time as f32;
        let mut frame = app.render(options.width, height, limit);
        if let Some(count) = options.benchmark {
            let start = Instant::now();
            for i in 0..count {
                app.animation = options.time as f32 + i as f32 / options.fps as f32;
                app.scene.spare = frame.release();
                frame = app.render(options.width, height, limit);
                std::hint::black_box(&frame.pixels);
            }
            let elapsed = start.elapsed().as_secs_f64();
            println!(
                "{} frames | {}x{} | {} visible | {:.2} ms/frame | {:.1} renderer fps on {} (excludes terminal transport)",
                count,
                frame.width,
                frame.height,
                app.scene.visible,
                elapsed * 1000.0 / count as f64,
                count as f64 / elapsed,
                app.renderer
            );
        }
        if let Some(path) = options.output {
            frame.write_png(&path)?;
            println!(
                "Wrote {} ({}x{}, {} visible)",
                path.display(),
                frame.width,
                frame.height,
                app.scene.visible
            );
        }
        return Ok(0);
    }
    let mut terminal = Terminal::enter(options.force_graphics, options.direct)?;
    app.scene.intro = true;
    app.keyboard = terminal.keyboard;
    let mut layout = Layout::measure(options.width, app.panel_shown())?;
    let tour_after = (options.tour > 0).then(|| Duration::from_secs(options.tour as u64));
    let origin = Instant::now();
    let mut last_sample = Instant::now();
    let mut last_frame = Instant::now();
    let frame_interval = Duration::from_secs_f64(1.0 / options.fps as f64);
    let mut wire = 0;
    let mut quit = false;
    let mut frames = 0_u64;
    let mut total_wire = 0_u64;
    let mut fed = 0.0;
    let mut demanded = app.demand();
    while !quit {
        let frame_start = Instant::now();
        let elapsed = last_frame.elapsed().as_secs_f32();
        last_frame = Instant::now();
        if !app.paused {
            app.animation += elapsed;
            if last_sample.elapsed() >= Duration::from_millis(options.sample_ms as u64) {
                let snapshot = if options.demo {
                    let previous = demo_time;
                    demo_time = origin.elapsed().as_secs_f64();
                    demo_snapshot(app.row(), previous, demo_time, options.processes as usize)
                } else {
                    collector.sample()?
                };
                app.push_sample(snapshot, options.history as usize);
                last_sample = Instant::now();
                if app.framed && app.tour.is_none() && app.scene.bounded() {
                    app.fit = true;
                }
            }
        }
        let goal = app.goal.clone();
        if !options.demo {
            if !app.journal_wanted(Instant::now()) {
                // Dropping it stops the follower and its reader.
                journal = None;
            } else if journal.is_none() {
                journal = Some(Journal::start());
            }
        }
        if app.demand().contains(Source::Journal) {
            let now = origin.elapsed().as_secs_f64();
            // Paused, the view keeps what it shows; the journal backlog is bounded meanwhile.
            if !app.paused {
                fed = f64::max(fed, now - 2.0);
                feed(&mut app, journal.as_ref(), options.demo, fed, now);
                fed = now;
            }
        }
        app.camera
            .approach(&goal, simulation::smoothing(elapsed, 0.2));
        app.update_tour(tour_after);
        let mut frame = app.render(layout.width, layout.height, limit);
        let render_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        let pixels = terminal.pixel_mouse && layout.cell.is_some();
        app.hover_over(&frame, layout.pick_radius(pixels));
        let (labels, panels) = app.overlay(&mut frame, &layout, layout.pick_radius(pixels));
        let transport = if terminal.shared_memory {
            "shared memory".to_owned()
        } else {
            format!("{:.1} KiB/frame inline", wire as f32 / 1024.0)
        };
        let text = if app.panel_shown() {
            app.text(render_ms, &transport, options.fps, limit)
        } else {
            Vec::new()
        };
        wire = terminal.present(&frame, layout.columns, layout.rows, &labels, &panels, &text)?;
        frames += 1;
        total_wire += wire as u64;
        let deadline = frame_start + frame_interval;
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            if !event::poll(wait)? {
                break;
            }
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    let shown = app.panel_shown();
                    let (code, modifiers) = typed(&key);
                    quit = app.key(code, modifiers, layout.width as f32 * 0.04);
                    if app.panel_shown() != shown {
                        layout = Layout::measure(options.width, app.panel_shown())?;
                        terminal.clear()?;
                    }
                }
                Event::Resize(_, _) => {
                    layout = Layout::measure(options.width, app.panel_shown())?;
                    terminal.clear()?;
                }
                Event::Mouse(mouse) => {
                    app.touch();
                    app.mouse(mouse, &layout, &frame, &panels, pixels);
                }
                _ => {}
            }
            // Past the deadline, poll(0) keeps draining queued input; motion floods must not delay keys.
            if quit {
                break;
            }
        }
        if app.demand() != demanded {
            demanded = app.demand();
            collector.demand(demanded);
        }
        if options
            .duration
            .is_some_and(|duration| origin.elapsed().as_secs_f64() >= duration)
        {
            quit = true;
        }
        app.scene.spare = frame.release();
    }
    drop(terminal);
    if options.duration.is_some() {
        let seconds = origin.elapsed().as_secs_f64();
        eprintln!(
            "isotop: {frames} frames in {seconds:.2}s | {:.1} presented fps | {:.2} MiB/s inline graphics (display latency not measured)",
            frames as f64 / seconds,
            total_wire as f64 / 1048576.0 / seconds
        );
    }
    Ok(0)
}

/// The demo workload at `time` for `count` processes, with what `row`, the shown view's registry
/// row, asks the demo to synthesize. Every demo snapshot goes through here, so a view sees the
/// same workload whether it is the first sample, a warm-up replay or a live one. `previous` is
/// the time of the sample before: with churn, the snapshot's life events are those in between.
fn demo_snapshot(row: &ViewInfo, previous: f64, time: f64, count: usize) -> Snapshot {
    let mut snapshot = model::demo_with(time, count, row.demo);
    if row.demo.churn {
        snapshot.life = model::demo_events(previous, time, count, row.demo);
        // A view that demands the events is told where they come from, as a live run tells it.
        if row.sources.contains(Source::LifeEvents) {
            snapshot.sources = snapshot.sources.union(model::demo_life_sources(row.demo));
        }
    }
    snapshot
}

/// Settles what a headless `--output` or `--benchmark` frame depends on before it is captured:
/// the shared history, then the shown view's warm-up phases (`ViewInfo::warmup`) whose condition
/// holds. The golden render test runs this too, so the hashes it checks are the PNGs' frames.
fn warm_up(app: &mut App, options: &Options, journal: Option<&Journal>) {
    replay_history(app, options);
    run_phases(app, options, journal);
}

/// Whether a warm-up phase runs in this run.
fn due(warmup: &Warmup, options: &Options) -> bool {
    match warmup.when {
        When::DemoOutput => options.demo && options.output.is_some(),
        When::Output => options.output.is_some(),
        When::Headless => options.headless(),
        When::DemoHeadless => options.demo && options.headless(),
    }
}

/// Fills the scene's shared history, which a live run has by the time a view opens, with the
/// demo samples of the `history::SERIES` whole seconds before `--time`, skipping those before 0.
/// The samples go straight into `scene.history`, not through `Scene::record`, so no view's own
/// record hook sees them and the views that warm up through `Phase::Record` are unchanged. A row
/// whose due `Phase::History` already fills the history while it rasterizes skips the replay.
fn replay_history(app: &mut App, options: &Options) {
    let row = app.row();
    let filled = row
        .warmup
        .iter()
        .any(|warmup| matches!(warmup.phase, Phase::History { .. }) && due(warmup, options));
    if !options.demo || !options.headless() || filled {
        return;
    }
    let count = options.processes as usize;
    for back in (1..=history::SERIES).rev() {
        let at = options.time - back as f64;
        if at >= 0.0 {
            app.scene
                .history
                .record(&demo_snapshot(row, at - 1.0, at, count), at as f32);
        }
    }
}

/// Runs the shown view's warm-up phases whose condition holds.
fn run_phases(app: &mut App, options: &Options, journal: Option<&Journal>) {
    let (width, height, limit) = (
        options.width,
        options.headless_height(),
        options.limit as usize,
    );
    let count = options.processes as usize;
    let row = app.row();
    for warmup in row.warmup {
        if !due(warmup, options) {
            continue;
        }
        match warmup.phase {
            Phase::History { seconds } => {
                for back in (1..=seconds).rev() {
                    let at = options.time - back as f64;
                    app.history = VecDeque::from([demo_snapshot(row, at - 1.0, at, count)]);
                    app.animation = at as f32;
                    let frame = app.render(width, height, limit);
                    app.scene.spare = frame.release();
                }
                app.history = VecDeque::from([demo_snapshot(
                    row,
                    options.time - options.sample_seconds(),
                    options.time,
                    count,
                )]);
            }
            Phase::Record { seconds } => {
                for back in (1..=seconds).rev() {
                    let at = options.time - back as f64;
                    if at >= 0.0 {
                        let snapshot = demo_snapshot(row, at - 1.0, at, count);
                        app.scene.record(&snapshot, at as f32);
                    }
                }
            }
            Phase::Replay {
                seconds,
                before_start,
            } => {
                let step = options.sample_seconds();
                let samples = (f64::from(seconds) / step).round() as u32;
                for back in (1..=samples).rev() {
                    let at = options.time - f64::from(back) * step;
                    if at >= 0.0 || before_start {
                        let snapshot = demo_snapshot(row, at - step, at, count);
                        app.scene.record(&snapshot, at as f32);
                    }
                }
            }
            Phase::Play { seconds, step } => {
                let interval = options.sample_seconds();
                let samples = (f64::from(seconds) / interval).round() as u32;
                let frames = (interval / f64::from(step)).round().max(1.0) as u32;
                for back in (1..=samples).rev() {
                    let at = options.time - f64::from(back) * interval;
                    if at < 0.0 {
                        continue;
                    }
                    app.history = VecDeque::from([demo_snapshot(row, at - interval, at, count)]);
                    for index in 0..frames {
                        app.animation = (at + f64::from(index) * f64::from(step)) as f32;
                        let frame = app.draw(width, height, limit);
                        app.scene.spare = frame.release();
                    }
                }
                app.history = VecDeque::from([demo_snapshot(
                    row,
                    options.time - interval,
                    options.time,
                    count,
                )]);
            }
            Phase::Settle { frames, step, lead } => {
                for index in 0..frames {
                    app.animation = options.time as f32 - lead + index as f32 * step;
                    let frame = app.render(width, height, limit);
                    app.scene.spare = frame.release();
                }
            }
            Phase::Journal { frames, step, lead } => {
                let mut fed = options.time - lead;
                for index in 0..frames {
                    let at = options.time - lead + index as f64 * step;
                    feed(app, journal, options.demo, fed, at);
                    fed = at;
                    app.animation = at as f32;
                    let frame = app.render(width, height, limit);
                    app.scene.spare = frame.release();
                }
            }
        }
    }
}

fn main() {
    match run(Options::parse()) {
        Ok(0) => {}
        Ok(status) => std::process::exit(status),
        Err(error) => {
            eprintln!("isotop: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;
    use crate::model::DemoOptions;
    use crate::simulation::hash::fnv1a_64_bytes as fnv1a;
    use crate::views::view;
    use std::collections::HashMap;

    /// Options as `isotop` parses `arguments`.
    fn options(arguments: &[&str]) -> Options {
        Options::parse_from(std::iter::once("isotop").chain(arguments.iter().copied()))
    }

    /// What `preflight` returns and writes to standard output and error for `arguments`, with
    /// the views named in `missing` unavailable.
    fn preflight_with(arguments: &[&str], missing: &[&str]) -> (Preflight, String, String) {
        let available: Vec<bool> = View::all()
            .map(|view| !missing.contains(&view.name()))
            .collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = preflight(&options(arguments), &available, &mut out, &mut err);
        let text = |bytes: Vec<u8>| String::from_utf8(bytes).expect("the output is text");
        (result, text(out), text(err))
    }

    #[test]
    fn demo_snapshots_follow_the_shown_views_options() {
        let rows = views::testing::DEMO_ROWS;
        let (plain, demanding) = (View::at(0), View::at(1));
        let asked = DemoOptions {
            churn: true,
            exact_cpu_time: true,
            ..DemoOptions::NONE
        };
        let mut app = App::new(plain, model::demo(1.0, 16), vec![true; rows.len()]);
        app.scene = Scene::with_rows(rows);
        assert_eq!(app.row().demo, DemoOptions::default());
        assert_eq!(app.scene.row(demanding).demo, asked);
        let processes = |app: &App, time: f64| {
            let row = app.row();
            demo_snapshot(row, time - 1.0, time, 64).processes
        };
        // The first view asks for nothing, so it gets `demo` exactly.
        assert_eq!(
            format!("{:?}", processes(&app, 40.0)),
            format!("{:?}", model::demo(40.0, 64).processes)
        );
        // Switching to a view that asks for more changes the next sample, not the history.
        let kept = app.history.len();
        app.switch(demanding);
        assert_eq!(app.history.len(), kept, "the history is kept as it was");
        let busy = processes(&app, 40.0);
        assert!(busy.len() > 64, "transients follow the base processes");
        assert_eq!(
            f64::from(busy[5].cpu_time),
            f64::from(model::demo_cpu_seconds(40.0, 5) as f32)
        );
        assert_eq!(
            format!("{:?}", demo_snapshot(app.row(), 39.0, 40.0, 64).processes),
            format!("{:?}", model::demo_with(40.0, 64, asked).processes)
        );
        // And back again.
        app.switch(plain);
        assert_eq!(processes(&app, 40.0).len(), 64);
        // The headless warm-up replays history through the same options.
        app.switch(demanding);
        let options = Options::parse_from([
            "isotop",
            "--demo",
            "--time",
            "12",
            "--processes",
            "64",
            "--width",
            "160",
            "--output",
            "warm.png",
        ]);
        warm_up(&mut app, &options, None);
        assert_eq!(app.history.len(), 1);
        assert_eq!(
            format!("{:?}", app.history[0].processes),
            format!("{:?}", model::demo_with(12.0, 64, asked).processes)
        );
        // With churn a sample carries the life events since the one a `--sample-ms` before it;
        // without, none.
        assert_eq!(
            app.history[0].life,
            model::demo_events(11.0, 12.0, 64, asked)
        );
        let interval = |view| demo_snapshot(app.scene.row(view), 0.0, 120.0, 64).life;
        assert_eq!(
            interval(demanding),
            model::demo_events(0.0, 120.0, 64, asked)
        );
        assert!(!interval(demanding).is_empty());
        assert!(interval(plain).is_empty());
    }

    /// The frame a headless demo run of `name` captures, and the app it came from.
    fn warmed_frame(name: &str, replay: bool) -> (App, Vec<u8>) {
        let options = Options::parse_from([
            "isotop",
            "--demo",
            "--time",
            "75",
            "--processes",
            "64",
            "--width",
            "160",
            "--renderer",
            "cpu",
            "--output",
            "warm.png",
            "--view",
            name,
        ]);
        let view = View::all().find(|view| view.name() == name).unwrap();
        let mut app = App::new(
            view,
            demo_snapshot(view.info(), options.time - 1.0, options.time, 64),
            all_available(),
        );
        if replay {
            warm_up(&mut app, &options, None);
        } else {
            run_phases(&mut app, &options, None);
        }
        app.animation = options.time as f32;
        let frame = app.render(160, options.headless_height(), options.limit as usize);
        (app, frame.pixels.to_vec())
    }

    #[test]
    fn headless_replay_leaves_coop_warm_up_unchanged() {
        let (replayed, with) = warmed_frame("coop", true);
        let (bare, without) = warmed_frame("coop", false);
        assert!(with == without, "the replay changed coop's frame");
        // The replay fills the shared history, one sample a second before the captured one; coop's
        // own `Record` phase reaches it through `Scene::record` for its 30 seconds only.
        assert_eq!(replayed.scene.history.window(75.0, 60.0).count(), 61);
        assert_eq!(bare.scene.history.window(75.0, 60.0).count(), 31);
        // Strata fills it by rasterizing, so it is not replayed on top.
        let (strata, _) = warmed_frame("strata", true);
        assert_eq!(strata.scene.history.window(75.0, 60.0).count(), 61);
    }

    #[test]
    fn registry_names_are_unique_lowercase_and_parse_on_the_command_line() {
        let names: Vec<&str> = View::all().map(View::name).collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        for view in View::all() {
            let name = view.name();
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()),
                "{name}"
            );
            assert_eq!(options(&["--view", name]).view, Some(view));
        }
        let error = Options::try_parse_from(["isotop", "--view", "atlantis"])
            .err()
            .expect("an unknown view is a usage error");
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
        assert!(
            error
                .to_string()
                .contains("[possible values: city, orbit, ripple"),
            "{error}"
        );
    }

    #[test]
    fn view_defaults_to_city() {
        assert_eq!(options(&[]).view, None);
        assert_eq!(preflight_with(&[], &[]).0, Preflight::Start(view("city")));
    }

    #[test]
    fn verbose_names_each_unavailable_view_once() {
        let (result, out, err) = preflight_with(&["--verbose"], &["orbit", "reef"]);
        assert_eq!(result, Preflight::Start(view("city")));
        assert_eq!(out, "");
        assert_eq!(
            err,
            "skipping view orbit: not supported on this platform\n\
             skipping view reef: not supported on this platform\n"
        );
        assert_eq!(
            preflight_with(&[], &["orbit"]).2,
            "",
            "silent without --verbose"
        );
        assert_eq!(preflight_with(&["--verbose"], &[]).2, "");
    }

    #[test]
    fn list_views_prints_only_available_views_in_registry_order() {
        let (result, out, err) = preflight_with(&["--list-views"], &["orbit", "matrix"]);
        assert_eq!(result, Preflight::Exit(0));
        let expected: String = REGISTRY
            .iter()
            .filter(|row| !["orbit", "matrix"].contains(&row.name))
            .map(|row| format!("{}\n", row.name))
            .collect();
        assert_eq!(out, expected);
        assert!(out.starts_with("city\nripple\nflow\n"), "{out}");
        assert_eq!(err, "");
        let (_, every, _) = preflight_with(&["--list-views", "--view", "globe"], &[]);
        assert_eq!(
            every.lines().collect::<Vec<_>>(),
            View::all().map(View::name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_unavailable_view_on_the_command_line_falls_back_to_the_first_available() {
        let (result, out, err) = preflight_with(&["--view", "orbit"], &["city", "orbit"]);
        assert_eq!(result, Preflight::Start(view("ripple")));
        assert_eq!((out.as_str(), err.as_str()), ("", ""));
        assert_eq!(
            preflight_with(&["--view", "coop"], &["city"]).0,
            Preflight::Start(view("coop")),
            "an available view starts as asked"
        );
    }

    #[test]
    fn an_unavailable_view_in_a_headless_run_exits_with_status_one() {
        for headless in [["--output", "orbit.png"], ["--benchmark", "5"]] {
            let arguments = [&["--view", "orbit"][..], &headless].concat();
            let (result, out, err) = preflight_with(&arguments, &["orbit"]);
            assert_eq!(result, Preflight::Exit(1), "{arguments:?}");
            assert_eq!(out, "");
            assert_eq!(
                err,
                "isotop: view orbit is not supported on this platform\n"
            );
        }
        let (result, _, err) = preflight_with(&["--view", "orbit", "--output", "x.png"], &["city"]);
        assert_eq!(
            (result, err.as_str()),
            (Preflight::Start(view("orbit")), "")
        );
    }

    #[test]
    fn a_headless_run_without_view_falls_back_when_the_default_is_unavailable() {
        for headless in [["--output", "x.png"], ["--benchmark", "5"]] {
            let (result, out, err) = preflight_with(&headless, &["city"]);
            assert_eq!(result, Preflight::Start(view("orbit")), "{headless:?}");
            assert_eq!((out.as_str(), err.as_str()), ("", ""));
        }
    }

    #[test]
    fn a_replay_feeds_demo_samples_at_the_sample_interval_and_only_to_demo_runs() {
        let records = |arguments: &[&str]| {
            let rows = views::testing::REPLAYING;
            let mut app = App::new(View::at(0), model::demo(12.0, 16), vec![true; rows.len()]);
            app.scene = Scene::with_rows(rows);
            let mut all = vec!["--time", "12", "--processes", "16"];
            all.extend_from_slice(arguments);
            warm_up(&mut app, &options(&all), None);
            app.scene.legend(app.view, app.snapshot())
        };
        let counted = |count: usize| format!(" probe: 0 draws | {count} records | keys []");
        assert_eq!(records(&["--demo", "--output", "x.png"]), counted(4));
        assert_eq!(records(&["--demo", "--benchmark", "3"]), counted(4));
        assert_eq!(
            records(&["--demo", "--output", "x.png", "--sample-ms", "250"]),
            counted(16)
        );
        assert_eq!(
            records(&["--output", "x.png"]),
            counted(0),
            "a live run keeps demo samples out of its history"
        );
        assert_eq!(
            records(&["--demo"]),
            counted(0),
            "only headless runs warm up"
        );
    }

    #[test]
    fn a_play_draws_frames_between_demo_samples_without_rasterizing_them() {
        let frames = |arguments: &[&str]| {
            let rows = views::testing::PLAYING;
            let mut app = App::new(View::at(0), model::demo(12.0, 16), vec![true; rows.len()]);
            app.scene = Scene::with_rows(rows);
            let mut all = vec!["--time", "12", "--processes", "16"];
            all.extend_from_slice(arguments);
            warm_up(&mut app, &options(&all), None);
            app.scene.legend(app.view, app.snapshot())
        };
        // The first frame is drawn twice, the second time with the camera fitted to it.
        assert_eq!(
            frames(&["--demo", "--output", "x.png"]),
            " player: 8.0@8.0 8.0@8.0 8.5@8.0 9.0@9.0 9.5@9.0 10.0@10.0 10.5@10.0 11.0@11.0 11.5@11.0"
        );
        assert_eq!(
            frames(&["--demo", "--benchmark", "3", "--sample-ms", "500"]),
            " player: 8.0@8.0 8.0@8.0 8.5@8.5 9.0@9.0 9.5@9.5 10.0@10.0 10.5@10.5 11.0@11.0 11.5@11.5"
        );
        assert_eq!(
            frames(&["--output", "x.png"]),
            " player: ",
            "not in a live run"
        );
        let rows = views::testing::PLAYING;
        let mut app = App::new(View::at(0), model::demo(12.0, 16), vec![true; rows.len()]);
        app.scene = Scene::with_rows(rows);
        let frame = app.draw(160, 90, 512);
        assert!(frame.pixels.is_empty(), "a drawn frame is not rasterized");
        assert!(!frame.items.is_empty());
    }

    #[test]
    fn a_replay_from_before_the_start_feeds_the_demo_samples_before_time_zero_too() {
        let rows = views::testing::REMEMBERING;
        let records = |arguments: &[&str]| {
            let mut app = App::new(View::at(0), model::demo(12.0, 16), vec![true; rows.len()]);
            app.scene = Scene::with_rows(rows);
            let mut all = vec!["--time", "12", "--processes", "16"];
            all.extend_from_slice(arguments);
            warm_up(&mut app, &options(&all), None);
            app.scene.legend(app.view, app.snapshot())
        };
        let counted = |count: usize| format!(" probe: 0 draws | {count} records | keys []");
        assert_eq!(records(&["--demo", "--output", "x.png"]), counted(20));
        assert_eq!(records(&["--output", "x.png"]), counted(0));
    }

    /// The checked-in golden hashes, which are those of Linux on x86_64.
    const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden.txt");

    /// FNV-1a 64 of a render's colour, pick and depth buffers.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Hashes {
        pixels: u64,
        picks: u64,
        depth: u64,
    }

    /// The frame `isotop --demo --time 12 --renderer cpu --width 480 --output` writes for `view`.
    fn golden_render(view: View) -> Hashes {
        let name = view.name();
        let options = Options::parse_from([
            "isotop",
            "--demo",
            "--time",
            "12",
            "--renderer",
            "cpu",
            "--width",
            "480",
            "--output",
            "golden.png",
            "--view",
            name,
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
        app.animation = options.time as f32;
        let frame = app.render(
            options.width,
            options.headless_height(),
            options.limit as usize,
        );
        Hashes {
            pixels: fnv1a(frame.pixels.iter().copied()),
            picks: crate::simulation::hash::fnv1a_64_bytes(
                frame.picks.iter().flat_map(|pick| pick.to_le_bytes()),
            ),
            depth: fnv1a(
                frame
                    .depth()
                    .iter()
                    .flat_map(|depth| depth.to_bits().to_le_bytes()),
            ),
        }
    }

    /// Every registered view's golden render, in registry order, each view on its own thread.
    fn render_every_view() -> Vec<(String, Hashes)> {
        std::thread::scope(|scope| {
            let renders: Vec<_> = View::all()
                .map(|view| scope.spawn(move || (view.name().to_owned(), golden_render(view))))
                .collect();
            renders
                .into_iter()
                .map(|render| render.join().expect("the view renders"))
                .collect()
        })
    }

    /// The renders shared by the tests below, so each view is rendered once for both of them.
    fn first_renders() -> &'static [(String, Hashes)] {
        static RENDERS: std::sync::OnceLock<Vec<(String, Hashes)>> = std::sync::OnceLock::new();
        RENDERS.get_or_init(render_every_view)
    }

    fn golden_line(hashes: &Hashes) -> String {
        format!(
            "{:016x} {:016x} {:016x}",
            hashes.pixels, hashes.picks, hashes.depth
        )
    }

    /// Compares `renders` with the golden file at `path`, or rewrites the file when `bless` is set.
    fn check_golden(
        path: &std::path::Path,
        renders: &[(String, Hashes)],
        bless: bool,
    ) -> Result<(), String> {
        if bless {
            let mut text = String::from(
                "# view pixels picks depth: FNV-1a 64 of `--demo --time 12 --renderer cpu --width 480`\n",
            );
            for (name, hashes) in renders {
                text += &format!("{name} {}\n", golden_line(hashes));
            }
            return std::fs::write(path, text)
                .map_err(|error| format!("{}: {error}", path.display()));
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let stored: HashMap<&str, Vec<&str>> = text
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                Some((fields.next()?, fields.collect()))
            })
            .collect();
        let mut changed = Vec::new();
        for (name, hashes) in renders {
            let rendered = golden_line(hashes);
            match stored.get(name.as_str()) {
                None => changed.push(format!("{name}: no golden hash")),
                Some(fields) => {
                    let buffers: Vec<&str> = ["pixels", "picks", "depth"]
                        .into_iter()
                        .zip(rendered.split(' '))
                        .enumerate()
                        .filter(|(index, (_, hash))| fields.get(*index) != Some(hash))
                        .map(|(_, (buffer, _))| buffer)
                        .collect();
                    if !buffers.is_empty() {
                        changed.push(format!("{name}: {} changed", buffers.join(", ")));
                    }
                }
            }
        }
        let mut retired: Vec<&str> = stored
            .keys()
            .copied()
            .filter(|name| !renders.iter().any(|(rendered, _)| rendered == name))
            .collect();
        retired.sort_unstable();
        changed.extend(
            retired
                .iter()
                .map(|name| format!("{name}: no longer a view")),
        );
        if changed.is_empty() {
            return Ok(());
        }
        Err(format!(
            "golden render hashes in {} differ:\n  {}\nIf the change is intended, rewrite the file with \
             `ISOTOP_BLESS=1 cargo test every_view_matches` and say why in the PR description, with the renders.",
            path.display(),
            changed.join("\n  ")
        ))
    }

    /// Renders every view as a headless `--output` run does and compares the FNV-1a 64 hashes of
    /// its colour, pick and depth buffers with `tests/golden.txt`. `ISOTOP_BLESS=1` rewrites it.
    ///
    /// Transcendental functions (`sin`, `exp`, `powf`) are not guaranteed to round the same way
    /// across libm implementations and architectures, so the checked-in hashes are those of Linux
    /// on x86_64, which CI's `ubuntu-latest` runs, and only that platform compares them. Elsewhere
    /// every view still renders, which catches panics, and the comparison is skipped.
    ///
    /// `ISOTOP_GOLDEN_FILE=<path>` reads, and in bless mode writes, that file instead, and compares
    /// on any platform: bless a local baseline before a change and compare against it after.
    #[test]
    fn every_view_matches_its_golden_hash() {
        let renders = first_renders();
        let bless = std::env::var_os("ISOTOP_BLESS").is_some_and(|value| value == "1");
        let (path, compare) = match std::env::var_os("ISOTOP_GOLDEN_FILE") {
            Some(path) => (PathBuf::from(path), true),
            None => (
                PathBuf::from(GOLDEN),
                cfg!(all(target_os = "linux", target_arch = "x86_64")),
            ),
        };
        if !compare {
            assert!(
                !bless,
                "the checked-in hashes are blessed on Linux x86_64 only; \
                 set ISOTOP_GOLDEN_FILE to bless a local baseline"
            );
            return;
        }
        if let Err(message) = check_golden(&path, renders, bless) {
            panic!("{message}");
        }
    }

    #[test]
    fn every_view_renders_the_same_frame_twice() {
        let second = render_every_view();
        for ((name, first), (_, second)) in first_renders().iter().zip(&second) {
            assert_eq!(
                first, second,
                "{name} drew a different frame from the same inputs"
            );
        }
    }

    #[test]
    fn bless_mode_rewrites_the_golden_file() {
        let path = std::env::temp_dir().join(format!("isotop-golden-{}.txt", std::process::id()));
        std::fs::write(&path, "city 0 0 0\nretired 0 0 0\n").expect("the file is written");
        let hashes = |pixels, picks, depth| Hashes {
            pixels,
            picks,
            depth,
        };
        let mut renders = vec![
            ("city".to_owned(), hashes(1, 2, 3)),
            ("orbit".to_owned(), hashes(4, 5, 6)),
        ];
        let stale = check_golden(&path, &renders, false);
        let blessed = check_golden(&path, &renders, true);
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        let fresh = check_golden(&path, &renders, false);
        renders[1].1.picks = 7;
        let changed = check_golden(&path, &renders, false);
        std::fs::remove_file(&path).ok();
        let stale = stale.expect_err("a stale file fails");
        assert!(
            stale.contains("city: pixels, picks, depth changed"),
            "{stale}"
        );
        assert!(stale.contains("orbit: no golden hash"), "{stale}");
        assert!(stale.contains("retired: no longer a view"), "{stale}");
        assert!(stale.contains("ISOTOP_BLESS=1"), "{stale}");
        assert_eq!(blessed, Ok(()));
        assert!(
            written.contains("\norbit 0000000000000004 0000000000000005 0000000000000006\n"),
            "{written}"
        );
        assert!(!written.contains("retired"), "{written}");
        assert_eq!(fresh, Ok(()));
        let changed = changed.expect_err("a changed buffer fails");
        assert!(changed.contains("orbit: picks changed"), "{changed}");
        assert!(!changed.contains("city:"), "{changed}");
    }

    /// Paths relative to `src/` that the first column of AGENTS.md's tables names. Only table rows
    /// count, so a path mentioned in prose does not.
    fn agents_table_paths() -> Vec<String> {
        let guide = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/AGENTS.md"))
            .expect("AGENTS.md is readable");
        guide
            .lines()
            .filter(|line| line.starts_with("| `"))
            .filter_map(|line| line.split('|').nth(1))
            .flat_map(|cell| cell.split('`').skip(1).step_by(2))
            .map(str::to_owned)
            .collect()
    }

    fn source_files(directory: &std::path::Path, prefix: &str, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(directory).expect("src is readable") {
            let entry = entry.expect("directory entries are readable");
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_dir() {
                source_files(&path, &format!("{prefix}{name}/"), found);
            } else if name.ends_with(".rs") {
                found.push(format!("{prefix}{name}"));
            }
        }
    }

    #[test]
    fn views_flag_rejects_unknown_names_and_drops_duplicates() {
        let parse = |arguments: &[&str]| {
            Options::try_parse_from(std::iter::once("isotop").chain(arguments.iter().copied()))
        };
        let error = parse(&["--views", "city,nope"])
            .err()
            .expect("nope is unknown");
        let message = error.to_string();
        assert!(message.contains("nope"), "{message}");
        assert!(
            message.contains("[possible values: city, orbit, ripple"),
            "{message}"
        );
        let named = options(&["--views", "reef,city,reef,coop,city"]);
        let every = all_available();
        let names: Vec<_> = picker::playlist(&named.views, &every)
            .iter()
            .map(|view| view.name())
            .collect();
        assert_eq!(
            names,
            ["reef", "city", "coop"],
            "first occurrences, in order"
        );
        let (result, _, err) = preflight_with(&["--views", "reef,city,reef"], &[]);
        assert_eq!((result, err.as_str()), (Preflight::Start(view("reef")), ""));
        let (result, _, _) = preflight_with(&["--views", "reef,city", "--view", "orbit"], &[]);
        assert_eq!(result, Preflight::Start(view("orbit")), "--view wins");
        let (result, _, err) = preflight_with(&["--views", "orbit,reef", "--verbose"], &["orbit"]);
        assert_eq!(result, Preflight::Start(view("reef")), "orbit is skipped");
        assert!(
            err.contains("skipping view orbit: not supported on this platform\n"),
            "{err}"
        );
        let headless = ["--views", "coop,city", "--output", "x.png"];
        assert_eq!(
            preflight_with(&headless, &[]).0,
            Preflight::Start(view("coop"))
        );
        let mut app = App::new(view("reef"), model::demo(1.0, 16), every);
        app.playlist = picker::playlist(&named.views, &app.available);
        assert_eq!(playlist_names(&app), ["reef", "city", "coop"]);
    }

    #[test]
    fn views_flag_with_no_available_name_exits_with_an_error() {
        let (result, out, err) = preflight_with(&["--views", "orbit,matrix"], &["orbit", "matrix"]);
        assert_eq!(result, Preflight::Exit(1));
        assert_eq!(out, "");
        assert_eq!(err, "isotop: no available view in --views\n");
        let (result, _, err) = preflight_with(
            &["--views", "orbit", "--verbose", "--output", "x.png"],
            &["orbit"],
        );
        assert_eq!(result, Preflight::Exit(1));
        assert_eq!(
            err,
            "skipping view orbit: not supported on this platform\n\
             isotop: no available view in --views\n"
        );
        let (result, _, err) = preflight_with(&["--views", "orbit", "--view", "city"], &["orbit"]);
        assert_eq!(
            (result, err.as_str()),
            (Preflight::Exit(1), "isotop: no available view in --views\n")
        );
    }

    #[test]
    fn agents_file_table_lists_every_source_file() {
        let listed = agents_table_paths();
        let mut found = Vec::new();
        source_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            "",
            &mut found,
        );
        found.sort();
        let missing: Vec<&String> = found.iter().filter(|file| !listed.contains(file)).collect();
        assert!(
            missing.is_empty(),
            "AGENTS.md's file table has no row for: {missing:?}. Add one, naming the file by its \
             path under src/ in the first column"
        );
    }

    #[test]
    fn agents_file_table_names_only_files_that_exist() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let gone: Vec<String> = agents_table_paths()
            .into_iter()
            .filter(|file| !source.join(file).is_file())
            .collect();
        assert!(
            gone.is_empty(),
            "AGENTS.md's file table names files that do not exist under src/: {gone:?}"
        );
    }
}
