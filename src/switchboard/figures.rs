//! What stands at each position of the switchboard: the operator in her chair, the telephone on
//! the shelf, and the clock and the two inter-office lamps on the rail. The shapes follow the
//! prototype; motion is the busy share of the CPU, the CPU's timer count, and the rates of the
//! named interrupt rows.

use std::f32::consts::{PI, TAU};

use crate::palette::{mix, tint};
use crate::render::{Color, NONE};

use super::cord::Spot;
use super::scenery::{OPERATOR_DEPTH, Pen, Plan, TOP_Z};

const DRESS: [Color; 6] = [
    [52, 58, 86],
    [86, 38, 44],
    [40, 70, 60],
    [70, 66, 74],
    [90, 70, 50],
    [44, 48, 70],
];
const HAIR: [Color; 4] = [[40, 26, 18], [24, 18, 16], [92, 52, 28], [60, 38, 24]];
const SKIN: Color = [214, 168, 136];
const NECK: Color = [190, 146, 118];
const CHAIR: Color = [86, 50, 28];
const LEG: Color = [58, 34, 20];
const GREEN_OFF: Color = [30, 46, 30];
const GREEN_ON: Color = [120, 240, 140];
const RED_OFF: Color = [54, 22, 18];
const RED_ON: Color = [255, 70, 56];

/// How far an operator reaches, from her busy share and the two phases of her motion.
pub struct Motion {
    pub busy: f32,
    pub sway: f32,
    pub reach: f32,
}

/// Layers are this far apart in depth, so a figure's parts overlap in a fixed order.
const LAYER: f32 = 0.6;

/// An operator seen from behind, seated in front of the keyshelf.
pub fn operator(pen: &mut Pen, plan: &Plan, seat: usize, motion: &Motion) {
    let s = plan.scale;
    let body = plan.operator_x(seat) - 20.0 * s;
    let shoulder = plan.shoulder_y();
    let at = |x: f32, y: f32, layer: f32| -> Spot {
        [
            body + x * s,
            OPERATOR_DEPTH + layer * LAYER,
            TOP_Z - shoulder - y * s,
        ]
    };
    let dress = DRESS[seat % DRESS.len()];
    let hair = HAIR[(seat * 3) % HAIR.len()];
    let busy = motion.busy.clamp(0.0, 1.0);
    let swing = motion.sway.sin() * (1.0 + 5.0 * busy);
    let reach = 0.5 + 0.5 * motion.reach.sin();
    let lean = 0.3 * swing;
    let hand_right = (19.0 + 5.0 * reach * busy, -20.0 - 26.0 * busy * reach);
    let arms = [
        (
            (-17.0 + lean, 16.0),
            (-27.0, 34.0),
            (-19.0 - 3.0 * busy, -12.0),
            0.85,
        ),
        (
            (17.0 + lean, 16.0),
            (27.0, 34.0 - 8.0 * busy * reach),
            hand_right,
            0.78,
        ),
    ];
    for (from, elbow, hand, shade) in arms {
        pen.stroke(
            &[
                at(from.0, from.1, -2.0),
                at(elbow.0, elbow.1, -2.0),
                at(hand.0, hand.1, -2.0),
            ],
            7.0 * s,
            tint(dress, shade),
            NONE,
        );
        pen.ball(at(hand.0, hand.1, -2.0), 3.6 * s, SKIN, NONE);
    }
    let torso = [
        (-21.0, 70.0),
        (21.0, 70.0),
        (20.0, 14.0),
        (12.0, 6.0),
        (-12.0, 6.0),
        (-20.0, 14.0),
    ]
    .map(|(x, y)| at(x + lean, y, 0.0));
    pen.corners([torso[0], torso[1], torso[2], torso[5]], dress, NONE);
    pen.corners([torso[5], torso[2], torso[3], torso[4]], dress, NONE);
    pen.stroke(
        &[at(lean, 10.0, 0.5), at(lean, 64.0, 0.5)],
        s,
        tint(dress, 0.7),
        NONE,
    );
    let head = (0.45 * swing, -6.0);
    pen.stroke(
        &[at(head.0, head.1 + 8.0, 0.5), at(lean, 8.0, 0.5)],
        7.0 * s,
        NECK,
        NONE,
    );
    pen.ball(at(head.0, head.1, 1.0), 11.5 * s, hair, NONE);
    for (dx, dy, radius, shade) in [
        (-7.0, -8.0, 5.5, 1.15),
        (7.0, -8.0, 5.5, 1.15),
        (0.0, 9.0, 6.5, 1.25),
    ] {
        pen.ball(
            at(head.0 + dx, head.1 + dy, 1.5),
            radius * s,
            tint(hair, shade),
            NONE,
        );
    }
    let band: Vec<Spot> = (0..=8)
        .map(|k| {
            let angle = (185.0 + 170.0 * k as f32 / 8.0).to_radians();
            at(
                head.0 + 12.5 * angle.cos(),
                head.1 - 1.0 + 13.5 * angle.sin(),
                2.0,
            )
        })
        .collect();
    pen.stroke(&band, 1.8 * s, [34, 34, 38], NONE);
    pen.ball(
        at(head.0 + 12.0, head.1 + 1.0, 2.0),
        4.2 * s,
        [28, 28, 32],
        NONE,
    );
    pen.stroke(
        &[
            at(head.0 + 11.0, head.1 + 5.0, 2.0),
            at(head.0 + 6.0, head.1 + 20.0, 2.0),
        ],
        s,
        [30, 30, 34],
        NONE,
    );
    let back = [
        (-16.0, 92.0),
        (16.0, 92.0),
        (17.0, 46.0),
        (11.0, 38.0),
        (-11.0, 38.0),
        (-17.0, 46.0),
    ];
    let chair = |scale: f32, color: Color, pen: &mut Pen, layer: f32| {
        let p = back.map(|(x, y)| at(x * scale, 60.0 + (y - 60.0) * scale, layer));
        pen.corners([p[0], p[1], p[2], p[5]], color, NONE);
        pen.corners([p[5], p[2], p[3], p[4]], color, NONE);
    };
    chair(1.0, CHAIR, pen, 3.0);
    chair(0.86, [104, 62, 34], pen, 3.5);
    for side in [-1.0, 1.0] {
        pen.stroke(
            &[at(14.0 * side, 92.0, 3.0), at(16.0 * side, 140.0, 3.0)],
            3.5 * s,
            LEG,
            NONE,
        );
    }
}

