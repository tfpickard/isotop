//! The ink on the board: letters, digits, YES, NO and GOODBYE as Hershey strokes lying on the
//! board's plane, the double border, the sun and the moon with their ornaments. Every stroke is
//! a world-space ribbon, so it foreshortens with the board and both rasterizers fill it alike.
//! Only the letters and the words say anything; the rest is decoration.

use std::f32::consts::TAU;

use super::layout::{self, MOON, Mark, Plane, SCALE, SUN, Spot, WORDS};
use super::scenery::{PAINT, Scenery};
use crate::hershey::{Align, Face};
use crate::palette::tint;
use crate::render::strokes::Width;
use crate::render::{Camera, Color, Frame, NONE, Point, SCALE as PIXELS};

/// How far the ink stands above the board's wood, in board widths: enough that no depth tie lets
/// the grain through.
const INK_LIFT: f32 = 0.0006;
/// The width of a letter's stroke, and of the border's two lines, in board widths.
const STROKE: f32 = 0.0033;
const OUTER_LINE: f32 = 0.0030;
const INNER_LINE: f32 = 0.0016;
/// The ink of a letter the planchette is over, which glows.
const LIT: Color = [255, 238, 176];

/// Pixels in a board width at `camera`, for widths that are fractions of it.
fn pixels_per_width(camera: &Camera) -> f32 {
    SCALE * camera.zoom * PIXELS
}

/// A polyline of the board `lift` above the wood, `width` board widths wide (never under one
/// pixel, which would be drawn as a line the rasterizers disagree on).
fn stroke(
    frame: &mut Frame,
    camera: &Camera,
    plane: &Plane,
    spots: &[Spot],
    (width, lift): (f32, f32),
    color: Color,
) {
    let points: Vec<Point> = spots.iter().map(|&spot| plane.world(spot, lift)).collect();
    let pixels = (width * pixels_per_width(camera)).max(1.0);
    frame.ribbon(camera, &points, Width::Pixels(pixels), color, NONE, None);
}

