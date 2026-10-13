pub(crate) use camera::SCALE;
pub use camera::{Camera, ISOMETRIC, LOWEST_PITCH, ORIGIN_Y};
pub(crate) use draw::{cover, glows, mass_radius, similar};
pub use frame::{Backdrop, Buffers, Card, Frame, Item, Sky, THING, Target, Thing};
pub use scene::{Scene, label_name, without_apple_prefix};
pub use stage::{RenderInput, Title, TourTarget};
pub(crate) use stage::{Stage, ring_bounds};
mod camera;
mod draw;
pub mod field;
mod frame;
mod raster;
mod scene;
pub mod shared;
mod stage;
pub(crate) mod strokes;

pub type Color = [u8; 3];
pub type Point = [f32; 3];
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "first used by a view that draws a field")
)]
pub use field::{Field, Placement, Relief, Shading};
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "first used by a view that draws a field")
)]
pub use shared::LIGHT;
pub use shared::NONE;
pub(crate) const WARM: Color = [255, 190, 110];
pub(crate) const NVIDIA: Color = [118, 214, 60];
pub(crate) const OUTSIDE: Color = [255, 110, 200];
pub(crate) const MIN_BODY_PX: f32 = 3.5;

/// Which socket links to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Links {
    All,
    Focused,
    Off,
}

impl Links {
    pub fn cycle(&mut self) {
        *self = match self {
            Self::All => Self::Focused,
            Self::Focused => Self::Off,
            Self::Off => Self::All,
        };
    }
}

/// Shared checks that a view keeps its layout while focus or the process limit hides processes.
#[cfg(test)]
pub(crate) mod hiding {
    use std::collections::HashMap;

    use super::*;
    use crate::model::{Identity, Process, Snapshot};
    use crate::views::View;

    /// Records one small frame of `view` and returns where each drawn process stands.
    pub fn place(
        scene: &mut Scene,
        snapshot: &Snapshot,
        view: View,
        time: f32,
        limit: usize,
        selected: Option<Identity>,
        focus: Option<Identity>,
    ) -> HashMap<Identity, Point> {
        let frame = scene.render(&RenderInput {
            width: 160,
            height: 90,
            selected,
            limit,
            focus,
            ..RenderInput::small(snapshot, view, &Camera::default(), time)
        });
        scene.spare = frame.release();
        scene.positions.clone()
    }

    /// `view` after five seconds of frames, so whatever moves has left the spot it began at;
    /// returns the scene, the time of its last frame and where everyone stands. The families
    /// of sixteen of `model::demo` arrive one by one, the last first, so the layout has a
    /// history, as a live one does: placing everything again at once, largest first, would
    /// not land where it is.
    pub fn settled(view: View, sample: &Snapshot) -> (Scene, f32, HashMap<Identity, Point>) {
        let family = |process: &Process| process.id.pid.saturating_sub(1000) as usize / 16;
        let families = sample
            .processes
            .iter()
            .map(family)
            .max()
            .map_or(0, |f| f + 1);
        let mut scene = Scene::new();
        let mut time = 30.0;
        let mut places = HashMap::new();
        for step in 0..50 {
            let arrived = (1 + step * families / 45).min(families);
            let mut partial = sample.clone();
            partial
                .processes
                .retain(|process| family(process) >= families - arrived);
            // Each frame is a sample of its own, ending at the sample itself.
            partial.elapsed = sample.elapsed - (49 - step) as f64 * 0.1;
            places = place(&mut scene, &partial, view, time, 4096, None, None);
            time += 0.1;
        }
        (scene, time - 0.1, places)
    }

    /// Asserts that every process drawn now stands where it stood in `before`.
    pub fn in_place(now: &HashMap<Identity, Point>, before: &HashMap<Identity, Point>) {
        for (id, point) in now {
            assert_eq!(before.get(id), Some(point), "{id:?}");
        }
    }

    /// Focuses one family of sixteen of `model::demo` and clears the focus again, repeating the
    /// last frame's time so nothing steps and anything that moves was moved by the change of
    /// focus. The family is the first with a drawn member after init's, which holds everyone.
    pub fn focus_and_clear(view: View, sample: &Snapshot) {
        focus_and_clear_by(view, sample, |point| point);
    }