/// The name plate's place: under the chair, in board coordinates.
pub fn plate(plan: &Plan, seat: usize) -> Spot {
    let s = plan.scale;
    [
        plan.operator_x(seat) - 16.0 * s,
        OPERATOR_DEPTH,
        TOP_Z - plan.shoulder_y() - 160.0 * s,
    ]
}

/// The plug seats of an operator on the keyshelf: five at least, and one for every plug.
pub fn holes(pen: &mut Pen, plan: &Plan, seat: usize, plugs: usize) {
    for slot in 0..plugs.max(5) {
        let [x, d, z] = plan.hole(seat, slot);
        pen.flat(
            [x, d, TOP_Z - z - 0.05],
            (3.0 * plan.scale, 1.6 * plan.scale),
            6,
            [14, 9, 6],
        );
    }
}

/// The telephone of an operator, on the shelf; `rattle` is the scene time when its line is
/// ringing, which shakes the handset and draws the ring marks.
pub fn telephone(pen: &mut Pen, plan: &Plan, seat: usize, rattle: Option<f32>) {
    let s = plan.scale;
    let x = plan.operator_x(seat) + 30.0 * s;
    let z = TOP_Z - plan.shelf_y();
    let depth = 50.0;
    let body = [12, 10, 9];
    let jitter = rattle.map_or(0.0, |time| (time * 95.0 + seat as f32).sin() * 1.6 * s);
    pen.corners(
        [
            [x - 11.0 * s, depth + 6.0 * s, z + 1.0],
            [x + 11.0 * s, depth + 6.0 * s, z + 1.0],
            [x + 8.0 * s, depth - 6.0 * s, z + 6.0 * s],
            [x - 8.0 * s, depth - 6.0 * s, z + 6.0 * s],
        ],
        body,
        NONE,
    );
    pen.corners(
        [
            [x - 11.0 * s, depth + 6.0 * s, z],
            [x + 11.0 * s, depth + 6.0 * s, z],
            [x + 11.0 * s, depth + 6.0 * s, z + 1.0],
            [x - 11.0 * s, depth + 6.0 * s, z + 1.0],
        ],
        body,
        NONE,
    );
    let lift =
        (8.0 + if rattle.is_some() {
            3.5 * s + jitter.abs()
        } else {
            0.0
        }) * s.max(0.5);
    let left: Spot = [x - 11.0 * s, depth, z + 6.0 * s + lift];
    let right: Spot = [x + 11.0 * s, depth, z + 6.0 * s + lift - jitter * 0.6];
    pen.stroke(&[left, right], 4.2 * s, body, NONE);
    pen.ball(left, 3.4 * s, body, NONE);
    pen.ball(right, 3.4 * s, body, NONE);
    pen.ball(
        [x, depth + 2.0 * s, z + 6.5 * s],
        3.0 * s,
        [210, 200, 170],
        NONE,
    );
    if rattle.is_some() {
        let centre: Spot = [x, depth, z + 6.0 * s + lift + 4.0 * s];
        for radius in [17.0 * s, 23.0 * s] {
            for (from, to) in [(150.0_f32, 210.0_f32), (-30.0, 30.0)] {
                let arc: Vec<Spot> = (0..=6)
                    .map(|k| {
                        let angle = (from + (to - from) * k as f32 / 6.0).to_radians();
                        [
                            centre[0] + radius * angle.cos(),
                            centre[1],
                            centre[2] + radius * 0.85 * angle.sin(),
                        ]
                    })
                    .collect();
                pen.stroke(&arc, 1.4 * s, [255, 240, 200], NONE);
            }
        }
    }
}

