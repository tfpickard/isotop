//! The view registry. Every view is one row of `REGISTRY`, and the row holds everything that sets
//! the view apart: its names, sky, legend, record and key hooks, headless warm-up, link rule, the
//! sources it reads and the platforms it runs on. `Scene`, `App` and the headless path read the
//! row instead of naming the view, so a new view is one module plus one row.
//!
//! The eleven views that predate the registry are built in: `Scene` keeps their state in typed
//! fields and draws them in its own arms, picked by `BuiltIn`. A new view is a module view: a type
//! that implements `ViewModule`, which `Scene` builds from the row and calls through the trait.

use std::collections::HashMap;
use std::fmt;

use crate::model::{DemoOptions, Identity, Process, Snapshot};
use crate::platform::{self, Source, Sources};
use crate::render::{Backdrop, Camera, Point, Scene, Stage};
use crate::{cells, cores, globe};

/// A view, as its index into the table the scene draws from: `REGISTRY`, outside tests.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct View(usize);

impl View {
    /// The view in row `index` of the table.
    pub fn at(index: usize) -> View {
        View(index)
    }

    /// The registered view whose command-line name is `name`.
    pub fn named(name: &str) -> Option<View> {
        REGISTRY.iter().position(|row| row.name == name).map(View)
    }

    /// Every registered view, in Tab order.
    pub fn all() -> impl Iterator<Item = View> {
        (0..REGISTRY.len()).map(View)
    }

    /// The view's row number: its place in Tab order.
    pub fn index(self) -> usize {
        self.0
    }

    /// The view's row in `REGISTRY`. Code that holds a `Scene` asks `Scene::row` instead, which
    /// reads the table the scene was built from.
    pub fn info(self) -> &'static ViewInfo {
        &REGISTRY[self.0]
    }

    /// The view's command-line name.
    pub fn name(self) -> &'static str {
        self.info().name
    }
}

/// Prints the registered name, so a failed assertion reads `orbit` rather than an index.
impl fmt::Debug for View {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match REGISTRY.get(self.0) {
            Some(row) => formatter.write_str(row.name),
            None => write!(formatter, "View({})", self.0),
        }
    }
}

/// One row of the registry: everything that sets a view apart from the others. `ViewInfo::new`
/// fills every field after `hooks` with its default, which is how the views without that
/// special case behave, so a row sets only what differs.
pub struct ViewInfo {
    /// The lowercase name `--view` and `--list-views` use.
    pub name: &'static str,
    /// The name on the status line, such as `CITY`.
    pub title: &'static str,
    /// The sky behind the view.
    pub backdrop: Backdrop,
    /// Whether the view runs on this platform. `run` asks once, at startup; an unavailable view
    /// is skipped by Tab and the digits, and a headless run of it exits 1. A runtime probe lives
    /// in `platform/`. Default: everywhere.
    pub available: fn() -> bool,
    /// How the scene draws the view and asks for its legend, records and keys.
    pub hooks: Hooks,
    /// What a headless run does before the captured frame, phase by phase in slice order.
    /// Default: nothing.
    pub warmup: &'static [Warmup],
    /// Which socket links the scene draws over the view. Default: `LinkRule::Quiet`.
    pub links: LinkRule,
    /// The generic exit flash: an expanding ring and glow where a process left. Default: on.
    pub flash: bool,
    /// The pressure haze tinting the sky. Default: on.
    pub haze: bool,
    /// The camera pitch set when the view is switched to and on `r`. Default: None, which leaves
    /// the pitch where it is.
    pub pitch: Option<f32>,
    /// Whether `g` and the idle tour visit processes in this view; switching to a view without
    /// the tour ends a running one. Default: on.
    pub tour: bool,
    /// The visible and collapsed counts on the status line. Default: None, which shows
    /// `Scene::visible` and `Scene::collapsed`.
    pub header: Option<Header>,
    /// The camera's rotation and pitch that turn a world point towards the viewer, for a tour or
    /// search target that would otherwise face away. Default: None, which only pans to it.
    pub face: Option<Face>,
    /// A sphere the wheel turns and tilts by hand instead of panning, which then holds still
    /// until `r` or `g` lets it go on turning by itself (`Camera::hold`). Default: None, where
    /// the wheel pans.
    pub turn: Option<Turn>,
    /// Optional sources the view reads, which run only while it needs them: `Locations` turns
    /// GeoIP lookups on while the view is shown, and `Journal` runs the journal follower while it
    /// is shown and for `JOURNAL_LINGER` after. Default: none.
    pub sources: Sources,
    /// What the demo workload synthesizes for the view: births and exits, an exact CPU-time
    /// integral. Every demo snapshot taken while the view is shown follows it. Default: none,
    /// which is `model::demo` exactly.
    pub demo: DemoOptions,
    /// What the shared view harness expects of the view (`harness.rs`). Default: it draws
    /// processes, checked where they are drawn within 1 world unit, and fails nothing.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the view harness reads the contract")
    )]
    pub contract: Contract,
}

