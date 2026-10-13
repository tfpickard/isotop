//! The room of the switchboard: where everything stands, and the furniture that carries no data
//! (the wood of the board, the keyshelf, the cabinet and the floor). The numbers are the
//! prototype's design pixels, `x` along the board, `d` towards the viewer and `z` up, turned
//! into world units by `Pen`.

use std::f32::consts::{FRAC_1_SQRT_2, TAU};

use crate::palette::tint;
use crate::render::strokes::Width;
use crate::render::{Camera, Color, Frame, NONE, Point};
use crate::simulation::hash::unit_from;

use super::cord::Spot;

/// World units per design pixel.
pub const UNIT: f32 = 0.1;
/// How far a point towards the viewer slides left on the screen, per unit of depth: the slight
/// yaw of the prototype's oblique view, built into the layout because the camera has no shear.
const SHEAR: f32 = -0.2;
/// Height of the face-pixel row 0 above the floor of the prototype's drawing.
pub const TOP_Z: f32 = 700.0;

pub const BOARD: (f32, f32) = (62.0, 1216.0);
pub const CORNICE: f32 = 64.0;
pub const FIRST_ROW: f32 = 124.0;
pub const ROW_PITCH: f32 = 82.0;
const FIRST_COLUMN: f32 = 112.0;
const COLUMN_SPAN: f32 = 1012.0;
pub const SHELF_DEPTH: f32 = 74.0;
pub const OPERATOR_DEPTH: f32 = 150.0;
pub const SOFTIRQ_BOARD: (f32, f32) = (1252.0, 1578.0);
/// The deepest the cornice and the cabinets reach behind the face.
pub const BEHIND: f32 = 46.0;
/// Seats along the operators' row share this width.
const OPERATOR_SPAN: f32 = 1113.6;
const OPERATOR_PITCH: f32 = OPERATOR_SPAN / 16.0;
const OPERATORS_CENTRE: f32 = 620.0;

const OAK: Color = [118, 70, 38];
const DARK_OAK: Color = [74, 44, 26];
const FLOOR: Color = [46, 30, 22];
const BAKELITE: Color = [24, 17, 14];
const IVORY: Color = [208, 192, 152];

/// Where the pieces of the board stand for a grid of `columns` by `rows` jacks and `seats`
/// operators.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub columns: usize,
    pub rows: usize,
    pub seats: usize,
    /// How far the lower furniture sits below the prototype's, for rows beyond the fourth.
    pub drop: f32,
    /// The distance between operators, and the scale of an operator's figure.
    pub pitch: f32,
    pub scale: f32,
    /// The distance between the lamps of a softirq row.
    pub lamp_pitch: f32,
}

impl Plan {
    pub fn new(columns: usize, rows: usize, seats: usize) -> Plan {
        let seats = seats.max(1);
        let pitch = (OPERATOR_SPAN / seats as f32).clamp(OPERATOR_SPAN / 64.0, 111.0);
        Plan {
            columns,
            rows,
            seats,
            drop: (rows.max(4) - 4) as f32 * ROW_PITCH,
            pitch,
            scale: (pitch / OPERATOR_PITCH).min(1.0),
            lamp_pitch: (230.0 / seats as f32).min(14.6),
        }
    }

    /// The lamp of jack place `place`: its x and its face y. The socket is 24 below.
    pub fn jack(&self, place: usize) -> (f32, f32) {
        let (row, column) = (place / self.columns, place % self.columns);
        let step = COLUMN_SPAN / (self.columns as f32 - 1.0).max(1.0);
        (
            FIRST_COLUMN + column as f32 * step,
            FIRST_ROW + row as f32 * ROW_PITCH,
        )
    }

    /// The distance between jack columns.
    pub fn column_step(&self) -> f32 {
        COLUMN_SPAN / (self.columns as f32 - 1.0).max(1.0)
    }

    pub fn last_row_y(&self) -> f32 {
        FIRST_ROW + (self.rows.max(4) - 1) as f32 * ROW_PITCH
    }

    /// Face y of the back edge of the keyshelf.
    pub fn shelf_y(&self) -> f32 {
        482.0 + self.drop
    }

    /// Face y of the clocks and the inter-office lamps' rail.
    pub fn rail_y(&self) -> f32 {
        self.last_row_y() + 83.0
    }

    /// The x that the prototype measures an operator's things from.
    pub fn operator_x(&self, seat: usize) -> f32 {
        OPERATORS_CENTRE + (seat as f32 - (self.seats as f32 - 1.0) / 2.0) * self.pitch
    }