/// What the rail of one operator shows.
pub struct Rail {
    /// Turns of the clock's hand, in revolutions (the fraction is the hand's place); None when
    /// the machine has no timer row, and the clock stops at twelve.
    pub revolutions: Option<f64>,
    /// How lit the green inter-office lamp is, 0 to 1.
    pub calls: f32,
    /// Whether a non-maskable interrupt lit the red lamp lately.
    pub nmi: bool,
}

/// The clock and the two inter-office lamps of an operator.
pub fn rail(pen: &mut Pen, plan: &Plan, seat: usize, rail: &Rail) {
    let s = plan.scale;
    let x = plan.operator_x(seat);
    let y = plan.rail_y();
    pen.disc([x - 8.0 * s, y, 1.0], 11.0 * s, 12, [90, 66, 40], NONE);
    pen.disc([x - 8.0 * s, y, 1.3], 9.8 * s, 12, [214, 204, 176], NONE);
    let turn = rail.revolutions.map_or(0.0, |revolutions| {
        // The hand ticks: it moves in sixtieths of a turn.
        ((revolutions.rem_euclid(1.0) * 60.0).floor() / 60.0) as f32
    });
    let angle = -PI / 2.0 + turn * TAU;
    let hand = |length: f32, angle: f32| -> [Spot; 2] {
        let z = TOP_Z - y;
        [
            super::scenery::flat_spot(x - 8.0 * s, 1.8, z),
            super::scenery::flat_spot(
                x - 8.0 * s + length * s * angle.cos(),
                1.8,
                z - length * s * angle.sin(),
            ),
        ]
    };
    pen.stroke(&hand(8.0, angle), 1.2 * s.max(0.6), [40, 20, 14], NONE);
    pen.stroke(
        &hand(5.0, angle / 12.0 + 1.0),
        1.5 * s.max(0.6),
        [40, 30, 24],
        NONE,
    );
    let lamp = |pen: &mut Pen, dy: f32, off: Color, on: Color, lit: f32| {
        let color = mix(off, on, lit.clamp(0.0, 1.0));
        pen.disc(
            [x + 12.0 * s, y + dy * s, 1.0],
            4.4 * s,
            8,
            [20, 16, 14],
            NONE,
        );
        pen.disc([x + 12.0 * s, y + dy * s, 1.3], 3.6 * s, 8, color, NONE);
        if lit > 0.05 {
            let radius = pen.glow_radius(7.0 * (0.5 + 0.5 * s));
            pen.glow(
                [x + 12.0 * s, 2.0, TOP_Z - y - dy * s],
                radius,
                on,
                0.4 * lit,
            );
        }
    };
    lamp(pen, -7.0, GREEN_OFF, GREEN_ON, rail.calls);
    lamp(pen, 7.0, RED_OFF, RED_ON, if rail.nmi { 1.0 } else { 0.0 });
}
