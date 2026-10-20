//! Wide strokes made of ordinary triangles: `Frame::ribbon` (a screen-facing, optionally dashed
//! band along a polyline) and `Frame::tube` (a flat-shaded prism around one). Neither adds an
//! `Item` or touches a rasterizer, so the CPU and the GPU draw them alike.
//!
//! A triangle scans its whole bounding box on the CPU, so every piece is kept to at most
//! `PIECE` pixels along each of its two directions, which bounds its box at 64 x 64 pixels.

use std::f32::consts::TAU;

use super::{Camera, Color, Frame, Item, Point, SCALE, shared};
use crate::geometry::{dot, normalize};
use crate::palette::tint;

/// The longest edge of one piece, in pixels.
const PIECE: f32 = 32.0;
/// Segments whose projection is longer are skipped, as `Frame::line` skips them.
const REACH: f32 = 12000.0;
/// A ribbon narrower than this many pixels is drawn as one-pixel lines, which the CPU and the GPU
/// rasterize differently; a view whose ribbons must match on both keeps them at least this wide.
pub(crate) const THINNEST_RIBBON: f32 = 1.0;
/// A tube segment thinner than this radius in pixels is drawn as a one-pixel line.
const THINNEST_TUBE: f32 = 0.5;
/// A segment holding more dashes than this is drawn solid: they would be too fine to tell apart.
const MOST_DASHES: f32 = 4096.0;

/// How wide a ribbon is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Width {
    /// World units, scaled with the zoom: `w * zoom * SCALE` pixels.
    World(f32),
    /// Frame pixels, whatever the zoom.
    Pixels(f32),
}

/// A dash pattern in world units of arc length along the path. The pattern starts with an "on"
/// part at distance `-phase` from the first point, so a larger `phase` slides it back along the
/// path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dash {
    pub on: f32,
    pub off: f32,
    pub phase: f32,
}

impl Dash {
    /// The arc-length intervals inside `from..to` that the pattern draws.
    fn spans(self, from: f32, to: f32) -> Vec<(f32, f32)> {
        let period = self.on + self.off;
        if self.on.is_nan() || self.on <= 0.0 {
            return Vec::new();
        }
        if self.off.is_nan()
            || self.off <= 0.0
            || !period.is_finite()
            || (to - from) / period > MOST_DASHES
        {
            return vec![(from, to)];
        }
        let mut spans = Vec::new();
        let mut cycle = ((from + self.phase) / period).floor();
        loop {
            let start = cycle * period - self.phase;
            if start >= to {
                return spans;
            }
            let (low, high) = (start.max(from), (start + self.on).min(to));
            if high > low {
                spans.push((low, high));
            }
            cycle += 1.0;
        }
    }
}

/// The brightness of a face with outward unit `normal`: ambient 0.35, plus up to 0.65 from the
/// shared `LIGHT`.
pub(crate) fn lit(color: Color, normal: Point) -> Color {
    tint(color, 0.35 + 0.65 * dot(normal, shared::LIGHT).max(0.0))
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scaled(a: Point, factor: f32) -> Point {
    a.map(|v| v * factor)
}

fn lerp(a: Point, b: Point, t: f32) -> Point {
    add(a, scaled(sub(b, a), t))
}

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: Point) -> f32 {
    dot(a, a).sqrt()
}

/// Distance in the picture plane, which is what the splitting rule measures.
fn screen_distance(a: Point, b: Point) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The cross-section frame of a tube at one vertex: the unit tangent and two unit vectors
/// perpendicular to it and to each other.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ring {
    pub tangent: Point,
    pub across: Point,
    pub up: Point,
}

/// Turns `vector` by the smallest rotation that takes unit `from` to unit `to` (Rodrigues).
fn rotate_between(vector: Point, from: Point, to: Point) -> Point {
    let axis = cross(from, to);
    let sine = length(axis);
    let cosine = dot(from, to);
    if sine < 1e-6 {
        // Parallel keeps the vector; opposite tangents leave it perpendicular to both.
        return vector;
    }
    let axis = scaled(axis, 1.0 / sine);
    add(
        scaled(vector, cosine),
        add(
            scaled(cross(axis, vector), sine),
            scaled(axis, dot(axis, vector) * (1.0 - cosine)),
        ),
    )
}