    /// The plug seat `slot` of operator `seat` on the keyshelf: eight to a row, rows going back.
    pub fn hole(&self, seat: usize, slot: usize) -> Spot {
        let (row, column) = ((slot / 8).min(3), slot % 8);
        [
            self.operator_x(seat) + (-14.0 + column as f32 * 8.5) * self.scale,
            26.0 + 10.0 * row as f32,
            TOP_Z - self.shelf_y(),
        ]
    }

    /// The height of an operator's shoulders, in face y at the operator's depth.
    pub fn shoulder_y(&self) -> f32 {
        548.0 + self.drop - 0.486 * OPERATOR_DEPTH
    }

    /// The floor's height above the drawing's zero.
    pub fn floor(&self) -> f32 {
        TOP_Z - self.shelf_y() - 158.0
    }

    /// The corners of everything in the room, for the camera fit.
    pub fn bounds(&self) -> Vec<Point> {
        let (left, right) = (BOARD.0 - 8.0, SOFTIRQ_BOARD.1 + 8.0);
        let top = TOP_Z - CORNICE + 4.0;
        let bottom = self
            .floor()
            .min(TOP_Z - self.shoulder_y() - 175.0 * self.scale)
            - 6.0;
        let mut corners = Vec::new();
        for x in [left, right] {
            for d in [-BEHIND, OPERATOR_DEPTH + 20.0] {
                for z in [bottom, top] {
                    corners.push(point(x, d, z));
                }
            }
        }
        corners
    }
}

/// A board point in world space.
pub fn point(x: f32, d: f32, z: f32) -> Point {
    let across = (x + SHEAR * d) * UNIT;
    let along = d * UNIT;
    [
        (across + along) * FRAC_1_SQRT_2,
        (along - across) * FRAC_1_SQRT_2,
        z * UNIT,
    ]
}

/// A hash of `seed` as a number from 0 up to 1 (exclusive).
pub fn unit(seed: u32) -> f32 {
    unit_from(seed.wrapping_mul(0x9e37_79b1) ^ 0x85eb_ca6b)
}

/// Draws in board coordinates.
pub struct Pen<'a> {
    pub frame: &'a mut Frame,
    pub camera: &'a Camera,
}

