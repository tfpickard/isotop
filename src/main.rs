mod app;
mod arbor;
mod blackbody;
mod cells;
mod cli;
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
mod headless;
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

use std::error::Error;
use std::io;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};

use app::{App, Layout, Renderer, feed, typed};
use cli::{Options, Preflight, preflight};
use headless::warm_up;
use journal::Journal;
use model::{Collector, Demand, Snapshot};
use platform::Source;
use terminal::Terminal;
use views::ViewInfo;

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
    app.set_renderer(Renderer::choose(options.renderer, options.gpu_power)?);
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
                app.renderer.kind()
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
        if !app.history.is_paused() {
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
                if app.camera.is_framed() && app.tour.is_none() && app.scene.bounded() {
                    app.camera.refit();
                }
            }
        }
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
            if !app.history.is_paused() {
                fed = f64::max(fed, now - 2.0);
                feed(&mut app, journal.as_ref(), options.demo, fed, now);
                fed = now;
            }
        }
        app.camera.approach(elapsed);
        app.update_tour(tour_after);
        let mut frame = app.render(layout.width, layout.height, limit);
        let render_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        let pixels = terminal.pixel_mouse && layout.cell.is_some();
        app.pointer.hover_over(&frame, layout.pick_radius(pixels));
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
