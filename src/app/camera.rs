//! The camera the user moves and the scene is drawn from: easing towards a goal, refitting the
//! view when it grows, and the globe's hold.

use crate::render::{Camera, Point, Scene};
use crate::simulation;

use super::App;

/// The time constant, in seconds, of the camera's easing towards its goal.
const EASING: f32 = 0.2;

/// How much a visit zooms in on a process, in fits: the zoom of a framed view times this.
const VISIT_ZOOM: f32 = 2.2;

/// The camera and the goal it eases towards, and whether it still frames the whole view.
pub(crate) struct CameraController {
    /// The camera the frame is drawn from.
    pub(crate) current: Camera,
    /// Where the camera is going; input and the tour move this one.
    pub(crate) goal: Camera,
    /// The camera that drew `Scene::positions`, which `current` has since moved on from.
    pub(crate) rendered: Camera,
    /// The goal is to be fitted to the whole view in the next frame.
    pub(super) fit: bool,
    /// The camera still frames the whole view, so views that grow are refitted as they do.
    pub(super) framed: bool,
    /// The next fit moves the camera to the goal at once, instead of easing there.
    snap: bool,
    /// The zoom of the last fit.
    pub(crate) fit_zoom: f32,
}

impl Default for CameraController {
    fn default() -> Self {
        Self {
            current: Camera::default(),
            goal: Camera::default(),
            rendered: Camera::default(),
            fit: true,
            framed: true,
            snap: true,
            fit_zoom: 5.0,
        }
    }
}

impl CameraController {
    /// Applies `change` to the camera and to its goal, so it takes effect at once.
    pub(crate) fn both(&mut self, change: impl Fn(&mut Camera)) {
        change(&mut self.current);
        change(&mut self.goal);
    }

    /// Eases the camera towards its goal over `elapsed` seconds.
    pub(crate) fn approach(&mut self, elapsed: f32) {
        self.current
            .approach(&self.goal, simulation::smoothing(elapsed, EASING));
    }

    /// Frames the whole view again in the next frame.
    pub(crate) fn refit(&mut self) {
        self.fit = true;
    }

    /// Whether the next frame refits the goal.
    #[cfg(test)]
    pub(crate) fn fit_pending(&self) -> bool {
        self.fit
    }

    /// Whether the camera still frames the whole view.
    pub(crate) fn is_framed(&self) -> bool {
        self.framed
    }

    /// After something moved the goal (`before` is how it stood): moving the camera by hand
    /// stops it following the view, and fitting it again resumes.
    pub(crate) fn follow(&mut self, before: &Camera) {
        if self.fit {
            self.framed = true;
        } else if self.goal != *before {
            self.framed = false;
        }
    }

    /// Fits the goal to the view `scene` drew, if a fit is due, and says whether the camera is
    /// to snap to it: the frame is then drawn again from the goal (see `snap_to_goal`).
    pub(crate) fn fit_to(&mut self, scene: &Scene, width: u32, height: u32) -> bool {
        if !self.fit {
            return false;
        }
        scene.fit(&mut self.goal, width, height);
        self.fit_zoom = self.goal.zoom;
        self.fit = false;
        std::mem::take(&mut self.snap)
    }

    /// Puts the camera on its goal.
    pub(crate) fn snap_to_goal(&mut self) {
        self.current = self.goal.clone();
    }

    /// Notes that a frame was drawn from the camera as it stands.
    pub(crate) fn drawn(&mut self) {
        self.rendered = self.current.clone();
    }

    /// The default camera, at `pitch` if the view has one, easing in from where the camera is.
    /// A held globe is released first: the camera must stop being held before it eases to the
    /// default camera, or it would copy the goal's release and spin a second time.
    pub(crate) fn reset(&mut self, time: f32, pitch: Option<f32>) {
        self.current.release(time);
        self.goal = Camera::default();
        if let Some(pitch) = pitch {
            self.goal.pitch = pitch;
        }
        self.fit = true;
    }

    /// Zooms the goal in on a process to visit, from the zoom of the last fit.
    pub(crate) fn zoom_to_visit(&mut self) {
        self.goal.zoom = self.fit_zoom * VISIT_ZOOM;
    }
}

