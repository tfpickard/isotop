//! The Hershey Roman Simplex and Roman Triplex stroke fonts: polylines per glyph, advance widths
//! and a layout from a string to positioned strokes. Pure data and arithmetic; `render/text.rs`
//! draws the result.
//!
//! The data is the Usenet distribution of the Hershey fonts (comp.sources.unix volume 4, formatted
//! by James Hurt), kept byte for byte in `hershey/romans.jhf` and `hershey/romant.jhf`. Its notice
//! is `hershey/NOTICE` and must stay distributed with the data; `hershey/PROVENANCE` says where
//! the files came from. The notice forbids converting the data into the format of the U.S. NTIS
//! and allows any other, but the files are parsed here as they are, so there is no second copy
//! of the glyphs in another form.
//!
//! A `.jhf` record is a 5-column glyph number, a 3-column vertex count, then that many coordinate
//! pairs of two characters each. The first pair holds the glyph's left and right bounds, and it
//! counts in the vertex count; each later pair is a vertex, and the pair `" R"` lifts the pen.
//! A coordinate is its character's code minus that of `'R'`, and y grows downward. Long records
//! wrap onto continuation lines (at 72 columns in the original distribution), so a record ends
//! when it holds its vertex count, not at a newline. The glyphs of a file are the printable ASCII
//! characters from space (32) to tilde (126) in order; a file may carry further records, which are
//! not loaded.
//!
//! Text safety: strokes are drawn into the frame, never sent to the terminal, so no `SafeText` is
//! needed to lay them out. Text that comes from a process (names, command lines, journal lines)
//! must still be cleaned the way `terminal::SafeText::new` cleans it before it is drawn, as for
//! every text a view draws. Beyond that, no character can do harm here: one this font lacks is
//! drawn as `?`, or skipped by a caller that asks with `Font::get`.

use std::fmt;
use std::sync::OnceLock;

/// The first character of a font file: space.
pub const FIRST: char = ' ';
/// The last character of a font file: tilde.
pub const LAST: char = '~';
/// How many glyphs a face loads from its file.
const GLYPHS: usize = (LAST as usize) - (FIRST as usize) + 1;
/// The character drawn in place of one the font lacks.
const FALLBACK: char = '?';
/// The character whose height is the font's cap height, and whose lowest point is the baseline.
const CAP_MEASURE: char = 'H';
/// A coordinate character's code minus this is the coordinate.
const ORIGIN: u8 = b'R';
/// The side bearing, in font units, given to a record whose bounds do not enclose its ink.
const BEARING: i8 = 2;
/// The pair that lifts the pen.
const PEN_UP: &[u8; 2] = b" R";
/// Columns of the glyph number, then of the vertex count, at the start of a record.
const NUMBER_COLUMNS: usize = 5;
const COUNT_COLUMNS: usize = 3;

const SIMPLEX: &str = include_str!("hershey/romans.jhf");
const TRIPLEX: &str = include_str!("hershey/romant.jhf");

/// Which of the two faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// Roman Simplex, the single-stroke face.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "ouija draws in Triplex, whose strokes stand out at a weight"
        )
    )]
    Simplex,
    /// Roman Triplex, the same letters drawn with extra strokes, so they stand out at a weight.
    Triplex,
}

/// Where text sits along its `right` direction in relation to the point it is placed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// The point is the left edge of the first glyph's advance.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "ouija centres its text; no view writes from the left yet"
        )
    )]
    Start,
    /// The point is the middle of the whole advance.
    Middle,
    /// The point is the right edge of the last glyph's advance.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "ouija centres its text; no view writes from the right yet"
        )
    )]
    End,
}

/// One glyph in the font's own units, as the file gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// Where the glyph's advance box starts, left of the pen's origin for most letters.
    pub left: i8,
    /// Where it ends. `right - left` is how far the pen moves.
    pub right: i8,
    /// The pen-down runs, in the order the file gives them, each of at least two vertices.
    /// x grows to the right and y downward. A space has none.
    pub strokes: Vec<Vec<[i8; 2]>>,
}

