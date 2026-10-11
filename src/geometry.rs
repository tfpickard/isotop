//! Small vector and box helpers shared by the views: dot products, normalising, and the shaded
//! box that Coop builds its houses and fences from.

use crate::palette::tint;
use crate::render::{Camera, Color, Frame, NONE, Point};

pub(crate) fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn normalize(a: Point) -> Point {
    let length = dot(a, a).sqrt().max(1e-6);
    a.map(|v| v / length)
}

/// A flat vector scaled to length one, or zero when it is too short to have a direction. Unlike
/// [`normalize`] it measures with `hypot` and does not clamp the length, so tiny vectors vanish
/// instead of staying tiny.
pub(crate) fn normalize_or_zero(v: [f32; 2]) -> [f32; 2] {
    let length = v[0].hypot(v[1]);
    if length > 1e-6 {
        [v[0] / length, v[1] / length]
    } else {
        [0.0, 0.0]
    }
}

/// Shading of a vertical face by its outward direction, from a fixed light in the north west.
pub(crate) fn shade(color: Color, normal: [f32; 2]) -> Color {
    tint(color, 0.72 + 0.2 * (-0.6 * normal[0] + 0.8 * normal[1]))
}

/// An axis-aligned box: its top and four sides, each shaded by facing.
pub(crate) fn cuboid(frame: &mut Frame, camera: &Camera, low: Point, high: Point, color: Color) {
    let [x0, y0, z0] = low;
    let [x1, y1, z1] = high;
    frame.quad(
        camera,
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        tint(color, 1.05),
        NONE,
    );
    for (points, normal) in [
        (
            [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
            [0.0, -1.0],
        ),
        (
            [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
            [0.0, 1.0],
        ),
        (
            [[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]],
            [-1.0, 0.0],
        ),
        (
            [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
            [1.0, 0.0],
        ),
    ] {
        frame.quad(camera, points, shade(color, normal), NONE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_or_zero_drops_vectors_too_short_to_have_a_direction() {
        assert_eq!(normalize_or_zero([3.0, 4.0]), [0.6, 0.8]);
        assert_eq!(normalize_or_zero([1e-7, 0.0]), [0.0, 0.0]);
    }

    #[test]
    fn normalize_keeps_unit_length_and_clamps_the_zero_vector() {
        let unit = normalize([0.0, 3.0, 4.0]);
        assert!((dot(unit, unit) - 1.0).abs() < 1e-6);
        assert_eq!(normalize([0.0; 3]), [0.0; 3]);
    }
}