/// Points around a circle of `radius` at `centre`, from `from` radians, `count` + 1 of them to
/// close it.
fn circle(centre: Spot, radius: f32, from: f32, count: usize) -> Vec<Spot> {
    (0..=count)
        .map(|step| {
            let angle = from + TAU * step as f32 / count as f32;
            [
                centre[0] + radius * angle.cos(),
                centre[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

/// A rectangle `half` either side of the centre with its corners rounded to `radius`, closed.
fn rounded(half: Spot, radius: f32) -> Vec<Spot> {
    let mut spots = Vec::new();
    for (corner, start) in [
        ([half[0] - radius, half[1] - radius], 0.0),
        ([-half[0] + radius, half[1] - radius], 0.25),
        ([-half[0] + radius, -half[1] + radius], 0.5),
        ([half[0] - radius, -half[1] + radius], 0.75),
    ] {
        for step in 0..=6 {
            let angle = TAU * (start + 0.25 * step as f32 / 6.0);
            spots.push([
                corner[0] + radius * angle.cos(),
                corner[1] + radius * angle.sin(),
            ]);
        }
    }
    spots.push(spots[0]);
    spots
}

/// A diamond `size` high and wide either side of `at`, closed.
fn diamond(at: Spot, size: f32) -> Vec<Spot> {
    vec![
        [at[0], at[1] + size],
        [at[0] + size, at[1]],
        [at[0], at[1] - size],
        [at[0] - size, at[1]],
        [at[0], at[1] + size],
    ]
}

/// A four-pointed star `size` board widths from its centre to a tip, closed.
fn star(at: Spot, size: f32) -> Vec<Spot> {
    let tips = [
        [0.0, -2.0],
        [0.5, -0.5],
        [2.0, 0.0],
        [0.5, 0.5],
        [0.0, 2.0],
        [-0.5, 0.5],
        [-2.0, 0.0],
        [-0.5, -0.5],
        [0.0, -2.0],
    ];
    tips.iter()
        .map(|[x, y]| [at[0] + x * size, at[1] + y * size])
        .collect()
}

/// The ink and the light it is seen by: what each stroke needs to be drawn.
pub struct Ink<'a> {
    pub frame: &'a mut Frame,
    pub camera: &'a Camera,
    pub plane: &'a Plane,
    /// How strongly each candle burns now, for the tone of the paint.
    pub flicker: [f32; 2],
}

impl Ink<'_> {
    /// The paint colour at `spot`: the worn cream of the mockup, as bright as the candles make
    /// the wood there.
    fn paint(&self, spot: Spot) -> Color {
        let light = Scenery::light_at(spot, self.flicker);
        tint(PAINT, (0.45 + 0.55 * light).clamp(0.55, 1.2))
    }

    fn line(&mut self, spots: &[Spot], width: f32, color: Color) {
        stroke(
            self.frame,
            self.camera,
            self.plane,
            spots,
            (width, INK_LIFT),
            color,
        );
    }

    /// Writes `text` centred on `at`, `height` board widths tall, its up turned `tilt` radians
    /// from the far edge, in Hershey Triplex strokes `weight` board widths wide.
    pub fn write(
        &mut self,
        text: &str,
        at: Spot,
        tilt: f32,
        height: f32,
        (weight, lift): (f32, f32),
        color: Color,
    ) {
        let right = self.plane.direction(false, tilt);
        let up = self.plane.direction(true, tilt);
        // The baseline is half a height below the centre, along the letter's own up.
        let baseline = [
            at[0] + 0.5 * height * tilt.sin(),
            at[1] - 0.5 * height * tilt.cos(),
        ];
        let origin = self.plane.world(baseline, lift);
        // Converted to world units and back inside `ribbon`, a width of exactly one pixel could
        // come out a rounding under it and be drawn as a line; a little over cannot.
        let pixels = (weight * pixels_per_width(self.camera)).max(1.05);
        let world = pixels / (self.camera.zoom * PIXELS);
        self.frame.stroke_text(
            self.camera,
            Face::Triplex,
            text,
            origin,
            right,
            up,
            height * SCALE,
            color,
            world,
            Align::Middle,
        );
    }

    /// Every letter, digit and word of the board, with the one the planchette is over lit.
    pub fn lettering(&mut self, lit: Option<Mark>) {
        for placed in layout::letters() {
            let mark = Mark::Letter(placed.character);
            let lit = lit == Some(mark);
            let color = if lit { LIT } else { self.paint(placed.at) };
            let (weight, lift) = if lit {
                (STROKE * 1.5, INK_LIFT * 2.0)
            } else {
                (STROKE, INK_LIFT)
            };
            let mut buffer = [0; 4];
            let text: &str = placed.character.encode_utf8(&mut buffer);
            self.write(
                text,
                placed.at,
                placed.tilt,
                placed.height,
                (weight, lift),
                color,
            );
        }
        for (word, at, height) in WORDS {
            let mark = match word {
                "YES" => Mark::Yes,
                "NO" => Mark::No,
                _ => Mark::Goodbye,
            };
            let lit = lit == Some(mark);
            let color = if lit { LIT } else { self.paint(at) };
            let weight = if lit { STROKE * 1.5 } else { STROKE };
            self.write(word, at, 0.0, height, (weight, INK_LIFT), color);
        }
    }

    /// The double border, the rules under GOODBYE and at the top, and the sun and the moon.
    pub fn ornaments(&mut self) {
        let half = [0.5, layout::HALF_HEIGHT];
        let outer = rounded([half[0] - 0.0192, half[1] - 0.0192], 0.0292);
        let inner = rounded([half[0] - 0.0292, half[1] - 0.0292], 0.0225);
        let color = self.paint([0.0, 0.0]);
        self.line(&outer, OUTER_LINE, color);
        self.line(&inner, INNER_LINE, color);
        self.rules(color);
        self.sun(color);
        self.moon(color);
    }

    /// The rule with three diamonds at the top, and the flourishes either side of GOODBYE.
    fn rules(&mut self, color: Color) {
        self.line(&[[-0.1083, 0.2542], [0.1083, 0.2542]], INNER_LINE, color);
        for (x, size) in [(0.0, 0.0067), (-0.0625, 0.0038), (0.0625, 0.0038)] {
            self.line(&diamond([x, 0.2542], size), INNER_LINE, color);
        }
        for side in [-1.0, 1.0] {
            self.line(
                &[[side * 0.1375, -0.2133], [side * 0.2333, -0.2133]],
                INNER_LINE,
                color,
            );
            self.line(&diamond([side * 0.2458, -0.2133], 0.005), INNER_LINE, color);
        }
    }

    /// The sun: a ring with a face and sixteen rays of two lengths.
    fn sun(&mut self, color: Color) {
        let radius = 0.0325;
        self.line(&circle(SUN, radius, 0.0, 24), OUTER_LINE, color);
        for side in [-1.0, 1.0] {
            let eye = [SUN[0] + side * 0.0125, SUN[1] + 0.006];
            self.line(&[eye, [eye[0], eye[1] + 0.0045]], OUTER_LINE * 1.3, color);
        }
        let smile: Vec<Spot> = (0..=8)
            .map(|step| {
                let angle = TAU * (0.55 + 0.4 * step as f32 / 8.0);
                [
                    SUN[0] + 0.0142 * angle.cos(),
                    SUN[1] - 0.0021 + 0.0142 * angle.sin(),
                ]
            })
            .collect();
        self.line(&smile, INNER_LINE * 1.4, color);
        let mut rays = Vec::new();
        for ray in 0..16 {
            let angle = TAU * ray as f32 / 16.0;
            let (base, tip, width) = if ray % 2 == 0 {
                (radius + 0.0075, radius + 0.0258, 0.144)
            } else {
                (radius + 0.0075, radius + 0.0158, 0.112)
            };
            for (reach, turn) in [(base, -width), (tip, 0.0), (base, width)] {
                rays.push([
                    SUN[0] + reach * (angle + turn).cos(),
                    SUN[1] + reach * (angle + turn).sin(),
                ]);
            }
        }
        rays.push(rays[0]);
        self.line(&rays, INNER_LINE * 1.2, color);
    }

    /// The moon: the two arcs of a crescent, and three stars.
    fn moon(&mut self, color: Color) {
        let (outer, cut) = (0.0383, 0.0358);
        let cut_centre = [MOON[0] + 0.0192, MOON[1] + 0.0075];
        // The crescent's two arcs are the parts of each circle outside and inside the other; the
        // outside of the first and the inside of the second are centred on the direction from the
        // cut to the moon's centre.
        let away = (MOON[1] - cut_centre[1]).atan2(MOON[0] - cut_centre[0]);
        let reach = |spot: Spot, centre: Spot| (spot[0] - centre[0]).hypot(spot[1] - centre[1]);
        let arc = |centre: Spot, radius: f32, keep: &dyn Fn(Spot) -> bool| -> Vec<Spot> {
            circle(centre, radius, away - std::f32::consts::PI, 48)
                .into_iter()
                .filter(|&spot| keep(spot))
                .collect()
        };
        let outside = arc(MOON, outer, &|spot| reach(spot, cut_centre) >= cut);
        let inside = arc(cut_centre, cut, &|spot| reach(spot, MOON) <= outer);
        self.line(&outside, OUTER_LINE, color);
        self.line(&inside, OUTER_LINE, color);
        for (x, y, size) in [
            (0.0292, 0.0292, 0.0083),
            (0.0417, -0.0125, 0.0054),
            (0.0167, -0.0333, 0.0042),
        ] {
            self.line(&star([MOON[0] + x, MOON[1] + y], size), INNER_LINE, color);
        }
    }
}