/// A face as the file holds it, and the two numbers that turn font units into cap heights.
#[derive(Debug)]
pub struct Font {
    glyphs: Vec<Glyph>,
    /// The font's y of the baseline: the lowest point of `H`.
    baseline: i8,
    /// The height of `H` in font units, which is 1.0 in the units this module reports.
    cap: f32,
}

/// What went wrong parsing a `.jhf` file, and in which record (counting from 0).
#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    pub record: usize,
    pub reason: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "record {}: {}", self.record, self.reason)
    }
}

impl std::error::Error for ParseError {}

/// Parses every record of a `.jhf` file. Blank lines between records are skipped; a record is
/// complete once it holds its vertex count's worth of pairs, wherever the lines break.
pub fn parse(text: &str) -> Result<Vec<Glyph>, ParseError> {
    let mut lines = text.lines();
    let mut glyphs = Vec::new();
    while let Some(first) = lines.next() {
        if first.trim().is_empty() {
            continue;
        }
        let record = glyphs.len();
        let fail = |reason| ParseError { record, reason };
        if !first.is_ascii() {
            return Err(fail("a record holds only ASCII"));
        }
        let header = NUMBER_COLUMNS + COUNT_COLUMNS;
        if first.len() < header {
            return Err(fail("a record starts with a number and a vertex count"));
        }
        first[..NUMBER_COLUMNS]
            .trim()
            .parse::<u32>()
            .map_err(|_| fail("the glyph number is not a number"))?;
        let count: usize = first[NUMBER_COLUMNS..header]
            .trim()
            .parse()
            .map_err(|_| fail("the vertex count is not a number"))?;
        if count == 0 {
            return Err(fail("the vertex count leaves no room for the bounds"));
        }
        let mut data = first[header..].to_owned();
        while data.len() < 2 * count {
            let more = lines
                .next()
                .ok_or_else(|| fail("the file ends inside a record"))?;
            if !more.is_ascii() {
                return Err(fail("a record holds only ASCII"));
            }
            data.push_str(more);
        }
        if data.len() != 2 * count {
            return Err(fail("the record holds more than its vertex count"));
        }
        glyphs.push(decode(data.as_bytes()).map_err(fail)?);
    }
    Ok(glyphs)
}

/// The bounds pair and the strokes of one record's coordinate pairs.
fn decode(data: &[u8]) -> Result<Glyph, &'static str> {
    let coordinate = |byte: u8| i8::try_from(i16::from(byte) - i16::from(ORIGIN));
    let pair_of = |pair: &[u8]| -> Result<(i8, i8), &'static str> {
        match (coordinate(pair[0]), coordinate(pair[1])) {
            (Ok(first), Ok(second)) => Ok((first, second)),
            _ => Err("a coordinate is out of range"),
        }
    };
    let (mut left, mut right) = pair_of(&data[..2])?;
    let mut strokes = Vec::new();
    let mut stroke: Vec<[i8; 2]> = Vec::new();
    for pair in data[2..].as_chunks::<2>().0 {
        if pair == PEN_UP {
            strokes.push(std::mem::take(&mut stroke));
        } else {
            let (x, y) = pair_of(pair)?;
            stroke.push([x, y]);
        }
    }
    strokes.push(stroke);
    // A run of one vertex draws nothing, and a trailing or doubled pen lift leaves an empty one.
    strokes.retain(|run| run.len() >= 2);
    // The bounds of a record must enclose its ink. The one record of the two faces that does not
    // (Triplex `J`: bounds -8 to -8 around ink from -6 to 6, a zero advance) takes the extent of
    // its ink plus the 2 units of side bearing that its neighbours have, which is -8 to 8.
    let ink = strokes.iter().flatten().map(|vertex| vertex[0]);
    if let (Some(least), Some(most)) = (ink.clone().min(), ink.max())
        && (least < left || most > right)
    {
        left = least.saturating_sub(BEARING);
        right = most.saturating_add(BEARING);
    }
    Ok(Glyph {
        left,
        right,
        strokes,
    })
}

