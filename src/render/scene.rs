use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use super::stage::FrameClock;
use super::*;
use crate::history::History;
use crate::model::{Identity, Process, Snapshot, bounded};
use crate::palette::tint;
#[cfg(test)]
use crate::simulation::growth;
use crate::simulation::smoothing;
use crate::views::systems::Orbit;
use crate::views::{LinkRule, REGISTRY, View, ViewInfo, ViewKey, ViewModule};

const FADE_SECONDS: f32 = 1.2;
const LINK: Color = [80, 175, 235];
const LINK_HOT: Color = [150, 232, 255];

/// How many arrivals of processes absent from the shown sample the scene remembers, oldest
/// forgotten first.
const ARRIVALS_KEPT: usize = 4096;

/// The longest process name a scene label shows, in terminal cells. Linux `comm` stops here, so
/// only the longer macOS names are ever cut.
pub const LABEL_NAME_CELLS: usize = 15;

/// A name without Apple's own `com.apple.` prefix, which says nothing on macOS. Other vendors'
/// reverse-DNS names stay whole, and so does a name that is nothing but the prefix.
pub fn without_apple_prefix(name: &str) -> &str {
    name.strip_prefix("com.apple.")
        .filter(|rest| !rest.is_empty())
        .unwrap_or(name)
}

/// A process name as a scene label: the Apple prefix dropped, then anything still longer than
/// `LABEL_NAME_CELLS` keeps its first 13 characters and ends in `..`, so a label is never wider
/// than 15 cells. The ellipsis is ASCII because `terminal::SafeText` turns every other character
/// into `?`. Hover tags, the inspector and search keep the full name.
pub fn label_name(name: &str) -> Cow<'_, str> {
    let name = without_apple_prefix(name);
    if name.chars().count() <= LABEL_NAME_CELLS {
        return Cow::Borrowed(name);
    }
    let mut cut: String = name.chars().take(LABEL_NAME_CELLS - 2).collect();
    cut.push_str("..");
    Cow::Owned(cut)
}

/// A thread name as a scene label: its first `LABEL_NAME_CELLS` characters, raw like the name
/// itself. `SafeText` turns each character into one cell on its way to the terminal, so the cut
/// is a width in cells there.
pub fn thread_label(name: &str) -> String {
    name.chars().take(LABEL_NAME_CELLS).collect()
}