impl App {
    /// Lets a held globe turn with time again from where it stands, and stands it upright.
    /// Does nothing unless the earth was held, or in a view with no sphere to turn, where a
    /// tour must leave the hold for the next visit to the globe.
    pub(super) fn release_globe(&mut self) {
        if self.row().turn.is_none() {
            return;
        }
        let time = self.render_time();
        self.camera.both(|c| c.release(time));
        self.camera.goal.globe_tilt = 0.0;
    }

    /// In a view whose row can face a point, turns the camera so `point` is on the near side
    /// and centres the point itself. On the globe the world spins, so a process above home is
    /// behind the earth for half of every turn, and it floats far above the ground the camera
    /// otherwise centres on.
    ///
    /// The point was drawn by `rendered`, whose lean the camera is still easing away from, so
    /// on a sphere the wheel can lean the goal aims at where the point will be once the sphere
    /// stands upright.
    pub(super) fn face(&mut self, point: Point) {
        let Some(face) = self.row().face else {
            return;
        };
        let point = match self.row().turn {
            Some(turn) => {
                self.camera.goal.globe_tilt = 0.0;
                (turn.upright)(point, &self.camera.rendered)
            }
            None => point,
        };
        (self.camera.goal.rotation, self.camera.goal.pitch) = face(point, &self.camera.goal);
        self.camera.goal.look_at(point);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;

    #[test]
    fn only_the_first_fit_snaps_the_camera() {
        let scene = Scene::new();
        let mut camera = CameraController::default();
        assert!(camera.fit_pending());
        assert!(camera.fit_to(&scene, 640, 360), "the first fit snaps");
        assert!(!camera.fit_pending());
        assert!(!camera.fit_to(&scene, 640, 360), "nothing is due");
        camera.refit();
        assert!(!camera.fit_to(&scene, 640, 360), "later fits ease");
    }

    #[test]
    fn moving_the_goal_by_hand_stops_the_camera_following_the_view_until_it_is_fitted_again() {
        let mut camera = CameraController::default();
        camera.fit_to(&Scene::new(), 640, 360);
        let before = camera.goal.clone();
        camera.follow(&before);
        assert!(camera.is_framed(), "nothing moved");
        camera.goal.pan(5.0, 0.0);
        camera.follow(&before);
        assert!(!camera.is_framed());
        camera.refit();
        camera.follow(&before);
        assert!(camera.is_framed(), "a fit is due, so it frames again");
    }

    #[test]
    fn reset_eases_in_from_where_the_camera_is_at_the_views_pitch() {
        let mut camera = CameraController::default();
        camera.goal.pan(30.0, 0.0);
        camera.current.pan(30.0, 0.0);
        camera.fit_to(&Scene::new(), 640, 360);
        camera.reset(0.0, Some(0.5));
        assert_eq!(camera.goal.center, Camera::default().center);
        assert_eq!(camera.goal.pitch, 0.5);
        assert_ne!(camera.current.center, camera.goal.center, "it eases there");
        assert!(camera.fit_pending());
    }

    #[test]
    fn approach_moves_the_camera_towards_the_goal_and_never_past_it() {
        let mut camera = CameraController::default();
        camera.goal.zoom = camera.current.zoom * 4.0;
        let start = camera.current.zoom;
        camera.approach(0.1);
        assert!(camera.current.zoom > start && camera.current.zoom < camera.goal.zoom);
        for _ in 0..200 {
            camera.approach(0.5);
        }
        assert!((camera.current.zoom - camera.goal.zoom).abs() < 1e-3);
    }

    #[test]
    fn camera_follows_growing_views_until_moved_by_hand() {
        let mut app = App::new(view("cells"), model::demo(1.0, 64), all_available());
        app.render(320, 180, 512);
        assert!(app.camera.is_framed() && app.scene.bounded());
        app.key(KeyCode::Left, KeyModifiers::NONE, 10.0);
        assert!(!app.camera.is_framed(), "panning takes over the camera");
        app.key(KeyCode::Char('8'), KeyModifiers::NONE, 10.0);
        assert!(
            app.camera.is_framed() && app.view == view("globe"),
            "switching view frames it again"
        );
    }

    #[test]
    fn a_turned_globe_holds_still_until_reset_and_then_spins_on_without_a_jump() {
        let mut app = globe_at(10.0);
        // Holding the earth freezes it where it is, however far the clock has run.
        let free = home_on_screen(&mut app);
        app.camera.both(|c| c.hold(10.0));
        let frozen = home_on_screen(&mut app);
        assert!(
            (free[0] - frozen[0]).abs() < 0.01 && (free[1] - frozen[1]).abs() < 0.01,
            "holding jumped {free:?} to {frozen:?}"
        );
        app.camera.both(|c| c.release(10.0));
        scroll(&mut app, MouseEventKind::ScrollRight, 3);
        scroll(&mut app, MouseEventKind::ScrollDown, 2);
        let held = home_on_screen(&mut app);
        for time in [40.0, 150.0] {
            app.animation = time;
            let later = home_on_screen(&mut app);
            assert!(
                (held[0] - later[0]).abs() < 0.01 && (held[1] - later[1]).abs() < 0.01,
                "the held earth moved by {held:?} to {later:?} at {time} s"
            );
        }
        // r renders straight after the key, before the camera has eased anywhere.
        app.animation = 150.0;
        app.key(KeyCode::Char('r'), KeyModifiers::NONE, 10.0);
        assert!(!app.camera.current.globe_held && !app.camera.goal.globe_held);
        let straight_after = home_on_screen(&mut app);
        assert!(
            (held[0] - straight_after[0]).abs() < 0.01
                && (held[1] - straight_after[1]).abs() < 0.01,
            "r made the earth jump from {held:?} to {straight_after:?}"
        );
        // Then the camera eases back to the default while the earth turns on.
        let mut last = straight_after;
        for frame in 1..=120 {
            let elapsed = 1.0 / 20.0;
            app.animation += elapsed;
            let goal = app.camera.goal.clone();
            app.camera
                .current
                .approach(&goal, 1.0 - (-elapsed / 0.2_f32).exp());
            let now = home_on_screen(&mut app);
            let jump = (now[0] - last[0]).hypot(now[1] - last[1]);
            assert!(jump < 100.0, "frame {frame} jumped {jump} px");
            last = now;
        }
        assert!(app.camera.current.globe_tilt.abs() < 1e-3);
        assert!(
            (app.camera.current.globe_turn.rem_euclid(TAU) - app.camera.goal.globe_turn).abs()
                < 1e-3
                || (app.camera.current.globe_turn.rem_euclid(TAU) - TAU).abs() < 1e-3
        );
        // It is the globe a new session shows at that time: nothing doubled the spin.
        let mut fresh = globe_at(app.animation);
        let (a, b) = (home_on_screen(&mut app), home_on_screen(&mut fresh));
        assert!(
            (a[0] - b[0]).abs() < 2.0 && (a[1] - b[1]).abs() < 2.0,
            "{a:?} {b:?}"
        );
    }

    #[test]
    fn holding_and_releasing_while_paused_or_rewound_does_not_jump() {
        let mut app = globe_at(0.0);
        app.history.samples_mut().push_back(model::demo(50.0, 64));
        app.history.samples_mut().push_back(model::demo(100.0, 64));
        app.animation = 500.0;
        // [ pauses and steps back to the sample at 50 s, whatever the animation clock says.
        app.key(KeyCode::Char('['), KeyModifiers::NONE, 10.0);
        assert!(app.history.is_paused());
        assert_eq!(app.render_time(), 50.0);
        let free = home_on_screen(&mut app);
        let time = app.render_time();
        app.camera.both(|c| c.hold(time));
        let held = home_on_screen(&mut app);
        assert!(
            (free[0] - held[0]).abs() < 0.01 && (free[1] - held[1]).abs() < 0.01,
            "holding jumped {free:?} to {held:?}"
        );
        scroll(&mut app, MouseEventKind::ScrollRight, 2);
        let turned = home_on_screen(&mut app);
        assert!(turned[0] < held[0], "the scroll turned it");
        // g releases it, and the earth stays where the hand left it.
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        assert!(!app.camera.current.globe_held);
        let released = home_on_screen(&mut app);
        assert!(
            (turned[0] - released[0]).abs() < 0.01 && (turned[1] - released[1]).abs() < 0.01,
            "releasing jumped {turned:?} to {released:?}"
        );
        // Rewinding further carries the free earth with the sample time, not the clock.
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        app.key(KeyCode::Char('['), KeyModifiers::NONE, 10.0);
        assert_eq!(app.render_time(), 1.0);
    }
}
