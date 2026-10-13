use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use super::Point;
use crate::globe;

/// Screen pixels per world unit at zoom 1: sqrt(3/2), the isometric foreshortening.
pub(crate) const SCALE: f32 = 1.224_745;
/// Elevation of a true isometric view, asin(1/sqrt(3)).
pub const ISOMETRIC: f32 = 0.615_480;
/// The lowest camera elevation allowed: the camera never looks from below the ground.
pub const LOWEST_PITCH: f32 = 0.35;
/// The camera centre sits slightly below the middle to leave headroom for buildings.
pub const ORIGIN_Y: f32 = 0.57;

#[derive(Clone, PartialEq)]
pub struct Camera {
    pub center: [f32; 2],
    pub zoom: f32,
    pub rotation: f32,
    /// Elevation above the ground plane: isometric by default, top-down at pi/2.
    pub pitch: f32,
    /// Globe only: how far the earth has been turned by hand about its axis, in radians.
    pub globe_turn: f32,
    /// Globe only: how far the earth leans about the screen-horizontal axis, in radians;
    /// positive brings the southern hemisphere up. Used clamped to the camera's pitch +- pi/2,
    /// because the pitch changes without touching it.
    pub globe_tilt: f32,
    /// Globe only: the earth holds still at `globe_turn` instead of turning with time.
    pub globe_held: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            zoom: 5.0,
            rotation: 0.0,
            pitch: ISOMETRIC,
            globe_turn: 0.0,
            globe_tilt: 0.0,
            globe_held: false,
        }
    }
}

impl Camera {
    /// Orthographic view space: screen x and y offsets before zoom, and depth (larger is nearer).
    pub(super) fn view(&self, p: Point) -> Point {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        let x = p[0] - self.center[0];
        let y = p[1] - self.center[1];
        let across = x * c - y * s;
        let along = x * s + y * c;
        let (ps, pc) = self.pitch.sin_cos();
        [
            across * SCALE,
            (along * ps - p[2] * pc) * SCALE,
            along * pc + p[2] * ps,
        ]
    }

    /// Ground-plane offset that appears at a screen offset of (dx, dy) frame pixels.
    pub(crate) fn ground(&self, dx: f32, dy: f32) -> [f32; 2] {
        let across = dx / (SCALE * self.zoom);
        let along = dy / (SCALE * self.zoom * self.pitch.sin());
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        [across * c + along * s, along * c - across * s]
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        let [x, y] = self.ground(dx, dy);
        self.center[0] += x;
        self.center[1] += y;
    }

