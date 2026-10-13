use std::f32::consts::TAU;

use super::camera::{Camera, ORIGIN_Y, SCALE};
use super::frame::{Frame, Item};
use super::{Color, MIN_BODY_PX, NONE, NVIDIA, Point, WARM};
use crate::medium::Grid;
use crate::model::{Process, bounded};
use crate::palette::tint;
use crate::views::systems::Orbit;

/// The longest piece of a `Frame::hairline` one pair of triangles covers, in pixels.
const HAIRLINE_PIECE: f32 = 8.0;

impl Frame {
    pub(crate) fn project(&self, camera: &Camera, p: Point) -> Point {
        let v = camera.view(p);
        [
            self.width as f32 * 0.5 + v[0] * camera.zoom,
            self.height as f32 * ORIGIN_Y + v[1] * camera.zoom,
            v[2],
        ]
    }

    pub(crate) fn facet(&mut self, camera: &Camera, points: [Point; 3], color: Color, pick: u32) {
        let p = points.map(|p| self.project(camera, p));
        self.items.push(Item::Triangle(p, color, pick));
    }

    pub(crate) fn quad(&mut self, camera: &Camera, points: [Point; 4], color: Color, pick: u32) {
        let p = points.map(|p| self.project(camera, p));
        self.items
            .push(Item::Triangle([p[0], p[1], p[2]], color, pick));
        self.items
            .push(Item::Triangle([p[0], p[2], p[3]], color, pick));
    }

    pub(crate) fn line(&mut self, camera: &Camera, a: Point, b: Point, color: Color) {
        let a = self.project(camera, a);
        let b = self.project(camera, b);
        // Skip unbounded work when the camera is far inside a large scene.
        if (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) <= 12000.0 {
            self.items.push(Item::Line(a, b, color));
        }
    }

    pub(crate) fn trace(&mut self, camera: &Camera, orbit: &Orbit, color: Color) {
        let mut last = orbit.point(0.0);
        for segment in 1..=72 {
            let next = orbit.point(segment as f32 / 72.0 * TAU);
            self.line(camera, last, next, color);
            last = next;
        }
    }

    pub(crate) fn glow(
        &mut self,
        camera: &Camera,
        position: Point,
        radius: f32,
        color: Color,
        strength: f32,
    ) {
        let p = self.project(camera, position);
        self.items.push(Item::Glow {
            center: [p[0], p[1]],
            radius,
            color,
            strength,
        });
    }

    pub(crate) fn beam(&mut self, camera: &Camera, a: Point, b: Point, color: Color, weight: f32) {
        let a = self.project(camera, a);
        let b = self.project(camera, b);
        if (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) <= 12000.0 {
            self.items
                .push(Item::Beam([a[0], a[1]], [b[0], b[1]], color, weight));
        }
    }

    /// A segment as a ribbon `width` pixels wide, in pieces of at most `HAIRLINE_PIECE` pixels so
    /// no triangle scans a large box. Like a line it is depth-tested and leaves nothing pickable
    /// where it is drawn; unlike a one-pixel line or beam, its triangles rasterize alike on the
    /// CPU and the GPU.
    pub(crate) fn hairline(
        &mut self,
        camera: &Camera,
        a: Point,
        b: Point,
        color: Color,
        width: f32,
    ) {
        let a = self.project(camera, a);
        let b = self.project(camera, b);
        self.hairline_between(a, b, color, width);
    }

    /// `hairline` between two points already in frame space (x, y, depth), for a view that draws
    /// in screen space, such as a lensed image, and sets the depth of its layers itself.
    pub(crate) fn hairline_between(&mut self, a: Point, b: Point, color: Color, width: f32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = dx.hypot(dy);
        if !(1e-3..=12000.0).contains(&length) {
            return;
        }
        let half = width * 0.5;
        let (nx, ny) = (-dy / length * half, dx / length * half);
        let pieces = (length / HAIRLINE_PIECE).ceil() as usize;
        let at = |t: f32| [a[0] + dx * t, a[1] + dy * t, a[2] + (b[2] - a[2]) * t];
        for piece in 0..pieces {
            let p = at(piece as f32 / pieces as f32);
            let q = at((piece + 1) as f32 / pieces as f32);
            let corners = [
                [p[0] + nx, p[1] + ny, p[2]],
                [q[0] + nx, q[1] + ny, q[2]],
                [q[0] - nx, q[1] - ny, q[2]],
                [p[0] - nx, p[1] - ny, p[2]],
            ];
            self.items.push(Item::Triangle(
                [corners[0], corners[1], corners[2]],
                color,
                NONE,
            ));
            self.items.push(Item::Triangle(
                [corners[0], corners[2], corners[3]],
                color,
                NONE,
            ));
        }
    }

    pub(crate) fn sphere(
        &mut self,
        camera: &Camera,
        position: Point,
        radius: f32,
        color: Color,
        pick: u32,
        selected: bool,
    ) {
        let center = self.project(camera, position);
        self.items.push(Item::Sphere {
            center,
            radius: (radius * camera.zoom).max(MIN_BODY_PX),
            depth: radius,
            color,
            pick,
            selected,
        });
    }