/// The visible and collapsed counts a view puts on the status line.
pub type Header = fn(&Scene) -> (usize, usize);

/// The camera rotation and pitch that turn a world point towards the viewer.
pub type Face = fn(Point, &Camera) -> (f32, f32);

/// The sphere a view lets the wheel turn (scroll sideways) and tilt (scroll up and down).
#[derive(Clone, Copy)]
pub struct Turn {
    /// Its radius in world units, which sets how far one wheel step turns it.
    pub radius: f32,
    /// A point drawn with the given camera, as it would be drawn with the sphere standing upright
    /// again, for aiming a tour or search target while the camera is still easing out of a lean.
    pub upright: fn(Point, &Camera) -> Point,
}

impl ViewInfo {
    /// A row with every field after `hooks` at its default.
    pub const fn new(
        name: &'static str,
        title: &'static str,
        backdrop: Backdrop,
        hooks: Hooks,
    ) -> ViewInfo {
        ViewInfo {
            name,
            title,
            backdrop,
            available: everywhere,
            hooks,
            warmup: &[],
            links: LinkRule::Quiet,
            flash: true,
            haze: true,
            pitch: None,
            tour: true,
            header: None,
            face: None,
            turn: None,
            sources: Sources::of(&[]),
            demo: DemoOptions {
                churn: false,
                exact_cpu_time: false,
            },
            contract: Contract::DEFAULT,
        }
    }
}

/// What the shared view harness (`harness.rs`) checks a view against, and the debt it accepts.
/// Every registered view is checked for every `Property`; only tests read this.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the view harness reads the contract")
)]
pub struct Contract {
    /// Whether the view draws processes, so the demo must show some and leave something to
    /// click. Default: true. Matrix draws journal lines instead.
    pub draws_processes: bool,
    /// How far, in world units, a surviving process may move when resources or the population
    /// change. Default: 1.
    pub tolerance: f32,
    /// Where the stability properties look for a process instead of where it is drawn: its
    /// anchor (seat, house), for a view whose creatures wander. Default: None, which checks
    /// `Scene::positions`.
    pub anchors: Option<Anchors>,
    /// Properties the view is known to fail, each with the issue that tracks it. The harness
    /// still checks them, reports one that starts to pass, and does not fail on them.
    pub known_unstable: &'static [Unstable],
}

/// Where each drawn process is anchored, read from the scene after a frame.
pub type Anchors = fn(&Scene) -> HashMap<Identity, Point>;

impl Contract {
    pub const DEFAULT: Contract = Contract {
        draws_processes: true,
        tolerance: 1.0,
        anchors: None,
        known_unstable: &[],
    };

    /// The accepted failure of `property`, if the view has one.
    #[cfg(test)]
    pub fn unstable(&self, property: Property) -> Option<&Unstable> {
        self.known_unstable
            .iter()
            .find(|entry| entry.property == property)
    }
}

/// A property a view is known to fail today, and why.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the view harness reads the contract")
)]
pub struct Unstable {
    pub property: Property,
    /// The issue that tracks the fix.
    pub issue: &'static str,
    pub reason: &'static str,
}