/// What persists between frames and belongs to the frame, not to any one view: the smoothing of
/// the sampled processes, their births and deaths, the links and highlights, the outputs of the
/// last frame the app reads back, the spare buffers, and the module of each view in its row's
/// slot. A view keeps its own state in its module and reaches the scene only through `Stage`.
/// Every field says in its doc comment why it is frame-wide (the lint below insists), and
/// `scene_has_no_view_specific_fields` fails to compile until a new field is listed in it, which
/// is the review: add one only if every view and the app share it.
#[deny(clippy::missing_docs_in_private_items)]
pub struct Scene {
    /// Each process's CPU and memory as eased towards the samples. Frame-wide: the easing
    /// belongs to the sample, so every view reads the same figures (`Stage::resources`) and a
    /// view that is opened later finds them already settled.
    resources: HashMap<Identity, (f32, f32)>,
    /// Frames rendered, whichever view was shown: `Stage::frames`.
    frames: u64,
    /// The time of the previous frame, to tell a rewind or a gap from a step and drop `resources`.
    last_time: Option<f32>,
    /// The step every `Stage` gets as `dt`.
    clock: FrameClock,
    /// When each process was first seen, for the growth every view eases new bodies in with.
    births: HashMap<Identity, f32>,
    /// When each process that arrived after the first frame was first seen: `Stage::age`.
    arrivals: HashMap<Identity, f32>,
    /// Where the previous frame drew each process, so the exit flash and the splash of the media
    /// know where a process that left stood, whichever view is shown.
    previous: HashMap<Identity, Point>,
    /// The exit flashes still fading: the point and the time of a death. Frame-wide with
    /// `previous`, drawn by the scene over any view whose row has the flash.
    deaths: Vec<(Point, f32)>,
    /// The last frame's `Stage::headroom`, so fitting leaves it room above the ground.
    headroom: f32,
    /// Where processes left since the last frame, as disturbances for the media of any view
    /// (`Stage::splashes`).
    splashes: Vec<Point>,
    /// The processes that arrived since the last frame (`Stage::newborn`).
    newborn: Vec<Identity>,
    /// Whether a frame has been drawn: processes present at the first one did not start while
    /// anyone watched.
    started: bool,
    /// Animate the processes that are already running when the first frame is drawn.
    pub intro: bool,
    /// Which socket links to draw: the `c` key, global to every view.
    pub links: Links,
    /// The processes whose links stand out: selected, hovered or toured, set by the app for
    /// whichever view is shown.
    pub highlight: Vec<Identity>,
    /// The last frame's `Stage::positions`: where each drawn process is, which picking, labels,
    /// the tour, the links and the exit flash all read.
    pub positions: HashMap<Identity, Point>,
    /// The last frame's `Stage::places`.
    pub places: Vec<(Point, String)>,
    /// The last frame's `Stage::titles`.
    pub titles: Vec<Title>,
    /// The last frame's `Stage::bounds`, which `fit` frames the camera to.
    pub(super) bounds: Vec<Point>,
    /// The last frame's `Stage::notes`: popup lines after the sockets.
    pub notes: HashMap<Identity, Vec<String>>,
    /// The last frame's `Stage::sparklines`.
    pub sparklines: HashMap<Identity, Vec<String>>,
    /// The last frame's `Stage::tour`: the stops the tour and the Space visits make.
    pub tour: Vec<TourTarget>,
    /// The last frame's `Stage::popup`: lines the inspector popup carries for the shown view.
    pub popup: Vec<(Identity, String)>,
    /// The last frame's `Stage::callout`: lines of the tour's callout.
    pub callout: Vec<(Identity, String)>,
    /// Every process's CPU and memory over the last samples, recorded whichever view is shown.
    pub(crate) history: History,
    /// The views this scene draws, one row each: `REGISTRY`, unless a test brings its own.
    rows: &'static [ViewInfo],
    /// The module of each view, in its row's slot, made by the row's `build` when the scene is
    /// created. Every module is built up front, not on first show, because `record` feeds every
    /// sample to every view and a view built late would miss the samples before it opened.
    modules: Vec<Box<dyn ViewModule>>,
    /// Allocations recycled from the previous frame.
    pub spare: Buffers,
    /// How many processes the shown view drew in the last frame: the status line's count.
    pub visible: usize,
    /// How many of the processes admitted to the last frame the shown view left out.
    pub collapsed: usize,
}

impl Scene {
    pub fn new() -> Self {
        Self::with_rows(REGISTRY)
    }

    /// A scene that draws the views of `rows`, building each module view's state.
    pub(crate) fn with_rows(rows: &'static [ViewInfo]) -> Self {
        Self {
            resources: HashMap::new(),
            frames: 0,
            last_time: None,
            clock: FrameClock::default(),
            births: HashMap::new(),
            arrivals: HashMap::new(),
            previous: HashMap::new(),
            deaths: Vec::new(),
            headroom: 0.0,
            splashes: Vec::new(),
            newborn: Vec::new(),
            started: false,
            intro: false,
            links: Links::All,
            highlight: Vec::new(),
            positions: HashMap::new(),
            places: Vec::new(),
            titles: Vec::new(),
            bounds: Vec::new(),
            notes: HashMap::new(),
            sparklines: HashMap::new(),
            tour: Vec::new(),
            popup: Vec::new(),
            callout: Vec::new(),
            history: History::default(),
            rows,
            modules: rows.iter().map(|row| (row.build)()).collect(),
            spare: Buffers::default(),
            visible: 0,
            collapsed: 0,
        }
    }

