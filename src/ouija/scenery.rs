//! The board's wood and the table's velvet as colour fields, lit by the two candles.
//!
//! Both are `Field`s (P3) of colours baked once: the wood grain, the worn edges and the velvet's
//! nap, each multiplied at every node by a light that is the sum of two candles' falloffs, which
//! are also computed once. Each frame only the candles' flicker is new: every node's colour is its
//! baked colour times the tone of `flicker . falloffs`, and the field is stamped with a new
//! revision when that tone moved by a visible step. Nothing here is measured; all of it is
//! decoration.

use super::layout::{HALF_HEIGHT, Plane, Spot, THICKNESS};
use crate::render::{Camera, Color, Field, Frame, Placement, Relief};
use crate::simulation::hash;

/// Nodes of the board's field along its width and its depth, and of the table's.
const BOARD_NODES: (usize, usize) = (400, 250);
const TABLE_NODES: (usize, usize) = (132, 88);
/// The table's extent: the rectangle of the board plane it covers, near left corner and far
/// right corner, in board widths. The velvet fades to black before it reaches the edge.
const TABLE_FROM: Spot = [-1.65, -1.05];
const TABLE_TO: Spot = [1.65, 1.15];

/// A candle on the table beside the board: where it stands, how tall it is and how wide, in
/// board widths.
#[derive(Clone, Copy, Debug)]
pub struct Candle {
    pub spot: Spot,
    pub height: f32,
    pub radius: f32,
}

pub const CANDLES: [Candle; 2] = [
    Candle {
        spot: [-0.64, 0.05],
        height: 0.20,
        radius: 0.04,
    },
    Candle {
        spot: [0.64, 0.16],
        height: 0.16,
        radius: 0.037,
    },
];

/// How far a flame stands above its candle's top, in board widths.
pub const FLAME_LIFT: f32 = 0.045;

/// How much light the candles give, and how softly it falls off near them.
const GAIN: f32 = 14.0;
const SOFTENING: f32 = 0.0195;
/// The tone (a multiplier of the baked colour) with no light, and the most the brightest light
/// reaches.
const AMBIENT: f32 = 0.10;
const PEAK: f32 = 1.7;
/// Flicker is quantised to this many steps per unit before a field is stamped anew.
const FLICKER_STEPS: f32 = 160.0;

/// How strongly each candle burns at `time`, around one: the prototype's three sinusoids, a
/// different phase for each candle.
pub fn flicker(time: f32) -> [f32; 2] {
    [0, 1].map(|candle| {
        let phase = candle as f32;
        1.0 + 0.07 * (time * 11.3 + phase * 2.0).sin()
            + 0.05 * (time * 23.7 + phase * 5.0).sin()
            + 0.04 * (time * 5.1 + phase).sin()
    })
}

/// The light one candle gives a point of a surface facing up: `distance` board widths from
/// below the flame, with the flame `above` the surface.
fn falloff(distance: f32, above: f32) -> f32 {
    let squared = distance * distance + above * above;
    GAIN * 0.019 * above / squared.sqrt() / (squared + SOFTENING)
}

/// The tone `light` makes of a baked colour: ambient with nothing, rising smoothly towards
/// `PEAK` and never past it.
fn tone(light: f32) -> f32 {
    AMBIENT + (PEAK - AMBIENT) * light / (1.0 + light)
}

/// One lit surface: the field drawn, the colour each node has before light, and how much of each
/// candle's light reaches it.
struct Surface {
    field: Field,
    base: Vec<[f32; 3]>,
    falloffs: [Vec<f32>; 2],
    /// The flicker the field was last stamped with, in steps.
    stamped: Option<[i32; 2]>,
}

impl Surface {
    /// A surface of `columns` by `rows` nodes over `from` to `to`, `down` board widths below
    /// the board's top, whose node (i, j) is coloured by `paint(spot)`. A candle's light on it
    /// dies away in a pool of about `pool` board widths round the candle, the velvet's, where the
    /// room is dark beyond the candles.
    fn new(
        key: &str,
        (columns, rows): (usize, usize),
        (from, to): (Spot, Spot),
        (down, pool): (f32, f32),
        paint: impl Fn(Spot) -> [f32; 3],
    ) -> Self {
        let mut field = Field::new(hash::fnv1a_64(key), columns, rows);
        let mut base = Vec::with_capacity(columns * rows);
        let mut falloffs = [Vec::new(), Vec::new()];
        for j in 0..rows {
            for i in 0..columns {
                let spot = [
                    from[0] + (to[0] - from[0]) * i as f32 / (columns - 1) as f32,
                    from[1] + (to[1] - from[1]) * j as f32 / (rows - 1) as f32,
                ];
                base.push(paint(spot));
                for (light, candle) in falloffs.iter_mut().zip(CANDLES) {
                    let apart = (spot[0] - candle.spot[0]).hypot(spot[1] - candle.spot[1]);
                    let pooled = (-(apart / pool).powi(2)).exp();
                    light.push(falloff(apart, candle.height + FLAME_LIFT + down) * pooled);
                }
            }
        }
        field.colors = vec![[0; 3]; columns * rows];
        Self {
            field,
            base,
            falloffs,
            stamped: None,
        }
    }

