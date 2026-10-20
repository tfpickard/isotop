//! What stands on the table besides the board: the edge of the board, two candles with their
//! flames, and the planchette, a heart of pale wood with a brass rim and a round window over the
//! letter it points at. All of it is decoration but the planchette's place and its lit window.

use std::f32::consts::TAU;

use super::layout::{HALF_HEIGHT, Plane, SCALE, Spot, THICKNESS};
use super::scenery::{CANDLES, Candle, FLAME_LIFT, Scenery};
use crate::palette::{mix, tint};
use crate::render::strokes::Width;
use crate::render::{Camera, Color, Frame, Item, NONE, Point, SCALE as PIXELS};

/// The sides of the board, shaded: the front and the two flanks.
const EDGE_FRONT: Color = [66, 40, 24];
const EDGE_FLANK: Color = [46, 28, 17];
const WAX_LOW: Color = [224, 150, 96];
const WAX: Color = [255, 214, 150];
const WAX_LIT: Color = [255, 252, 232];
const DISH: Color = [198, 150, 72];
const FLAME_OUTER: Color = [255, 150, 48];
const FLAME_MIDDLE: Color = [255, 208, 96];
const FLAME_CORE: Color = [255, 244, 200];
const PLANCHETTE_TOP: Color = [184, 166, 130];
const PLANCHETTE_SIDE: Color = [104, 90, 68];
const BRASS: Color = [218, 172, 90];
const AMBER: Color = [255, 190, 72];

/// How far a candle's halo reaches at most, as a share of the frame's height.
const HALO: f32 = 0.12;
/// How high the planchette stands over the board, in board widths, and how far its window is
/// from its centre: none. Its half width, and the scale of the heart curve to board widths.
const HEIGHT: f32 = 0.016;
const HALF_WIDTH: f32 = 0.085;
const HEART: f32 = HALF_WIDTH / 16.0;
/// The window's radius, in board widths: less than a letter's spacing, so it holds one.
const WINDOW: f32 = 0.024;
/// Where in the heart curve's own units the window's centre is: up from its middle, into the lobes.
const WINDOW_AT: f32 = 2.0;
/// Points on the heart's outline.
const OUTLINE: usize = 40;

/// The four sides of the board, each a quad from the top of the wood down to the table.
pub fn board_edge(frame: &mut Frame, camera: &Camera, plane: &Plane) {
    let half = [0.5, HALF_HEIGHT];
    let top = plane.corners(half, 0.0);
    let bottom = plane.corners(half, -THICKNESS);
    for (side, color) in [
        (0, EDGE_FRONT),
        (1, EDGE_FLANK),
        (2, EDGE_FLANK),
        (3, EDGE_FLANK),
    ] {
        let next = (side + 1) % 4;
        frame.quad(
            camera,
            [top[side], top[next], bottom[next], bottom[side]],
            color,
            NONE,
        );
    }
}

/// The candles' bodies and dishes, and the flames of both, drawn as one: the wax first, then
/// the flames over it and last the halos, which blend over everything.
pub fn candles(frame: &mut Frame, camera: &Camera, plane: &Plane, flicker: [f32; 2], time: f32) {
    for candle in CANDLES {
        let base = plane.world(candle.spot, -THICKNESS);
        let rise = |height: f32| [base[0], base[1], base[2] + height * SCALE];
        let radius = candle.radius * SCALE;
        frame.tube(
            camera,
            &[base, rise(0.008)],
            &[radius * 1.8; 2],
            DISH,
            NONE,
            16,
            true,
        );
        // The wax is warmer and brighter nearer the flame that lights it.
        for (from, to, color) in [
            (0.008, 0.6 * candle.height, WAX_LOW),
            (0.6 * candle.height, 0.86 * candle.height, WAX),
            (0.86 * candle.height, candle.height, WAX_LIT),
        ] {
            frame.tube(
                camera,
                &[rise(from), rise(to)],
                &[radius; 2],
                color,
                NONE,
                14,
                true,
            );
        }
    }
    for (index, candle) in CANDLES.into_iter().enumerate() {
        flame(frame, camera, plane, (index, candle), flicker[index], time);
    }
}

