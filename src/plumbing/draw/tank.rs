//! A tank: a riveted iron cylinder with a band in its kind's colour, a domed cap and a valve
//! wheel, and on the side that faces the viewer a pressure gauge. The gauge shows the tank's
//! attributed outflow, never the fill of a pipe, which no one can measure: a needle on a log
//! scale, pinned in the red when the process is blocked writing to a pipe, and a plain face with
//! a slash and no needle when the flow was not measured.

use std::f32::consts::PI;

use super::{BRASS, IRON, Ink, RED, add, highlighted, scale};
use crate::palette::tint;
use crate::render::{Color, Point};

/// What the gauge of a tank reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reading {
    /// The outflow was not measured: no needle, and the face says so.
    Missing,
    /// A needle at `fill` (0 to 1 of the sweep), pinned in the red when `blocked`.
    Needle { fill: f32, blocked: bool },
}

/// A tank to draw.
#[derive(Clone, Copy, Debug)]
pub struct Tank {
    pub at: [f32; 2],
    pub radius: f32,
    pub height: f32,
    pub band: Color,
    /// What clicking it picks: the index of its process.
    pub pick: u32,
    /// The app highlights it (selected, hovered or toured).
    pub hot: bool,
}

/// The radius of the tank of a process holding `memory` bytes, in world units: 0.3 up to 2 MiB,
/// then 0.085 for every doubling, to 0.9 at about 267 MiB.
pub fn radius_of(memory: u64) -> f32 {
    let mebibytes = (memory as f32 / 1_048_576.0).max(1.0);
    (0.30 + 0.085 * (mebibytes / 2.0).log2()).clamp(0.3, 0.9)
}

/// The height of a tank of `radius`, in world units.
pub fn height_of(radius: f32) -> f32 {
    1.6 * radius + 0.55
}

/// How far the cap rises above the cylinder.
fn dome(radius: f32) -> f32 {
    0.22 * radius + 0.1
}

/// The height of the top of the valve, where an overhead tube leaves.
pub fn top_of(radius: f32) -> f32 {
    height_of(radius) + dome(radius) + 0.34
}

/// The radius of the gauge face on a tank of `radius`.
fn face_of(radius: f32) -> f32 {
    (0.45 * radius).clamp(0.15, 0.5)
}

/// Draws `tank` and its gauge.
pub fn draw(ink: &mut Ink, tank: &Tank, reading: Reading) {
    let [x, y] = tank.at;
    let (radius, height) = (tank.radius, tank.height);
    let pick = tank.pick;
    let iron = highlighted(IRON, tank.hot);
    ink.prism(
        &[[x, y, 0.0], [x, y, height]],
        &[radius, radius],
        iron,
        pick,
        12,
        true,
    );
    ink.prism(
        &[[x, y, height * 0.58], [x, y, height * 0.74]],
        &[radius * 1.02, radius * 1.02],
        highlighted(tank.band, tank.hot),
        pick,
        12,
        true,
    );
    ink.prism(
        &[[x, y, height * 0.3 - 0.025], [x, y, height * 0.3 + 0.025]],
        &[radius * 1.015, radius * 1.015],
        tint(iron, 0.55),
        pick,
        12,
        true,
    );
    let cap = dome(radius);
    ink.prism(
        &[[x, y, height], [x, y, height + cap]],
        &[radius * 0.98, radius * 0.5],
        tint(iron, 1.15),
        pick,
        12,
        true,
    );
    ink.prism(
        &[[x, y, height - 0.03], [x, y, height + 0.05]],
        &[radius * 1.05, radius * 1.05],
        BRASS,
        pick,
        12,
        true,
    );
    let stem = height + cap + 0.34;
    ink.pipe(
        &[[x, y, height + cap * 0.6], [x, y, stem]],
        0.06,
        BRASS,
        pick,
    );
    let wheel: Vec<Point> = (0..=10)
        .map(|k| {
            let angle = k as f32 / 10.0 * 2.0 * PI;
            [x + 0.2 * angle.cos(), y + 0.2 * angle.sin(), stem]
        })
        .collect();
    ink.band(&wheel, 0.055, RED, None);
    if ink.pixels(2.0 * radius) > 24.0 {
        rivets(ink, tank);
    }
    gauge(ink, tank, reading);
    if tank.hot {
        let radius_px = ink.pixels(radius) * 2.4;
        ink.halo([x, y, height * 0.5], radius_px, [255, 190, 110], 0.4);
    }
}

