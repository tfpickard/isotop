//! The room round the tanks: a slab of floor with the bays painted on it and, on the two sides
//! that face away from the camera, brick walls with arched windows. None of it measures anything
//! (the bays' places and names follow the cgroups, but the walls, tiles and windows are
//! decoration), and it is all opaque triangles.

use std::f32::consts::PI;

use super::{Ink, add, scale};
use crate::palette::tint;
use crate::plumbing::floor::Bay;
use crate::render::strokes::lit;
use crate::render::{Color, NONE, Point};

const SLAB: Color = [46, 48, 54];
const SIDE: Color = [40, 38, 44];
const BAY_FLOOR: Color = [56, 58, 66];
const PAINT: Color = [186, 152, 66];
const BRICK: Color = [176, 88, 66];
const STONE: Color = [128, 120, 108];
const GLASS_PANE: Color = [44, 82, 120];

/// The height of the walls, in world units.
pub const WALL: f32 = 3.0;
/// How thick the slab is.
const THICK: f32 = 0.55;

/// The floor slab over `low` to `high`, its four sides, the grid lines every four units, and the
/// walls on the sides that face away from the camera.
pub fn room(ink: &mut Ink, low: [f32; 2], high: [f32; 2]) {
    let corner = |x: f32, y: f32, z: f32| [x, y, z];
    ink.quad(
        [
            corner(low[0], low[1], 0.0),
            corner(high[0], low[1], 0.0),
            corner(high[0], high[1], 0.0),
            corner(low[0], high[1], 0.0),
        ],
        SLAB,
        NONE,
    );
    for (a, b) in [
        ([low[0], low[1]], [high[0], low[1]]),
        ([high[0], low[1]], [high[0], high[1]]),
        ([high[0], high[1]], [low[0], high[1]]),
        ([low[0], high[1]], [low[0], low[1]]),
    ] {
        ink.quad(
            [
                corner(a[0], a[1], 0.0),
                corner(b[0], b[1], 0.0),
                corner(b[0], b[1], -THICK),
                corner(a[0], a[1], -THICK),
            ],
            SIDE,
            NONE,
        );
    }
    let faint = tint(SLAB, 0.72);
    let mut x = (low[0] / 4.0).ceil() * 4.0;
    while x < high[0] {
        ink.band(
            &[corner(x, low[1], 0.01), corner(x, high[1], 0.01)],
            0.04,
            faint,
            None,
        );
        x += 4.0;
    }
    let mut y = (low[1] / 4.0).ceil() * 4.0;
    while y < high[1] {
        ink.band(
            &[corner(low[0], y, 0.01), corner(high[0], y, 0.01)],
            0.04,
            faint,
            None,
        );
        y += 4.0;
    }
    walls(ink, low, high);
}

/// A bay: its floor a little lighter than the slab, and its edge painted in safety yellow.
pub fn bay(ink: &mut Ink, bay: &Bay) {
    let ([x0, y0], [x1, y1]) = (bay.low, bay.high);
    ink.quad(
        [
            [x0, y0, 0.02],
            [x1, y0, 0.02],
            [x1, y1, 0.02],
            [x0, y1, 0.02],
        ],
        BAY_FLOOR,
        NONE,
    );
    let edge = [
        [x0, y0, 0.03],
        [x1, y0, 0.03],
        [x1, y1, 0.03],
        [x0, y1, 0.03],
        [x0, y0, 0.03],
    ];
    ink.band(&edge, 0.09, PAINT, None);
}

/// The walls on the sides of the room that face away from the camera, so they never stand
/// between it and what is inside.
fn walls(ink: &mut Ink, low: [f32; 2], high: [f32; 2]) {
    let toward = ink.facing;
    // (start, end) of each side, and the unit normal pointing into the room.
    let sides: [([f32; 2], [f32; 2], [f32; 2]); 4] = [
        ([low[0], low[1]], [low[0], high[1]], [1.0, 0.0]),
        ([low[0], low[1]], [high[0], low[1]], [0.0, 1.0]),
        ([high[0], low[1]], [high[0], high[1]], [-1.0, 0.0]),
        ([low[0], high[1]], [high[0], high[1]], [0.0, -1.0]),
    ];
    for (start, end, inward) in sides {
        // A wall is at the back when its inward normal points toward the viewer: the viewer
        // looks at it from inside the room.
        if inward[0] * toward[0] + inward[1] * toward[1] > 0.0 {
            wall(ink, start, end, inward);
        }
    }
}