/// One flame: three nested tongues in screen space that sway, and two halos whose strength is the
/// candle's flicker. The halo's radius is capped at 0.12 of the frame's height.
fn flame(
    frame: &mut Frame,
    camera: &Camera,
    plane: &Plane,
    (index, candle): (usize, Candle),
    strength: f32,
    time: f32,
) {
    let top = plane.world(candle.spot, -THICKNESS + candle.height + FLAME_LIFT * 0.4);
    let at = frame.project(camera, top);
    let height = frame.height as f32;
    let size = 0.05 * height;
    let sway = 0.07 * size * (time * 7.0 + index as f32 * 2.3).sin();
    let depth = at[2] + 0.6;
    let tongue = |scale: f32, color: Color, lift: f32| {
        let (width, rise) = (0.34 * scale * size, scale * size);
        let base = at[1] - lift * size;
        let point = |x: f32, y: f32| [at[0] + x, y, depth + lift];
        [
            Item::Triangle(
                [
                    point(sway * scale, base - rise),
                    point(-width, base + 0.12 * rise),
                    point(width, base + 0.12 * rise),
                ],
                color,
                NONE,
            ),
            Item::Triangle(
                [
                    point(-width, base + 0.12 * rise),
                    point(width, base + 0.12 * rise),
                    point(0.0, base + 0.42 * rise),
                ],
                color,
                NONE,
            ),
        ]
    };
    for (scale, color, lift) in [
        (1.0, FLAME_OUTER, 0.0),
        (0.72, FLAME_MIDDLE, 0.04),
        (0.4, FLAME_CORE, 0.08),
    ] {
        frame.items.extend(tongue(scale, color, lift));
    }
    let centre = [at[0] + sway * 0.4, at[1] - 0.3 * size];
    for (radius, color, weight) in [
        (HALO * height, [255, 120, 50], 0.55),
        (0.05 * height, [255, 190, 90], 0.55),
    ] {
        frame.items.push(Item::Puff {
            center: centre,
            radius: radius.min(HALO * height),
            color,
            strength: (weight * strength).clamp(0.0, 1.0),
        });
    }
}

/// The heart curve's outline in board coordinates around the window's centre, its point to the
/// far edge, turned by `tilt` radians: `OUTLINE` points, each with its angle round the window.
fn outline(tilt: f32) -> Vec<(Spot, f32)> {
    let (sin, cos) = tilt.sin_cos();
    (0..OUTLINE)
        .map(|step| {
            let t = TAU * step as f32 / OUTLINE as f32;
            let x = 16.0 * t.sin().powi(3);
            let y =
                13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            // The heart's point is down in its own units; the board's far edge is up.
            let (x, y) = (x * HEART, -(y - WINDOW_AT) * HEART);
            ([x * cos - y * sin, x * sin + y * cos], y.atan2(x) + tilt)
        })
        .collect()
}

/// The planchette with its window over `at`, turned `tilt` radians: a shadow, the heart's sides
/// and its top with a hole for the window, the brass of its rim and of the window's ring, and
/// the glow of the letter beneath when `lit`.
pub fn planchette(
    frame: &mut Frame,
    camera: &Camera,
    plane: &Plane,
    (at, tilt): (Spot, f32),
    (flicker, lit): ([f32; 2], bool),
) {
    let height = frame.height as f32;
    let per_width = SCALE * camera.zoom * PIXELS;
    let shadow = frame.project(camera, plane.world([at[0] + 0.012, at[1] - 0.02], 0.0));
    frame.items.push(Item::Puff {
        center: [shadow[0], shadow[1]],
        radius: 0.1 * per_width,
        color: [0, 0, 0],
        strength: 0.55,
    });
    let outline = outline(tilt);
    let place = |[x, y]: Spot, lift: f32| plane.world([at[0] + x, at[1] + y], lift);
    let window = |angle: f32| [WINDOW * angle.cos(), WINDOW * angle.sin()];
    let shade = |color: Color, spot: Spot, share: f32| {
        let light = Scenery::light_at([at[0] + spot[0], at[1] + spot[1]], flicker);
        tint(color, ((0.45 + 0.55 * light) * share).clamp(0.2, 1.25))
    };
    for (index, &(spot, angle)) in outline.iter().enumerate() {
        let (next, next_angle) = outline[(index + 1) % OUTLINE];
        frame.quad(
            camera,
            [
                place(spot, 0.0),
                place(next, 0.0),
                place(next, HEIGHT),
                place(spot, HEIGHT),
            ],
            shade(PLANCHETTE_SIDE, spot, 1.0),
            NONE,
        );
        frame.quad(
            camera,
            [
                place(spot, HEIGHT),
                place(next, HEIGHT),
                place(window(next_angle), HEIGHT),
                place(window(angle), HEIGHT),
            ],
            shade(PLANCHETTE_TOP, spot, 1.0),
            NONE,
        );
    }
    let rim: Vec<Point> = outline
        .iter()
        .chain(outline.first())
        .map(|&(spot, _)| place([spot[0] * 0.92, spot[1] * 0.92], HEIGHT * 1.05))
        .collect();
    frame.ribbon(
        camera,
        &rim,
        Width::Pixels((0.0024 * per_width).max(1.0)),
        BRASS,
        NONE,
        None,
    );
    let ring: Vec<Point> = (0..=24)
        .map(|step| place(window(TAU * step as f32 / 24.0 * 0.999_9), HEIGHT * 1.1))
        .collect();
    frame.ribbon(
        camera,
        &ring,
        Width::Pixels((0.0034 * per_width).max(1.0)),
        BRASS,
        NONE,
        None,
    );
    if lit {
        let glow = frame.project(camera, place([0.0, 0.0], 0.0));
        for (radius, color, strength) in [
            (0.055 * per_width, AMBER, 0.55),
            (0.03 * per_width, mix(AMBER, [255, 250, 220], 0.6), 0.5),
        ] {
            frame.items.push(Item::Puff {
                center: [glow[0], glow[1]],
                radius: radius.min(HALO * height),
                color,
                strength,
            });
        }
    }
}