/// Rivets along the band's edges and the lower seam, on the side that faces the viewer, clear of
/// the gauge.
fn rivets(ink: &mut Ink, tank: &Tank) {
    let [x, y] = tank.at;
    let (radius, height) = (tank.radius, tank.height);
    let toward = ink.facing[1].atan2(ink.facing[0]);
    let size = 0.045 + 0.02 * radius.min(1.5);
    let face = face_of(radius);
    for z in [height * 0.3, height * 0.58, height * 0.74] {
        for k in 0..14 {
            let angle = toward + (k as f32 / 13.0 - 0.5) * 2.9;
            let near_gauge = (angle - toward).abs() < (1.5 * face / radius).min(1.2)
                && (z - 0.42 * height).abs() < face * 1.4;
            if near_gauge {
                continue;
            }
            ink.ball(
                [
                    x + radius * 1.03 * angle.cos(),
                    y + radius * 1.03 * angle.sin(),
                    z + if z > height * 0.5 { 0.06 } else { 0.0 },
                ],
                size,
                tint(IRON, 1.7),
            );
        }
    }
}

/// The gauge: three discs (brass rim, brass bevel, face), the red zone, the needle or the slash.
fn gauge(ink: &mut Ink, tank: &Tank, reading: Reading) {
    let [x, y] = tank.at;
    let (radius, height) = (tank.radius, tank.height);
    let face = face_of(radius);
    let normal = [ink.facing[0], ink.facing[1], 0.0];
    let right = [-ink.facing[1], ink.facing[0], 0.0];
    let up = [0.0, 0.0, 1.0];
    let centre = [
        x + ink.facing[0] * (radius + 0.03),
        y + ink.facing[1] * (radius + 0.03),
        0.42 * height,
    ];
    let at = |lift: f32| add(centre, scale(normal, lift));
    let steps = if ink.pixels(face) < 5.0 { 8 } else { 18 };
    let pick = tank.pick;
    ink.disc(
        at(0.0),
        right,
        up,
        face * 1.2,
        tint(BRASS, 0.55),
        pick,
        steps,
    );
    ink.disc(at(0.03), right, up, face * 1.08, BRASS, pick, steps);
    let paper = match reading {
        Reading::Needle { .. } => [240, 234, 214],
        Reading::Missing => [150, 158, 172],
    };
    ink.disc(at(0.06), right, up, face, paper, pick, steps);
    // The needle's direction at `fill` and the direction across it: 225 degrees at 0, sweeping
    // clockwise through the top to -45 degrees at 1.
    let spoke = |fill: f32| {
        let (sine, cosine) = (225.0 - 270.0 * fill).to_radians().sin_cos();
        (
            add(scale(right, cosine), scale(up, sine)),
            add(scale(right, -sine), scale(up, cosine)),
        )
    };
    let detailed = ink.pixels(face) >= 5.0;
    if detailed && matches!(reading, Reading::Needle { .. }) {
        // The red zone: the last fifth of the sweep.
        for k in 0..8 {
            let (a, b) = (
                0.78 + 0.22 * k as f32 / 8.0,
                0.78 + 0.22 * (k + 1) as f32 / 8.0,
            );
            let corner = |fill: f32, reach: f32| add(at(0.08), scale(spoke(fill).0, face * reach));
            ink.quad(
                [
                    corner(a, 0.64),
                    corner(a, 0.86),
                    corner(b, 0.86),
                    corner(b, 0.64),
                ],
                RED,
                pick,
            );
        }
    }
    match reading {
        Reading::Needle { fill, blocked } => {
            let fill = if blocked { 1.0 } else { fill.clamp(0.0, 1.0) };
            let (toward, across) = spoke(fill);
            let base = at(0.1);
            let tip = add(base, scale(toward, face * 0.82));
            let half = scale(across, face * 0.07);
            let color = if blocked { [150, 20, 20] } else { [34, 28, 24] };
            ink.quad(
                [add(base, half), tip, tip, sub_point(base, half)],
                color,
                pick,
            );
        }
        Reading::Missing => {
            // A slash across the face, and nothing that could be read as a value.
            let (toward, across) = spoke(0.375);
            let base = at(0.1);
            let (a, b) = (
                add(base, scale(toward, -face * 0.7)),
                add(base, scale(toward, face * 0.7)),
            );
            let half = scale(across, face * 0.09);
            ink.quad(
                [
                    add(a, half),
                    add(b, half),
                    sub_point(b, half),
                    sub_point(a, half),
                ],
                [74, 80, 96],
                pick,
            );
        }
    }
    ink.disc(at(0.12), right, up, face * 0.14, [60, 46, 30], pick, 8);
    if matches!(reading, Reading::Needle { blocked: true, .. }) {
        ink.halo(at(0.1), ink.pixels(face) * 2.6, [255, 60, 40], 0.32);
    }
}

