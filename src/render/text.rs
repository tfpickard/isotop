//! `Frame::stroke_text`: Hershey stroke text (`hershey.rs`) drawn as scene geometry on any plane
//! of the world. It adds no `Item` and no shader work, so the CPU and the GPU draw it alike.
//!
//! Text passes through here only after the caller has cleaned it (as `terminal::SafeText::new` does): the
//! strokes are drawn into the frame, not sent to a terminal, but a character the font lacks is
//! drawn as `?` (`Font::get` is how a caller skips it instead). Overlay labels and popups stay
//! terminal text.

use super::strokes::Width;
use super::{Camera, Color, Frame, NONE, Point};
use crate::geometry::dot;
use crate::hershey::{self, Align, Face};

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scaled(a: Point, factor: f32) -> Point {
    a.map(|value| value * factor)
}

impl Frame {
    /// `text` in `face`, `height` world units from baseline to the top of a capital, on the
    /// plane through `origin` spanned by `right` (the direction of writing) and `up`, both unit
    /// world vectors, so the text can lie on any plane, such as a tilted board. `origin` is the
    /// point on the baseline that `align` names along `right`. Nothing in it is pickable.
    ///
    /// With `weight` 0 every stroke is a one-pixel `Frame::line`. With a `weight` above 0 (world
    /// units, as `Width::World`) every segment is its own `Frame::ribbon`, lengthened by
    /// `weight / 2` at both ends: ribbons overlap at a join with no miter, and the lengthening
    /// closes the wedge a sharp corner would leave outside. Below a pixel of projected width a
    /// ribbon falls back to a line, as `ribbon` does. A `height` that is not a positive finite
    /// number, or an `origin`, `right` or `up` that is not finite, draws nothing.
    #[expect(
        clippy::too_many_arguments,
        reason = "the issue fixes this signature, with the face it leaves implicit"
    )]
    pub(crate) fn stroke_text(
        &mut self,
        camera: &Camera,
        face: Face,
        text: &str,
        origin: Point,
        right: Point,
        up: Point,
        height: f32,
        color: Color,
        weight: f32,
        align: Align,
    ) {
        let finite = |point: Point| point.iter().all(|value| value.is_finite());
        if !(height.is_finite() && height > 0.0) || ![origin, right, up].into_iter().all(finite) {
            return;
        }
        let place = |[across, rise]: [f32; 2]| {
            add(
                origin,
                add(scaled(right, across * height), scaled(up, rise * height)),
            )
        };
        for run in hershey::font(face).lay_out(text, align) {
            for pair in run.windows(2) {
                let (a, b) = (place(pair[0]), place(pair[1]));
                if weight > 0.0 {
                    let along = sub(b, a);
                    let size = dot(along, along).sqrt();
                    if size < 1e-9 {
                        continue;
                    }
                    let reach = scaled(along, 0.5 * weight / size);
                    self.ribbon(
                        camera,
                        &[sub(a, reach), add(b, reach)],
                        Width::World(weight),
                        color,
                        NONE,
                        None,
                    );
                } else {
                    self.line(camera, a, b, color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2};

    use super::*;
    use crate::hershey::font;
    use crate::render::{Backdrop, Buffers, Item, SCALE, Sky};
    use crate::simulation::hash::fnv1a_64_bytes;

    const RED: Color = [255, 0, 0];
    /// The writing direction of an overhead camera is the screen's right, and up its top.
    const RIGHT: Point = [FRAC_1_SQRT_2, -FRAC_1_SQRT_2, 0.0];
    const UP: Point = [-FRAC_1_SQRT_2, -FRAC_1_SQRT_2, 0.0];
    const ORIGIN: Point = [0.0, 0.0, 0.0];

    fn blank(width: u32, height: u32) -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Void,
            haze: [0.0; 3],
            time: 0.0,
        };
        Frame::new(width, height, sky, Buffers::default())
    }

    /// Straight down, with `pixels` screen pixels to a world unit.
    fn overhead(pixels: f32) -> Camera {
        Camera {
            pitch: FRAC_PI_2,
            zoom: pixels / SCALE,
            ..Camera::default()
        }
    }

    fn write(text: &str, height: f32, weight: f32, align: Align, origin: Point) -> (Frame, Camera) {
        let camera = overhead(10.0);
        let mut frame = blank(320, 180);
        frame.stroke_text(
            &camera,
            Face::Simplex,
            text,
            origin,
            RIGHT,
            UP,
            height,
            RED,
            weight,
            align,
        );
        (frame, camera)
    }

    /// The screen-space end points of every line the frame recorded.
    fn ends(frame: &Frame) -> Vec<Point> {
        frame
            .items
            .iter()
            .flat_map(|item| match item {
                Item::Line(a, b, _) => vec![*a, *b],
                _ => Vec::new(),
            })
            .collect()
    }

    fn extent(points: &[Point], axis: usize) -> (f32, f32) {
        let values = points.iter().map(|point| point[axis]);
        (
            values.clone().fold(f32::MAX, f32::min),
            values.fold(f32::MIN, f32::max),
        )
    }

    #[test]
    fn stroke_text_scales_with_height_and_advances_by_glyph_width() {
        let simplex = font(Face::Simplex);
        let size = |text, height| {
            let (frame, _) = write(text, height, 0.0, Align::Start, ORIGIN);
            let points = ends(&frame);
            let (left, right) = extent(&points, 0);
            let (top, bottom) = extent(&points, 1);
            (right - left, bottom - top)
        };
        // At 10 pixels a unit, H stands 14/21 of its height wide and one height tall.
        let (width, tall) = size("H", 3.0);
        assert!((tall - 30.0).abs() < 0.01, "{tall}");
        assert!((width - 30.0 * 14.0 / 21.0).abs() < 0.01, "{width}");
        // Twice the height, twice the picture.
        let (double_width, double_tall) = size("H", 6.0);
        assert!(
            (double_width - 2.0 * width).abs() < 0.01 && (double_tall - 2.0 * tall).abs() < 0.01
        );
        // A second H starts one advance after the first.
        let (two_width, _) = size("HH", 3.0);
        assert!(
            (two_width - (width + 30.0 * simplex.advance("H"))).abs() < 0.01,
            "{two_width}"
        );
    }

    #[test]
    fn stroke_text_alignment_places_the_start_middle_and_end_at_the_origin() {
        let origin = [2.0, -3.0, 0.0];
        let (start, camera) = write("HI", 2.0, 0.0, Align::Start, origin);
        let anchor = start.project(&camera, origin)[0];
        let (start_left, _) = extent(&ends(&start), 0);
        // The ink of H starts 4 of 21 cap heights inside its advance box, which starts at the
        // origin; the whole advance of "HI" is `width` pixels.
        assert!((start_left - (anchor + 4.0 / 21.0 * 20.0)).abs() < 0.01);
        let width = font(Face::Simplex).advance("HI") * 2.0 * 10.0;
        for (align, shift) in [
            (Align::Start, 0.0),
            (Align::Middle, -0.5),
            (Align::End, -1.0),
        ] {
            let (aligned, _) = write("HI", 2.0, 0.0, align, origin);
            let (left, _) = extent(&ends(&aligned), 0);
            assert!(
                (left - (start_left + shift * width)).abs() < 0.01,
                "{align:?}: ink starts at {left}, wanted {}",
                start_left + shift * width
            );
        }
        // H is symmetric about its advance box, so its ink is centred on a Middle origin.
        let (frame, camera) = write("H", 3.0, 0.0, Align::Middle, origin);
        let (left, right) = extent(&ends(&frame), 0);
        assert!((0.5 * (left + right) - frame.project(&camera, origin)[0]).abs() < 0.01);
    }

    #[test]
    fn unknown_characters_draw_as_a_question_mark() {
        let (unknown, _) = write("\u{e9}\u{1}", 3.0, 0.0, Align::Start, ORIGIN);
        let (question, _) = write("??", 3.0, 0.0, Align::Start, ORIGIN);
        assert!(!unknown.items.is_empty());
        assert_eq!(ends(&unknown), ends(&question));
    }

    #[test]
    fn a_weight_draws_a_ribbon_per_segment_and_zero_draws_lines() {
        let segments: usize = font(Face::Simplex)
            .lay_out("YES", Align::Start)
            .iter()
            .map(|run| run.len() - 1)
            .sum();
        let (thin, _) = write("YES", 3.0, 0.0, Align::Start, ORIGIN);
        assert_eq!(thin.items.len(), segments);
        assert!(thin.items.iter().all(|item| matches!(item, Item::Line(..))));
        let (wide, _) = write("YES", 3.0, 0.2, Align::Start, ORIGIN);
        assert!(
            wide.items
                .iter()
                .all(|item| matches!(item, Item::Triangle(_, _, pick) if *pick == NONE))
        );
        assert!(wide.items.len() >= 2 * segments);
    }

    #[test]
    fn a_ribbon_is_lengthened_by_half_its_weight_at_both_ends() {
        // One vertical stroke: I is a single stem, 21 units tall.
        let height = 4.0;
        let weight = 0.4;
        let (frame, _) = write("I", height, weight, Align::Start, ORIGIN);
        let corners: Vec<Point> = frame
            .items
            .iter()
            .flat_map(|item| match item {
                Item::Triangle(points, ..) => points.to_vec(),
                _ => Vec::new(),
            })
            .collect();
        let (top, bottom) = extent(&corners, 1);
        let stem = font(Face::Simplex).lay_out("I", Align::Start);
        let (low, high) = {
            let ys: Vec<f32> = stem.iter().flatten().map(|point| point[1]).collect();
            (
                ys.iter().copied().fold(f32::MAX, f32::min),
                ys.iter().copied().fold(f32::MIN, f32::max),
            )
        };
        let expected = ((high - low) * height + weight) * 10.0;
        assert!(
            (bottom - top - expected).abs() < 0.05,
            "{} vs {expected}",
            bottom - top
        );
    }

    #[test]
    fn stroked_text_lights_pixels_inside_its_box_and_is_unpickable() {
        let origin = [0.0, 0.0, 0.0];
        for weight in [0.0, 0.3] {
            let (mut frame, camera) = write("YES NO", 4.0, weight, Align::Middle, origin);
            frame.rasterize();
            // The box of the text on the screen: centred, 40 pixels tall, its advance wide.
            let centre = frame.project(&camera, origin);
            let half_width = 0.5 * font(Face::Simplex).advance("YES NO") * 40.0 + 3.0;
            let (x0, x1) = (centre[0] - half_width, centre[0] + half_width);
            let (y0, y1) = (centre[1] - 40.0 - 3.0, centre[1] + 3.0);
            let mut lit = 0;
            for y in 0..180 {
                for x in 0..320 {
                    let index = y * 320 + x;
                    if frame.pixels[index * 3..][..3] == RED {
                        lit += 1;
                        assert!(
                            (x0..=x1).contains(&(x as f32)) && (y0..=y1).contains(&(y as f32)),
                            "weight {weight}: pixel ({x}, {y}) is outside the box"
                        );
                    }
                }
            }
            assert!(lit > 100, "weight {weight}: only {lit} pixels lit");
            assert!(
                frame.picks.iter().all(|&pick| pick == NONE),
                "weight {weight}"
            );
        }
    }

    #[test]
    fn text_lies_on_a_tilted_plane() {
        // Writing along world y with the tops toward world z: a wall seen at the default pitch.
        // The I is one stem a cap height tall, so on the screen it is that high times the
        // foreshortening of a vertical, cos(pitch).
        let camera = Camera::default();
        let mut frame = blank(320, 180);
        frame.stroke_text(
            &camera,
            Face::Simplex,
            "I",
            ORIGIN,
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            3.0,
            RED,
            0.0,
            Align::Start,
        );
        let points = ends(&frame);
        let (top, bottom) = extent(&points, 1);
        let expected = 3.0 * camera.pitch.cos() * SCALE * camera.zoom;
        assert!(
            (bottom - top - expected).abs() < 0.01,
            "{} vs {expected}",
            bottom - top
        );
        // The stem rises toward the viewer's up, so it is nearer at the top: depth grows with z.
        let (far, near) = extent(&points, 2);
        assert!(near - far > 1.0, "{far} {near}");
    }

    #[test]
    fn degenerate_input_draws_nothing() {
        let camera = overhead(10.0);
        let mut frame = blank(320, 180);
        let nan = f32::NAN;
        for (origin, height) in [
            (ORIGIN, 0.0),
            (ORIGIN, -1.0),
            (ORIGIN, nan),
            ([nan, 0.0, 0.0], 1.0),
        ] {
            frame.stroke_text(
                &camera,
                Face::Simplex,
                "HI",
                origin,
                RIGHT,
                UP,
                height,
                RED,
                0.0,
                Align::Start,
            );
        }
        frame.stroke_text(
            &camera,
            Face::Simplex,
            "",
            ORIGIN,
            RIGHT,
            UP,
            1.0,
            RED,
            0.0,
            Align::Start,
        );
        assert!(frame.items.is_empty());
    }

    #[test]
    fn the_same_text_renders_the_same_pixels_twice() {
        let render = || {
            let (mut frame, _) = write("YES NO 0123", 3.0, 0.25, Align::Middle, [0.0, 0.0, 0.0]);
            frame.rasterize();
            fnv1a_64_bytes(frame.pixels.iter().copied())
        };
        let hash = render();
        assert_eq!(hash, render());
        // The CPU rasterizer rounds alike on every platform the golden test runs on.
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(
            hash, RENDER_HASH,
            "re-bless RENDER_HASH if the strokes changed on purpose"
        );
    }

    /// FNV-1a over the pixels of `YES NO 0123` at weight 0.25, 320 x 180, on the CPU.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const RENDER_HASH: u64 = 5949690470215290757;

    /// Writes the sample sheet of the issue to `std::env::temp_dir()`. Run with
    /// `cargo test stroke_font_sample_png -- --ignored`.
    #[test]
    #[ignore = "writes a PNG to look at, asserts nothing"]
    fn stroke_font_sample_png() {
        const SAMPLE: &str = "YES NO GOODBYE 0123456789 ABC";
        let faces = [
            (Face::Simplex, 0.0),
            (Face::Simplex, 0.2),
            (Face::Triplex, 0.0),
            (Face::Triplex, 0.2),
        ];
        let mut frame = blank(1600, 1000);
        // Flat to the screen, a row each: the two faces at one pixel, then at a visible weight.
        let flat = overhead(16.0);
        for (row, (face, weight)) in faces.into_iter().enumerate() {
            let origin = scaled(UP, 20.0 - 3.4 * row as f32);
            let color = [255, 214, 140];
            frame.stroke_text(
                &flat,
                face,
                SAMPLE,
                origin,
                RIGHT,
                UP,
                2.0,
                color,
                weight,
                Align::Middle,
            );
        }
        // On a board that leans away from the viewer, as the talking board will.
        let board = Camera {
            zoom: 12.0,
            ..Camera::default()
        };
        let lean = (30.0_f32).to_radians();
        let away = [FRAC_1_SQRT_2, FRAC_1_SQRT_2, 0.0];
        let up = add(
            scaled([0.0, 0.0, 1.0], lean.cos()),
            scaled(away, lean.sin()),
        );
        for (row, (face, weight)) in faces.into_iter().enumerate() {
            let origin = add([-1.0, -1.0, 0.0], scaled(away, -3.4 * row as f32));
            let color = [140, 214, 255];
            frame.stroke_text(
                &board,
                face,
                SAMPLE,
                origin,
                RIGHT,
                up,
                2.0,
                color,
                weight,
                Align::Middle,
            );
        }
        frame.rasterize();
        let path = std::env::temp_dir().join("isotop-stroke-font-sample.png");
        frame.write_png(&path).expect("the PNG is written");
        println!("{}", path.display());
    }
}