    /// The row of `view` in the table this scene draws from.
    pub fn row(&self, view: View) -> &'static ViewInfo {
        &self.rows[view.index()]
    }

    /// The third status line for `view` while no process is inspected.
    pub fn legend(&self, view: View, snapshot: &Snapshot) -> String {
        self.modules[view.index()].legend(snapshot)
    }

    /// The line `view` shows in place of the inspector, if it draws no processes to inspect.
    pub fn inspector_line(&self, view: View) -> Option<String> {
        self.modules[view.index()].inspector_line()
    }

    /// The inspector card of `thing`, which `view`, the shown view, drew: built now from the
    /// module's own state, or None once it no longer shows the thing.
    pub fn card(&self, view: View, thing: Thing, snapshot: &Snapshot) -> Option<Card> {
        self.modules[view.index()].card(thing, snapshot)
    }

    /// Offers a key to `view`, the shown view; see `ViewKey` for what the result means.
    pub fn key(&mut self, view: View, key: ViewKey) -> bool {
        self.modules[view.index()].key(key)
    }

    /// The state of the module view `view` draws, if it is a `T` and lets its tests see it
    /// (`ViewModule::state`).
    #[cfg(test)]
    pub fn module<T: 'static>(&self, view: View) -> Option<&T> {
        self.modules[view.index()].state()?.downcast_ref()
    }

    /// Where the view harness looks for `view`'s processes after a frame: the module's own
    /// anchors. None checks `positions`.
    #[cfg(test)]
    pub fn anchors(&self, view: View) -> Option<HashMap<Identity, Point>> {
        self.modules[view.index()].anchors()
    }

    /// Notes a splash at `at` as an exit would, for the next frame, for a test that stirs the
    /// water.
    #[cfg(test)]
    pub(crate) fn splash(&mut self, at: Point) {
        self.splashes.push(at);
    }

    /// The easing fraction `Stage::growth` hands a view, for the tests of the births.
    #[cfg(test)]
    fn growth(&self, id: Identity, time: f32) -> f32 {
        growth(&self.births, id, time)
    }

    /// Whether the last view drawn gave explicit bounds; such views grow as data arrives.
    pub fn bounded(&self) -> bool {
        !self.bounds.is_empty()
    }

    /// Feeds a new sample to every view's record hook in registry order, whichever view is
    /// shown, so views that keep their own history have it when they open.
    pub fn record(&mut self, snapshot: &Snapshot, time: f32) {
        self.history.record(snapshot, time);
        for module in &mut self.modules {
            module.record(snapshot, time);
        }
    }

    pub fn fit(&self, camera: &mut Camera, width: u32, height: u32) {
        if self.positions.is_empty() && self.titles.is_empty() && self.bounds.is_empty() {
            return;
        }
        let probe = Camera {
            center: [0.0, 0.0],
            zoom: 1.0,
            ..camera.clone()
        };
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        let points: Vec<Point> = if self.bounds.is_empty() {
            // The drawn processes, and the systems' centres a hub stands at, which are no process.
            let centres = self.titles.iter().map(|title| title.at);
            self.positions
                .values()
                .copied()
                .chain(centres)
                .map(|p| [p[0], p[1], 0.0])
                .collect()
        } else {
            self.bounds.clone()
        };
        for &p in &points {
            let q = probe.view(p);
            for i in 0..2 {
                min[i] = min[i].min(q[i]);
                max[i] = max[i].max(q[i]);
            }
        }
        camera.center = probe.ground((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5);
        if !self.bounds.is_empty() {
            camera.zoom =
                (width as f32 / (max[0] - min[0])).min(height as f32 / (max[1] - min[1])) * 0.9;
            // Frames are drawn around a point ORIGIN_Y down the screen; centre the bounds exactly.
            let lift = (ORIGIN_Y - 0.5) * height as f32 / camera.zoom;
            camera.center = probe.ground((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5 + lift);
            return;
        }
        // The vertical margin leaves room for the tallest thing above the ground plane.
        let headroom = 24.0_f32.max(self.headroom * camera.pitch.cos() * SCALE + 8.0);
        camera.zoom = (width as f32 / (max[0] - min[0] + 12.0))
            .min(height as f32 / (max[1] - min[1] + headroom))
            * 0.8;
    }

    /// Records a frame; call `Frame::rasterize` (or a GPU backend) to turn it into pixels.
    pub fn render(&mut self, input: &RenderInput) -> Frame {
        let RenderInput {
            snapshot,
            view,
            camera,
            width,
            height,
            selected,
            thing: _,
            time,
            limit,
            focus,
            paused,
            toured: _,
        } = *input;
        self.frames += 1;
        // The sample is already in when the main loop recorded it; headless renders and tests
        // that never call `record` fill the history here.
        self.history.record(snapshot, time);
        let dt = self.clock.step(time, paused);
        let alive: HashSet<_> = snapshot.processes.iter().map(|p| p.id).collect();
        let visual = self.ease(snapshot, &alive, time);
        let processes = admitted(&visual, snapshot, selected, limit, focus);
        let row = self.row(view);
        let [cpu, memory, io] = snapshot.pressure;
        let (c, m, i) = if row.haze {
            (bounded(cpu, 20.0), bounded(memory, 10.0), bounded(io, 20.0))
        } else {
            (0.0, 0.0, 0.0)
        };
        let sky = Sky {
            backdrop: row.backdrop,
            haze: [
                c * 110.0 + m * 140.0 + i * 30.0,
                c * 70.0 + m * 20.0 + i * 50.0,
                c * 10.0 + m * 60.0 + i * 150.0,
            ],
            time,
        };
        let mut frame = Frame::new(width, height, sky, std::mem::take(&mut self.spare));
        frame.identities = processes.iter().map(|p| p.id).collect();
        frame.things.clear();
        debug_assert!(
            frame.identities.len() <= THING as usize,
            "process indices stay below the thing bit"
        );
        self.visible = processes.len();
        self.collapsed = 0;
        self.places.clear();
        self.titles.clear();
        self.bounds.clear();
        self.notes.clear();
        self.sparklines.clear();
        self.tour.clear();
        self.popup.clear();
        self.callout.clear();
        // A view that stands things on the ground reports their height again every frame it is
        // shown.
        self.headroom = 0.0;
        self.draw_view(&mut frame, input, &processes, dt);
        let lift = row.link_lift;
        self.splashes.clear();
        self.newborn.clear();
        match row.links {
            LinkRule::All => self.draw_links(&mut frame, camera, snapshot, time, lift, false, row),
            LinkRule::Quiet => self.draw_links(&mut frame, camera, snapshot, time, lift, true, row),
            LinkRule::DrawnByView => {}
        }
        self.flash_exits(&mut frame, camera, &alive, time, row.flash);
        frame.anchor = selected
            .and_then(|id| self.positions.get(&id))
            .and_then(|&p| frame.locate(camera, p));
        frame
    }

    /// Forgets the arrivals of processes that are gone, except the latest `ARRIVALS_KEPT`: a
    /// rewind to before a process started also finds it absent, and jumping forward again must
    /// give it back the age it had, not a new one.
    fn keep_arrivals(&mut self, alive: &HashSet<Identity>) {
        let absent =
            self.arrivals.len() - self.arrivals.keys().filter(|id| alive.contains(id)).count();
        let Some(excess) = absent
            .checked_sub(ARRIVALS_KEPT)
            .filter(|&excess| excess > 0)
        else {
            return;
        };
        let mut gone: Vec<(f32, Identity)> = self
            .arrivals
            .iter()
            .filter(|(id, _)| !alive.contains(id))
            .map(|(&id, &arrived)| (arrived, id))
            .collect();
        gone.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, id) in gone.into_iter().take(excess) {
            self.arrivals.remove(&id);
        }
    }

    /// Takes in the frame's sample: notes births, and returns its processes with CPU and memory
    /// eased towards the sampled values.
    fn ease(&mut self, snapshot: &Snapshot, alive: &HashSet<Identity>, time: f32) -> Vec<Process> {
        let dt = self.last_time.map_or(0.0, |previous| time - previous);
        if !(0.0..=2.0).contains(&dt) {
            self.resources.clear();
        }
        self.last_time = Some(time);
        let alpha = smoothing(dt.max(0.0), 0.3);
        self.resources.retain(|id, _| alive.contains(id));
        self.births.retain(|id, _| alive.contains(id));
        self.keep_arrivals(alive);
        let born = if self.started || self.intro {
            time
        } else {
            f32::NEG_INFINITY
        };
        let fresh = self.started;
        self.started = true;
        for p in &snapshot.processes {
            if let std::collections::hash_map::Entry::Vacant(entry) = self.births.entry(p.id) {
                entry.insert(born);
                if fresh {
                    self.newborn.push(p.id);
                    self.arrivals.entry(p.id).or_insert(time);
                }
            }
        }
        snapshot
            .processes
            .iter()
            .map(|p| {
                let memory = p.memory as f32;
                let current = self.resources.entry(p.id).or_insert((p.cpu, memory));
                current.0 += alpha * (p.cpu - current.0);
                current.1 += alpha * (memory - current.1);
                let mut p = p.clone();
                p.cpu = current.0;
                p.memory = current.1 as u64;
                p
            })
            .collect()
    }

    /// Draws the shown view.
    fn draw_view(
        &mut self,
        frame: &mut Frame,
        input: &RenderInput,
        processes: &[&Process],
        dt: f32,
    ) {
        let RenderInput {
            snapshot,
            view,
            camera,
            selected,
            thing,
            time,
            limit,
            focus,
            toured,
            ..
        } = *input;
        self.positions.clear();
        let mut stage = Stage {
            frame,
            camera,
            selected,
            thing,
            toured,
            focus,
            links: self.links,
            highlight: &self.highlight,
            limit,
            time,
            frames: self.frames,
            dt,
            history: &self.history,
            births: &self.births,
            arrivals: &self.arrivals,
            resources: &self.resources,
            splashes: &self.splashes,
            newborn: &self.newborn,
            positions: &mut self.positions,
            places: &mut self.places,
            bounds: &mut self.bounds,
            notes: &mut self.notes,
            sparklines: &mut self.sparklines,
            tour: &mut self.tour,
            popup: &mut self.popup,
            callout: &mut self.callout,
            titles: &mut self.titles,
            headroom: &mut self.headroom,
        };
        let shown = self.modules[view.index()].draw(&mut stage, processes, snapshot);
        self.visible = shown;
        self.collapsed = processes.len().saturating_sub(shown);
    }

    /// Marks where processes that left were last drawn and, where the row has the flash, draws
    /// an expanding ring and glow there for `FADE_SECONDS`.
    fn flash_exits(
        &mut self,
        frame: &mut Frame,
        camera: &Camera,
        alive: &HashSet<Identity>,
        time: f32,
        flash: bool,
    ) {
        for (id, &point) in &self.previous {
            if !alive.contains(id) {
                self.deaths.push((point, time));
                self.splashes.push(point);
            }
        }
        self.previous = self.positions.clone();
        self.deaths
            .retain(|&(_, died)| (0.0..FADE_SECONDS).contains(&(time - died)));
        let flashes: &[(Point, f32)] = if flash { &self.deaths } else { &[] };
        for &(point, died) in flashes {
            let age = (time - died) / FADE_SECONDS;
            let ring = Orbit {
                center: point,
                a: 0.6 + 3.4 * age,
                e: 0.0,
                omega: 0.0,
            };
            frame.trace(camera, &ring, tint([255, 196, 140], 1.0 - age));
            if age < 0.35 {
                let radius = (2.0 * camera.zoom).max(10.0);
                frame.glow(camera, point, radius, [255, 226, 186], 1.0 - age / 0.35);
            }
        }
    }

    /// Socket links as arcs lifted above the plane with travelling pulses, and connections that
    /// leave the machine as beams rising off the body. Links touching highlighted processes glow.
    #[expect(
        clippy::too_many_arguments,
        reason = "the frame's link inputs and the row"
    )]
    fn draw_links(
        &self,
        frame: &mut Frame,
        camera: &Camera,
        snapshot: &Snapshot,
        time: f32,
        lift: f32,
        quiet: bool,
        row: &ViewInfo,
    ) {
        if self.links == Links::Off {
            return;
        }
        let mode = if quiet { Links::Focused } else { self.links };
        let hot = |id: &Identity| self.highlight.contains(id);
        for &(a, b, count) in &snapshot.links {
            let (Some(&pa), Some(&pb)) = (self.positions.get(&a), self.positions.get(&b)) else {
                continue;
            };
            let glowing = hot(&a) || hot(&b);
            if mode == Links::Focused && !glowing {
                continue;
            }
            let (color, weight) = if glowing {
                (LINK_HOT, 0.8)
            } else {
                (LINK, (0.1 + 0.05 * (count as f32).ln()).min(0.3))
            };
            let span = (pa[0] - pb[0]).hypot(pa[1] - pb[1]);
            let start = [pa[0], pa[1], pa[2] + lift];
            let end = [pb[0], pb[1], pb[2] + lift];
            let control = [
                (pa[0] + pb[0]) * 0.5,
                (pa[1] + pb[1]) * 0.5,
                (pa[2] + pb[2]) * 0.5 + lift + span * 0.35,
            ];
            let curve = |t: f32| {
                let u = 1.0 - t;
                [0, 1, 2].map(|k| u * u * start[k] + 2.0 * u * t * control[k] + t * t * end[k])
            };
            let mut last = start;
            for step in 1..=20 {
                let next = curve(step as f32 / 20.0);
                if row.link_strokes {
                    frame.hairline(camera, last, next, tint(color, 0.35 + 0.65 * weight), 1.1);
                } else {
                    frame.beam(camera, last, next, color, weight);
                }
                last = next;
            }
            let phase = (a.pid.wrapping_mul(31) ^ b.pid.wrapping_mul(17)) % 100;
            let t = (time * 0.35 + phase as f32 / 100.0).fract();
            let strength = if glowing { 0.9 } else { 0.35 };
            frame.glow(camera, curve(t), 5.0, color, strength);
        }
        // Overlapping beams blend in drawing order, and a `HashMap` iterates in a different order
        // every run.
        let mut outside: Vec<_> = snapshot.outside.iter().collect();
        outside.sort_unstable_by_key(|&(id, _)| *id);
        for (id, &count) in outside {
            let Some(&p) = self.positions.get(id) else {
                continue;
            };
            let glowing = hot(id);
            if mode == Links::Focused && !glowing {
                continue;
            }
            let height = 5.0 + 3.0 * (count as f32).ln_1p();
            let strength = if glowing { 0.9 } else { 0.45 };
            for step in 0..10 {
                let a = [p[0], p[1], p[2] + lift + height * step as f32 / 10.0];
                let b = [p[0], p[1], p[2] + lift + height * (step + 1) as f32 / 10.0];
                let weight = strength * (1.0 - step as f32 / 10.0);
                if row.link_strokes {
                    frame.hairline(camera, a, b, tint(OUTSIDE, 0.35 + 0.65 * weight), 1.1);
                } else {
                    frame.beam(camera, a, b, OUTSIDE, weight);
                }
            }
        }
    }
}

