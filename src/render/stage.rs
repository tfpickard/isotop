use std::collections::HashMap;
use std::f32::consts::TAU;

use super::camera::Camera;
use super::frame::{Frame, Thing};
use super::{Links, Point};
use crate::history::History;
use crate::model::{Identity, Process, Snapshot};
use crate::simulation::growth;
use crate::views::View;

/// Everything a frame depends on besides the scene's own state: what `Scene::render` draws.
#[derive(Clone, Copy)]
pub struct RenderInput<'a> {
    /// The sample to draw: the newest, or the one rewound to while paused.
    pub snapshot: &'a Snapshot,
    pub view: View,
    pub camera: &'a Camera,
    /// The frame's size in pixels.
    pub width: u32,
    pub height: u32,
    pub selected: Option<Identity>,
    /// The selected thing, or else the hovered one, for the view to highlight.
    pub thing: Option<Thing>,
    /// Seconds on the animation clock, or the shown sample's `elapsed` while paused.
    pub time: f32,
    /// The most processes drawn; the rest of the PID order is left out, the selection kept.
    pub limit: usize,
    /// The root of the subtree to draw alone, if one is focused.
    pub focus: Option<Identity>,
    /// Whether the app is paused, so `Stage::dt` is 0 whatever `time` does.
    pub paused: bool,
    /// The process the tour is visiting, if a tour runs.
    pub toured: Option<Identity>,
}

#[cfg(test)]
impl<'a> RenderInput<'a> {
    /// A 320 by 180 frame of `view` at `time`, running, with nothing selected or focused and at
    /// most 512 processes, as most tests draw.
    pub fn small(snapshot: &'a Snapshot, view: View, camera: &'a Camera, time: f32) -> Self {
        RenderInput {
            snapshot,
            view,
            camera,
            width: 320,
            height: 180,
            selected: None,
            thing: None,
            time,
            limit: 512,
            focus: None,
            paused: false,
            toured: None,
        }
    }
}

/// The longest step `Stage::dt` reports, in seconds. A longer gap (a stall, a slow frame) counts
/// as this much, so a simulation never leaps.
pub(crate) const MAX_DT: f32 = 0.5;

/// The one step policy behind `Stage::dt`: the time of the last frame that moved forward.
#[derive(Default)]
pub(super) struct FrameClock {
    last: Option<f32>,
}

impl FrameClock {
    /// The step for a frame at `time`. Paused frames leave the clock where it was: the app's
    /// animation clock stands still while paused, so the first frame after it steps one frame.
    /// A frame whose time went back restarts the count from its own time.
    pub(super) fn step(&mut self, time: f32, paused: bool) -> f32 {
        if paused {
            return 0.0;
        }
        let dt = match self.last {
            Some(last) if time > last => (time - last).min(MAX_DT),
            _ => 0.0,
        };
        self.last = Some(time);
        dt
    }
}

/// A process a view asks the tour (`g` and the idle tour) to visit, besides the busiest and the
/// largest processes it always picks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TourTarget {
    pub id: Identity,
    /// Where the tour centres the camera and points its callout.
    pub position: Point,
    /// The tour visits heavier targets first, as Orbit's stars by the processes they hold.
    pub weight: f32,
}

/// The name of a system a view draws, which the app labels in bright text just under the system's
/// outer edge (or at `at` when that edge is far below it): a `Title` for each system worth a
/// label, in the order the labels are offered a place.
#[derive(Clone, Debug, PartialEq)]
pub struct Title {
    /// The system's centre, which the label is placed from.
    pub at: Point,
    /// How far the system reaches from `at` in world units.
    pub reach: f32,
    /// The label, already cut to fit (`label_name`).
    pub text: String,
}