/// One behaviour the shared view harness checks for every view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the view harness checks properties")
)]
pub enum Property {
    /// The same inputs give the same pixels and picks.
    Deterministic,
    /// The demo shows processes and leaves a pixel to click.
    NonEmpty,
    /// Every pick names a process of the frame.
    ValidPicks,
    /// No surviving process moves when CPU and memory change.
    ResourceChange,
    /// No surviving process moves when a tenth of the processes exit and as many start.
    Churn,
    /// An empty machine draws without a panic.
    EmptyMachine,
}

#[cfg(test)]
impl Property {
    pub const ALL: [Property; 6] = [
        Property::Deterministic,
        Property::NonEmpty,
        Property::ValidPicks,
        Property::ResourceChange,
        Property::Churn,
        Property::EmptyMachine,
    ];

    /// The name a harness report uses, as in `flow: churn`.
    pub fn name(self) -> &'static str {
        match self {
            Property::Deterministic => "deterministic",
            Property::NonEmpty => "non-empty",
            Property::ValidPicks => "valid picks",
            Property::ResourceChange => "resource change",
            Property::Churn => "churn",
            Property::EmptyMachine => "empty machine",
        }
    }
}

fn everywhere() -> bool {
    true
}

/// How the scene draws a view and reaches its legend, record and key hooks. Each hook lives in
/// exactly one place: on the row for a built-in view, on the trait for a module view.
pub enum Hooks {
    /// One of the views that predate the registry, drawn by its arm of `Scene::render`.
    BuiltIn {
        arm: BuiltIn,
        /// The third status line while no process is inspected.
        legend: fn(&Scene, &Snapshot) -> String,
        /// Takes in each new sample, whichever view is shown.
        record: Option<fn(&mut Scene, &Snapshot, f32)>,
        /// Replaces the inspector line, the second status line, and hides the inspector popup:
        /// for a view that draws no processes.
        inspector_line: Option<fn(&Scene) -> String>,
    },
    /// A view drawn through `ViewModule`. The scene builds one with this function when it is
    /// created and owns it from then on; nothing outside the module reaches into its state.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no view is a module view until the first new view"
        )
    )]
    Module(fn() -> Box<dyn ViewModule>),
}

/// The arm of `Scene::render` that draws a built-in view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltIn {
    City,
    Orbit,
    Ripple,
    Flow,
    Cores,
    Cells,
    Strata,
    Globe,
    Reef,
    Matrix,
    Coop,
}

/// Which socket links the scene draws over a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkRule {
    /// Every link, as the `c` key sets them.
    All,
    /// Only the links of highlighted processes: selected, hovered or toured.
    Quiet,
    /// None: the view draws its own.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no view draws its own links until the first new view"
        )
    )]
    DrawnByView,
}

/// One phase of a headless warm-up and when it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Warmup {
    pub phase: Phase,
    pub when: When,
}

/// What a warm-up phase does. `time` below is `--time`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// For each whole second from `seconds` back to 1 before `time`, makes the demo sample of
    /// that second the whole history and rasterizes a frame at it; then leaves the history at the
    /// sample of `time`. It replays demo samples, so it belongs under `When::DemoOutput`.
    History { seconds: u32 },
    /// Feeds the demo samples of the `seconds` whole seconds before `time` to `Scene::record`,
    /// skipping those before 0, without rasterizing.
    Record { seconds: u32 },
    /// Rasterizes `frames` frames `step` seconds apart, starting `lead` seconds before `time`.
    Settle { frames: u32, step: f32, lead: f32 },
    /// The same as `Settle`, in `f64` seconds, handing the journal lines that arrived since the
    /// previous frame to the scene before each one.
    Journal { frames: u32, step: f64, lead: f64 },
}

/// Which headless runs a warm-up phase runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// `--demo` with `--output`.
    DemoOutput,
    /// `--output`, live or demo.
    Output,
    /// `--output` or `--benchmark`, live or demo. A new view's warm-up uses this.
    Headless,
}

/// The settling the simulated media and creatures need before a captured frame.
const SETTLE: Warmup = Warmup {
    phase: Phase::Settle {
        frames: 80,
        step: 0.05,
        lead: 4.0,
    },
    when: When::Output,
};