/// One frame per point, carried along the path without twist: the first is built from the first
/// segment, and each later one is the previous turned by the smallest rotation between the
/// tangents. An interior tangent is the average of its two segments' directions; a segment of no
/// length takes its neighbour's direction.
pub(crate) fn ring_frames(points: &[Point]) -> Vec<Ring> {
    if points.len() < 2 {
        return Vec::new();
    }
    let mut directions: Vec<Option<Point>> = points
        .windows(2)
        .map(|pair| {
            let step = sub(pair[1], pair[0]);
            let size = length(step);
            (size > 1e-9).then(|| scaled(step, 1.0 / size))
        })
        .collect();
    let first = directions.iter().flatten().next().copied();
    let mut last = first.unwrap_or([0.0, 0.0, 1.0]);
    for direction in &mut directions {
        last = *direction.get_or_insert(last);
    }
    let directions: Vec<Point> = directions.into_iter().flatten().collect();
    let tangents: Vec<Point> = (0..points.len())
        .map(|i| {
            if i == 0 {
                directions[0]
            } else if i == points.len() - 1 {
                directions[i - 1]
            } else {
                let sum = add(directions[i - 1], directions[i]);
                if length(sum) > 1e-6 {
                    normalize(sum)
                } else {
                    directions[i - 1]
                }
            }
        })
        .collect();
    let reference = if tangents[0][2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let mut across = normalize(cross(reference, tangents[0]));
    let mut rings = Vec::with_capacity(points.len());
    for (i, &tangent) in tangents.iter().enumerate() {
        if i > 0 {
            across = rotate_between(across, tangents[i - 1], tangent);
            across = normalize(sub(across, scaled(tangent, dot(across, tangent))));
        }
        rings.push(Ring {
            tangent,
            across,
            up: cross(tangent, across),
        });
    }
    rings
}

/// One flat face of a tube in world space: a quad (a cap's fan triangle repeats its centre) with
/// its outward unit normal, and the index of the segment it belongs to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Face {
    pub corners: [Point; 4],
    pub normal: Point,
    pub segment: usize,
}

/// The faces of a prism of `sides` sides around `points`, with `radii` at each point, and the
/// two end caps if `caps`. Faces of no area are left out.
pub(crate) fn tube_faces(points: &[Point], radii: &[f32], sides: usize, caps: bool) -> Vec<Face> {
    let sides = sides.max(3);
    let count = points.len().min(radii.len());
    let rings = ring_frames(&points[..count]);
    if rings.len() < 2 {
        return Vec::new();
    }
    let section: Vec<Vec<Point>> = rings
        .iter()
        .zip(points.iter().zip(radii))
        .map(|(ring, (&center, &radius))| {
            (0..sides)
                .map(|k| {
                    let angle = TAU * k as f32 / sides as f32;
                    let offset = add(
                        scaled(ring.across, angle.cos()),
                        scaled(ring.up, angle.sin()),
                    );
                    add(center, scaled(offset, radius))
                })
                .collect()
        })
        .collect();
    let mut faces = Vec::with_capacity(sides * (rings.len() - 1) + 2 * sides);
    for segment in 0..rings.len() - 1 {
        let (near, far) = (&section[segment], &section[segment + 1]);
        for k in 0..sides {
            let next = (k + 1) % sides;
            let corners = [near[k], near[next], far[next], far[k]];
            let normal = cross(sub(corners[2], corners[0]), sub(corners[3], corners[1]));
            if length(normal) < 1e-12 {
                continue;
            }
            let outward = [
                sub(corners[0], points[segment]),
                sub(corners[1], points[segment]),
                sub(corners[2], points[segment + 1]),
                sub(corners[3], points[segment + 1]),
            ]
            .into_iter()
            .fold([0.0; 3], add);
            let normal = normalize(normal);
            let normal = if dot(normal, outward) < 0.0 {
                scaled(normal, -1.0)
            } else {
                normal
            };
            faces.push(Face {
                corners,
                normal,
                segment,
            });
        }
    }
    if caps {
        for (end, segment, direction) in [(0, 0, -1.0), (rings.len() - 1, rings.len() - 2, 1.0)] {
            if radii[end].abs() < 1e-9 {
                continue;
            }
            let center = points[end];
            let normal = scaled(rings[end].tangent, direction);
            for k in 0..sides {
                let next = (k + 1) % sides;
                faces.push(Face {
                    corners: [center, center, section[end][next], section[end][k]],
                    normal,
                    segment,
                });
            }
        }
    }
    faces
}