/// The processes a frame draws, in PID order: the focused subtree, or every process, cut to
/// `limit` with the selection kept.
fn admitted<'a>(
    visual: &'a [Process],
    snapshot: &Snapshot,
    selected: Option<Identity>,
    limit: usize,
    focus: Option<Identity>,
) -> Vec<&'a Process> {
    let allowed = focus.map(|root| descendants(snapshot, root));
    let mut processes: Vec<&Process> = visual
        .iter()
        .filter(|p| allowed.as_ref().is_none_or(|set| set.contains(&p.id)))
        .collect();
    processes.sort_by_key(|p| p.id);
    if processes.len() > limit {
        // A stable PID order keeps resource changes from continuously evicting buildings.
        if let Some(id) = selected
            && let Some(index) = processes.iter().position(|p| p.id == id)
            && index >= limit
        {
            processes.swap(index, limit - 1);
        }
        processes.truncate(limit);
    }
    processes
}

pub fn descendants(snapshot: &Snapshot, root: Identity) -> HashSet<Identity> {
    let mut result = HashSet::from([root]);
    let mut pids = HashSet::from([root.pid]);
    loop {
        let before = result.len();
        for p in &snapshot.processes {
            if pids.contains(&p.parent) {
                result.insert(p.id);
                pids.insert(p.id.pid);
            }
        }
        if result.len() == before {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests;