fn sub_point(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Steam from the relief valve of a tank that is blocked, as drifting puffs: the heat of a
/// process that cannot get rid of what it writes. Decorative.
pub fn steam(ink: &mut Ink, tank: &Tank, time: f32) {
    let [x, y] = tank.at;
    let top = top_of(tank.radius);
    for k in 0..6 {
        let phase = (time * 0.45 + k as f32 / 6.0).fract();
        let drift = (k as f32 * 1.7 + time * 1.3).sin() * 0.3 + phase * 0.8;
        let at = [
            x + drift * ink.facing[1],
            y - drift * ink.facing[0],
            top + 0.2 + phase * 2.4,
        ];
        let radius = ink.pixels(0.2 + 0.6 * phase);
        ink.halo(
            at,
            radius,
            [208, 208, 214],
            0.24 * (1.0 - phase) * (phase * 6.0).min(1.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tank_is_as_wide_as_its_memory_says_between_the_clamps() {
        let mib = |n: u64| n * 1_048_576;
        assert_eq!(radius_of(0), 0.3);
        assert_eq!(radius_of(mib(2)), 0.3);
        assert!((radius_of(mib(16)) - (0.3 + 0.085 * 3.0)).abs() < 1e-5);
        assert!((radius_of(mib(256)) - (0.3 + 0.085 * 7.0)).abs() < 1e-5);
        assert_eq!(radius_of(mib(300)), 0.9, "saturates at about 267 MiB");
        assert_eq!(radius_of(mib(40_000)), 0.9);
        let saturates = 2.0 * 2.0_f32.powf((0.9 - 0.3) / 0.085);
        assert!((saturates - 267.0).abs() < 1.0, "{saturates}");
    }

    #[test]
    fn a_tank_is_as_tall_as_its_radius_says() {
        assert!((height_of(0.3) - 1.03).abs() < 1e-5);
        assert!((height_of(0.9) - 1.99).abs() < 1e-5);
        assert!(top_of(0.3) > height_of(0.3));
    }

    #[test]
    fn a_wide_tank_fits_its_cell_and_a_pipe_passes_between_two_of_them() {
        let widest = radius_of(u64::MAX / 2);
        assert!(2.0 * widest <= 2.0, "the tank fits the 2-unit cell");
        assert!(crate::plumbing::floor::PITCH - 2.0 * widest > 1.0);
    }
}