fn wall(ink: &mut Ink, start: [f32; 2], end: [f32; 2], inward: [f32; 2]) {
    let at = |t: f32, z: f32| -> Point {
        [
            start[0] + (end[0] - start[0]) * t,
            start[1] + (end[1] - start[1]) * t,
            z,
        ]
    };
    let size = (end[0] - start[0]).hypot(end[1] - start[1]);
    let normal = [inward[0], inward[1], 0.0];
    ink.quad(
        [at(0.0, 0.0), at(1.0, 0.0), at(1.0, WALL), at(0.0, WALL)],
        lit(BRICK, normal),
        NONE,
    );
    // Courses of brick.
    let mortar = tint(lit(BRICK, normal), 0.55);
    let mut z = 0.5;
    while z < WALL - 0.3 {
        ink.band(&[at(0.0, z), at(1.0, z)], 0.025, mortar, None);
        z += 0.5;
    }
    // A stone coping along the top and a plinth at the foot.
    ink.band(
        &[at(0.0, WALL), at(1.0, WALL)],
        0.18,
        lit(STONE, [0.0, 0.0, 1.0]),
        None,
    );
    ink.band(
        &[at(0.0, 0.18), at(1.0, 0.18)],
        0.3,
        lit(STONE, normal),
        None,
    );
    // Arched windows every six units, set a little into the room's side of the wall.
    let windows = (size / 6.0).floor() as usize;
    let nudge = scale(normal, 0.02);
    for k in 0..windows {
        let centre = (k as f32 + 0.5) / windows as f32;
        let along = [
            (end[0] - start[0]) / size.max(1e-3),
            (end[1] - start[1]) / size.max(1e-3),
            0.0,
        ];
        let base = add(at(centre, 0.0), nudge);
        let corner = |across: f32, up: f32| add(base, add(scale(along, across), [0.0, 0.0, up]));
        let frame = tint(lit(STONE, normal), 1.1);
        ink.quad(
            [
                corner(-0.62, 0.8),
                corner(0.62, 0.8),
                corner(0.62, 2.1),
                corner(-0.62, 2.1),
            ],
            frame,
            NONE,
        );
        let pane = tint(GLASS_PANE, 1.3);
        let nudge2 = scale(normal, 0.03);
        let inset = |across: f32, up: f32| add(corner(across, up), nudge2);
        ink.quad(
            [
                inset(-0.5, 0.88),
                inset(0.5, 0.88),
                inset(0.5, 2.1),
                inset(-0.5, 2.1),
            ],
            pane,
            NONE,
        );
        // The arch above it: a half disc of pane in a half ring of stone.
        for (radius, color) in [(0.62_f32, frame), (0.5, pane)] {
            let rim = |step: usize| {
                let angle = PI * step as f32 / 12.0;
                add(
                    corner(radius * angle.cos(), 2.1 + radius * angle.sin()),
                    if radius > 0.55 { [0.0; 3] } else { nudge2 },
                )
            };
            let centre = add(
                corner(0.0, 2.1),
                if radius > 0.55 { [0.0; 3] } else { nudge2 },
            );
            for step in 0..12 {
                ink.frame
                    .facet(ink.camera, [centre, rim(step), rim(step + 1)], color, NONE);
            }
        }
        // The mullions.
        let bar = tint(STONE, 0.5);
        ink.band(&[inset(0.0, 0.88), inset(0.0, 2.6)], 0.04, bar, None);
        ink.band(&[inset(-0.5, 1.5), inset(0.5, 1.5)], 0.04, bar, None);
    }
}