    /// Recolours the nodes for candles burning at `flicker`, unless that is within a step of
    /// what they were last coloured for.
    fn relight(&mut self, flicker: [f32; 2]) {
        let steps = flicker.map(|strength| (strength * FLICKER_STEPS).round() as i32);
        if self.stamped == Some(steps) {
            return;
        }
        self.stamped = Some(steps);
        let strength = steps.map(|step| step as f32 / FLICKER_STEPS);
        let [near, far] = &self.falloffs;
        for (((color, base), near), far) in self
            .field
            .colors
            .iter_mut()
            .zip(&self.base)
            .zip(near)
            .zip(far)
        {
            let light = tone(strength[0] * near + strength[1] * far);
            *color = base.map(|channel| (channel * light).min(255.0) as u8);
        }
        self.field.revision += 1;
    }
}

/// The wood and the velvet.
pub struct Scenery {
    board: Surface,
    table: Surface,
}

impl Scenery {
    pub fn new() -> Self {
        Self {
            board: Surface::new(
                "ouija board",
                BOARD_NODES,
                ([-0.5, -HALF_HEIGHT], [0.5, HALF_HEIGHT]),
                (0.0, f32::INFINITY),
                wood,
            ),
            table: Surface::new(
                "ouija table",
                TABLE_NODES,
                (TABLE_FROM, TABLE_TO),
                (THICKNESS, 0.62),
                velvet,
            ),
        }
    }

    /// Lights both fields for candles burning at `flicker`.
    pub fn relight(&mut self, flicker: [f32; 2]) {
        self.board.relight(flicker);
        self.table.relight(flicker);
    }

    /// Records the velvet and then the board into `frame`.
    pub fn draw(&self, frame: &mut Frame, camera: &Camera, plane: &Plane) {
        let corners = [
            TABLE_FROM,
            [TABLE_TO[0], TABLE_FROM[1]],
            TABLE_TO,
            [TABLE_FROM[0], TABLE_TO[1]],
        ]
        .map(|spot| plane.world(spot, -THICKNESS));
        frame.field(
            camera,
            Placement::Corners(corners),
            &self.table.field,
            Relief::Flat,
        );
        let top = plane.corners([0.5, HALF_HEIGHT], 0.0);
        frame.field(
            camera,
            Placement::Corners(top),
            &self.board.field,
            Relief::Flat,
        );
    }

    /// The tone the board's light has at `spot` for candles burning at `flicker`, for the paint
    /// on it: one is the colour of the paint as the mockup has it.
    pub fn light_at(spot: Spot, flicker: [f32; 2]) -> f32 {
        let light: f32 = CANDLES
            .iter()
            .zip(flicker)
            .map(|(candle, strength)| {
                let apart = (spot[0] - candle.spot[0]).hypot(spot[1] - candle.spot[1]);
                strength * falloff(apart, candle.height + FLAME_LIFT)
            })
            .sum();
        tone(light)
    }
}

/// The wood: dark brown with long grain, a few knots and the edges worn dark.
fn wood([x, y]: Spot) -> [f32; 3] {
    let warp = fbm(x * 2.0 + 3.0, y * 2.0, 11, 3);
    let rings = ((y + 0.07 * warp) * 150.0 + 4.0 * (x * 3.2 + warp).sin()).sin();
    let fine = ((y * 1.7 + 0.04 * warp) * 262.0).sin();
    let fiber = fbm(x * 3.0, y * 110.0, 29, 2) - 0.5;
    let mut grain = 0.82 + 0.14 * rings + 0.03 * fine + 0.22 * fiber + 0.10 * warp;
    for (knot_x, knot_y) in [(-0.31, 0.18), (0.22, -0.2), (0.41, 0.07)] {
        let apart = ((x - knot_x) / 3.0).hypot(y - knot_y) * 2400.0 / 40.0 / 2.4;
        grain -= 0.25 * (-apart).exp();
    }
    let edge = ((0.5 - x.abs()) / 0.06)
        .min((HALF_HEIGHT - y.abs()) / 0.09)
        .clamp(0.0, 1.0);
    let worn = 0.70 + 0.30 * edge;
    [176.0, 100.0, 52.0].map(|channel| channel * grain * worn)
}

