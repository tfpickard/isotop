//! Copper runs: a pipe along its route with a glass window in its longest leg, brass collars at
//! the window and flanges at the tanks, elbows at the corners, and in the window the state of
//! its water. A manifold where a writer's flow splits among several outlets, and a pipe that
//! bursts when its last reader is gone.
//!
//! The glass is not transparent (the rasterizers blend only halos), so the window is drawn as the
//! water inside it, a bright reflection along its top and brass at both ends; an empty window is
//! a pale tube with a few drops in it.

use std::f32::consts::TAU;

use super::{BRASS, COPPER, DULL, Ink, PIPE_R, PIPE_Z, WATER, WATER_HI, add, highlighted, scale};
use crate::palette::tint;
use crate::plumbing::path;
use crate::render::strokes::Dash;
use crate::render::{Color, Point};
use crate::simulation::Rng;

/// The state of the water in a pipe's window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Water {
    /// A reader waiting in `pipe_read` for an empty pipe: an empty window.
    Dry,
    /// A writer blocked in `pipe_write` on a full pipe: the glass bulges, packed with still water.
    Packed,
    /// A measured flow of zero: calm water, no motion.
    Calm,
    /// A measured flow: dashes that have travelled `forward` along the run, and `backward` the
    /// other way for a FIFO that runs both ways. Distances in world units, wrapped to the
    /// dash period by the caller.
    Flowing { forward: f32, backward: Option<f32> },
    /// A flow nobody measured: grey water, still. Not zero.
    Unmeasured,
}

/// A run to draw.
#[derive(Clone, Copy, Debug)]
pub struct Run<'a> {
    /// The route at `PIPE_Z`, tank wall to tank wall.
    pub points: &'a [Point],
    pub water: Water,
    /// Which thing clicking it picks.
    pub pick: u32,
    pub hot: bool,
    pub time: f32,
}

/// The pitch of the dashes of water, and how much of it is bright.
pub const DASH_PERIOD: f32 = 0.55;
const DASH_ON: f32 = 0.2;

/// The longest and shortest window, and what a window leaves of its leg at both ends.
const WINDOW: f32 = 2.2;
const WINDOW_PACKED: f32 = 2.6;
const MARGIN: f32 = 0.8;
const SHORTEST: f32 = 0.6;

/// How far the packed glass swells at its middle, in radii.
const BULGE: f32 = 0.75;

fn leg_lengths(points: &[Point]) -> Vec<f32> {
    points
        .windows(2)
        .map(|pair| path::distance(pair[0], pair[1]))
        .collect()
}

/// A short brass ring round the pipe at `point`, along `direction`.
fn collar(ink: &mut Ink, point: Point, direction: Point, color: Color, pick: u32) {
    let (before, after) = (
        add(point, scale(direction, -0.09)),
        add(point, scale(direction, 0.09)),
    );
    ink.prism(
        &[before, after],
        &[PIPE_R * 1.4, PIPE_R * 1.4],
        color,
        pick,
        8,
        true,
    );
}

/// Draws `run`.
pub fn pipe(ink: &mut Ink, run: &Run) {
    let points = run.points;
    if points.len() < 2 {
        return;
    }
    let copper = highlighted(COPPER, run.hot);
    let brass = highlighted(BRASS, run.hot);
    let total = path::length(points);
    let longest = if run.water == Water::Packed {
        WINDOW_PACKED
    } else {
        WINDOW
    };
    let window = path::window(&leg_lengths(points), longest, MARGIN, SHORTEST);
    match window {
        Some((from, to)) => {
            for piece in [
                path::between(points, 0.0, from),
                path::between(points, to, total),
            ] {
                if piece.len() >= 2 {
                    ink.pipe(&piece, PIPE_R, copper, run.pick);
                }
            }
            let glass = path::between(points, from, to);
            if glass.len() >= 2 {
                window_of(ink, &glass, run, from);
            }
            for at in [from, to] {
                if let Some((point, direction)) = path::at(points, at) {
                    collar(ink, point, direction, brass, run.pick);
                }
            }
        }
        None => ink.pipe(points, PIPE_R, copper, run.pick),
    }
    for corner in &points[1..points.len() - 1] {
        ink.knob(*corner, PIPE_R * 1.25, tint(copper, 0.95), run.pick);
    }
    for (end, next) in [(0, 1), (points.len() - 1, points.len() - 2)] {
        let direction = path::at(&[points[end], points[next]], 0.0).map(|(_, d)| d);
        if let Some(direction) = direction {
            collar(
                ink,
                add(points[end], scale(direction, 0.12)),
                direction,
                brass,
                run.pick,
            );
        }
    }
}