    /// A pick-only disc of `radius` pixels at the projection of `position`, depth-tested there:
    /// it keeps splatted or translucent content under it pickable.
    #[cfg_attr(not(test), expect(dead_code, reason = "first used by a view"))]
    pub(crate) fn pick_disc(&mut self, camera: &Camera, position: Point, radius: f32, pick: u32) {
        let center = self.project(camera, position);
        self.items.push(Item::Pick {
            center: [center[0], center[1]],
            radius,
            depth: center[2],
            pick,
        });
    }

    /// The silhouette of a sphere `sphere` would draw, as a ring of lines facing the viewer with
    /// a fainter ring inside it: a body drawn hollow. Lines draw alike in both rasterizers and,
    /// like every line, leave nothing pickable.
    pub(crate) fn outline(&mut self, camera: &Camera, position: Point, radius: f32, color: Color) {
        let center = self.project(camera, position);
        let outer = (radius * camera.zoom).max(MIN_BODY_PX);
        for (scale, shade) in [(1.0, 1.0), (0.6, 0.45)] {
            let point = |k: usize| {
                let angle = k as f32 / 48.0 * TAU;
                [
                    center[0] + outer * scale * angle.cos(),
                    center[1] + outer * scale * angle.sin(),
                    center[2],
                ]
            };
            for k in 0..48 {
                self.items
                    .push(Item::Line(point(k), point(k + 1), tint(color, shade)));
            }
        }
    }

    /// A sphere at its exact projected size, for large bodies with geometry drawn on their
    /// surface. `sphere` draws bodies at `radius * zoom` pixels, about 18% smaller than their
    /// projection, which the other views' spacing was tuned with.
    pub(crate) fn world_sphere(
        &mut self,
        camera: &Camera,
        position: Point,
        radius: f32,
        color: Color,
    ) {
        let center = self.project(camera, position);
        self.items.push(Item::Sphere {
            center,
            radius: radius * camera.zoom * SCALE,
            depth: radius,
            color,
            pick: NONE,
            selected: false,
        });
    }
}

/// Media survive small layout changes; a different footprint starts a fresh simulation.
pub(crate) fn similar(a: &Grid, b: &Grid) -> bool {
    a.columns == b.columns
        && a.rows == b.rows
        && (a.cell / b.cell - 1.0).abs() < 0.05
        && (a.origin[0] - b.origin[0]).abs() < a.cell * 2.0
        && (a.origin[1] - b.origin[1]).abs() < a.cell * 2.0
}

/// The ground-plane grid covering points with a margin and about `cells` cells along the longer
/// side, or None for no points: the water of Ripple and the sheet of Flow.
pub(crate) fn cover(located: &[[f32; 2]], cells: usize) -> Option<Grid> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for p in located {
        for k in 0..2 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    if located.is_empty() {
        return None;
    }
    let margin = 8.0 + 0.08 * (max[0] - min[0]).max(max[1] - min[1]);
    Some(Grid::covering(
        [min[0] - margin, min[1] - margin],
        [max[0] + margin, max[1] + margin],
        cells,
    ))
}

/// Warm CPU glow and green NVIDIA glow around a body, as in the orbit view.
pub(crate) fn glows(frame: &mut Frame, camera: &Camera, process: &Process, at: Point, radius: f32) {
    let core = (radius * camera.zoom).max(MIN_BODY_PX);
    if process.cpu > 1.0 {
        let glow = core * (2.2 + 2.0 * bounded(process.cpu, 50.0));
        frame.glow(
            camera,
            at,
            glow,
            WARM,
            0.3 + 0.6 * bounded(process.cpu, 40.0),
        );
    }
    if let Some(gpu) = process.gpu_memory {
        let mib = gpu.bytes() as f32 / 1048576.0;
        frame.glow(
            camera,
            at,
            core * (2.6 + 0.5 * (mib / 64.0).ln_1p()),
            NVIDIA,
            0.6,
        );
    }
}

/// Body radius from memory in bytes: volume proportional to memory, so a 1 GiB process is about
/// five times as wide as a 6 MiB one and idle kernel threads stay specks.
pub(crate) fn mass_radius(bytes: f32) -> f32 {
    (0.24 * (bytes / 1048576.0).max(0.0).cbrt()).clamp(0.25, 4.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::frame::blank;
    #[test]
    fn a_screen_space_hairline_draws_the_same_ribbon_as_a_world_one() {
        let camera = Camera::default();
        let (a, b) = ([0.0, 0.0, 0.0], [4.0, 1.0, 0.0]);
        let mut world = blank(100, 100);
        world.hairline(&camera, a, b, [200, 100, 50], 2.0);
        let mut screen = blank(100, 100);
        let (a, b) = (screen.project(&camera, a), screen.project(&camera, b));
        screen.hairline_between(a, b, [200, 100, 50], 2.0);
        assert!(!world.items.is_empty());
        assert_eq!(format!("{:?}", world.items), format!("{:?}", screen.items));
    }
}