/// The keys a view may claim for itself. Every other key is global.
pub const LOCAL_KEYS: &str = "bijkmopuxyz,.;";

/// A key press offered to the shown view, while no search is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKey {
    /// One of `LOCAL_KEYS`, which have no global meaning.
    Local(char),
    /// Home or `r`. The global reset happens as well, whatever the hook returns.
    Reset,
    /// Esc. When the hook returns true the press is consumed; the view then leaves the next Esc
    /// to its global meaning.
    Escape,
}

/// A view that the scene draws, records and asks for its legend through this trait. Its state
/// lives in the implementing type, which the registry row's `Hooks::Module` builds.
pub trait ViewModule {
    /// Draws the view onto the stage and returns how many of `processes` it showed; the scene
    /// counts the rest as collapsed. Fill `stage.positions` for every drawn process, and
    /// `stage.bounds` when the content moves by itself. Step by `stage.dt`, and tell the app
    /// anything else only through the other `Stage` outputs, such as `tour` and `popup`.
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize;

    /// Takes in each new sample, whichever view is shown, for history the view needs when it
    /// opens.
    fn record(&mut self, _snapshot: &Snapshot, _time: f32) {}

    /// The third status line while no process is inspected. It starts with a space.
    fn legend(&self, snapshot: &Snapshot) -> String;

    /// Handles a key while the view is shown and no search is open; see `ViewKey` for what the
    /// return value means.
    fn key(&mut self, _key: ViewKey) -> bool {
        false
    }

    /// A line that replaces the inspector line, the second status line, and hides the inspector
    /// popup, for a view that draws no processes. It starts with a space.
    fn inspector_line(&self) -> Option<String> {
        None
    }
}

/// Whether each registered view runs on this platform, in registry order.
pub fn availability() -> Vec<bool> {
    REGISTRY.iter().map(|row| (row.available)()).collect()
}