/// The window of a run: `glass` is its path, which starts `start` along the run.
fn window_of(ink: &mut Ink, glass: &[Point], run: &Run, start: f32) {
    let pick = run.pick;
    let toward = ink.toward;
    let lifted = |points: &[Point], by: f32| -> Vec<Point> {
        points.iter().map(|&p| add(p, scale(toward, by))).collect()
    };
    match run.water {
        Water::Dry => {
            ink.pipe(glass, PIPE_R, tint(super::GLASS, 0.9), pick);
            let length = path::length(glass);
            for k in 1..4 {
                if let Some((at, _)) = path::at(glass, length * (0.2 + 0.2 * k as f32)) {
                    ink.ball([at[0], at[1], at[2] - PIPE_R * 0.62], 0.045, WATER_HI);
                }
            }
        }
        Water::Packed => {
            let length = path::length(glass).max(1e-3);
            let pulse = 1.0 + 0.05 * (run.time * 7.0).sin();
            let (sections, radii) = swollen(glass, length, pulse);
            ink.prism(&sections, &radii, tint(WATER, 1.15), pick, 8, true);
            let shine: Vec<f32> = radii.iter().map(|r| r * 0.75).collect();
            let top: Vec<Point> = sections
                .iter()
                .zip(&shine)
                .map(|(p, r)| add(*p, [0.0, 0.0, *r]))
                .collect();
            ink.band(&lifted(&top, 0.0), 0.05, [236, 248, 255], None);
        }
        Water::Calm | Water::Flowing { .. } | Water::Unmeasured => {
            let body = if run.water == Water::Unmeasured {
                tint(DULL, 0.8)
            } else {
                WATER
            };
            ink.pipe(glass, PIPE_R * 0.9, highlighted(body, run.hot), pick);
            let top: Vec<Point> = glass
                .iter()
                .map(|&p| add(p, [0.0, 0.0, PIPE_R * 0.7]))
                .collect();
            ink.band(&top, 0.035, [236, 248, 255], None);
            let rise = PIPE_R * 0.92;
            match run.water {
                Water::Flowing { forward, backward } => {
                    // The dash pattern runs along the glass, which starts `start` along the run.
                    let dash = |travelled: f32| Dash {
                        on: DASH_ON,
                        off: DASH_PERIOD - DASH_ON,
                        phase: start - travelled,
                    };
                    ink.band(&lifted(glass, rise), 0.1, WATER_HI, Some(dash(forward)));
                    if let Some(back) = backward {
                        let reversed: Vec<Point> = glass.iter().rev().copied().collect();
                        let down: Vec<Point> = lifted(&reversed, rise * 0.8)
                            .into_iter()
                            .map(|p| add(p, [0.0, 0.0, -PIPE_R * 0.7]))
                            .collect();
                        let length = path::length(glass);
                        ink.band(
                            &down,
                            0.09,
                            tint(WATER_HI, 0.85),
                            Some(Dash {
                                on: DASH_ON,
                                off: DASH_PERIOD - DASH_ON,
                                phase: (length - start) - back,
                            }),
                        );
                    }
                }
                Water::Unmeasured => {
                    // Still, grey dashes: the pattern of moving water, standing, in no colour of
                    // water.
                    ink.band(
                        &lifted(glass, rise),
                        0.1,
                        tint(DULL, 1.35),
                        Some(Dash {
                            on: DASH_ON,
                            off: DASH_PERIOD - DASH_ON,
                            phase: 0.0,
                        }),
                    );
                }
                _ => {}
            }
        }
    }
}

/// `glass` cut into short sections with the radius of each, swollen by `BULGE` radii at its
/// middle: a window packed under pressure.
fn swollen(glass: &[Point], length: f32, pulse: f32) -> (Vec<Point>, Vec<f32>) {
    let steps = (length / 0.2).ceil().max(6.0) as usize;
    let mut points = Vec::with_capacity(steps + 1);
    let mut radii = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let u = k as f32 / steps as f32;
        if let Some((point, _)) = path::at(glass, u * length) {
            points.push(point);
            radii.push(PIPE_R * 0.95 * (1.0 + BULGE * pulse * (PI_F * u).sin().max(0.0).powf(1.6)));
        }
    }
    (points, radii)
}

