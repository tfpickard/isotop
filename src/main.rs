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
mod interactive;
mod journal;
mod keys;
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
use std::time::Duration;

use app::{App, Renderer};
use cli::{Options, Preflight, preflight};
#[cfg(test)]
use headless::warm_up;
use journal::Journal;
use model::{Collector, Demand, Snapshot};
use platform::Source;
use views::{View, ViewInfo};

/// Everything a run is set up with before it splits into the headless and the interactive one.
struct Startup {
    app: App,
    collector: Collector,
    journal: Option<Journal>,
}

/// Runs isotop and returns the exit status for `main`.
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
    let Startup {
        mut app,
        collector,
        journal,
    } = start(&options, view, available)?;
    if options.headless() {
        headless::run(&mut app, &options, journal.as_ref())?;
    } else {
        interactive::run(app, collector, journal, &options)?;
    }
    Ok(0)
}

/// The first sample and the app that shows it, on `view`: the demo's or the collector's, the
/// latter taken twice, a `--sample-ms` apart, in a headless run so that rates are known.
fn start(options: &Options, view: View, available: Vec<bool>) -> Result<Startup, Box<dyn Error>> {
    let demand = Demand::from(view.info().sources);
    let mut collector = Collector::new(options.geoip.clone(), options.home);
    collector.demand(demand);
    let journal = (demand.contains(Source::Journal) && !options.demo).then(Journal::start);
    let mut snapshot = if options.demo {
        // The demo's clock starts at --time in a headless run and at 0 in a live one.
        let time = if options.headless() {
            options.time
        } else {
            0.0
        };
        demo_snapshot(
            view.info(),
            time - options.sample_seconds(),
            time,
            options.processes as usize,
        )
    } else {
        collector.sample()?
    };
    if !options.demo && options.headless() {
        std::thread::sleep(Duration::from_millis(u64::from(options.sample_ms)));
        snapshot = collector.sample()?;
    }
    let playlist = picker::playlist(&options.views, &available);
    let mut app = App::new(view, snapshot, available);
    app.playlist = playlist;
    app.set_renderer(Renderer::choose(options.renderer, options.gpu_power)?);
    Ok(Startup {
        app,
        collector,
        journal,
    })
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
    match run(cli::parse()) {
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