/// The velvet: deep red with a soft nap and folds, fading to black away from the candles.
fn velvet([x, y]: Spot) -> [f32; 3] {
    let nap = fbm(x * 9.0, y * 9.0, 41, 3);
    let folds = (x * 5.0 + 2.0 * fbm(x * 2.0, y * 2.0, 53, 2)).sin() * 0.10;
    let cloth = 0.85 + 0.12 * nap + folds;
    let reach = (x / 1.35).hypot((y - 0.05) / 0.95);
    let fade = ((1.0 - reach) / 0.5).clamp(0.0, 1.0).powf(1.4);
    [128.0, 30.0, 56.0].map(|channel| channel * cloth * fade)
}

/// Smooth value noise at (`x`, `y`) in 0..1: lattice values from a hash, blended with smoothstep.
fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (tx, ty) = (x - fx, y - fy);
    let (ix, iy) = (fx as i32 as u32, fy as i32 as u32);
    let lattice = |dx: u32, dy: u32| {
        hash::unit_from(
            ix.wrapping_add(dx).wrapping_mul(73_856_093)
                ^ iy.wrapping_add(dy).wrapping_mul(19_349_663)
                ^ seed,
        )
    };
    let blend = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (blend(tx), blend(ty));
    let top = lattice(0, 0) + (lattice(1, 0) - lattice(0, 0)) * u;
    let bottom = lattice(0, 1) + (lattice(1, 1) - lattice(0, 1)) * u;
    top + (bottom - top) * v
}

/// `octaves` layers of `noise`, each twice as fine and half as strong, scaled to 0..1.
fn fbm(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let (mut total, mut weight, mut scale) = (0.0, 0.0, 1.0);
    for octave in 0..octaves {
        total += noise(x * scale, y * scale, seed.wrapping_add(octave * 977)) / scale;
        weight += 1.0 / scale;
        scale *= 2.0;
    }
    total / weight
}

/// The paint of the letters and the board's other ink, as the mockup has it: a worn cream.
pub const PAINT: Color = [226, 204, 156];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_falls_off_with_distance_and_never_passes_the_peak() {
        let near = tone(falloff(0.05, 0.19));
        let far = tone(falloff(0.9, 0.19));
        assert!(near > far && far >= AMBIENT);
        assert!(tone(1e9) <= PEAK && tone(0.0) == AMBIENT);
        assert!(falloff(0.5, 0.19) > falloff(0.6, 0.19));
    }

    #[test]
    fn flicker_stays_near_one_and_differs_between_candles() {
        for step in 0..500 {
            let [a, b] = flicker(step as f32 * 0.07);
            assert!(
                (0.8..1.2).contains(&a) && (0.8..1.2).contains(&b),
                "{a} {b}"
            );
        }
        let [a, b] = flicker(3.3);
        assert!(a != b);
        assert_eq!(flicker(3.3), flicker(3.3));
    }

    #[test]
    fn a_field_is_stamped_again_only_when_the_flicker_moves_a_step() {
        let mut scenery = Scenery::new();
        scenery.relight([1.0, 1.0]);
        let first = scenery.board.field.revision;
        scenery.relight([1.0, 1.0]);
        scenery.relight([1.0 + 0.3 / FLICKER_STEPS, 1.0]);
        assert_eq!(scenery.board.field.revision, first, "within a step");
        scenery.relight([1.04, 1.0]);
        assert_eq!(scenery.board.field.revision, first + 1);
        assert_eq!(scenery.table.field.revision, first + 1);
        let bright: u32 = scenery
            .board
            .field
            .colors
            .iter()
            .map(|c| u32::from(c[0]))
            .sum();
        scenery.relight([0.8, 0.8]);
        let dim: u32 = scenery
            .board
            .field
            .colors
            .iter()
            .map(|c| u32::from(c[0]))
            .sum();
        assert!(dim < bright, "weaker candles light the board less");
    }

    #[test]
    fn the_velvet_fades_to_black_at_its_edge_and_the_wood_is_never_black() {
        assert_eq!(velvet(TABLE_FROM), [0.0; 3]);
        assert_eq!(velvet(TABLE_TO), [0.0; 3]);
        assert!(velvet([0.0, 0.0])[0] > 50.0);
        for step in 0..200 {
            let spot = [
                -0.5 + step as f32 * 0.005,
                HALF_HEIGHT * ((step % 7) as f32 / 3.5 - 1.0),
            ];
            assert!(wood(spot)[0] > 20.0, "{spot:?}");
        }
    }

    #[test]
    fn noise_is_smooth_bounded_and_the_same_every_time() {
        for step in 0..300 {
            let (x, y) = (step as f32 * 0.137, step as f32 * 0.071);
            let value = fbm(x, y, 7, 3);
            assert!((0.0..=1.0).contains(&value));
            assert_eq!(value, fbm(x, y, 7, 3));
            let nearby = fbm(x + 0.001, y, 7, 3);
            assert!((value - nearby).abs() < 0.02, "smooth");
        }
    }
}