const PI_F: f32 = std::f32::consts::PI;

/// A manifold: a brass box on the floor at `at` with a stopcock on top, joined to its tank by a
/// stub toward `tank`, and `stubs` capped stubs, one for each outlet nothing is drawn for. The
/// whole is `pick`, which is what clicking it selects.
pub fn manifold(
    ink: &mut Ink,
    at: [f32; 2],
    tank: [f32; 2],
    stubs: &[[f32; 2]],
    pick: u32,
    hot: bool,
) {
    let brass = highlighted(BRASS, hot);
    let [x, y] = at;
    let half = 0.42;
    ink.block(
        [x - half, y - half, PIPE_Z - 0.38],
        [x + half, y + half, PIPE_Z + 0.38],
        brass,
        pick,
    );
    let joint = [x + 0.0, y + 0.0, PIPE_Z];
    ink.pipe(
        &[[tank[0], tank[1], PIPE_Z], joint],
        PIPE_R * 1.1,
        highlighted(COPPER, hot),
        pick,
    );
    for stub in stubs {
        let direction = [stub[0] - x, stub[1] - y, 0.0];
        let size = direction[0].hypot(direction[1]).max(1e-3);
        let tip = [
            x + direction[0] / size * 0.95,
            y + direction[1] / size * 0.95,
            PIPE_Z,
        ];
        ink.pipe(&[[x, y, PIPE_Z], tip], PIPE_R * 0.8, brass, pick);
        ink.knob(tip, PIPE_R * 1.1, tint(brass, 1.1), pick);
    }
    let stem = [x, y, PIPE_Z + 0.38];
    ink.pipe(&[stem, [x, y, PIPE_Z + 0.78]], 0.06, brass, pick);
    let wheel: Vec<Point> = (0..=8)
        .map(|k| {
            let angle = k as f32 / 8.0 * TAU;
            [x + 0.27 * angle.cos(), y + 0.27 * angle.sin(), PIPE_Z + 0.8]
        })
        .collect();
    ink.band(&wheel, 0.05, super::RED, None);
}

/// A capped end where a pipe stops: a brass knob at `point`.
pub fn cap(ink: &mut Ink, point: Point, pick: u32) {
    ink.knob(point, PIPE_R * 1.35, BRASS, pick);
}

/// How the break of a pipe looks `age` seconds after its reader was gone: ragged copper ends
/// either side of `at`, a jet that falls as droplets for `SPRAY_SECONDS` and a puddle that
/// spreads, all seeded from `seed` so every frame draws the same droplets. `direction` runs along
/// the pipe toward where the reader was. After the spray the break is a capped end.
pub fn burst(ink: &mut Ink, at: Point, direction: Point, age: f32, mut seed: Rng, pick: u32) {
    let side = {
        let size = direction[0].hypot(direction[1]).max(1e-3);
        [-direction[1] / size, direction[0] / size, 0.0]
    };
    // Peeled petals of copper at the two ends of the break.
    for end in [-1.0_f32, 1.0] {
        let origin = add(at, scale(direction, 0.22 * end));
        for petal in 0..5 {
            let angle = petal as f32 / 5.0 * TAU + 0.4;
            let out = add(
                scale(side, angle.cos()),
                scale([0.0, 0.0, 1.0], angle.sin()),
            );
            let root = add(origin, scale(out, PIPE_R * 0.9));
            let tip = add(add(root, scale(direction, 0.12 * end)), scale(out, 0.16));
            ink.pipe(&[root, tip], 0.03, tint(COPPER, 0.9), pick);
        }
    }
    let spraying = (0.0..SPRAY_SECONDS).contains(&age);
    let grown = (0.35 + age.max(0.0) / 4.0).min(1.0);
    puddle(ink, [at[0], at[1]], 1.7 * grown, age);
    if !spraying {
        return;
    }
    for _ in 0..DROPLETS {
        let period = 0.45 + 0.45 * seed.unit();
        let offset = seed.unit() * period;
        let sign = if seed.unit() < 0.5 { -1.0 } else { 1.0 };
        let lateral = 0.6 + 1.8 * seed.unit();
        let along = 0.8 * seed.normal();
        let rise = 1.8 + 2.4 * seed.unit();
        let size = 0.04 + 0.05 * seed.unit();
        let step = (age + offset) % period;
        if age < step {
            continue;
        }
        let height = rise * step - 4.9 * step * step;
        if height < 0.0 {
            continue;
        }
        let position = add(
            add(at, scale(side, sign * lateral * step)),
            add(scale(direction, along * step), [0.0, 0.0, height + 0.05]),
        );
        ink.ball(position, size, [168, 214, 240]);
    }
    let mist = ink.pixels(1.6);
    ink.halo(add(at, [0.0, 0.0, 0.9]), mist, [130, 172, 204], 0.2);
    ink.halo(add(at, [0.0, 0.0, 0.3]), mist * 0.6, [150, 190, 220], 0.16);
}