impl Frame {
    /// A band that faces the screen along `points`, `width` wide, optionally dashed. Each segment
    /// is offset by half the width perpendicular to its screen direction, and the segments
    /// simply overlap at joins. Dashes follow the arc length in world units and continue across
    /// vertices. A ribbon under one pixel wide is drawn as lines, which leave nothing pickable.
    /// Pieces are at most 32 pixels each way, and a piece wholly outside the frame is dropped.
    pub(crate) fn ribbon(
        &mut self,
        camera: &Camera,
        points: &[Point],
        width: Width,
        color: Color,
        pick: u32,
        dash: Option<Dash>,
    ) {
        let pixels = match width {
            Width::World(world) => world * camera.zoom * SCALE,
            Width::Pixels(pixels) => pixels,
        };
        if pixels.is_nan() || pixels <= 0.0 || points.len() < 2 {
            return;
        }
        let mut distance = 0.0;
        for pair in points.windows(2) {
            let size = length(sub(pair[1], pair[0]));
            let (start, end) = (distance, distance + size);
            distance = end;
            let (a, b) = (self.project(camera, pair[0]), self.project(camera, pair[1]));
            if size < 1e-9 || (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) > REACH {
                continue;
            }
            let spans = match dash {
                None => vec![(start, end)],
                Some(dash) => dash.spans(start, end),
            };
            for (from, to) in spans {
                let p = lerp(a, b, (from - start) / size);
                let q = lerp(a, b, (to - start) / size);
                if pixels < THINNEST_RIBBON {
                    self.items.push(Item::Line(p, q, color));
                } else {
                    self.band(p, q, pixels * 0.5, color, pick);
                }
            }
        }
    }

    /// The rectangle `half` to each side of the screen segment `p` to `q`.
    fn band(&mut self, p: Point, q: Point, half: f32, color: Color, pick: u32) {
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let size = dx.hypot(dy);
        if size < 1e-3 {
            return;
        }
        let (nx, ny) = (-dy / size * half, dx / size * half);
        let shift =
            |point: Point, sign: f32| [point[0] + nx * sign, point[1] + ny * sign, point[2]];
        self.patch(
            [shift(p, 1.0), shift(p, -1.0), shift(q, -1.0), shift(q, 1.0)],
            color,
            pick,
        );
    }

    /// A quad in frame space whose corners run `c0 -> c1` across and `c0 -> c3` along, as
    /// triangles in cells of at most `PIECE` pixels per edge, leaving out cells wholly outside the
    /// frame and triangles of no area.
    fn patch(&mut self, corners: [Point; 4], color: Color, pick: u32) {
        if self.outside(&corners) {
            return;
        }
        let across =
            screen_distance(corners[0], corners[1]).max(screen_distance(corners[3], corners[2]));
        let along =
            screen_distance(corners[0], corners[3]).max(screen_distance(corners[1], corners[2]));
        let columns = (across / PIECE).ceil().max(1.0) as usize;
        let rows = (along / PIECE).ceil().max(1.0) as usize;
        let at = |s: f32, t: f32| {
            lerp(
                lerp(corners[0], corners[1], s),
                lerp(corners[3], corners[2], s),
                t,
            )
        };
        for row in 0..rows {
            let (t0, t1) = (row as f32 / rows as f32, (row + 1) as f32 / rows as f32);
            for column in 0..columns {
                let (s0, s1) = (
                    column as f32 / columns as f32,
                    (column + 1) as f32 / columns as f32,
                );
                let cell = [at(s0, t0), at(s1, t0), at(s1, t1), at(s0, t1)];
                if (columns > 1 || rows > 1) && self.outside(&cell) {
                    continue;
                }
                for triangle in [[cell[0], cell[1], cell[2]], [cell[0], cell[2], cell[3]]] {
                    let area = (triangle[1][0] - triangle[0][0])
                        * (triangle[2][1] - triangle[0][1])
                        - (triangle[2][0] - triangle[0][0]) * (triangle[1][1] - triangle[0][1]);
                    if area.abs() >= 0.001 {
                        self.items.push(Item::Triangle(triangle, color, pick));
                    }
                }
            }
        }
    }

