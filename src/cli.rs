//! The command line: `Options`, the choices it offers, and `preflight`, which decides what a run
//! does before the terminal is touched.

use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use clap::builder::{PossibleValuesParser, TypedValueParser};

use crate::picker;
use crate::views::{REGISTRY, View};

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum RendererChoice {
    Auto,
    Gpu,
    Cpu,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Power {
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
pub(crate) struct Options {
    /// Use a deterministic synthetic workload instead of /proc
    #[arg(long)]
    pub(crate) demo: bool,
    /// Initial visualization (city, or the first of --views)
    #[arg(long, value_parser = view_parser())]
    pub(crate) view: Option<View>,
    /// The views Tab and Shift+Tab cycle, as a comma-separated list such as reef,city: a name
    /// repeated or not supported on this platform is dropped. Without it, every supported view
    #[arg(long, value_delimiter = ',', value_parser = view_parser())]
    pub(crate) views: Vec<View>,
    /// Print the views this platform supports, one per line, and exit
    #[arg(long)]
    pub(crate) list_views: bool,
    /// Print startup notes to stderr, such as each view this platform does not support
    #[arg(long)]
    pub(crate) verbose: bool,
    /// Number of synthetic processes
    #[arg(long, default_value_t = 192, value_parser = clap::value_parser!(u32).range(1..=10000))]
    pub(crate) processes: u32,
    /// Maximum processes admitted into a scene
    #[arg(long, default_value_t = 1024, value_parser = clap::value_parser!(u32).range(1..=10000))]
    pub(crate) limit: u32,
    /// Maximum internal rendering width (height follows terminal aspect ratio)
    #[arg(long, default_value_t = 1920, value_parser = clap::value_parser!(u32).range(160..=2560))]
    pub(crate) width: u32,
    /// Target animation frame rate
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=60))]
    pub(crate) fps: u32,
    /// Process sampling interval in milliseconds
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(100..=10000))]
    pub(crate) sample_ms: u32,
    /// Number of snapshots retained for pause/rewind
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(2..=3600))]
    pub(crate) history: u32,
    /// Seconds without input before the guided tour starts (0 disables it)
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(0..=3600))]
    pub(crate) tour: u32,
    /// Render a single PNG and exit (no terminal required)
    #[arg(long)]
    pub(crate) output: Option<PathBuf>,
    /// Synthetic time for PNG output and benchmark
    #[arg(long, default_value_t = 12.0)]
    pub(crate) time: f64,
    /// Render this many frames headlessly and report renderer performance
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10000))]
    pub(crate) benchmark: Option<u32>,
    /// Bypass the terminal graphics capability query
    #[arg(long)]
    pub(crate) force_graphics: bool,
    /// Send frames inline (zlib + base64) even when the terminal can read shared memory
    #[arg(long)]
    pub(crate) direct: bool,
    /// Rasterizer: the GPU when one is available, falling back to the CPU
    #[arg(long, value_enum, default_value_t = RendererChoice::Auto)]
    pub(crate) renderer: RendererChoice,
    /// Which GPU to prefer on machines with more than one
    #[arg(long, value_enum, default_value_t = Power::High)]
    pub(crate) gpu_power: Power,
    /// Exit after this many seconds and report end-to-end presentation throughput
    #[arg(long)]
    pub(crate) duration: Option<f64>,
    /// City-level GeoIP database (MaxMind format, such as DB-IP Lite or GeoLite2 City) for the
    /// globe; found in ~/.local/share/isotop and /usr/share/GeoIP when not given
    #[arg(long)]
    pub(crate) geoip: Option<PathBuf>,
    /// This machine's location as LAT,LON for the globe; defaults to the system time zone's city
    #[arg(long, value_parser = parse_home, allow_hyphen_values = true)]
    pub(crate) home: Option<(f32, f32)>,
}

impl Options {
    /// Height of a headless frame: 16:9 at the requested width.
    pub(crate) fn headless_height(&self) -> u32 {
        self.width * 9 / 16
    }

    /// Whether the run draws without a terminal: `--output` or `--benchmark`.
    pub(crate) fn headless(&self) -> bool {
        self.output.is_some() || self.benchmark.is_some()
    }

    /// `--sample-ms` in seconds: how far back the sample before the first one is.
    pub(crate) fn sample_seconds(&self) -> f64 {
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
pub(crate) enum Preflight {
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
pub(crate) fn preflight(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;

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
}