impl Font {
    /// A face from the records of a file: the first 95 are the characters from space to tilde.
    /// The height of `H` sets the units, so it needs strokes of its own.
    pub fn new(records: Vec<Glyph>) -> Result<Self, ParseError> {
        let record = records.len();
        let fail = |reason| ParseError { record, reason };
        if records.len() < GLYPHS {
            return Err(fail(
                "the file holds fewer glyphs than the characters 32 to 126",
            ));
        }
        let mut glyphs = records;
        glyphs.truncate(GLYPHS);
        let measure = &glyphs[CAP_MEASURE as usize - FIRST as usize];
        let heights = measure.strokes.iter().flatten().map(|vertex| vertex[1]);
        let (Some(top), Some(bottom)) = (heights.clone().min(), heights.max()) else {
            return Err(fail("the glyph that sets the cap height has no strokes"));
        };
        if bottom <= top {
            return Err(fail("the glyph that sets the cap height has no height"));
        }
        Ok(Self {
            glyphs,
            baseline: bottom,
            cap: f32::from(bottom) - f32::from(top),
        })
    }

    /// The glyph for `character`, or None when this font lacks it: anything outside printable
    /// ASCII, and the control characters. Nothing is substituted; a caller that must not draw a
    /// wrong letter (the talking board takes only A to Z and 0 to 9) asks here.
    pub fn get(&self, character: char) -> Option<&Glyph> {
        let index = (character as usize).checked_sub(FIRST as usize)?;
        (character <= LAST).then(|| &self.glyphs[index])
    }

    /// The glyph for `character`; one this font lacks is drawn as `?`, as the bitmap font of
    /// `glyphs.rs` does. Use `get` to tell the two apart.
    pub fn glyph(&self, character: char) -> &Glyph {
        self.get(character)
            .unwrap_or_else(|| &self.glyphs[FALLBACK as usize - FIRST as usize])
    }

    /// Whether every character of `text` has a glyph of its own.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "ouija filters its text by the board, not by the font"
        )
    )]
    pub fn covers(&self, text: &str) -> bool {
        text.chars().all(|character| self.get(character).is_some())
    }

    /// A vertex in cap heights: x along the writing direction with the font's 0 at the pen, y up
    /// from the baseline, so the top of `H` is at 1.0.
    pub fn unit(&self, vertex: [i8; 2]) -> [f32; 2] {
        [
            f32::from(vertex[0]) / self.cap,
            (f32::from(self.baseline) - f32::from(vertex[1])) / self.cap,
        ]
    }

    /// How far the pen moves past one glyph, in cap heights.
    fn step(&self, character: char) -> f32 {
        let glyph = self.glyph(character);
        (f32::from(glyph.right) - f32::from(glyph.left)) / self.cap
    }

    /// The length of `text` along its line in cap heights: the sum of the glyphs' advances. There
    /// is no kerning. A character the font lacks counts as `?`, as it draws.
    pub fn advance(&self, text: &str) -> f32 {
        text.chars().map(|character| self.step(character)).sum()
    }

    /// The strokes of `text` as polylines in cap heights, baseline at y = 0, x measured from the
    /// anchor `align` names. Each glyph's advance box begins where the last one's ended. A
    /// character the font lacks is laid out as `?`.
    pub fn lay_out(&self, text: &str, align: Align) -> Vec<Vec<[f32; 2]>> {
        let mut pen = match align {
            Align::Start => 0.0,
            Align::Middle => -0.5 * self.advance(text),
            Align::End => -self.advance(text),
        };
        let mut strokes = Vec::new();
        for character in text.chars() {
            let glyph = self.glyph(character);
            for run in &glyph.strokes {
                strokes.push(
                    run.iter()
                        .map(|&[x, y]| {
                            let [across, up] = self.unit([x.saturating_sub(glyph.left), y]);
                            [pen + across, up]
                        })
                        .collect(),
                );
            }
            pen += self.step(character);
        }
        strokes
    }
}