    pub fn scale(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.3, 65.0);
    }

    /// Zooms while keeping the ground point under the screen offset (dx, dy) in place.
    pub fn scale_at(&mut self, factor: f32, dx: f32, dy: f32) {
        let before = self.ground(dx, dy);
        self.scale(factor);
        let after = self.ground(dx, dy);
        self.center[0] += before[0] - after[0];
        self.center[1] += before[1] - after[1];
    }

    pub fn tilt(&mut self, delta: f32) {
        self.pitch = (self.pitch + delta).clamp(LOWEST_PITCH, FRAC_PI_2);
    }

    /// Unit vector from the scene towards the viewer, for specular highlights.
    pub(crate) fn viewer(&self) -> Point {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        let (ps, pc) = self.pitch.sin_cos();
        [s * pc, c * pc, ps]
    }

    /// Centres the view on `p` itself rather than the ground below it: a raised point appears
    /// above its ground point by its height over tan(pitch) along the view direction.
    pub fn look_at(&mut self, p: Point) {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        let along = p[2] / self.pitch.tan();
        self.center = [p[0] - along * s, p[1] - along * c];
    }

    /// The ground point `distance` world units from `p` towards the viewer (lowest on screen).
    pub fn front(&self, p: Point, distance: f32) -> Point {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        [p[0] + distance * s, p[1] + distance * c, p[2]]
    }

    /// Moves `amount` of the way towards `goal`, turning the short way round.
    pub fn approach(&mut self, goal: &Camera, amount: f32) {
        for i in 0..2 {
            self.center[i] += (goal.center[i] - self.center[i]) * amount;
        }
        self.zoom *= (goal.zoom / self.zoom).powf(amount);
        self.rotation += ((goal.rotation - self.rotation + PI).rem_euclid(TAU) - PI) * amount;
        self.pitch += (goal.pitch - self.pitch) * amount;
        self.globe_turn += ((goal.globe_turn - self.globe_turn + PI).rem_euclid(TAU) - PI) * amount;
        self.globe_tilt += (goal.globe_tilt - self.globe_tilt) * amount;
        self.globe_held = goal.globe_held;
    }

    /// Globe only: freezes the earth where it is at `time`, folding the turn it has made with
    /// time into `globe_turn`, so nothing moves on screen. Does nothing if already held.
    pub fn hold(&mut self, time: f32) {
        if !self.globe_held {
            self.globe_turn = (self.globe_turn + globe::drift(time)).rem_euclid(TAU);
            self.globe_held = true;
        }
    }

    /// Globe only: lets a held earth turn with time again from where it stands at `time`.
    /// Does nothing unless held.
    pub fn release(&mut self, time: f32) {
        if self.globe_held {
            self.globe_turn = (self.globe_turn - globe::drift(time)).rem_euclid(TAU);
            self.globe_held = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::frame::blank;
    #[test]
    fn approach_turns_the_globe_the_short_way_leans_it_linearly_and_follows_the_hold() {
        let mut camera = Camera {
            globe_turn: 0.1,
            globe_tilt: 1.0,
            ..Camera::default()
        };
        let goal = Camera {
            globe_turn: TAU - 0.1,
            globe_tilt: 0.0,
            globe_held: true,
            ..Camera::default()
        };
        camera.approach(&goal, 0.5);
        assert!(camera.globe_turn.abs() < 1e-5, "{}", camera.globe_turn);
        assert!((camera.globe_tilt - 0.5).abs() < 1e-6);
        assert!(camera.globe_held);
        camera.approach(&Camera::default(), 1.0);
        assert!(!camera.globe_held && camera.globe_tilt == 0.0);
    }

    #[test]
    fn holding_and_releasing_change_nothing_unless_the_state_changes() {
        let mut camera = Camera::default();
        camera.release(40.0);
        assert!(
            camera == Camera::default(),
            "releasing a free earth is a no-op"
        );
        camera.hold(40.0);
        let held = camera.clone();
        assert!(held.globe_held);
        camera.hold(90.0);
        assert!(camera == held, "holding again must not fold the turn twice");
        camera.release(40.0);
        assert!(
            (camera.globe_turn.rem_euclid(TAU)).abs() < 1e-4
                || (camera.globe_turn - TAU).abs() < 1e-4
        );
        assert!(!camera.globe_held);
    }

    #[test]
    fn default_camera_is_isometric_and_ground_inverts_the_projection() {
        let v = Camera::default().view([1.0, 0.0, 0.0]);
        assert!((v[0] - 0.866).abs() < 1e-3 && (v[1] - 0.5).abs() < 1e-3);
        assert!((Camera::default().view([0.0, 0.0, 1.0])[1] + 1.0).abs() < 1e-3);
        for (rotation, pitch) in [(0.0, ISOMETRIC), (1.3, 0.5), (-2.0, FRAC_PI_2)] {
            let camera = Camera {
                center: [3.0, -2.0],
                zoom: 7.0,
                rotation,
                pitch,
                ..Camera::default()
            };
            let q = camera.view([10.0, 4.0, 0.0]);
            let g = camera.ground(q[0] * camera.zoom, q[1] * camera.zoom);
            assert!(
                (g[0] - 7.0).abs() < 1e-3 && (g[1] - 6.0).abs() < 1e-3,
                "{g:?}"
            );
        }
    }

    #[test]
    fn the_ground_offset_of_a_screen_offset_projects_back_to_it() {
        let camera = Camera {
            rotation: 0.7,
            pitch: 0.9,
            zoom: 3.0,
            ..Camera::default()
        };
        let frame = blank(200, 120);
        let origin = frame.project(&camera, [camera.center[0], camera.center[1], 0.0]);
        let [x, y] = camera.ground(40.0, -25.0);
        let back = frame.project(&camera, [camera.center[0] + x, camera.center[1] + y, 0.0]);
        assert!((back[0] - origin[0] - 40.0).abs() < 1e-3);
        assert!((back[1] - origin[1] + 25.0).abs() < 1e-3);
    }
}
