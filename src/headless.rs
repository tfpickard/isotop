//! Headless runs: the warm-up that settles a frame before it is captured.

use crate::app::App;
use crate::cli::Options;
use crate::demo_snapshot;
use crate::history;
use crate::journal::Journal;
use crate::views::{Phase, Warmup, When};

/// Settles what a headless `--output` or `--benchmark` frame depends on before it is captured:
/// the shared history, then the shown view's warm-up phases (`ViewInfo::warmup`) whose condition
/// holds. The golden render test runs this too, so the hashes it checks are the PNGs' frames.
pub(crate) fn warm_up(app: &mut App, options: &Options, journal: Option<&Journal>) {
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
                    app.history.replace(demo_snapshot(row, at - 1.0, at, count));
                    app.animation = at as f32;
                    let frame = app.render(width, height, limit);
                    app.scene.spare = frame.release();
                }
                app.history.replace(demo_snapshot(
                    row,
                    options.time - options.sample_seconds(),
                    options.time,
                    count,
                ));
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
                    app.history
                        .replace(demo_snapshot(row, at - interval, at, count));
                    for index in 0..frames {
                        app.animation = (at + f64::from(index) * f64::from(step)) as f32;
                        let frame = app.draw(width, height, limit);
                        app.scene.spare = frame.release();
                    }
                }
                app.history.replace(demo_snapshot(
                    row,
                    options.time - interval,
                    options.time,
                    count,
                ));
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
                    app.feed_journal(journal, options.demo, fed, at);
                    fed = at;
                    app.animation = at as f32;
                    let frame = app.render(width, height, limit);
                    app.scene.spare = frame.release();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::app::fixtures::*;
    use crate::model::DemoOptions;
    use crate::simulation::hash::fnv1a_64_bytes as fnv1a;

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
            format!("{:?}", app.history.samples()[0].processes),
            format!("{:?}", model::demo_with(12.0, 64, asked).processes)
        );
        // With churn a sample carries the life events since the one a `--sample-ms` before it;
        // without, none.
        assert_eq!(
            app.history.samples()[0].life,
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
}