    /// Whether the bounding box of `points` lies wholly outside the frame.
    fn outside(&self, points: &[Point]) -> bool {
        let (width, height) = (self.width as f32, self.height as f32);
        points.iter().all(|p| p[0] < 0.0)
            || points.iter().all(|p| p[0] > width)
            || points.iter().all(|p| p[1] < 0.0)
            || points.iter().all(|p| p[1] > height)
    }

    /// A prism of `sides` (at least 3) flat-shaded faces around `points`, `radii[i]` at
    /// `points[i]` (the lengths must match), without twist (`ring_frames`). Each face is lit by
    /// `lit` from its world-space normal. With `caps` both ends are closed with a fan and faces
    /// turned away from the viewer are left out, since a closed tube hides them; without, every
    /// face is drawn, inside included. A segment whose radius is under half a pixel is drawn as
    /// a line. Pieces follow the 64 x 64 pixel rule of `ribbon`.
    #[expect(
        clippy::too_many_arguments,
        reason = "the issue fixes this signature, which mirrors `ribbon`"
    )]
    pub(crate) fn tube(
        &mut self,
        camera: &Camera,
        points: &[Point],
        radii: &[f32],
        color: Color,
        pick: u32,
        sides: usize,
        caps: bool,
    ) {
        debug_assert_eq!(points.len(), radii.len(), "one radius per point");
        let count = points.len().min(radii.len());
        if count < 2 {
            return;
        }
        let scale = camera.zoom * SCALE;
        let screen: Vec<Point> = points[..count]
            .iter()
            .map(|&point| self.project(camera, point))
            .collect();
        let thin: Vec<Option<bool>> = (0..count - 1)
            .map(|i| {
                let (a, b) = (screen[i], screen[i + 1]);
                let reachable = (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) <= REACH;
                reachable.then(|| radii[i].max(radii[i + 1]) * scale < THINNEST_TUBE)
            })
            .collect();
        for (i, kind) in thin.iter().enumerate() {
            if *kind == Some(true) {
                self.items.push(Item::Line(screen[i], screen[i + 1], color));
            }
        }
        let viewer = camera.viewer();
        for face in tube_faces(&points[..count], &radii[..count], sides, caps) {
            if thin[face.segment] != Some(false) || (caps && dot(face.normal, viewer) <= 0.0) {
                continue;
            }
            let corners = face.corners.map(|corner| self.project(camera, corner));
            self.patch(corners, lit(color, face.normal), pick);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;
    use crate::render::{Backdrop, Buffers, NONE, Sky};

    const RED: Color = [255, 0, 0];

    fn blank(width: u32, height: u32) -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Void,
            haze: [0.0; 3],
            time: 0.0,
        };
        Frame::new(width, height, sky, Buffers::default())
    }

    /// Straight down, so a world step along `RIGHT` is a horizontal step on the screen of
    /// exactly its length times `px_per_world`.
    fn overhead(px_per_world: f32) -> Camera {
        Camera {
            pitch: FRAC_PI_2,
            zoom: px_per_world / SCALE,
            ..Camera::default()
        }
    }

    const RIGHT: Point = [
        std::f32::consts::FRAC_1_SQRT_2,
        -std::f32::consts::FRAC_1_SQRT_2,
        0.0,
    ];
    const DOWN: Point = [
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
    ];

    fn at(direction: Point, distance: f32) -> Point {
        scaled(direction, distance)
    }

    fn painted(frame: &Frame, x: usize, y: usize) -> bool {
        frame.pixels[(y * frame.width as usize + x) * 3..][..3] == RED
    }

    fn painted_at(frame: &Frame, camera: &Camera, point: Point) -> bool {
        let screen = frame.project(camera, point);
        painted(frame, screen[0] as usize, screen[1] as usize)
    }

    fn triangles(frame: &Frame) -> Vec<[Point; 3]> {
        frame
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(points, ..) => Some(*points),
                _ => None,
            })
            .collect()
    }

    fn box_of(points: &[Point]) -> (f32, f32) {
        let extent = |axis: usize| {
            let high = points.iter().map(|p| p[axis]).fold(f32::MIN, f32::max);
            let low = points.iter().map(|p| p[axis]).fold(f32::MAX, f32::min);
            high - low
        };
        (extent(0), extent(1))
    }

    #[test]
    fn the_shared_light_is_a_unit_vector_toward_the_upper_left() {
        let [x, y, z] = shared::LIGHT;
        assert!(((x * x + y * y + z * z) - 1.0).abs() < 1e-6);
        let expected = normalize([-0.4, -0.5, 0.75]);
        for (light, expected) in shared::LIGHT.into_iter().zip(expected) {
            assert!((light - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn a_ribbon_is_as_wide_as_asked_at_a_known_zoom() {
        for (width, expected) in [(Width::World(1.0), 12.0), (Width::Pixels(7.0), 7.0)] {
            let camera = overhead(12.0);
            let mut frame = blank(400, 300);
            let path = [at(RIGHT, -10.0), at(RIGHT, 10.0)];
            frame.ribbon(&camera, &path, width, RED, 3, None);
            frame.rasterize();
            let x = 200;
            let thickness = (0..300).filter(|&y| painted(&frame, x, y)).count();
            assert!(
                (thickness as f32 - expected).abs() <= 1.0,
                "{width:?} drew {thickness} px, wanted {expected}"
            );
            let length = (0..400).filter(|&x| painted(&frame, x, 171)).count();
            assert!((length as f32 - 240.0).abs() <= 1.0, "{length}");
        }
    }

    #[test]
    fn dashes_cover_the_on_fraction_and_move_with_the_phase() {
        let camera = overhead(10.0);
        let path = [at(RIGHT, -10.0), at(RIGHT, 10.0)];
        let covered = |phase: f32| {
            let mut frame = blank(400, 300);
            let dash = Dash {
                on: 3.0,
                off: 1.0,
                phase,
            };
            frame.ribbon(&camera, &path, Width::Pixels(4.0), RED, 0, Some(dash));
            frame.rasterize();
            frame
        };
        let frame = covered(0.0);
        let row = (0.57 * 300.0) as usize;
        let on = (0..400).filter(|&x| painted(&frame, x, row)).count();
        assert!((on as f32 - 150.0).abs() <= 4.0, "{on} of 200 px");
        // The first dash starts at the path's start; a phase of 3 begins in the gap.
        assert!(painted_at(&frame, &camera, at(RIGHT, -9.0)));
        let shifted = covered(3.0);
        assert!(!painted_at(&shifted, &camera, at(RIGHT, -9.5)));
        assert!(painted_at(&shifted, &camera, at(RIGHT, -8.5)));
    }

    #[test]
    fn dashes_continue_across_a_vertex() {
        let camera = overhead(10.0);
        let corner = at(RIGHT, 0.0);
        let end = add(corner, at(DOWN, 4.0));
        let start = add(corner, at(RIGHT, -4.0));
        let mut frame = blank(400, 300);
        let dash = Dash {
            on: 3.0,
            off: 3.0,
            phase: 0.0,
        };
        frame.ribbon(
            &camera,
            &[start, corner, end],
            Width::Pixels(6.0),
            RED,
            0,
            Some(dash),
        );
        frame.rasterize();
        // Arc 0..3 is on, 3..6 off, 6..9 on. The second segment begins at arc 4.
        assert!(painted_at(&frame, &camera, at(RIGHT, -2.0)));
        assert!(!painted_at(&frame, &camera, at(RIGHT, -0.5)));
        for off in [0.5, 1.5] {
            assert!(!painted_at(&frame, &camera, add(corner, at(DOWN, off))));
        }
        for on in [2.5, 3.5] {
            assert!(painted_at(&frame, &camera, add(corner, at(DOWN, on))));
        }
    }

    #[test]
    fn ribbon_and_tube_triangles_never_exceed_the_bounding_box_limit() {
        let camera = Camera {
            zoom: 10.0,
            ..Camera::default()
        };
        let mut frame = blank(1280, 720);
        let long = [[-300.0, -200.0, 0.0], [300.0, 250.0, 5.0]];
        frame.ribbon(&camera, &long, Width::Pixels(3.0), RED, 0, None);
        frame.ribbon(&camera, &long, Width::Pixels(150.0), RED, 0, None);
        frame.ribbon(&camera, &long, Width::World(8.0), RED, 0, None);
        let bent = [[-40.0, 0.0, 0.0], [0.0, 30.0, 0.0], [40.0, -20.0, 10.0]];
        frame.tube(&camera, &bent, &[20.0, 5.0, 12.0], RED, 0, 8, false);
        frame.tube(&camera, &bent, &[30.0, 30.0, 30.0], RED, 0, 5, true);
        let all = triangles(&frame);
        assert!(all.len() > 100);
        for triangle in all {
            let (width, height) = box_of(&triangle);
            assert!(width <= 64.0 && height <= 64.0, "{triangle:?}");
        }
    }

    #[test]
    fn off_screen_segments_emit_nothing() {
        let camera = overhead(10.0);
        let mut frame = blank(200, 100);
        let far = [at(RIGHT, 500.0), at(RIGHT, 520.0)];
        frame.ribbon(&camera, &far, Width::Pixels(8.0), RED, 0, None);
        frame.tube(&camera, &far, &[1.0, 1.0], RED, 0, 6, false);
        assert!(frame.items.is_empty());
        // Too long to project, though it crosses the frame.
        let huge = [at(RIGHT, -2000.0), at(RIGHT, 2000.0)];
        frame.ribbon(&camera, &huge, Width::Pixels(8.0), RED, 0, None);
        assert!(frame.items.is_empty());
        // One segment on screen and one off draws only the first.
        let two = [at(RIGHT, -5.0), at(RIGHT, 5.0), at(RIGHT, 500.0)];
        frame.ribbon(&camera, &two, Width::Pixels(8.0), RED, 0, None);
        let drawn = triangles(&frame);
        assert!(!drawn.is_empty());
        assert!(drawn.iter().flatten().all(|p| p[0] < 400.0));
    }

    #[test]
    fn a_pickable_ribbon_is_picked_and_a_thin_one_falls_back_to_a_line() {
        let camera = overhead(10.0);
        let path = [at(RIGHT, -5.0), at(RIGHT, 5.0)];
        let (x, y) = (100, (0.57 * 100.0) as usize);
        let mut frame = blank(200, 100);
        frame.ribbon(&camera, &path, Width::Pixels(6.0), RED, 9, None);
        frame.rasterize();
        assert_eq!(frame.picks[y * 200 + x], 9);

        let mut thin = blank(200, 100);
        thin.ribbon(&camera, &path, Width::Pixels(0.5), RED, 9, None);
        assert!(thin.items.iter().all(|item| matches!(item, Item::Line(..))));
        assert!(!thin.items.is_empty());
        thin.rasterize();
        assert!(thin.picks.iter().all(|&pick| pick == NONE));
        assert!((0..100).any(|y| painted(&thin, x, y)));
    }

    #[test]
    fn tube_faces_facing_the_light_are_brighter() {
        let axis = [[-3.0, 0.0, 0.0], [3.0, 0.0, 0.0]];
        let faces = tube_faces(&axis, &[1.0, 1.0], 16, false);
        assert_eq!(faces.len(), 16);
        let brightness = |face: &Face| {
            let [r, g, b] = lit([200, 200, 200], face.normal);
            r as u32 + g as u32 + b as u32
        };
        let brightest = faces.iter().max_by_key(|face| brightness(face)).unwrap();
        let darkest = faces.iter().min_by_key(|face| brightness(face)).unwrap();
        let centre = |face: &Face| {
            let sum = face.corners.into_iter().fold([0.0; 3], add);
            [0.0, sum[1] / 4.0, sum[2] / 4.0]
        };
        assert!(dot(centre(brightest), shared::LIGHT) > 0.5);
        assert!(dot(centre(darkest), shared::LIGHT) < 0.0);
        assert!(brightness(brightest) > brightness(darkest));
        assert_eq!(
            lit([200, 100, 40], [0.0, 0.0, -1.0]),
            tint([200, 100, 40], 0.35)
        );
        // The colours that reach the frame are exactly those.
        let mut frame = blank(200, 100);
        frame.tube(
            &Camera::default(),
            &axis,
            &[1.0, 1.0],
            [200, 200, 200],
            0,
            16,
            false,
        );
        let shades: Vec<Color> = frame
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(_, color, _) => Some(*color),
                _ => None,
            })
            .collect();
        let expected: Vec<Color> = faces
            .iter()
            .map(|f| lit([200, 200, 200], f.normal))
            .collect();
        assert!(shades.iter().all(|shade| expected.contains(shade)));
        assert!(expected.iter().all(|shade| shades.contains(shade)));
    }

    #[test]
    fn a_capped_tube_culls_faces_turned_away_from_the_viewer() {
        let camera = Camera::default();
        let axis = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let radii = [0.5, 0.5];
        let count = |caps: bool| {
            let mut frame = blank(200, 100);
            frame.tube(&camera, &axis, &radii, RED, 0, 8, caps);
            triangles(&frame).len()
        };
        let uncapped = count(false);
        assert_eq!(uncapped, 8 * 2, "every face is drawn, two triangles each");
        let viewer = camera.viewer();
        let faces = tube_faces(&axis, &radii, 8, true);
        assert!(
            faces
                .iter()
                .all(|face| (length(face.normal) - 1.0).abs() < 1e-5)
        );
        let toward = |cap: bool| {
            faces
                .iter()
                .filter(|face| (face.corners[0] == face.corners[1]) == cap)
                .filter(|face| dot(face.normal, viewer) > 0.0)
                .count()
        };
        let (sides, fans) = (toward(false), toward(true));
        assert!(sides < 8 && sides > 0, "{sides}");
        assert_eq!(count(true), sides * 2 + fans);
        // The cap that points away from the viewer is not drawn at all.
        assert_eq!(fans, 8);
    }

    #[test]
    fn a_tube_does_not_twist_along_a_curved_path() {
        // A planar arc: transporting a frame about the plane's normal never changes how far
        // that frame's vectors lean out of the plane.
        let arc: Vec<Point> = (0..=40)
            .map(|i| {
                let angle = i as f32 / 40.0 * 2.5;
                [10.0 * angle.sin(), 10.0 * (1.0 - angle.cos()), 0.0]
            })
            .collect();
        let rings = ring_frames(&arc);
        for ring in &rings {
            assert!((ring.across[2] - rings[0].across[2]).abs() < 1e-4);
            assert!((ring.up[2] - rings[0].up[2]).abs() < 1e-4);
        }
        // A helix: each step turns the frame no more than the tangent turns.
        let helix: Vec<Point> = (0..=120)
            .map(|i| {
                let angle = i as f32 * 0.1;
                [4.0 * angle.cos(), 4.0 * angle.sin(), angle]
            })
            .collect();
        let rings = ring_frames(&helix);
        for pair in rings.windows(2) {
            let tangent_turn = dot(pair[0].tangent, pair[1].tangent)
                .clamp(-1.0, 1.0)
                .acos();
            let frame_turn = dot(pair[0].across, pair[1].across).clamp(-1.0, 1.0).acos();
            assert!(
                frame_turn <= tangent_turn + 1e-3,
                "{frame_turn} > {tangent_turn}"
            );
            assert!(dot(pair[1].across, pair[1].tangent).abs() < 1e-4);
            assert!(dot(pair[1].up, pair[1].across).abs() < 1e-4);
        }
    }

    #[test]
    fn a_tube_with_repeated_points_and_a_reversal_still_has_frames() {
        let points = [[0.0; 3], [0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        let rings = ring_frames(&points);
        assert_eq!(rings.len(), 4);
        for ring in rings {
            assert!(ring.across.iter().chain(&ring.up).all(|v| v.is_finite()));
        }
        assert!(tube_faces(&[[0.0; 3]], &[1.0], 6, true).is_empty());
    }
}