    /// As `focus_and_clear`, comparing places by `by`: the part of a position that is layout,
    /// for views whose medium also lifts what floats on it.
    pub fn focus_and_clear_by(view: View, sample: &Snapshot, by: fn(Point) -> Point) {
        let measure = |places: HashMap<Identity, Point>| -> HashMap<Identity, Point> {
            places
                .into_iter()
                .map(|(id, point)| (id, by(point)))
                .collect()
        };
        let (mut scene, time, before) = settled(view, sample);
        let before = measure(before);
        let first = before
            .keys()
            .filter(|id| id.pid >= 1016)
            .min()
            .expect("a process outside init's family is drawn");
        let root = sample.processes[(first.pid as usize - 1000) / 16 * 16].id;
        let focused = measure(place(
            &mut scene,
            sample,
            view,
            time,
            4096,
            None,
            Some(root),
        ));
        assert!(!focused.is_empty() && focused.len() < before.len());
        in_place(&focused, &before);
        let cleared = measure(place(&mut scene, sample, view, time, 4096, None, None));
        assert_eq!(cleared, before);
    }

    /// Cuts the population in half with the limit, swaps the last process drawn before it
    /// in across it as a selection, and lifts the limit again, at one time as above.
    pub fn limit_and_lift(view: View, sample: &Snapshot) {
        limit_and_lift_by(view, sample, |point| point);
    }

    /// As `limit_and_lift`, comparing places by `by` as `focus_and_clear_by` does.
    pub fn limit_and_lift_by(view: View, sample: &Snapshot, by: fn(Point) -> Point) {
        let measure = |places: HashMap<Identity, Point>| -> HashMap<Identity, Point> {
            places
                .into_iter()
                .map(|(id, point)| (id, by(point)))
                .collect()
        };
        let (mut scene, time, before) = settled(view, sample);
        let before = measure(before);
        let half = sample.processes.len() / 2;
        let limited = measure(place(&mut scene, sample, view, time, half, None, None));
        assert!(!limited.is_empty() && limited.len() < before.len());
        in_place(&limited, &before);
        let selected = *before.keys().max().expect("something is drawn");
        assert!(!limited.contains_key(&selected));
        let swapped = measure(place(
            &mut scene,
            sample,
            view,
            time,
            half,
            Some(selected),
            None,
        ));
        assert!(swapped.contains_key(&selected));
        in_place(&swapped, &before);
        let lifted = measure(place(&mut scene, sample, view, time, 4096, None, None));
        assert_eq!(lifted, before);
    }

    /// Asserts that the envelope `view` gives the camera fit does not depend on what is drawn:
    /// hiding all but one family, or half the population, leaves it as the whole population
    /// had it, at one time as above. Only for views whose envelope is fixed by their layout.
    pub fn bounds_ignore_hiding(view: View, sample: &Snapshot) {
        let (mut scene, time, before) = settled(view, sample);
        let whole = scene.bounds.clone();
        assert!(!whole.is_empty(), "the view gives explicit bounds");
        let root = sample.processes[16].id;
        assert!(before.contains_key(&root));
        place(&mut scene, sample, view, time, 4096, None, Some(root));
        assert_eq!(scene.bounds, whole, "focused");
        place(
            &mut scene,
            sample,
            view,
            time,
            sample.processes.len() / 2,
            None,
            None,
        );
        assert_eq!(scene.bounds, whole, "limited");
    }

    /// Cuts the population in half with the limit and lifts it again, comparing places by
    /// `by`. A selection swapped in across the limit without its parent is drawn as a system
    /// of its own, so it has no place to keep and is left out.
    pub fn limit_and_lift_without_selection(view: View, by: fn(Point) -> Point) {
        let sample = crate::model::demo(30.0, 160);
        let measure = |places: HashMap<Identity, Point>| -> HashMap<Identity, Point> {
            places
                .into_iter()
                .map(|(id, point)| (id, by(point)))
                .collect()
        };
        let (mut scene, time, before) = settled(view, &sample);
        let before = measure(before);
        let limited = measure(place(&mut scene, &sample, view, time, 80, None, None));
        assert!(!limited.is_empty() && limited.len() < before.len());
        in_place(&limited, &before);
        let lifted = measure(place(&mut scene, &sample, view, time, 4096, None, None));
        assert_eq!(lifted, before);
    }

    /// A position on the ground plane, without what a medium lifts it by.
    pub fn ground([x, y, _]: Point) -> Point {
        [x, y, 0.0]
    }
}
