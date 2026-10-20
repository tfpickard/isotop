//! Where everything stands on the talking board, and the plane the board lies on in the world.
//! Pure geometry with no drawing, and constant: nothing the journal or the machine does moves a
//! letter.
//!
//! Board coordinates (`Spot`) have their origin at the board's centre, x to the right and y
//! towards the far edge, with the board's width as the unit, so the board spans x in -0.5..0.5
//! and y in -`HALF_HEIGHT`..`HALF_HEIGHT`. The arrangement is the common one of nineteenth-century
//! talking boards: two arcs of A to Z, a row of 1 to 0, YES and NO in the top corners and GOODBYE
//! at the bottom, with a sun and a moon for ornaments. The arcs, the row and their sizes are
//! those of `docs/mockups/code/ouija.py`, converted from its 2400 by 1500 texture.

use std::f32::consts::FRAC_PI_2;

use crate::render::Point;

/// A place on the board, in board widths.
pub type Spot = [f32; 2];

/// Half the board's height, in board widths: the board is 1.6 times as wide as it is deep.
pub const HALF_HEIGHT: f32 = 0.3125;

/// What the planchette can point at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// One of `A` to `Z` or `0` to `9`: the only characters the board has.
    Letter(char),
    /// The board's centre, below the digits: where a space is.
    Rest,
    Yes,
    No,
    Goodbye,
}

/// The centre of the circles the two arcs of letters lie on.
const ARC_CENTRE: Spot = [0.0, -0.541_67];
/// The outer arc of `A` to `M` and the inner arc of `N` to `Z`: radius in board widths, and the
/// angles of the first and the last letter, in degrees from the board's right.
const ARCS: [(f32, f32, f32); 2] = [(0.633_33, 122.0, 58.0), (0.541_67, 120.0, 60.0)];
/// The row of digits, left to right as `1234567890`: its height and the x of the first and last.
const DIGITS: (f32, f32, f32) = (-0.125, -0.266_67, 0.266_67);
const REST: Spot = [0.025, -0.166_67];
const YES: Spot = [-0.283_33, 0.208_33];
const NO: Spot = [0.291_67, 0.208_33];
const GOODBYE: Spot = [0.0, -0.210_42];
/// The ornaments' centres.
pub const SUN: Spot = [-0.404_17, 0.206_25];
pub const MOON: Spot = [0.404_17, 0.206_25];

/// The height of a capital on each part of the board, in board widths.
const LETTER_HEIGHT: f32 = 0.040;
const DIGIT_HEIGHT: f32 = 0.034;
const WORD_HEIGHT: f32 = 0.034;
const GOODBYE_HEIGHT: f32 = 0.032;

/// Where the planchette's window stands over `mark`.
pub fn spot(mark: Mark) -> Spot {
    match mark {
        Mark::Letter(character) => letter(character).map_or(REST, |placed| placed.at),
        Mark::Rest => REST,
        Mark::Yes => YES,
        Mark::No => NO,
        Mark::Goodbye => GOODBYE,
    }
}

/// A letter or digit placed on the board.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub character: char,
    pub at: Spot,
    /// How far its up turns from the board's far edge, counter-clockwise, in radians: the letters
    /// of an arc stand along its radius.
    pub tilt: f32,
    pub height: f32,
}

/// The placement of `character`, if the board has it.
pub fn letter(character: char) -> Option<Placed> {
    match character {
        'A'..='M' => Some(on_arc(0, character, character as u8 - b'A', 13)),
        'N'..='Z' => Some(on_arc(1, character, character as u8 - b'N', 13)),
        '0'..='9' => {
            let index = if character == '0' {
                9
            } else {
                character as u8 - b'1'
            };
            let (y, first, last) = DIGITS;
            Some(Placed {
                character,
                at: [first + (last - first) * f32::from(index) / 9.0, y],
                tilt: 0.0,
                height: DIGIT_HEIGHT,
            })
        }
        _ => None,
    }
}

fn on_arc(arc: usize, character: char, index: u8, count: u8) -> Placed {
    let (radius, first, last) = ARCS[arc];
    let degrees = first + (last - first) * f32::from(index) / f32::from(count - 1);
    let angle = degrees.to_radians();
    Placed {
        character,
        at: [
            ARC_CENTRE[0] + radius * angle.cos(),
            ARC_CENTRE[1] + radius * angle.sin(),
        ],
        // The letter's up is the radius, which is `angle` from the right, so `angle - 90` degrees
        // from the far edge.
        tilt: angle - FRAC_PI_2,
        height: LETTER_HEIGHT,
    }
}

/// Every letter and digit of the board, `A` to `Z` and then `1` to `0`.
pub fn letters() -> impl Iterator<Item = Placed> {
    ('A'..='Z')
        .chain('1'..='9')
        .chain('0'..='0')
        .filter_map(letter)
}

/// The words of the board: text, centre and capital height.
pub const WORDS: [(&str, Spot, f32); 3] = [
    ("YES", YES, WORD_HEIGHT),
    ("NO", NO, WORD_HEIGHT),
    ("GOODBYE", GOODBYE, GOODBYE_HEIGHT),
];

/// The board's centre in the world, and the world's vectors for its right and its far edge. The
/// default camera sees the world turned a quarter circle, so these are chosen to look square on
/// to it, and then yawed a few degrees as the mockup's board is.
pub struct Plane {
    centre: Point,
    right: Point,
    far: Point,
}

/// World units to a board width.
pub const SCALE: f32 = 32.0;
/// How thick the board is, and so how high its top stands over the table, in board widths.
pub const THICKNESS: f32 = 0.02;
/// The board's slight yaw, in degrees.
const YAW: f32 = -4.0;