/// What a view draws with, and its only way to talk back to the app. A view reads only `Stage`,
/// the process slice and `&Snapshot`, and talks back only through `Stage` outputs. What the
/// platform could not measure is `snapshot.missing`; `Stage` does not copy it.
pub(crate) struct Stage<'a> {
    pub frame: &'a mut Frame,
    pub camera: &'a Camera,
    pub selected: Option<Identity>,
    /// The selected thing, or the hovered one if none is selected, so the view can highlight
    /// it. Never Some while a process is selected.
    pub thing: Option<Thing>,
    /// The process the tour is visiting, if a tour runs; the tour clears the selection.
    pub toured: Option<Identity>,
    /// The root of the focused subtree, if one is focused: the process slice holds only that
    /// subtree, so a view leaves out what it draws besides the processes (exited ones).
    pub focus: Option<Identity>,
    /// Which links the `c` key shows. A view whose row says `LinkRule::DrawnByView` draws its
    /// own links and follows it: all of them, only those of the `highlight`, or none.
    pub links: Links,
    /// The processes whose links stand out: the selected, the hovered and the toured.
    pub highlight: &'a [Identity],
    /// `--limit`, the most processes a frame draws. The slice is already cut to it; a view that
    /// draws more than the slice (exited processes) keeps within it.
    pub limit: usize,
    pub time: f32,
    /// Frames the scene has rendered, this one included, whichever view was shown. A view that
    /// must tell a frame that follows its own last frame from one after a gap (another view was
    /// shown in between) compares it with the number it last drew at: the galaxy of Orbit
    /// turns only across consecutive frames (Flow's is held still).
    pub frames: u64,
    /// Seconds to advance a simulation by this frame, by the one policy for new code: 0 on the
    /// first frame, while paused, and when `time` went back (a rewind or a headless replay);
    /// otherwise the time since the last frame that moved forward, at most `MAX_DT`. The scene
    /// computes it once per frame whichever view is shown, so returning to a view never leaps.
    /// New views step with it, or with S2 (#23)'s `FixedStep` fed from `time`. The views that
    /// predate it keep their own clamps, because changing them changes pixels (A33 decides).
    pub dt: f32,
    /// Every process's CPU and memory over the last samples, as of this frame. The scene records
    /// each sample whichever view is shown; read the past with `History::window` and `series`,
    /// from the shown snapshot's `elapsed`.
    pub history: &'a History,
    pub(super) births: &'a HashMap<Identity, f32>,
    pub(super) arrivals: &'a HashMap<Identity, f32>,
    /// CPU and memory as eased for every sampled process, drawn or not.
    pub(super) resources: &'a HashMap<Identity, (f32, f32)>,
    /// Where processes that left were last drawn, as the scene found out about them in its
    /// previous frame: the ground for a splash.
    pub splashes: &'a [Point],
    /// The processes that appeared in this frame's sample after the scene's first frame.
    pub newborn: &'a [Identity],
    /// Where each drawn process is, for picking anchors, labels and the tour.
    pub positions: &'a mut HashMap<Identity, Point>,
    /// World-anchored text labels.
    pub places: &'a mut Vec<(Point, String)>,
    /// Points the camera fit must include; empty means fit the positions on the ground.
    pub bounds: &'a mut Vec<Point>,
    /// Extra lines for a process's inspector popup, after its sockets.
    pub notes: &'a mut HashMap<Identity, Vec<String>>,
    /// Braille sparkline lines for a process's inspector popup, right after its notes. Only
    /// generated Braille Patterns (U+2800 to U+28FF) and spaces: the terminal draws them through
    /// `terminal::clean_sparkline`, which turns everything else into `?`, so text from the system
    /// never belongs here.
    pub sparklines: &'a mut HashMap<Identity, Vec<String>>,
    /// Processes the tour should visit; a hub is passed over, since it has no callout.
    pub tour: &'a mut Vec<TourTarget>,
    /// Lines for a process's inspector popup, right after its own figures and in the order
    /// pushed: what the view knows about it beyond the sample, such as the memory of the
    /// subtree it anchors.
    pub popup: &'a mut Vec<(Identity, String)>,
    /// Lines for the tour's callout of a process, after its own figures and in the order pushed:
    /// what the view makes of the visit, such as a frame it cannot enter.
    pub callout: &'a mut Vec<(Identity, String)>,
    /// The names of the systems the view draws, labelled under their outer edges after
    /// `places`, in the order pushed. Each system's centre (`Title::at`) is kept in the camera
    /// fit with the `positions`, when the view gave no `bounds`.
    pub titles: &'a mut Vec<Title>,
    /// How far above the ground plane the view's content rises, in world units, for the camera
    /// fit to leave room for it when the view gave no `bounds`. Start at 0 each frame. Default:
    /// nothing rises.
    pub headroom: &'a mut f32,
}

impl Stage<'_> {
    pub fn growth(&self, id: Identity) -> f32 {
        growth(self.births, id, self.time)
    }

    /// Seconds since the scene first saw the process, or None for one that was there on the
    /// scene's first frame (it did not start while anyone watched) and while `time` is before
    /// that moment (a rewind). `growth` is an easing fraction that the app's intro also
    /// animates for processes already running; this is the true age of one that arrived.
    pub fn age(&self, id: Identity) -> Option<f32> {
        let arrived = *self.arrivals.get(&id)?;
        (self.time >= arrived).then_some(self.time - arrived)
    }

    /// A sampled process's CPU and memory as the scene eases them, whether it is drawn or not.
    /// A drawn process carries these very figures, so a view that sizes or ranks by processes it
    /// hides measures them as it measures the ones it shows.
    pub fn eased(&self, process: &Process) -> (f32, u64) {
        self.resources
            .get(&process.id)
            .map_or((process.cpu, process.memory), |&(cpu, memory)| {
                (cpu, memory as u64)
            })
    }
}