impl Pen<'_> {
    /// Screen pixels in one design pixel at the current zoom.
    pub fn pixel(&self) -> f32 {
        UNIT * crate::render::SCALE * self.camera.zoom
    }

    /// The radius in pixels of a glow of `design` design pixels: at most 12 pixels at 1080 lines,
    /// scaled with the frame, and never under 2.5.
    pub fn glow_radius(&self, design: f32) -> f32 {
        let most = 12.0 * self.frame.height as f32 / 1080.0;
        (design * self.pixel()).min(most).max(2.5_f32.min(most))
    }

    pub fn corners(&mut self, corners: [Spot; 4], color: Color, pick: u32) {
        let world = corners.map(|[x, d, z]| point(x, d, z));
        self.frame.quad(self.camera, world, color, pick);
    }

    /// A rectangle on the plane `d`, from face y `top` to `bottom`.
    pub fn face(&mut self, x: (f32, f32), y: (f32, f32), d: f32, color: Color) {
        self.face_picking(x, y, d, color, NONE);
    }

    pub fn face_picking(&mut self, x: (f32, f32), y: (f32, f32), d: f32, color: Color, pick: u32) {
        let (z0, z1) = (TOP_Z - y.0, TOP_Z - y.1);
        self.corners(
            [[x.0, d, z0], [x.1, d, z0], [x.1, d, z1], [x.0, d, z1]],
            color,
            pick,
        );
    }

    /// A horizontal rectangle at face y `y`, from depth `d.0` to `d.1`.
    pub fn top(&mut self, x: (f32, f32), d: (f32, f32), y: f32, color: Color) {
        let z = TOP_Z - y;
        self.corners(
            [[x.0, d.0, z], [x.1, d.0, z], [x.1, d.1, z], [x.0, d.1, z]],
            color,
            NONE,
        );
    }

    /// A rectangle on the plane `x`, from depth `d.0` to `d.1` and face y `top` to `bottom`.
    pub fn side(&mut self, x: f32, d: (f32, f32), y: (f32, f32), color: Color) {
        let (z0, z1) = (TOP_Z - y.0, TOP_Z - y.1);
        self.corners(
            [[x, d.0, z0], [x, d.1, z0], [x, d.1, z1], [x, d.0, z1]],
            color,
            NONE,
        );
    }

    /// A disc on the plane `d` facing the viewer, centred at `[x, face y, d]`.
    pub fn disc(&mut self, at: [f32; 3], radius: f32, sides: usize, color: Color, pick: u32) {
        let [x, y, d] = at;
        let centre = point(x, d, TOP_Z - y);
        let ring = |k: usize| {
            let angle = TAU * k as f32 / sides as f32;
            point(
                x + radius * angle.cos(),
                d,
                TOP_Z - y - radius * angle.sin(),
            )
        };
        for k in 0..sides {
            self.frame
                .facet(self.camera, [centre, ring(k), ring(k + 1)], color, pick);
        }
    }

    /// A disc lying flat at `[x, d, face y]`, `radii` wide and deep.
    pub fn flat(&mut self, at: [f32; 3], radii: (f32, f32), sides: usize, color: Color) {
        let [x, d, y] = at;
        let (rx, rd) = radii;
        let z = TOP_Z - y;
        let centre = point(x, d, z);
        let ring = |k: usize| {
            let angle = TAU * k as f32 / sides as f32;
            point(x + rx * angle.cos(), d + rd * angle.sin(), z)
        };
        for k in 0..sides {
            self.frame
                .facet(self.camera, [centre, ring(k), ring(k + 1)], color, NONE);
        }
    }

    /// A shaded ball of `radius` design pixels.
    pub fn ball(&mut self, at: Spot, radius: f32, color: Color, pick: u32) {
        let centre = point(at[0], at[1], at[2]);
        self.frame
            .sphere(self.camera, centre, radius * UNIT, color, pick, false);
    }

    /// A stroke `width` design pixels wide (at least `floor` screen pixels) along `points`.
    pub fn stroke(&mut self, points: &[Spot], width: f32, color: Color, pick: u32) {
        let world: Vec<Point> = points.iter().map(|&[x, d, z]| point(x, d, z)).collect();
        let pixels = (width * self.pixel()).max(1.2);
        self.frame.ribbon(
            self.camera,
            &world,
            Width::Pixels(pixels),
            color,
            pick,
            None,
        );
    }

    pub fn glow(&mut self, at: Spot, radius: f32, color: Color, strength: f32) {
        let p = self.frame.project(self.camera, point(at[0], at[1], at[2]));
        self.frame.items.push(crate::render::Item::Puff {
            center: [p[0], p[1]],
            radius,
            color,
            strength: strength.clamp(0.0, 1.0),
        });
    }
}

/// Planks along a vertical face: bands of slightly different tone with a seam between them, and
/// a few long darker grain lines in each.
fn planks(pen: &mut Pen, x: (f32, f32), y: (f32, f32), d: f32, tone: Color, seed: u32) {
    pen.face(x, y, d, tint(tone, 0.6));
    let mut top = y.0;
    let mut band = 0_u32;
    while top < y.1 {
        let height = 12.0 + 8.0 * unit(seed + band);
        let bottom = (top + height).min(y.1);
        let shade = 0.88 + 0.22 * unit(seed + band + 977);
        pen.face(
            (x.0, x.1),
            (top + 0.7, bottom - 0.5),
            d + 0.05,
            tint(tone, shade),
        );
        for line in 0..3_u32 {
            let key = seed + band * 7 + line + 3571;
            let start = x.0 + (x.1 - x.0) * unit(key);
            let length = 60.0 + 280.0 * unit(key + 1);
            let at = top + (bottom - top) * unit(key + 2);
            pen.face(
                (start, (start + length).min(x.1)),
                (at, at + 0.9),
                d + 0.1,
                tint(tone, shade * 0.82),
            );
        }
        top = bottom;
        band += 1;
    }
}

/// The same planks laid on a horizontal top, in bands of depth.
fn boards(pen: &mut Pen, x: (f32, f32), d: (f32, f32), y: f32, tone: Color, seed: u32) {
    pen.top(x, d, y, tint(tone, 0.6));
    let mut near = d.0;
    let mut band = 0_u32;
    while near < d.1 {
        let far = (near + 14.0).min(d.1);
        let shade = 0.9 + 0.2 * unit(seed + band);
        pen.top(x, (near + 0.6, far - 0.4), y + 0.05, tint(tone, shade));
        for line in 0..3_u32 {
            let key = seed + band * 5 + line + 211;
            let start = x.0 + (x.1 - x.0) * unit(key);
            let length = 80.0 + 300.0 * unit(key + 1);
            let at = near + (far - near) * unit(key + 2);
            pen.top(
                (start, (start + length).min(x.1)),
                (at, at + 0.8),
                y + 0.1,
                tint(tone, shade * 0.82),
            );
        }
        near = far;
        band += 1;
    }
}