/// Every view, in Tab order. The digits 1 to 9 and 0 pick the first ten rows, so never reorder
/// them; a new view goes at the end.
pub static REGISTRY: &[ViewInfo] = &[
    ViewInfo {
        links: LinkRule::All,
        ..ViewInfo::new(
            "city",
            "CITY",
            Backdrop::Dusk,
            Hooks::BuiltIn {
                arm: BuiltIn::City,
                legend: city_legend,
                record: None,
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        links: LinkRule::All,
        contract: Contract {
            known_unstable: &[
                Unstable {
                    property: Property::ResourceChange,
                    issue: "#114",
                    reason: "orbits and systems are sized by memory, so the galaxy is laid out again",
                },
                Unstable {
                    property: Property::Churn,
                    issue: "#114",
                    reason: "an exit or a birth lays the systems and their shells out again",
                },
            ],
            ..Contract::DEFAULT
        },
        ..ViewInfo::new(
            "orbit",
            "ORBIT",
            Backdrop::Space,
            Hooks::BuiltIn {
                arm: BuiltIn::Orbit,
                legend: orbit_legend,
                record: None,
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        links: LinkRule::All,
        warmup: &[SETTLE],
        contract: Contract {
            known_unstable: &[Unstable {
                property: Property::ResourceChange,
                issue: "#116",
                reason: "pebble clusters are placed again when memory grows their pebbles",
            }],
            ..Contract::DEFAULT
        },
        ..ViewInfo::new(
            "ripple",
            "RIPPLE",
            Backdrop::Space,
            Hooks::BuiltIn {
                arm: BuiltIn::Ripple,
                legend: ripple_legend,
                record: None,
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        warmup: &[SETTLE],
        contract: Contract {
            known_unstable: &[
                Unstable {
                    property: Property::ResourceChange,
                    issue: "#115",
                    reason: "wells are laid out like Orbit's systems, sized by memory",
                },
                Unstable {
                    property: Property::Churn,
                    issue: "#115",
                    reason: "an exit or a birth lays the wells out again, as in Orbit",
                },
            ],
            ..Contract::DEFAULT
        },
        ..ViewInfo::new(
            "flow",
            "FLOW",
            Backdrop::Space,
            Hooks::BuiltIn {
                arm: BuiltIn::Flow,
                legend: flow_legend,
                record: None,
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        warmup: &[SETTLE],
        contract: Contract {
            known_unstable: &[Unstable {
                property: Property::ResourceChange,
                issue: "#117",
                reason: "marbles lap at their CPU and no anchor (the lane) is declared",
            }],
            ..Contract::DEFAULT
        },
        ..ViewInfo::new(
            "cores",
            "CORES",
            Backdrop::Space,
            Hooks::BuiltIn {
                arm: BuiltIn::Cores,
                legend: |_, snapshot| cores::legend(snapshot),
                record: None,
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        contract: Contract {
            known_unstable: &[
                Unstable {
                    property: Property::ResourceChange,
                    issue: "#118",
                    reason: "the largest member is the nucleus at the centre, and memory sizes the cell",
                },
                Unstable {
                    property: Property::Churn,
                    issue: "#118",
                    reason: "when the nucleus exits, the next largest member moves to the centre",
                },
            ],
            ..Contract::DEFAULT
        },
        ..ViewInfo::new(
            "cells",
            "CELLS",
            Backdrop::Dusk,
            Hooks::BuiltIn {
                arm: BuiltIn::Cells,
                legend: |_, snapshot| cells::legend(snapshot),
                record: None,
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        // A minute of history, so the landscape is full in the captured frame.
        warmup: &[Warmup {
            phase: Phase::History { seconds: 60 },
            when: When::DemoOutput,
        }],
        ..ViewInfo::new(
            "strata",
            "STRATA",
            Backdrop::Dusk,
            Hooks::BuiltIn {
                arm: BuiltIn::Strata,
                legend: strata_legend,
                record: Some(|scene, snapshot, time| scene.strata.record(snapshot, time)),
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        face: Some(globe::face),
        turn: Some(Turn {
            radius: globe::RADIUS,
            upright: globe::upright,
        }),
        sources: Sources::of(&[Source::Locations]),
        ..ViewInfo::new(
            "globe",
            "GLOBE",
            Backdrop::Space,
            Hooks::BuiltIn {
                arm: BuiltIn::Globe,
                legend: |_, snapshot| globe::legend(snapshot),
                record: None,
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        warmup: &[SETTLE],
        contract: Contract {
            known_unstable: &[Unstable {
                property: Property::ResourceChange,
                issue: "#119",
                reason: "fish swim and crabs scuttle at their CPU and no anchor is declared",
            }],
            ..Contract::DEFAULT
        },
        ..ViewInfo::new(
            "reef",
            "REEF",
            Backdrop::Sea,
            Hooks::BuiltIn {
                arm: BuiltIn::Reef,
                legend: reef_legend,
                record: None,
                inspector_line: None,
            },
        )
    },
    ViewInfo {
        haze: false,
        sources: Sources::of(&[Source::Journal]),
        // The rain draws journal lines, not processes.
        contract: Contract {
            draws_processes: false,
            ..Contract::DEFAULT
        },
        // The rain needs several seconds to fill the screen.
        warmup: &[Warmup {
            phase: Phase::Journal {
                frames: 161,
                step: 0.05,
                lead: 8.0,
            },
            when: When::Headless,
        }],
        ..ViewInfo::new(
            "matrix",
            "MATRIX",
            Backdrop::Void,
            Hooks::BuiltIn {
                arm: BuiltIn::Matrix,
                legend: matrix_legend,
                record: None,
                inspector_line: Some(matrix_line),
            },
        )
    },
    ViewInfo {
        // The CPU history and fox baselines come from the samples before the frame.
        warmup: &[
            Warmup {
                phase: Phase::Record { seconds: 30 },
                when: When::DemoOutput,
            },
            SETTLE,
        ],
        // Chickens walk; their seats and houses are what must stay put.
        contract: Contract {
            anchors: Some(|scene| scene.coop.anchors()),
            ..Contract::DEFAULT
        },
        ..ViewInfo::new(
            "coop",
            "COOP",
            Backdrop::Dusk,
            Hooks::BuiltIn {
                arm: BuiltIn::Coop,
                legend: |scene, _| scene.coop.legend(),
                record: Some(|scene, snapshot, _| scene.coop.record(snapshot)),
                inspector_line: None,
            },
        )
    },
];

fn city_legend(_: &Scene, _: &Snapshot) -> String {
    format!(
        " Height = CPU | footprint = {} | district = cgroup | amber lights = CPU | cyan pulses = IO",
        platform::MEMORY_LABEL
    )
}

fn orbit_legend(scene: &Scene, snapshot: &Snapshot) -> String {
    format!(
        " Size = memory (stars: whole system) | rings = threads | glow + trail = CPU{} | cyan arcs = sockets, pink = outside{}",
        gpu_note(snapshot),
        hub_note(scene)
    )
}

fn ripple_legend(_: &Scene, _: &Snapshot) -> String {
    " Pebbles = processes, clustered by cgroup | ripples = CPU, each at its own pitch | size = memory | water tint = nearest process | drops = births, splashes = exits | swell = pressure".into()
}

fn flow_legend(scene: &Scene, _: &Snapshot) -> String {
    format!(
        " Wells = memory | whirlpools + coloured particles = CPU | two-lane rivers = sockets | rising sparks = outside | turbulence = pressure{}",
        hub_note(scene)
    )
}

fn strata_legend(_: &Scene, _: &Snapshot) -> String {
    " Ridge = process, height = CPU over the last minute, newest at the front | rows: kernel, system, session, containers".into()
}

fn reef_legend(_: &Scene, _: &Snapshot) -> String {
    " Coral = system services | fish = your session's apps | crabs = containers | plankton = kernel threads | glow = CPU | size = memory | bubbles = I/O".into()
}

fn matrix_legend(scene: &Scene, _: &Snapshot) -> String {
    format!(
        " Journal lines decode as the rain passes | red = error, amber = warning, green = info, teal = debug | {} queued{}",
        scene.matrix.pending(),
        match scene.matrix.skipped {
            0 => String::new(),
            skipped => format!(", {skipped} skipped"),
        }
    )
}

/// The newest journal line, in place of the inspector: the rain draws no processes.
fn matrix_line(scene: &Scene) -> String {
    format!(
        " > {}",
        scene
            .matrix
            .latest()
            .unwrap_or("waiting for the journal...")
    )
}

/// The Orbit legend's GPU entry, left out where per-process GPU memory is not measured.
fn gpu_note(snapshot: &Snapshot) -> &'static str {
    if snapshot.missing.contains(Source::GpuMemory) {
        ""
    } else {
        " | green = NVIDIA GPU"
    }
}

/// What the Orbit and Flow legends add while the frame draws a hub; nothing otherwise, so a
/// focused subtree or a platform that reads every process gets the legend it always had.
fn hub_note(scene: &Scene) -> &'static str {
    if scene.hub_names.is_empty() {
        ""
    } else {
        " | hollow star = a parent isotop can't read (run with sudo)"
    }
}

/// The registered view called `name`, for tests.
#[cfg(test)]
pub fn view(name: &str) -> View {
    View::named(name).unwrap_or_else(|| panic!("no view is called {name}"))
}

/// Module views for tests, and a table of them to build a `Scene` from.
#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::render::TourTarget;

    /// A module view that seats the first half of the processes on a line and lets the rest
    /// collapse. Its legend counts what reached it through the trait. `m` toggles a mode that
    /// the first Esc leaves, as a view-local mode would.
    #[derive(Default)]
    pub struct Probe {
        draws: usize,
        records: usize,
        keys: String,
        mode: bool,
    }

    impl ViewModule for Probe {
        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            _snapshot: &Snapshot,
        ) -> usize {
            self.draws += 1;
            let shown = processes.len().div_ceil(2);
            for (index, process) in processes.iter().take(shown).enumerate() {
                let position = [index as f32 * 4.0, 0.0, 0.0];
                stage.positions.insert(process.id, position);
                stage.frame.sphere(
                    stage.camera,
                    position,
                    1.0,
                    [200, 200, 200],
                    index as u32,
                    false,
                );
            }
            shown
        }

        fn record(&mut self, _snapshot: &Snapshot, _time: f32) {
            self.records += 1;
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            format!(
                " probe: {} draws | {} records | keys [{}]",
                self.draws, self.records, self.keys
            )
        }

        fn key(&mut self, key: ViewKey) -> bool {
            match key {
                ViewKey::Local(character) => {
                    self.keys.push(character);
                    if character == 'm' {
                        self.mode = !self.mode;
                    }
                    false
                }
                ViewKey::Reset => {
                    self.keys.push('R');
                    false
                }
                ViewKey::Escape => {
                    self.keys.push('E');
                    std::mem::take(&mut self.mode)
                }
            }
        }
    }

    /// The same as `Probe`, except that it replaces the inspector line.
    #[derive(Default)]
    pub struct Quiet(Probe);

    impl ViewModule for Quiet {
        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            snapshot: &Snapshot,
        ) -> usize {
            self.0.draw(stage, processes, snapshot)
        }

        fn record(&mut self, snapshot: &Snapshot, time: f32) {
            self.0.record(snapshot, time);
        }

        fn legend(&self, snapshot: &Snapshot) -> String {
            self.0.legend(snapshot)
        }

        fn key(&mut self, key: ViewKey) -> bool {
            self.0.key(key)
        }

        fn inspector_line(&self) -> Option<String> {
            Some(" > quiet".into())
        }
    }

    /// The line `Guide` adds to the popup of the process it tours.
    pub const GUIDE_LINE: &str = "guide: the last seat";

    /// A module view that seats every process on a line, asks the tour to visit the last one at
    /// a point beside its seat and adds a line to its popup. Its legend lists the `dt` of each
    /// frame it drew.
    #[derive(Default)]
    pub struct Guide {
        steps: Vec<f32>,
    }

    impl ViewModule for Guide {
        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            _snapshot: &Snapshot,
        ) -> usize {
            self.steps.push(stage.dt);
            for (index, process) in processes.iter().enumerate() {
                let position = [index as f32 * 4.0, 0.0, 0.0];
                stage.positions.insert(process.id, position);
                stage.frame.sphere(
                    stage.camera,
                    position,
                    1.0,
                    [200, 200, 200],
                    index as u32,
                    false,
                );
            }
            if let Some((index, last)) = processes.iter().enumerate().next_back() {
                stage.tour.push(TourTarget {
                    id: last.id,
                    position: [index as f32 * 4.0, 3.0, 0.0],
                    weight: 1.0,
                });
                stage.popup.push((last.id, GUIDE_LINE.into()));
            }
            processes.len()
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            let steps: Vec<String> = self.steps.iter().map(|dt| format!("{dt:.2}")).collect();
            format!(" guide: dt [{}]", steps.join(" "))
        }
    }

    /// `guide`, then `probe` to switch to and back.
    pub static GUIDED: &[ViewInfo] = &[
        ViewInfo::new(
            "guide",
            "GUIDE",
            Backdrop::Space,
            Hooks::Module(|| Box::new(Guide::default())),
        ),
        ViewInfo::new(
            "probe",
            "PROBE",
            Backdrop::Space,
            Hooks::Module(|| Box::new(Probe::default())),
        ),
    ];

    /// A module view that seats each process by its rank in the frame's process order, so one
    /// exit moves every process after it: the layout the view harness must catch.
    #[derive(Default)]
    pub struct Shuffle;

    impl ViewModule for Shuffle {
        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            _snapshot: &Snapshot,
        ) -> usize {
            for (index, process) in processes.iter().enumerate() {
                let position = [(index % 16) as f32 * 3.0, (index / 16) as f32 * 3.0, 0.0];
                stage.positions.insert(process.id, position);
                stage.frame.sphere(
                    stage.camera,
                    position,
                    1.0,
                    [200, 200, 200],
                    index as u32,
                    false,
                );
            }
            processes.len()
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            " shuffle".into()
        }
    }

    /// `shuffle`, then `listed`, the same view with its churn failure listed as known, and a
    /// resource-change failure it does not have.
    pub static SHUFFLED: &[ViewInfo] = &[
        ViewInfo::new(
            "shuffle",
            "SHUFFLE",
            Backdrop::Space,
            Hooks::Module(|| Box::new(Shuffle)),
        ),
        ViewInfo {
            contract: Contract {
                known_unstable: &[
                    Unstable {
                        property: Property::Churn,
                        issue: "ISSUE_LISTED_CHURN",
                        reason: "seats by rank",
                    },
                    Unstable {
                        property: Property::ResourceChange,
                        issue: "ISSUE_LISTED_RESOURCE_CHANGE",
                        reason: "listed by mistake",
                    },
                ],
                ..Contract::DEFAULT
            },
            ..ViewInfo::new(
                "listed",
                "LISTED",
                Backdrop::Space,
                Hooks::Module(|| Box::new(Shuffle)),
            )
        },
    ];

    /// Two module views: `probe`, with every optional field at its default, then `quiet`, which
    /// sets them otherwise: no links, flash, haze or tour, a pitch of its own and fixed counts.
    pub static ROWS: &[ViewInfo] = &[
        ViewInfo::new(
            "probe",
            "PROBE",
            Backdrop::Space,
            Hooks::Module(|| Box::new(Probe::default())),
        ),
        ViewInfo {
            links: LinkRule::DrawnByView,
            flash: false,
            haze: false,
            pitch: Some(std::f32::consts::FRAC_PI_2),
            tour: false,
            header: Some(|_| (7, 3)),
            ..ViewInfo::new(
                "quiet",
                "QUIET",
                Backdrop::Dusk,
                Hooks::Module(|| Box::new(Quiet::default())),
            )
        },
    ];

    /// Two module views for the demo options: `plain`, which asks for nothing, then `demanding`,
    /// which asks for births, exits and exact CPU time and replays two seconds of history before
    /// a captured frame.
    pub static DEMO_ROWS: &[ViewInfo] = &[
        ViewInfo::new(
            "plain",
            "PLAIN",
            Backdrop::Space,
            Hooks::Module(|| Box::new(Probe::default())),
        ),
        ViewInfo {
            warmup: &[
                Warmup {
                    phase: Phase::History { seconds: 2 },
                    when: When::DemoOutput,
                },
                Warmup {
                    phase: Phase::Record { seconds: 2 },
                    when: When::DemoOutput,
                },
            ],
            demo: DemoOptions {
                churn: true,
                exact_cpu_time: true,
            },
            ..ViewInfo::new(
                "demanding",
                "DEMANDING",
                Backdrop::Space,
                Hooks::Module(|| Box::new(Probe::default())),
            )
        },
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_look_up_their_rows_and_print_as_themselves() {
        let orbit = view("orbit");
        assert_eq!((orbit.index(), orbit.name()), (1, "orbit"));
        assert_eq!(orbit.info().title, "ORBIT");
        assert_eq!(View::at(1), orbit);
        assert_eq!(View::named("Orbit"), None, "names are exact");
        assert_eq!(format!("{orbit:?}"), "orbit");
        assert_eq!(View::all().count(), REGISTRY.len());
        assert_eq!(availability(), vec![true; REGISTRY.len()]);
    }

    #[test]
    fn only_strata_and_coop_record_and_only_matrix_replaces_the_inspector() {
        let with = |test: fn(&Hooks) -> bool| -> Vec<&str> {
            REGISTRY
                .iter()
                .filter(|row| test(&row.hooks))
                .map(|row| row.name)
                .collect()
        };
        assert_eq!(
            with(|hooks| matches!(
                hooks,
                Hooks::BuiltIn {
                    record: Some(_),
                    ..
                }
            )),
            ["strata", "coop"],
            "Strata records before Coop, as it always has"
        );
        assert_eq!(
            with(|hooks| matches!(
                hooks,
                Hooks::BuiltIn {
                    inspector_line: Some(_),
                    ..
                }
            )),
            ["matrix"]
        );
        assert!(with(|hooks| matches!(hooks, Hooks::Module(_))).is_empty());
    }
}