impl Plane {
    /// The plane of the board for the default camera, which looks along the world's
    /// (-1, -1) diagonal: a board width to the screen's right is world `(1, -1)` over root two.
    pub fn new() -> Self {
        let (sin, cos) = YAW.to_radians().sin_cos();
        let turn = |[x, y]: [f32; 2]| [x * cos - y * sin, x * sin + y * cos, 0.0];
        let root = 0.5_f32.sqrt();
        Self {
            centre: [0.0; 3],
            right: turn([root, -root]),
            far: turn([-root, -root]),
        }
    }

    /// The world point `lift` board widths above the board's top at `spot`; a negative `lift`
    /// goes into the wood, and `-THICKNESS` is the table.
    pub fn world(&self, spot: Spot, lift: f32) -> Point {
        let across = spot[0] * SCALE;
        let along = spot[1] * SCALE;
        [
            self.centre[0] + self.right[0] * across + self.far[0] * along,
            self.centre[1] + self.right[1] * across + self.far[1] * along,
            (THICKNESS + lift) * SCALE,
        ]
    }

    /// The world vector that is board-right (`false`) or board-up the far edge (`true`) turned
    /// `tilt` radians counter-clockwise, for text that lies on the board.
    pub fn direction(&self, far: bool, tilt: f32) -> Point {
        let (sin, cos) = tilt.sin_cos();
        let (right, up) = (
            [
                cos * self.right[0] + sin * self.far[0],
                cos * self.right[1] + sin * self.far[1],
            ],
            [
                cos * self.far[0] - sin * self.right[0],
                cos * self.far[1] - sin * self.right[1],
            ],
        );
        let chosen = if far { up } else { right };
        [chosen[0], chosen[1], 0.0]
    }

    /// The four corners of a rectangle of the board `half` board widths either side of its
    /// centre, near left first and counter-clockwise on the screen, `lift` above the top.
    pub fn corners(&self, half: Spot, lift: f32) -> [Point; 4] {
        [
            [-half[0], -half[1]],
            [half[0], -half[1]],
            [half[0], half[1]],
            [-half[0], half[1]],
        ]
        .map(|corner| self.world(corner, lift))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_letter_and_digit_has_one_place_inside_the_board() {
        let placed: Vec<Placed> = letters().collect();
        assert_eq!(placed.len(), 36);
        for (index, a) in placed.iter().enumerate() {
            assert!(a.at[0].abs() < 0.5 && a.at[1].abs() < HALF_HEIGHT, "{a:?}");
            for b in &placed[index + 1..] {
                let apart = (a.at[0] - b.at[0]).hypot(a.at[1] - b.at[1]);
                assert!(
                    apart > 0.03,
                    "{} and {} are {apart} apart",
                    a.character,
                    b.character
                );
            }
        }
        assert!(letter('a').is_none() && letter('!').is_none());
    }

    #[test]
    fn the_arcs_run_left_to_right_and_the_digits_in_order() {
        let x = |character| letter(character).unwrap().at[0];
        assert!(x('A') < x('G') && x('G') < x('M'));
        assert!(x('N') < x('T') && x('T') < x('Z'));
        assert!(x('1') < x('5') && x('5') < x('9') && x('9') < x('0'));
        let y = |character| letter(character).unwrap().at[1];
        assert!(y('G') > y('A'), "the arc rises to the middle");
        assert!(
            y('A') > y('N') && y('G') > y('T'),
            "A to M is the outer arc"
        );
        let upright = letter('G').unwrap().tilt;
        assert!(
            upright.abs() < 0.2,
            "the middle of the arc stands upright: {upright}"
        );
        assert!(letter('A').unwrap().tilt > 0.0 && letter('M').unwrap().tilt < 0.0);
    }

    #[test]
    fn the_words_and_the_rest_stand_where_the_mockup_puts_them() {
        assert!(spot(Mark::Yes)[0] < 0.0 && spot(Mark::No)[0] > 0.0);
        assert_eq!(spot(Mark::Yes)[1], spot(Mark::No)[1]);
        assert!(spot(Mark::Goodbye)[1] < spot(Mark::Rest)[1]);
        assert!(
            spot(Mark::Rest)[1] < letter('5').unwrap().at[1],
            "between the digits and GOODBYE"
        );
        assert_eq!(
            spot(Mark::Letter('q')),
            REST,
            "a mark the board lacks stands at rest"
        );
    }

    #[test]
    fn the_plane_keeps_right_and_far_square_and_looks_square_on_to_the_default_camera() {
        let plane = Plane::new();
        let right = plane.direction(false, 0.0);
        let far = plane.direction(true, 0.0);
        let dot = right[0] * far[0] + right[1] * far[1];
        assert!(dot.abs() < 1e-5, "{dot}");
        let unit = |v: Point| v[0].hypot(v[1]);
        assert!((unit(right) - 1.0).abs() < 1e-5 && (unit(far) - 1.0).abs() < 1e-5);
        // Turned a quarter, board-up becomes board-left.
        let left = plane.direction(false, FRAC_PI_2);
        let up_turned = plane.direction(true, FRAC_PI_2);
        assert!((up_turned[0] + right[0]).abs() < 1e-5 && (up_turned[1] + right[1]).abs() < 1e-5);
        assert!((left[0] - far[0]).abs() < 1e-5);
        let a = plane.world([0.0, 0.0], 0.0);
        let b = plane.world([0.1, 0.0], 0.0);
        assert!((b[0] - a[0]).hypot(b[1] - a[1]) - 0.1 * SCALE < 1e-4);
        assert_eq!(a[2], THICKNESS * SCALE);
    }
}