/// The floor, the board with its cornice and bakelite strips, the keyshelf, the cabinet below it
/// and the softirq board's frame: none of it shows data.
pub fn furniture(pen: &mut Pen, plan: &Plan) {
    floor(pen, plan);
    let last = plan.last_row_y();
    let shelf = plan.shelf_y();
    let (left, right) = (BOARD.0 - 8.0, BOARD.1 + 8.0);
    for (x0, x1, seed) in [
        (left, right, 11),
        (SOFTIRQ_BOARD.0 - 8.0, SOFTIRQ_BOARD.1 + 8.0, 31),
    ] {
        let bottom = if seed == 11 {
            shelf + 4.0
        } else {
            560.0 + plan.drop
        };
        pen.side(
            x1,
            (0.0, -BEHIND),
            (CORNICE, bottom + 60.0),
            tint(DARK_OAK, 0.75),
        );
        boards(
            pen,
            (x0, x1),
            (-BEHIND, 0.0),
            CORNICE,
            tint(OAK, 1.15),
            seed + 1,
        );
        planks(pen, (x0, x1), (CORNICE, bottom), 0.0, OAK, seed);
        pen.face((x0, x1), (CORNICE, CORNICE + 14.0), 0.3, tint(OAK, 1.25));
        pen.face(
            (x0, x1),
            (CORNICE + 14.0, CORNICE + 17.0),
            0.3,
            [40, 24, 14],
        );
    }
    let strips = (BOARD.0 + 14.0, BOARD.1 - 14.0);
    for row in 0..plan.rows.max(4) {
        let y = FIRST_ROW + row as f32 * ROW_PITCH;
        pen.face(strips, (y - 19.0, y + 36.0), 0.5, BAKELITE);
        pen.face(strips, (y - 36.0, y - 19.0), 0.5, IVORY);
        pen.face(strips, (y + 36.0, y + 39.0), 0.5, [52, 32, 20]);
    }
    pen.face(strips, (last + 62.0, last + 104.0), 0.5, [30, 21, 16]);
    // The keyshelf: its top, its front and the cabinet under it.
    boards(
        pen,
        (left, right),
        (0.0, SHELF_DEPTH),
        shelf,
        tint(OAK, 1.18),
        51,
    );
    pen.face(
        (left, right),
        (shelf, shelf + 22.0),
        SHELF_DEPTH,
        tint(DARK_OAK, 1.0),
    );
    pen.face(
        (left, right),
        (shelf + 22.0, shelf + 25.0),
        SHELF_DEPTH,
        [20, 12, 8],
    );
    pen.face(
        (left, right),
        (shelf + 25.0, shelf + 150.0),
        SHELF_DEPTH,
        tint(DARK_OAK, 0.55),
    );
    let mut door = BOARD.0 + 60.0;
    while door < BOARD.1 {
        pen.face(
            (door, door + 100.0),
            (shelf + 36.0, shelf + 140.0),
            SHELF_DEPTH + 0.2,
            tint(DARK_OAK, 0.45),
        );
        door += 140.0;
    }
    // The softirq board's dark panel and ivory strip.
    let (sx0, sx1) = SOFTIRQ_BOARD;
    pen.face(
        (sx0 + 12.0, sx1 - 12.0),
        (110.0, 540.0 + plan.drop),
        0.5,
        BAKELITE,
    );
    pen.face((sx0 + 12.0, sx1 - 12.0), (80.0, 100.0), 0.5, IVORY);
}

fn floor(pen: &mut Pen, plan: &Plan) {
    let y = TOP_Z - plan.floor();
    // `top` takes a face y, which is the height measured down from `TOP_Z`.
    let mut x = -240.0;
    let mut plank = 0_u32;
    while x < 1900.0 {
        let width = 48.0 + 10.0 * unit(plank + 4001);
        let shade = 0.72 + 0.3 * unit(plank + 17);
        pen.top(
            (x + 0.6, x + width - 0.4),
            (-260.0, 330.0),
            y,
            tint(FLOOR, shade),
        );
        x += width;
        plank += 1;
    }
}

/// A point of the board plane `d` at height `z`.
pub fn flat_spot(x: f32, d: f32, z: f32) -> Spot {
    [x, d, z]
}