/// Points around a horizontal circle at two heights, for fitting the camera to a round view.
pub(crate) fn ring_bounds(center: [f32; 2], radius: f32, low: f32, high: f32) -> Vec<Point> {
    (0..24)
        .flat_map(|k| {
            let angle = k as f32 / 24.0 * TAU;
            let [x, y] = [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ];
            [[x, y, low], [x, y, high]]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;
    use crate::render::scene::Scene;
    use crate::views::testing;
    /// A scene over `testing::GUIDED`, and a function that draws its `view` at `time`, paused or
    /// not.
    fn guided() -> (Scene, impl Fn(&mut Scene, View, f32, bool)) {
        let snapshot = demo(10.0, 8);
        let draw = move |scene: &mut Scene, view: View, time: f32, paused: bool| {
            scene.render(&RenderInput {
                paused,
                ..RenderInput::small(&snapshot, view, &Camera::default(), time)
            });
        };
        (Scene::with_rows(testing::GUIDED), draw)
    }

    #[test]
    fn stage_dt_is_zero_on_the_first_frame_when_paused_and_after_rewind() {
        let (mut scene, draw) = guided();
        let guide = View::at(0);
        draw(&mut scene, guide, 10.0, false);
        draw(&mut scene, guide, 10.1, false);
        // Paused, the time is the shown sample's, which moves either way; the clock stands still.
        draw(&mut scene, guide, 10.4, true);
        draw(&mut scene, guide, 9.0, true);
        draw(&mut scene, guide, 10.2, false);
        // A rewind, or a headless replay, restarts the count from the earlier time.
        draw(&mut scene, guide, 8.0, false);
        draw(&mut scene, guide, 8.25, false);
        assert_eq!(
            scene.legend(guide, &Snapshot::default()),
            " guide: dt [0.00 0.10 0.00 0.00 0.10 0.00 0.25]"
        );
    }

    #[test]
    fn the_stage_counts_frames_and_hands_over_the_splashes_and_the_newborn() {
        let (mut scene, draw) = guided();
        let (guide, probe) = (View::at(0), View::at(1));
        draw(&mut scene, guide, 10.0, false);
        draw(&mut scene, probe, 10.1, false);
        draw(&mut scene, guide, 10.2, false);
        let sample = demo(10.0, 8);
        let mut fewer = sample.clone();
        let gone = fewer.processes.remove(3);
        let render = |scene: &mut Scene, snapshot: &Snapshot, time: f32| {
            scene.render(&RenderInput::small(
                snapshot,
                guide,
                &Camera::default(),
                time,
            ));
        };
        render(&mut scene, &fewer, 10.3);
        // The exit is noticed after the frame it left, so the next frame has its splash.
        render(&mut scene, &fewer, 10.4);
        let mut more = fewer.clone();
        let mut arrival = more.processes[0].clone();
        arrival.id.pid += 10_000;
        more.processes.push(arrival);
        render(&mut scene, &more, 10.5);
        let seen = scene
            .module::<testing::Guide>(guide)
            .expect("the guide's state");
        assert_eq!(seen.frames, [1, 3, 4, 5, 6], "the probe's frame is counted");
        assert_eq!(
            seen.disturbances,
            [(0, 0), (0, 0), (0, 0), (1, 0), (0, 1)],
            "{gone:?} splashed once and one process arrived"
        );
    }

    #[test]
    fn stage_dt_is_capped() {
        let (mut scene, draw) = guided();
        let guide = View::at(0);
        for time in [10.0, 13.0, 13.2, 100.0] {
            draw(&mut scene, guide, time, false);
        }
        assert_eq!(
            scene.legend(guide, &Snapshot::default()),
            format!(" guide: dt [0.00 {MAX_DT:.2} 0.20 {MAX_DT:.2}]")
        );
        assert!((MAX_DT - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn a_view_shown_again_steps_one_frame_not_the_time_it_was_hidden() {
        let (mut scene, draw) = guided();
        let (guide, probe) = (View::at(0), View::at(1));
        draw(&mut scene, guide, 20.0, false);
        for frame in 1..=40 {
            draw(&mut scene, probe, 20.0 + frame as f32 * 0.1, false);
        }
        draw(&mut scene, guide, 24.1, false);
        assert_eq!(
            scene.legend(guide, &Snapshot::default()),
            " guide: dt [0.00 0.10]"
        );
    }
}