/// The face, parsed on first use.
pub fn font(face: Face) -> &'static Font {
    static SIMPLEX_FONT: OnceLock<Font> = OnceLock::new();
    static TRIPLEX_FONT: OnceLock<Font> = OnceLock::new();
    let (cell, data) = match face {
        Face::Simplex => (&SIMPLEX_FONT, SIMPLEX),
        Face::Triplex => (&TRIPLEX_FONT, TRIPLEX),
    };
    cell.get_or_init(|| {
        parse(data)
            .and_then(Font::new)
            .expect("the committed .jhf data parses")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACES: [Face; 2] = [Face::Simplex, Face::Triplex];

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn extent(strokes: &[Vec<[f32; 2]>], axis: usize) -> (f32, f32) {
        let values = strokes.iter().flatten().map(|point| point[axis]);
        (
            values.clone().fold(f32::MAX, f32::min),
            values.fold(f32::MIN, f32::max),
        )
    }

    #[test]
    fn hershey_parses_every_printable_ascii_glyph_in_both_faces() {
        for face in FACES {
            let font = font(face);
            for character in FIRST..=LAST {
                assert!(
                    font.get(character).is_some(),
                    "{face:?} lacks {character:?}"
                );
            }
            for character in ('A'..='Z').chain('0'..='9') {
                let glyph = font.get(character).expect("a letter or digit");
                assert!(
                    !glyph.strokes.is_empty(),
                    "{face:?} {character} has no strokes"
                );
            }
            assert!(font.glyph(' ').strokes.is_empty(), "a space draws nothing");
        }
        // The files carry one record beyond the 95 characters; it is parsed but not loaded.
        for data in [SIMPLEX, TRIPLEX] {
            assert_eq!(parse(data).expect("the data parses").len(), GLYPHS + 1);
        }
    }

    #[test]
    fn every_glyph_has_finite_bounded_coordinates_and_a_positive_advance() {
        for face in FACES {
            let font = font(face);
            for character in FIRST..=LAST {
                let glyph = font.glyph(character);
                assert!(glyph.right > glyph.left, "{face:?} {character:?} advance");
                assert!(font.step(character) > 0.0 && font.step(character) < 2.0);
                for run in &glyph.strokes {
                    assert!(run.len() >= 2, "{face:?} {character:?} has a lone vertex");
                    for &[x, y] in run {
                        assert!(
                            (glyph.left..=glyph.right).contains(&x),
                            "{face:?} {character:?} vertex x {x} leaves the advance box"
                        );
                        assert!(
                            x.abs() <= 20 && y.abs() <= 20,
                            "{face:?} {character:?} vertex ({x}, {y}) is out of range"
                        );
                    }
                }
                for point in font.lay_out(&character.to_string(), Align::Start).concat() {
                    assert!(point.iter().all(|value| value.is_finite()));
                    assert!(
                        (-0.5..=1.5).contains(&point[1]),
                        "{face:?} {character:?} reaches {point:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_cap_height_of_h_is_one_and_the_baseline_is_zero() {
        for face in FACES {
            let strokes = font(face).lay_out("H", Align::Start);
            let (low, high) = extent(&strokes, 1);
            assert!(near(low, 0.0) && near(high, 1.0), "{face:?} {low} {high}");
        }
        // Simplex H: two stems and a bar, the stems 7 font units either side of the pen origin
        // and 21 units tall.
        let strokes = font(Face::Simplex).lay_out("H", Align::Start);
        assert_eq!(strokes.len(), 3);
        let (left, right) = extent(&strokes, 0);
        assert!(near(right - left, 14.0 / 21.0), "{left} {right}");
    }

    #[test]
    fn known_glyphs_have_their_strokes() {
        let simplex = font(Face::Simplex);
        let count = |face: &Font, character: char| face.glyph(character).strokes.len();
        assert_eq!(count(simplex, 'H'), 3);
        assert_eq!(count(simplex, 'I'), 1);
        assert_eq!(count(simplex, 'E'), 4);
        assert_eq!(count(simplex, 'T'), 2);
        assert_eq!(count(simplex, ' '), 0);
        assert_eq!(
            simplex.glyph('H').strokes,
            vec![
                vec![[-7, -12], [-7, 9]],
                vec![[7, -12], [7, 9]],
                vec![[-7, -2], [7, -2]]
            ]
        );
        assert_eq!(
            (simplex.glyph('H').left, simplex.glyph('H').right),
            (-11, 11)
        );
        assert_eq!((simplex.glyph(' ').left, simplex.glyph(' ').right), (-8, 8));
        // Triplex doubles up the strokes of the same letter.
        assert!(count(font(Face::Triplex), 'H') > count(simplex, 'H'));
        assert_ne!(simplex.glyph('A'), font(Face::Triplex).glyph('A'));
    }

    #[test]
    fn bounds_that_miss_the_ink_take_the_ink_and_a_bearing() {
        // Triplex J is the one record of the two faces whose bounds (-8 to -8) miss its ink.
        let triplex = font(Face::Triplex);
        let j = triplex.glyph('J');
        assert_eq!((j.left, j.right), (-8, 8));
        assert!(triplex.advance("J") > 0.0);
        // The same repair on a hand-made record: ink from 3 to 4, bounds that stop short of it.
        let glyphs = parse("  100  3JKUWVY\n").expect("parses");
        assert_eq!((glyphs[0].left, glyphs[0].right), (1, 6));
    }

    #[test]
    fn wrapped_jhf_records_parse_like_single_line_ones() {
        for data in [SIMPLEX, TRIPLEX] {
            let single = parse(data).expect("the data parses");
            for columns in [72, 71, 31, 9] {
                let wrapped: String = data
                    .lines()
                    .map(|line| {
                        let pieces: Vec<&str> = line
                            .as_bytes()
                            .chunks(columns)
                            .map(|piece| std::str::from_utf8(piece).expect("ASCII"))
                            .collect();
                        pieces.join("\n") + "\n"
                    })
                    .collect();
                assert!(
                    wrapped.lines().count() > data.lines().count(),
                    "the test must really wrap"
                );
                assert_eq!(
                    parse(&wrapped).expect("wrapped data parses"),
                    single,
                    "{columns}"
                );
            }
        }
    }

    #[test]
    fn a_pen_lift_split_across_two_lines_still_lifts_the_pen() {
        // Bounds, then (0,0)-(1,1), a pen lift split between the lines, then (2,2)-(3,3).
        let text = "  100  6JZRRSS \nRTTUU\n";
        let glyphs = parse(text).expect("parses");
        assert_eq!(glyphs.len(), 1);
        assert_eq!(
            glyphs[0].strokes,
            vec![vec![[0, 0], [1, 1]], vec![[2, 2], [3, 3]]]
        );
    }

    #[test]
    fn malformed_records_are_reported_not_guessed() {
        let reason = |text: &str| parse(text).expect_err("rejected").reason;
        assert!(reason("  100  3JZRR").contains("ends inside"));
        assert!(reason("  100  1JZRR").contains("more than"));
        assert!(reason("  abc  1JZ").contains("glyph number"));
        assert!(reason("  100  xJZ").contains("vertex count"));
        assert!(reason("  100  0").contains("bounds"));
        assert!(reason("  100  1J\u{e9}").contains("ASCII"));
        assert!(Font::new(parse("  100  1JZ\n").expect("parses")).is_err());
        assert_eq!(parse("\n\n").expect("blank lines are skipped"), vec![]);
    }

    #[test]
    fn unknown_characters_have_no_glyph_and_draw_as_a_question_mark() {
        for face in FACES {
            let font = font(face);
            for character in ['\u{e9}', '\u{7f}', '\n', '\t', '\u{0}', '\u{1f600}'] {
                assert!(font.get(character).is_none(), "{character:?}");
                assert_eq!(font.glyph(character), font.glyph('?'));
            }
            assert!(!font.covers("OK \u{e9}") && font.covers("OK ~"));
            assert_eq!(
                font.lay_out("\u{e9}", Align::Start),
                font.lay_out("?", Align::Start)
            );
            assert!(near(font.advance("\u{e9}\u{e9}"), 2.0 * font.advance("?")));
        }
    }

    #[test]
    fn advance_is_the_sum_of_the_glyph_widths_in_cap_heights() {
        for face in FACES {
            let font = font(face);
            assert_eq!(font.advance(""), 0.0);
            assert!(near(font.advance(" "), 16.0 / 21.0));
            let width = f32::from(font.glyph('H').right - font.glyph('H').left);
            assert!(near(font.advance("H"), width / 21.0));
            assert!(near(font.advance("HH"), 2.0 * font.advance("H")));
            assert!(near(
                font.advance("H H"),
                2.0 * font.advance("H") + font.advance(" ")
            ));
        }
    }

    #[test]
    fn layout_places_each_glyph_one_advance_after_the_last() {
        let font = font(Face::Simplex);
        let one = font.lay_out("H", Align::Start);
        let two = font.lay_out("HH", Align::Start);
        assert_eq!(two.len(), 2 * one.len());
        let step = font.advance("H");
        for (before, after) in one.iter().zip(&two[one.len()..]) {
            for (a, b) in before.iter().zip(after) {
                assert!(near(a[0] + step, b[0]) && near(a[1], b[1]));
            }
        }
        // The ink of the first glyph starts inside its advance box, 4 of 22 units in.
        let (left, _) = extent(&one, 0);
        assert!(near(left, 4.0 / 21.0));
    }

    #[test]
    fn alignment_moves_the_anchor_along_the_line() {
        let font = font(Face::Simplex);
        let centre = |align| {
            let (low, high) = extent(&font.lay_out("H", align), 0);
            0.5 * (low + high)
        };
        // H is symmetric about the middle of its advance box, which is 22 font units wide.
        let half = font.advance("H") / 2.0;
        assert!(near(centre(Align::Middle), 0.0));
        assert!(near(centre(Align::Start), half));
        assert!(near(centre(Align::End), -half));
    }

    #[test]
    fn the_hershey_notice_ships_verbatim() {
        let notice = include_str!("hershey/NOTICE");
        // Every line of the distribution's notice, tabs included: the Hurt and NBS
        // acknowledgements and the NTIS clause. Edit this list only to follow a new copy of it.
        let expected = [
            "This distribution of the Hershey Fonts may be used by anyone for",
            "any purpose, commercial or otherwise, providing that:",
            "\t1. The following acknowledgements must be distributed with",
            "\t\tthe font data:",
            "\t\t- The Hershey Fonts were originally created by Dr.",
            "\t\t\tA. V. Hershey while working at the U. S.",
            "\t\t\tNational Bureau of Standards.",
            "\t\t- The format of the Font data in this distribution",
            "\t\t\twas originally created by",
            "\t\t\t\tJames Hurt",
            "\t\t\t\tCognition, Inc.",
            "\t\t\t\t900 Technology Park Drive",
            "\t\t\t\tBillerica, MA 01821",
            "\t\t\t\t(mit-eddie!ci-dandelion!hurt)",
            "\t2. The font data in this distribution may be converted into",
            "\t\tany other format *EXCEPT* the format distributed by",
            "\t\tthe U.S. NTIS (which organization holds the rights",
            "\t\tto the distribution and use of the font data in that",
            "\t\tparticular format). Not that anybody would really",
            "\t\t*want* to use their format... each point is described",
            "\t\tin eight bytes as \"xxx yyy:\", where xxx and yyy are",
            "\t\tthe coordinate values as ASCII numbers.",
        ];
        assert_eq!(notice.lines().collect::<Vec<_>>(), expected);
        assert!(notice.ends_with("coordinate values as ASCII numbers.\n"));
        let readme = include_str!("../README.md");
        assert!(readme.contains("## Credits"));
        assert!(readme.contains("NWL Report 2101"));
        assert!(readme.contains("src/hershey/NOTICE"));
    }
}
