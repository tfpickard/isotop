//! The view registry. Every view is one row of `REGISTRY`, and the row holds everything that sets
//! the view apart: its names, sky, the constructor of its module, headless warm-up, link rule,
//! the sources it reads and the platforms it runs on. `Scene`, `App` and the headless path read
//! the row instead of naming the view, so a new view is one module plus one row.
//!
//! There is one way to write a view: a type that implements `ViewModule`. The row's `build`
//! makes one when the `Scene` is created, the scene keeps it in its row's slot and calls it
//! through the trait (`draw`, `record`, `legend`, `key`, `inspector_line`, `card`), and nothing
//! outside the module reaches into its state.

mod city;
mod flow;
mod orbit;
mod ripple;
pub(crate) mod systems;

#[cfg(test)]
use std::collections::HashMap;
use std::fmt;

#[cfg(test)]
use crate::model::Identity;
use crate::model::{DemoOptions, Process, Snapshot};
use crate::platform::{Source, Sources};
use crate::render::{Backdrop, Camera, Card, ISOMETRIC, LOWEST_PITCH, Point, Scene, Stage, Thing};
use crate::{
    arbor, cells, coop, cores, cortex, globe, hive, horizon, hr, matrix, murmuration, necropolis,
    ptolemy, reef, strata, switchboard, tesseract,
};

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
/// fills every field after `build` with its default, which is how the views without that
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
    /// Makes the view's module, which draws the view and answers for its legend, records and
    /// keys. The scene calls it once per row when it is created and owns the module from then on.
    pub build: Build,
    /// What a headless run does before the captured frame, phase by phase in slice order.
    /// Default: nothing.
    pub warmup: &'static [Warmup],
    /// Which socket links the scene draws over the view. Default: `LinkRule::Quiet`.
    pub links: LinkRule,
    /// Whether the scene draws those links as hairlines (`Frame::hairline`, thin triangles), which the CPU
    /// and the GPU rasterize alike, rather than as beams, whose one-pixel lines they do not.
    /// Default: off, the beams the eleven original views were blessed with; a new view turns it on.
    pub link_strokes: bool,
    /// How high, in world units, the scene lifts the ends of its links above the points the view
    /// put in `Stage::positions`, so they clear the rooftops of a view that draws buildings.
    /// Default: 0, links on the ground.
    pub link_lift: f32,
    /// The generic exit flash: an expanding ring and glow where a process left. Default: on.
    pub flash: bool,
    /// The pressure haze tinting the sky. Default: on.
    pub haze: bool,
    /// The camera pitch set when the view is switched to and on `r`. Default: None, which leaves
    /// the pitch where it is.
    pub pitch: Option<f32>,
    /// Whether the view draws a plane that faces the viewer whatever the camera's rotation and
    /// pitch (a sky dome, built each frame from `Camera::ground`). Its `Stage::bounds` then hold
    /// only for the camera that drew them, so the camera fit measures them under that camera's
    /// rotation and pitch instead of the goal's, which a view switched to has not yet been
    /// drawn under. Default: off.
    pub billboard: bool,
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
    /// The view's own keys, for the `?` help line while it is shown, such as `b boost`; they
    /// reach the view as `ViewKey::Local`. Default: none.
    pub keys: &'static str,
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
    /// A row with every field after `build` at its default.
    pub const fn new(
        name: &'static str,
        title: &'static str,
        backdrop: Backdrop,
        build: Build,
    ) -> ViewInfo {
        ViewInfo {
            name,
            title,
            backdrop,
            available: everywhere,
            build,
            warmup: &[],
            links: LinkRule::Quiet,
            link_strokes: false,
            link_lift: 0.0,
            flash: true,
            haze: true,
            pitch: None,
            billboard: false,
            tour: true,
            header: None,
            face: None,
            turn: None,
            sources: Sources::of(&[]),
            demo: DemoOptions::NONE,
            keys: "",
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
    /// The demo time, in seconds, the harness checks the view at: the default reference time, or
    /// a later one for a view whose history window is not full by then. Default: 12.
    pub time: f64,
    /// Properties the view is known to fail, each with the issue that tracks it. The harness
    /// still checks them, reports one that starts to pass, and does not fail on them.
    pub known_unstable: &'static [Unstable],
}

impl Contract {
    pub const DEFAULT: Contract = Contract {
        draws_processes: true,
        tolerance: 1.0,
        time: 12.0,
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

/// The constructor a row holds: it makes the module that draws the view.
pub type Build = fn() -> Box<dyn ViewModule>;

/// Which socket links the scene draws over a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkRule {
    /// Every link, as the `c` key sets them.
    All,
    /// Only the links of highlighted processes: selected, hovered or toured.
    Quiet,
    /// None: the view draws its own.
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
    /// sample of `time`. It replays demo samples, so it belongs under `When::DemoOutput`. Every
    /// frame it rasterizes records its sample in the scene's history, so a row with a due
    /// `History` phase skips `main.rs`'s replay of the shared history.
    History { seconds: u32 },
    /// Feeds the demo samples of the `seconds` whole seconds before `time` to `Scene::record`,
    /// skipping those before 0, without rasterizing.
    Record { seconds: u32 },
    /// The same as `Record`, with the samples `--sample-ms` apart, as a live run takes them.
    /// With `before_start`, the samples before time 0 are replayed too (the demo workload is
    /// defined there as well), so a view whose record hook keeps minutes is settled at the
    /// reference time of 12 s.
    Replay { seconds: u32, before_start: bool },
    /// Plays the `seconds` before `time` for a view that builds its state inside `draw` from the
    /// samples as they arrived: at each `--sample-ms` the demo sample of that moment becomes the
    /// whole history, and the frames `step` seconds apart up to the next sample are drawn, not
    /// rasterized. The played samples are not recorded (`ViewModule::record` and the shared
    /// history see only what `warm_up`'s replay gave them), so a view that needs a record hook
    /// to settle uses `Replay`. It plays demo samples, so it belongs under `When::DemoHeadless`;
    /// samples before time 0 are skipped.
    Play { seconds: u32, step: f32 },
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
    /// `--demo` with `--output` or `--benchmark`: for a phase that replays demo samples, which
    /// a live run must not mix into its own.
    DemoHeadless,
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

/// Command-line settings that only some views read, handed to every module once, before its first
/// frame, through `ViewModule::configure`. A view takes the fields it knows and ignores the rest;
/// a new view's flag adds a field here and one line in `Options::view_options`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewOptions {
    /// `--hr-diagram`: the hr view starts in diagram mode instead of the sky.
    pub hr_diagram: bool,
}

/// A view that the scene draws, records and asks for its legend through this trait. Its state
/// lives in the implementing type, which the registry row's `build` makes.
pub trait ViewModule {
    /// Draws the view onto the stage and returns how many of `processes` it showed; the scene
    /// counts the rest as collapsed. Fill `stage.positions` for every drawn process, and
    /// `stage.bounds` when the content moves by itself. Step by `stage.dt`, and tell the app
    /// anything else only through the other `Stage` outputs, such as `tour` and `popup`.
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize;

    /// Takes in the command-line settings once, before the first frame, whichever view is shown.
    /// A view that has none of its own leaves it.
    fn configure(&mut self, _options: &ViewOptions) {}

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

    /// The inspector card of `thing`, one of the things the view drew with `Frame::thing_pick`,
    /// or None once it no longer shows it. Called only for the selected or hovered thing, so
    /// build the card here from the view's own state, never one per thing per frame. The
    /// snapshot is the shown sample, for figures the view does not keep itself.
    fn card(&self, _thing: Thing, _snapshot: &Snapshot) -> Option<Card> {
        None
    }

    /// The view's state as `Any`, for `Scene::module`: a module's own tests read what its legend
    /// and `Stage` outputs do not tell. None, the default, shows nothing.
    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Where the view harness looks for each drawn process instead of where it is drawn, for a
    /// view whose bodies move by themselves: the place each one keeps, read after a frame. None,
    /// the default, checks `Scene::positions`.
    #[cfg(test)]
    fn anchors(&self) -> Option<HashMap<Identity, Point>> {
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
        // Links rise above the rooftops.
        link_lift: 1.5,
        ..ViewInfo::new("city", "CITY", Backdrop::Dusk, || {
            Box::new(city::City::default())
        })
    },
    ViewInfo {
        links: LinkRule::All,
        ..ViewInfo::new("orbit", "ORBIT", Backdrop::Space, || {
            Box::new(orbit::Orbits::default())
        })
    },
    ViewInfo {
        links: LinkRule::All,
        warmup: &[SETTLE],
        ..ViewInfo::new("ripple", "RIPPLE", Backdrop::Space, || {
            Box::new(ripple::Ripple::default())
        })
    },
    ViewInfo {
        warmup: &[SETTLE],
        ..ViewInfo::new("flow", "FLOW", Backdrop::Space, || {
            Box::new(flow::Wells::default())
        })
    },
    ViewInfo {
        warmup: &[SETTLE],
        // Marbles lap at their CPU; the lane is what must stay put, and `Track::anchors` says
        // which one each rides.
        ..ViewInfo::new("cores", "CORES", Backdrop::Space, || {
            Box::new(cores::Track::default())
        })
    },
    ViewInfo::new("cells", "CELLS", Backdrop::Dusk, || {
        Box::new(cells::Dishes::default())
    }),
    ViewInfo {
        // A minute of history, so the landscape is full in the captured frame.
        warmup: &[Warmup {
            phase: Phase::History { seconds: 60 },
            when: When::DemoOutput,
        }],
        ..ViewInfo::new("strata", "STRATA", Backdrop::Dusk, || {
            Box::new(strata::Strata::default())
        })
    },
    ViewInfo {
        face: Some(globe::face),
        turn: Some(Turn {
            radius: globe::RADIUS,
            upright: globe::upright,
        }),
        sources: Sources::of(&[Source::Locations]),
        ..ViewInfo::new("globe", "GLOBE", Backdrop::Space, || {
            Box::new(globe::Globe::default())
        })
    },
    ViewInfo {
        warmup: &[SETTLE],
        // Fish swim and crabs scuttle at their CPU; their school's target and their seat are
        // what must stay put, and `Reef::anchors` says where those are.
        ..ViewInfo::new("reef", "REEF", Backdrop::Sea, || {
            Box::new(reef::Reef::default())
        })
    },
    ViewInfo {
        haze: false,
        sources: Sources::of(&[Source::Journal]),
        // The rain draws no processes and collapses none, whatever the machine runs.
        header: Some(|_| (0, 0)),
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
        ..ViewInfo::new("matrix", "MATRIX", Backdrop::Void, || {
            Box::new(matrix::Rain::default())
        })
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
        // Chickens walk; their seats and houses are what must stay put, and `Coop::anchors` says
        // where those are.
        ..ViewInfo::new("coop", "COOP", Backdrop::Dusk, || {
            Box::new(coop::Coop::default())
        })
    },
    ViewInfo {
        // Stars flash their own novae and fades.
        flash: false,
        // Trail strokes are opaque, blended against the plain space sky.
        haze: false,
        demo: DemoOptions {
            churn: true,
            observer: true,
            ..DemoOptions::NONE
        },
        // The window of 60 samples is full only after a minute.
        contract: Contract {
            time: 300.0,
            ..Contract::DEFAULT
        },
        ..ViewInfo::new("ptolemy", "PTOLEMY", Backdrop::Space, || {
            Box::new(ptolemy::Ptolemy::default())
        })
    },
    ViewInfo {
        links: LinkRule::All,
        link_strokes: true,
        keys: ", . quarter turn | ; plane | b boost",
        // A minute of worldlines, replayed as a live run would have recorded it.
        warmup: &[Warmup {
            phase: Phase::Replay {
                seconds: 60,
                before_start: false,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            churn: true,
            exact_cpu_time: true,
            ..DemoOptions::NONE
        },
        ..ViewInfo::new("tesseract", "TESSERACT", Backdrop::Dusk, || {
            Box::new(tesseract::Tesseract::default())
        })
    },
    ViewInfo {
        // Graves are drawn by the view; the zombies have no socket links worth drawing.
        links: LinkRule::DrawnByView,
        // A reaped zombie sinks into its pit instead of flashing; fog stands for memory pressure.
        flash: false,
        haze: false,
        // The tour visits processes, and the living are not drawn.
        tour: false,
        sources: Sources::of(&[Source::LifeEvents]),
        // Ten minutes of deaths, so the reference frame at 12 s shows ten rows.
        warmup: &[Warmup {
            phase: Phase::Replay {
                seconds: 600,
                before_start: true,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            churn: true,
            mortality: true,
            ..DemoOptions::NONE
        },
        ..ViewInfo::new("necropolis", "NECROPOLIS", Backdrop::Dusk, || {
            Box::new(necropolis::Necropolis::default())
        })
    },
    ViewInfo {
        // Axons are the view's own: they follow the `c` key and carry the spikes.
        links: LinkRule::DrawnByView,
        link_strokes: true,
        // The slice is a drawing, not a scene to haze.
        haze: false,
        // The slice faces the default camera.
        pitch: Some(ISOMETRIC),
        face: Some(|_, _| (0.0, ISOMETRIC)),
        sources: Sources::of(&[Source::Pipes]),
        // Twelve seconds of the demo as a live run drew it, so the raster is full at 12 s and the
        // simulation has settled before the ten seconds it shows begin.
        warmup: &[Warmup {
            phase: Phase::Play {
                seconds: 12,
                step: 0.1,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            channels: true,
            ..DemoOptions::NONE
        },
        ..ViewInfo::new("cortex", "CORTEX", Backdrop::Void, || {
            Box::new(cortex::Cortex::default())
        })
    },
    ViewInfo {
        // No sockets: the lensed images are not where processes talk. A process that vanishes
        // fades where it stood or plunges into the hole, by the view's own `Orbits`, so the
        // scene's expanding ring would say it twice (this is `own_exits` of the issue).
        links: LinkRule::DrawnByView,
        flash: false,
        // Scores and the kill counter are read only while the view is shown.
        sources: Sources::of(&[Source::Oom, Source::OomKills]),
        // The view takes in each sample to see who vanished; replayed as a live run takes them.
        warmup: &[Warmup {
            phase: Phase::Replay {
                seconds: 8,
                before_start: false,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            oom_and_memory_detail: true,
            kernel_counters: true,
            oom_cycle: true,
            ..DemoOptions::NONE
        },
        ..ViewInfo::new("horizon", "HORIZON", Backdrop::Void, || {
            Box::new(horizon::Horizon::default())
        })
    },
    ViewInfo {
        // Sockets join processes, and the flocks fly clear of one another.
        links: LinkRule::DrawnByView,
        // Threads that exit fall to the roost instead.
        flash: false,
        // The camera looks across the sky at the flock.
        pitch: Some(LOWEST_PITCH),
        // A flock flies high above the ground, so a tour or a search centres the flock itself
        // rather than the ground below it; the camera turns and tilts as it is.
        face: Some(|_, goal| (goal.rotation, goal.pitch)),
        sources: Sources::of(&[Source::Threads, Source::OomKills]),
        // Fifteen seconds of flight, a step at a time, as a live run would have drawn them: the
        // flock settles from its clumps, and the demo's samples set off its falcon.
        warmup: &[Warmup {
            phase: Phase::Play {
                seconds: 15,
                step: 0.05,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            threads: true,
            kernel_counters: true,
            ..DemoOptions::NONE
        },
        ..ViewInfo::new("murmuration", "MURMURATION", Backdrop::Twilight, || {
            Box::new(murmuration::Murmuration::default())
        })
    },
    ViewInfo {
        // Sockets join the tips of the processes that hold them, as hairlines.
        link_strokes: true,
        // A thread that exits drops its leaf, and a process that exits all of its leaves.
        flash: false,
        // The tree stands at its own point of the camera; a tour or a search centres the point
        // itself, since a twig is far above the ground under it.
        face: Some(|_, goal| (goal.rotation, goal.pitch)),
        // The camera looks across the canopy more than down on it.
        pitch: Some(0.48),
        sources: Sources::of(&[Source::Threads]),
        // Eight seconds of wind, blossoms and falling leaves, a frame at a time, as a live run
        // would have drawn them.
        warmup: &[Warmup {
            phase: Phase::Play {
                seconds: 8,
                step: 0.05,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            churn: true,
            threads: true,
            thread_churn: true,
            unreadable_parent: true,
            ..DemoOptions::NONE
        },
        ..ViewInfo::new("arbor", "ARBOR", Backdrop::Dusk, || {
            Box::new(arbor::Arbor::default())
        })
    },
    ViewInfo {
        // Stick figures are the view's own, along the socket links of a constellation.
        links: LinkRule::DrawnByView,
        link_strokes: true,
        // A birth is a nova and an exit a fade or a supernova, drawn by the view; the dome is the
        // sky, and pressure haze would tint it.
        flash: false,
        haze: false,
        // The dome faces the viewer whatever the rotation and pitch, and is framed as it was
        // drawn; top-down keeps its plane flat to the viewer from the start.
        billboard: true,
        pitch: Some(std::f32::consts::FRAC_PI_2),
        keys: "m diagram or sky",
        // Twelve seconds of samples as a live run takes them: the twinkle reads ten seconds of
        // CPU, and the births and the first supernova of the churn are in the history.
        warmup: &[Warmup {
            phase: Phase::Replay {
                seconds: 12,
                before_start: false,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            churn: true,
            early_giant: true,
            ..DemoOptions::NONE
        },
        ..ViewInfo::new("hr", "HR", Backdrop::Void, || Box::new(hr::Hr::default()))
    },
    ViewInfo {
        // Sockets join the frames of the processes that hold them, as hairlines.
        link_strokes: true,
        // A frame stands high above the ground under it, so a tour or a search centres the frame
        // itself; the camera turns and tilts as it is.
        face: Some(|_, goal| (goal.rotation, goal.pitch)),
        // The pale-nectar frames of processes the rollup has not reached, and the cells and bees
        // of the rest, come from the two sources only this view draws.
        sources: Sources::of(&[Source::MemoryDetail, Source::Threads]),
        // Ten seconds of bees, a frame at a time, as a live run would have drawn them.
        warmup: &[Warmup {
            phase: Phase::Play {
                seconds: 10,
                step: 0.05,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            oom_and_memory_detail: true,
            threads: true,
            ..DemoOptions::NONE
        },
        ..ViewInfo::new("hive", "HIVE", Backdrop::Dusk, || {
            Box::new(hive::Hive::default())
        })
    },
    ViewInfo {
        // Interrupts are a Linux table; macOS has no unprivileged per-IRQ or per-CPU counters.
        available: switchboard::available,
        // No process is drawn, so there are no sockets to link.
        links: LinkRule::DrawnByView,
        // The board faces the viewer; a slight shear is built into its layout.
        pitch: Some(0.45),
        // The view draws no processes, so there is nothing for the tour to visit.
        tour: false,
        // Only the interrupt table is read, and only while the view is shown.
        sources: Sources::of(&[Source::Interrupts, Source::Softirqs, Source::IrqAffinity]),
        // The header is the view's own: lines, rate, RES and operators.
        header: Some(|_| (0, 0)),
        haze: false,
        flash: false,
        // Three seconds of the demo as a live run drew them, so the frame at 12 s finds the
        // cord that irqbalance moved at 10.2 s a second into its swing.
        warmup: &[Warmup {
            phase: Phase::Play {
                seconds: 3,
                step: 0.1,
            },
            when: When::DemoHeadless,
        }],
        demo: DemoOptions {
            exchange: true,
            ..DemoOptions::NONE
        },
        // The board draws interrupt lines, not processes.
        contract: Contract {
            draws_processes: false,
            ..Contract::DEFAULT
        },
        ..ViewInfo::new("switchboard", "SWITCHBOARD", Backdrop::Dusk, || {
            Box::new(switchboard::Switchboard::default())
        })
    },
];

/// The registered view called `name`, for tests.
#[cfg(test)]
pub fn view(name: &str) -> View {
    View::named(name).unwrap_or_else(|| panic!("no view is called {name}"))
}

/// Module views for tests, and a table of them to build a `Scene` from.
#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::render::{Side, Tag, Title, TourTarget};

    /// A module view that seats the first half of the processes on a line and lets the rest
    /// collapse. Its legend counts what reached it through the trait. `m` toggles a mode that
    /// the first Esc leaves, as a view-local mode would.
    #[derive(Default)]
    pub struct Probe {
        draws: usize,
        records: usize,
        keys: String,
        mode: bool,
        /// The command-line settings `configure` took in, if it was called.
        configured: Option<ViewOptions>,
    }

    impl ViewModule for Probe {
        fn state(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }

        fn configure(&mut self, options: &ViewOptions) {
            self.configured = Some(*options);
        }

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
            let configured = self
                .configured
                .map_or_else(String::new, |options| format!(" | {options:?}"));
            format!(
                " probe: {} draws | {} records | keys [{}]{configured}",
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

        fn configure(&mut self, options: &ViewOptions) {
            self.0.configure(options);
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

    /// The sparkline `Guide` adds below the popup of the process it tours, forty cells wide.
    pub const GUIDE_SPARKLINE: &str = "⣀⣀⣀⣀⣀⣀⣀⣀⣀⣀⣤⣤⣤⣤⣤⣤⣤⣤⣤⣤⣶⣶⣶⣶⣶⣶⣶⣶⣶⣶⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿";

    /// The text `Guide` titles its system with.
    pub const GUIDE_TITLE: &str = "guide system";
    /// The tag the guide writes beside the last process.
    pub const GUIDE_TAG: &str = "guide tag";

    /// A module view that seats every process on a line, asks the tour to visit the last one at
    /// a point beside its seat, adds a line to its popup and titles the line as a system. Its
    /// legend lists the `dt` of each frame it drew, and its state what the stage handed it.
    #[derive(Default)]
    pub struct Guide {
        steps: Vec<f32>,
        /// The `Stage::frames` of each frame it drew.
        pub frames: Vec<u64>,
        /// How many `Stage::splashes` and `Stage::newborn` each frame it drew had.
        pub disturbances: Vec<(usize, usize)>,
    }

    impl ViewModule for Guide {
        fn state(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }

        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            _snapshot: &Snapshot,
        ) -> usize {
            self.steps.push(stage.dt);
            self.frames.push(stage.frames);
            self.disturbances
                .push((stage.splashes.len(), stage.newborn.len()));
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
                stage.titles.push(Title {
                    at: [index as f32 * 4.0, 0.0, 0.0],
                    reach: 3.0,
                    text: GUIDE_TITLE.into(),
                });
                stage.tags.push(Tag {
                    at: [index as f32 * 4.0, 0.0, 0.0],
                    side: Side::Right,
                    text: GUIDE_TAG.into(),
                });
                stage.tour.push(TourTarget {
                    id: last.id,
                    position: [index as f32 * 4.0, 3.0, 0.0],
                    weight: 1.0,
                });
                stage.popup.push((last.id, GUIDE_LINE.into()));
                stage
                    .sparklines
                    .entry(last.id)
                    .or_default()
                    .push(GUIDE_SPARKLINE.into());
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
        ViewInfo::new("guide", "GUIDE", Backdrop::Space, || {
            Box::new(Guide::default())
        }),
        ViewInfo::new("probe", "PROBE", Backdrop::Space, || {
            Box::new(Probe::default())
        }),
    ];

    /// A module view that draws nothing but a plane facing the viewer, 600 by 300 pixels at zoom
    /// 1, whose corners it gives as `Stage::bounds`: the ground points that appear at those
    /// offsets from the centre of the camera drawing it.
    #[derive(Default)]
    pub struct Plane;

    impl ViewModule for Plane {
        fn draw(
            &mut self,
            stage: &mut Stage,
            _processes: &[&Process],
            _snapshot: &Snapshot,
        ) -> usize {
            let zoom = stage.camera.zoom;
            stage.bounds.extend(
                [
                    (-300.0, -150.0),
                    (300.0, -150.0),
                    (300.0, 150.0),
                    (-300.0, 150.0),
                ]
                .map(|(x, y)| {
                    let [x, y] = stage.camera.ground(x * zoom, y * zoom);
                    [x, y, 0.0]
                }),
            );
            0
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            " plane".into()
        }
    }

    /// `plane`, a view of a plane that faces the viewer, as a billboard and as an ordinary view.
    pub static PLANES: &[ViewInfo] = &[
        ViewInfo {
            billboard: true,
            ..ViewInfo::new("billboard", "BILLBOARD", Backdrop::Void, || Box::new(Plane))
        },
        ViewInfo::new("ordinary", "ORDINARY", Backdrop::Void, || Box::new(Plane)),
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

    /// The line `Witness` adds to the tour callout of the process the tour visits.
    pub const WITNESS_CALLOUT: &str = "witness: toured";

    /// A module view that seats every process on a line and tells what the stage handed it: its
    /// legend shows the toured process, the focus and the limit of the last frame. It adds a
    /// line to the tour callout of the toured process.
    #[derive(Default)]
    pub struct Witness {
        seen: String,
        /// The `Stage::age` of every process of the last frame, by pid.
        pub ages: HashMap<u32, Option<f32>>,
        /// The `Stage::fps` of the last frame.
        pub fps: u32,
    }

    impl ViewModule for Witness {
        fn state(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }

        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            _snapshot: &Snapshot,
        ) -> usize {
            self.seen = format!(
                "toured {:?} | focus {:?} | limit {}",
                stage.toured.map(|id| id.pid),
                stage.focus.map(|id| id.pid),
                stage.limit
            );
            self.fps = stage.fps;
            self.ages = processes
                .iter()
                .map(|process| (process.id.pid, stage.age(process.id)))
                .collect();
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
            if let Some(id) = stage.toured {
                stage.callout.push((id, WITNESS_CALLOUT.into()));
            }
            processes.len()
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            format!(" witness: {}", self.seen)
        }
    }

    /// A module view that seats every process on a line and tells the link mode and the highlighted
    /// processes the stage handed it, in its legend.
    #[derive(Default)]
    pub struct Linker {
        seen: String,
    }

    impl ViewModule for Linker {
        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            snapshot: &Snapshot,
        ) -> usize {
            self.seen = format!(
                "links {:?} | highlight {:?}",
                stage.links,
                stage.highlight.iter().map(|id| id.pid).collect::<Vec<_>>()
            );
            Witness::default().draw(stage, processes, snapshot)
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            format!(" linker: {}", self.seen)
        }
    }

    /// `linker`, a view that draws its own links.
    pub static LINKER: &[ViewInfo] = &[ViewInfo {
        links: LinkRule::DrawnByView,
        ..ViewInfo::new("linker", "LINKER", Backdrop::Space, || {
            Box::new(Linker::default())
        })
    }];

    /// `witness`, whose keys the help line shows, then `probe`.
    pub static WITNESSED: &[ViewInfo] = &[
        ViewInfo {
            keys: "m witness mode",
            ..ViewInfo::new("witness", "WITNESS", Backdrop::Space, || {
                Box::new(Witness::default())
            })
        },
        ViewInfo::new("probe", "PROBE", Backdrop::Space, || {
            Box::new(Probe::default())
        }),
    ];

    /// `shuffle`, then `listed`, the same view with its churn failure listed as known, and a
    /// resource-change failure it does not have.
    pub static SHUFFLED: &[ViewInfo] = &[
        ViewInfo::new("shuffle", "SHUFFLE", Backdrop::Space, || Box::new(Shuffle)),
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
            ..ViewInfo::new("listed", "LISTED", Backdrop::Space, || Box::new(Shuffle))
        },
    ];

    /// A module view that seats processes by rank as `Shuffle` does, so one exit moves the rest,
    /// but anchors each at a place fixed by its pid, and shows nothing before 30 s of demo time:
    /// the harness must look at its anchors and run it at its own reference time to pass it.
    #[derive(Default)]
    pub struct Anchored {
        anchors: HashMap<Identity, Point>,
    }

    impl ViewModule for Anchored {
        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            snapshot: &Snapshot,
        ) -> usize {
            self.anchors.clear();
            if snapshot.elapsed < 30.0 {
                return 0;
            }
            for process in processes {
                self.anchors
                    .insert(process.id, [process.id.pid as f32 * 3.0, 0.0, 0.0]);
            }
            Shuffle.draw(stage, processes, snapshot)
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            " anchored".into()
        }

        fn anchors(&self) -> Option<HashMap<Identity, Point>> {
            Some(self.anchors.clone())
        }
    }

    /// `anchored`, checked at 40 s.
    pub static ANCHORED: &[ViewInfo] = &[ViewInfo {
        contract: Contract {
            time: 40.0,
            ..Contract::DEFAULT
        },
        ..ViewInfo::new("anchored", "ANCHORED", Backdrop::Space, || {
            Box::new(Anchored::default())
        })
    }];

    /// Two module views: `probe`, with every optional field at its default, then `quiet`, which
    /// sets them otherwise: no links, flash, haze or tour, a pitch of its own and fixed counts.
    pub static ROWS: &[ViewInfo] = &[
        ViewInfo::new("probe", "PROBE", Backdrop::Space, || {
            Box::new(Probe::default())
        }),
        ViewInfo {
            links: LinkRule::DrawnByView,
            flash: false,
            haze: false,
            pitch: Some(std::f32::consts::FRAC_PI_2),
            tour: false,
            header: Some(|_| (7, 3)),
            ..ViewInfo::new("quiet", "QUIET", Backdrop::Dusk, || {
                Box::new(Quiet::default())
            })
        },
    ];

    /// The kind of thing `Lanterns` draws.
    pub const LANTERN: &str = "lantern";

    /// A line the card of every lantern carries, with terminal controls in it, as untrusted
    /// text can.
    pub const WICK: &str = "wick \x1b[2J\x07lit";

    /// The title of lantern `key`'s card, which ends in a terminal control as well.
    pub fn lantern_title(key: u64) -> String {
        format!("lantern {key}\x1b]0;flare\x07")
    }

    /// A module view that seats each process by its pid, so its seat never moves, and hangs a
    /// lantern on a line in front of them for each of the first three, framing both: a thing drawn as a sphere with a pick-only halo around
    /// it. `x` puts out the last lantern still lit. Its legend names the key of the thing
    /// `Stage::thing` named last, and it builds a card for a lit lantern from what it keeps.
    #[derive(Default)]
    pub struct Lanterns {
        lit: Vec<(u64, Point)>,
        out: usize,
        highlighted: Option<Thing>,
    }

    impl Lanterns {
        /// Where lantern `index` hangs.
        pub fn position(index: usize) -> Point {
            [index as f32 * 4.0, -6.0, 0.0]
        }
    }

    impl ViewModule for Lanterns {
        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            _snapshot: &Snapshot,
        ) -> usize {
            self.highlighted = stage.thing;
            for (index, process) in processes.iter().enumerate() {
                let pid = process.id.pid;
                let position = [(pid % 32) as f32 * 3.0, (pid / 32 % 32) as f32 * 3.0, 0.0];
                stage.positions.insert(process.id, position);
                stage.bounds.push(position);
                stage.frame.sphere(
                    stage.camera,
                    position,
                    1.0,
                    [200, 200, 200],
                    index as u32,
                    false,
                );
            }
            stage.bounds.extend((0..3).map(Self::position));
            let count = processes.len().min(3).saturating_sub(self.out);
            self.lit = (0..count)
                .map(|index| (100 + index as u64, Self::position(index)))
                .collect();
            for &(key, position) in &self.lit {
                let pick = stage.frame.thing_pick(Thing { kind: LANTERN, key });
                stage
                    .frame
                    .sphere(stage.camera, position, 0.5, [255, 200, 90], pick, false);
                stage.frame.pick_disc(stage.camera, position, 12.0, pick);
            }
            processes.len()
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            match self.highlighted {
                Some(thing) => format!(" lanterns | highlighted {}", thing.key),
                None => " lanterns".into(),
            }
        }

        fn key(&mut self, key: ViewKey) -> bool {
            if key == ViewKey::Local('x') {
                self.out += 1;
            }
            false
        }

        fn card(&self, thing: Thing, _snapshot: &Snapshot) -> Option<Card> {
            let &(key, at) = self
                .lit
                .iter()
                .find(|(key, _)| thing.kind == LANTERN && *key == thing.key)?;
            Some(Card {
                title: lantern_title(key),
                lines: vec![WICK.into()],
                at,
            })
        }
    }

    /// `lanterns`, then `probe` to switch to.
    pub static THINGS: &[ViewInfo] = &[
        ViewInfo::new("lanterns", "LANTERNS", Backdrop::Space, || {
            Box::new(Lanterns::default())
        }),
        ViewInfo::new("probe", "PROBE", Backdrop::Space, || {
            Box::new(Probe::default())
        }),
    ];

    /// `beamed` and `stroked`, which draw every link, as beams and as strokes.
    pub static LINKED: &[ViewInfo] = &[
        ViewInfo {
            links: LinkRule::All,
            ..ViewInfo::new("beamed", "BEAMED", Backdrop::Space, || {
                Box::new(Probe::default())
            })
        },
        ViewInfo {
            links: LinkRule::All,
            link_strokes: true,
            ..ViewInfo::new("stroked", "STROKED", Backdrop::Space, || {
                Box::new(Probe::default())
            })
        },
    ];

    /// `replaying`, which replays four seconds of demo samples before a headless frame of a demo
    /// run.
    pub static REPLAYING: &[ViewInfo] = &[ViewInfo {
        warmup: &[Warmup {
            phase: Phase::Replay {
                seconds: 4,
                before_start: false,
            },
            when: When::DemoHeadless,
        }],
        ..ViewInfo::new("replaying", "REPLAYING", Backdrop::Space, || {
            Box::new(Probe::default())
        })
    }];

    /// A module view that remembers when each frame it drew was and which sample it showed, as
    /// `time@elapsed` in its legend.
    #[derive(Default)]
    pub struct Player {
        frames: Vec<String>,
    }

    impl ViewModule for Player {
        fn draw(
            &mut self,
            stage: &mut Stage,
            processes: &[&Process],
            snapshot: &Snapshot,
        ) -> usize {
            self.frames
                .push(format!("{:.1}@{:.1}", stage.time, snapshot.elapsed));
            Probe::default().draw(stage, processes, snapshot)
        }

        fn legend(&self, _snapshot: &Snapshot) -> String {
            format!(" player: {}", self.frames.join(" "))
        }
    }

    /// `playing`, which plays four seconds of demo frames, two a sample, before a headless frame
    /// of a demo run.
    pub static PLAYING: &[ViewInfo] = &[ViewInfo {
        warmup: &[Warmup {
            phase: Phase::Play {
                seconds: 4,
                step: 0.5,
            },
            when: When::DemoHeadless,
        }],
        ..ViewInfo::new("playing", "PLAYING", Backdrop::Space, || {
            Box::new(Player::default())
        })
    }];

    /// `remembering`, which replays twenty seconds of demo samples, those before time 0 as well,
    /// before a headless frame of a demo run.
    pub static REMEMBERING: &[ViewInfo] = &[ViewInfo {
        warmup: &[Warmup {
            phase: Phase::Replay {
                seconds: 20,
                before_start: true,
            },
            when: When::DemoHeadless,
        }],
        ..ViewInfo::new("remembering", "REMEMBERING", Backdrop::Space, || {
            Box::new(Probe::default())
        })
    }];

    /// Two module views for the demo options: `plain`, which asks for nothing, then `demanding`,
    /// which asks for births, exits and exact CPU time and replays two seconds of history before
    /// a captured frame.
    pub static DEMO_ROWS: &[ViewInfo] = &[
        ViewInfo::new("plain", "PLAIN", Backdrop::Space, || {
            Box::new(Probe::default())
        }),
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
                ..DemoOptions::NONE
            },
            ..ViewInfo::new("demanding", "DEMANDING", Backdrop::Space, || {
                Box::new(Probe::default())
            })
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
        // Every view runs everywhere except the switchboard, which reads Linux's interrupt table.
        let expected: Vec<bool> = REGISTRY
            .iter()
            .map(|row| row.name != "switchboard" || cfg!(target_os = "linux"))
            .collect();
        assert_eq!(availability(), expected);
    }

    #[test]
    fn a_process_that_arrives_after_the_first_frame_has_an_age_and_the_first_ones_have_none() {
        use crate::model::testing::process;
        use crate::render::{Camera, RenderInput};
        let rows = testing::WITNESSED;
        let at_start = Snapshot {
            processes: vec![process(10, 1), process(11, 1)],
            ..Snapshot::default()
        };
        let mut later = at_start.clone();
        later.processes.push(process(12, 1));
        let mut scene = Scene::with_rows(rows);
        let camera = Camera::default();
        let mut ages = |snapshot: &Snapshot, time: f32| {
            let input = RenderInput::small(snapshot, View::at(0), &camera, time);
            scene.render(&input);
            scene
                .module::<testing::Witness>(View::at(0))
                .expect("the witness has drawn")
                .ages
                .clone()
        };
        let first = ages(&at_start, 5.0);
        assert_eq!(first[&10], None, "present on the first frame: never new");
        let second = ages(&later, 6.0);
        assert_eq!(second[&10], None);
        assert_eq!(second[&12], Some(0.0), "first seen this frame");
        let third = ages(&later, 8.5);
        assert_eq!(third[&12], Some(2.5));
        assert_eq!(third[&11], None);
        let rewound = ages(&later, 5.0);
        assert_eq!(rewound[&12], None, "before it arrived");
    }

    #[test]
    fn the_stage_carries_the_target_frame_rate_the_app_set() {
        use crate::render::{Camera, RenderInput};
        let snapshot = Snapshot::default();
        let camera = Camera::default();
        let mut scene = Scene::with_rows(testing::WITNESSED);
        let fps = |scene: &mut Scene| {
            scene.render(&RenderInput::small(&snapshot, View::at(0), &camera, 1.0));
            scene
                .module::<testing::Witness>(View::at(0))
                .expect("the witness has drawn")
                .fps
        };
        assert_eq!(fps(&mut scene), 10, "--fps defaults to 10");
        scene.fps = 24;
        assert_eq!(fps(&mut scene), 24);
    }

    #[test]
    fn an_arrival_survives_a_rewind_to_before_the_birth_so_forward_navigation_restores_its_age() {
        use crate::model::testing::process;
        use crate::render::{Camera, RenderInput};
        let at_start = Snapshot {
            processes: vec![process(10, 1)],
            ..Snapshot::default()
        };
        let mut later = at_start.clone();
        later.processes.push(process(12, 1));
        let mut scene = Scene::with_rows(testing::WITNESSED);
        let camera = Camera::default();
        let mut age_of_twelve = |snapshot: &Snapshot, time: f32| {
            let input = RenderInput::small(snapshot, View::at(0), &camera, time);
            scene.render(&input);
            scene
                .module::<testing::Witness>(View::at(0))
                .expect("the witness has drawn")
                .ages
                .get(&12)
                .copied()
                .flatten()
        };
        assert_eq!(age_of_twelve(&at_start, 5.0), None);
        assert_eq!(age_of_twelve(&later, 6.0), Some(0.0));
        // Rewound to a sample from before it started, then forward again.
        assert_eq!(age_of_twelve(&at_start, 5.5), None);
        assert_eq!(
            age_of_twelve(&later, 8.5),
            Some(2.5),
            "the process keeps the age it had, not a fresh one"
        );
    }

    #[test]
    fn every_row_builds_its_own_module_and_only_views_without_processes_replace_the_inspector() {
        let scene = Scene::new();
        let snapshot = Snapshot::default();
        let replacing: Vec<&str> = View::all()
            .filter(|&view| scene.inspector_line(view).is_some())
            .map(View::name)
            .collect();
        assert_eq!(
            replacing,
            ["matrix", "switchboard"],
            "a view replaces the inspector line through ViewModule, and only a view that draws no \
             processes has reason to"
        );
        for view in View::all() {
            assert_eq!(
                view.info().contract.draws_processes,
                scene.inspector_line(view).is_none(),
                "{view:?}: the line replaces the inspector of a view that draws no processes"
            );
            assert!(
                scene.legend(view, &snapshot).starts_with(' '),
                "{view:?}: the legend the module answers with starts with a space"
            );
        }
    }
}