/// How long the spray lasts, in seconds.
pub const SPRAY_SECONDS: f32 = 6.0;

/// How many droplets one jet throws.
const DROPLETS: usize = 56;

/// A puddle of `radius` on the floor round `at`, with a few rings spreading over it.
fn puddle(ink: &mut Ink, at: [f32; 2], radius: f32, age: f32) {
    let color = [24, 46, 66];
    let rim = |k: usize| {
        let angle = k as f32 / 24.0 * TAU;
        let wobble = 1.0 + 0.12 * (3.0 * angle + 1.0).sin() + 0.08 * (5.0 * angle).sin();
        [
            at[0] + radius * wobble * angle.cos() * 1.2,
            at[1] + radius * wobble * angle.sin() * 0.9,
            0.03,
        ]
    };
    let centre = [at[0], at[1], 0.03];
    for k in 0..24 {
        ink.frame.facet(
            ink.camera,
            [centre, rim(k), rim(k + 1)],
            color,
            crate::render::NONE,
        );
    }
    for ring in 0..3 {
        let phase = (age * 0.9 + ring as f32 / 3.0).fract();
        let reach = phase * radius * 0.9;
        let points: Vec<Point> = (0..=16)
            .map(|k| {
                let angle = k as f32 / 16.0 * TAU;
                [
                    at[0] + reach * angle.cos(),
                    at[1] + reach * angle.sin(),
                    0.04,
                ]
            })
            .collect();
        ink.band(
            &points,
            0.04,
            tint([150, 200, 230], 0.6 * (1.0 - phase)),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Identity;

    #[test]
    fn a_swollen_window_is_widest_in_the_middle_and_plain_at_the_ends() {
        let glass = [[0.0, 0.0, PIPE_Z], [2.0, 0.0, PIPE_Z]];
        let (points, radii) = swollen(&glass, 2.0, 1.0);
        assert_eq!(points.len(), radii.len());
        let widest = radii.iter().copied().fold(0.0, f32::max);
        assert!(widest > PIPE_R * 1.6, "{widest}");
        assert!((radii[0] - PIPE_R * 0.95).abs() < 1e-5);
        assert!((radii[radii.len() - 1] - PIPE_R * 0.95).abs() < 1e-3);
        assert!(radii[radii.len() / 2] >= widest - 1e-3);
    }

    #[test]
    fn the_droplets_of_a_burst_are_the_same_every_frame_and_fall_to_the_floor() {
        let seed = |k: u64| Rng::for_identity(Identity { pid: 4, start: 4 }, k);
        let draw = |age: f32, k: u64| {
            let camera = crate::render::Camera {
                zoom: 40.0,
                ..crate::render::Camera::default()
            };
            let mut frame = crate::render::Frame::new(
                160,
                90,
                crate::render::Sky {
                    backdrop: crate::render::Backdrop::Dusk,
                    haze: [0.0; 3],
                    time: 0.0,
                },
                crate::render::Buffers::default(),
            );
            let mut ink = Ink::new(&mut frame, &camera);
            burst(
                &mut ink,
                [0.0, 0.0, PIPE_Z],
                [1.0, 0.0, 0.0],
                age,
                seed(k),
                5,
            );
            ink.finish();
            format!("{:?}", frame.items)
        };
        assert_eq!(
            draw(2.0, 1),
            draw(2.0, 1),
            "the same age and seed draw the same frame"
        );
        assert_ne!(
            draw(2.0, 1),
            draw(2.0, 2),
            "another pipe's burst is another jet"
        );
        assert_ne!(draw(2.0, 1), draw(2.3, 1), "the jet moves");
        // After the spray the break is a puddle and ragged ends and no droplets.
        let after = draw(SPRAY_SECONDS + 1.0, 1);
        assert!(!after.is_empty());
    }
}
