//! The ribbon under the board: the line being spelled, decoding as the planchette lands on its
//! letters, with the line's source and severity beneath. It is drawn in screen space with the
//! bitmap font, over a dark plate. Decoded characters are amber; the rest are scrambled violet,
//! which is decoration (a hash of the line, the character's place and the time, standing for
//! text not yet read) and carries nothing of the line but its length.
//!
//! The text is a journal line, which a machine controls, so it passes through `SafeText::new`
//! before it is laid out: one character per cell, and `?` for anything the terminal could act on.

use super::seance::Reading;
use crate::glyphs::{self, HEIGHT, WIDTH};
use crate::journal::JobResult;
use crate::render::{Color, Frame, Item, NONE};
use crate::simulation::hash;
use crate::terminal::SafeText;

/// How many characters of a line the ribbon shows at once.
pub const WINDOW: usize = 44;
/// How many characters before the cursor the window keeps when the line is longer.
const BEHIND: usize = 30;
const AMBER: Color = [255, 190, 70];
const JUST_LANDED: Color = [255, 244, 190];
const VIOLET: Color = [138, 98, 214];
const CAPTION: Color = [158, 148, 178];
const PLATE: Color = [9, 8, 14];
const RULE: Color = [70, 50, 84];
/// Times a second the scrambled characters change.
const SCRAMBLE_RATE: f32 = 8.0;

/// The syslog severity names, from 0.
const LEVELS: [&str; 8] = [
    "emergency",
    "alert",
    "critical",
    "error",
    "warning",
    "notice",
    "info",
    "debug",
];

/// The source of a journal line `source[pid]: message` or `source: message`: the text before the
/// first bracket or colon, cleaned and at most 24 characters.
pub fn source(text: &str) -> String {
    let end = text.find(['[', ':']).unwrap_or(text.len());
    SafeText::new(&text[..end])
        .as_str()
        .chars()
        .take(24)
        .collect()
}

/// The caption of a line: its source, its severity and the result of the job it ends.
pub fn caption(reading: &Reading) -> String {
    let entry = &reading.entry;
    let level = LEVELS[usize::from(entry.priority.min(7))];
    let job = match entry.job {
        None => String::new(),
        Some(JobResult::Done) => " | job done".to_owned(),
        Some(result) => format!(" | job {}", format!("{result:?}").to_lowercase()),
    };
    format!("{} | {level}{job}", source(&entry.text))
}

/// The first character of the window that shows `decoded` characters of a line of `length`.
fn window_start(decoded: usize, length: usize) -> usize {
    decoded
        .saturating_sub(BEHIND)
        .min(length.saturating_sub(WINDOW))
}

/// The characters the ribbon shows and the colour of each: decoded characters as they are, the
/// others scrambled while `time` runs.
fn cells(reading: &Reading, time: f32) -> Vec<(char, Color)> {
    let clean = SafeText::new(&reading.entry.text);
    let characters: Vec<char> = clean.as_str().chars().collect();
    let seed = hash::fnv1a_64(clean.as_str());
    let tick = (time.max(0.0) * SCRAMBLE_RATE) as u64;
    let start = window_start(reading.decoded, characters.len());
    characters
        .iter()
        .enumerate()
        .skip(start)
        .take(WINDOW)
        .map(|(index, &character)| {
            if index < reading.decoded {
                let color = if index + 1 == reading.decoded && !reading.finished() {
                    JUST_LANDED
                } else {
                    AMBER
                };
                (character, color)
            } else if character == ' ' {
                (' ', VIOLET)
            } else {
                let bits = hash::golden_gamma_high(
                    seed ^ (index as u64).wrapping_mul(0x85eb_ca6b)
                        ^ tick.wrapping_mul(0xc2b2_ae3d),
                );
                (char::from(b'!' + (bits % 94) as u8), VIOLET)
            }
        })
        .collect()
}

/// A screen-space rectangle of whole pixels at `depth`.
fn rect(frame: &mut Frame, [x0, y0, x1, y1]: [f32; 4], depth: f32, color: Color) {
    let corner = |x: f32, y: f32| [x, y, depth];
    frame.items.push(Item::Triangle(
        [corner(x0, y0), corner(x1, y0), corner(x1, y1)],
        color,
        NONE,
    ));
    frame.items.push(Item::Triangle(
        [corner(x0, y0), corner(x1, y1), corner(x0, y1)],
        color,
        NONE,
    ));
}

/// ASCII text in the bitmap font from (`x`, `y`), `scale` times its size.
fn text(frame: &mut Frame, (x, y): (f32, f32), scale: u32, items: &[(char, Color)]) {
    for (index, &(character, color)) in items.iter().enumerate() {
        if character != ' ' {
            frame.items.push(Item::Glyph {
                origin: [x + (index as u32 * WIDTH * scale) as f32, y],
                scale,
                bits: glyphs::glyph(character),
                color,
            });
        }
    }
}

/// Draws the plate, the ribbon and the caption near the bottom of `frame`, in front of everything
/// at `depth`. With nothing read yet it says so.
pub fn draw(frame: &mut Frame, reading: Option<&Reading>, depth: f32, time: f32) {
    let (width, height) = (frame.width as f32, frame.height as f32);
    let scale = (height / 480.0).round().max(1.0) as u32;
    let (cell, line) = ((WIDTH * scale) as f32, (HEIGHT * scale) as f32);
    let columns = (width / cell).floor() as usize;
    let shown = WINDOW.min(columns.saturating_sub(4));
    if shown < 8 {
        return;
    }
    let plate_width = (shown as f32 + 2.0) * cell;
    let (left, top) = (
        ((width - plate_width) / 2.0).round(),
        (0.795 * height).round(),
    );
    let plate = [left, top, left + plate_width, top + 2.7 * line];
    rect(frame, plate, depth, PLATE);
    for edge in [
        [plate[0], plate[1], plate[2], plate[1] + 1.0],
        [plate[0], plate[3] - 1.0, plate[2], plate[3]],
    ] {
        rect(frame, edge, depth + 0.1, RULE);
    }
    let (ribbon, note): (Vec<(char, Color)>, String) = match reading {
        Some(reading) => {
            let mut ribbon = cells(reading, time);
            ribbon.truncate(shown);
            (ribbon, caption(reading))
        }
        None => (
            "waiting for the journal"
                .chars()
                .map(|character| (character, CAPTION))
                .collect(),
            String::new(),
        ),
    };
    text(frame, (left + cell, top + 0.35 * line), scale, &ribbon);
    let note: Vec<(char, Color)> = note
        .chars()
        .take(shown)
        .map(|character| (character, CAPTION))
        .collect();
    text(frame, (left + cell, top + 1.5 * line), scale, &note);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Line;
    use crate::ouija::seance::Seance;
    use crate::render::{Backdrop, Buffers, Sky};

    /// A name that tries a window title, a screen clear, a C1 control, a bidirectional override
    /// and a line break.
    const HOSTILE: &str = "ev\x1b]0;pwned\x07il[1]\x1b[2J\u{9b}31m\u{85}\u{202e}\r\nz: more";

    /// A seance that has been reading `text` for `seconds`.
    fn reading(line: Line, seconds: f32) -> Seance {
        let mut seance = Seance::default();
        seance.push(&line);
        seance.advance(0.0);
        seance.advance(seconds);
        seance
    }

    fn frame(width: u32, height: u32) -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Void,
            haze: [0.0; 3],
            time: 0.0,
        };
        Frame::new(width, height, sky, Buffers::default())
    }

    #[test]
    fn a_hostile_line_reaches_the_frame_as_printable_ascii_only() {
        let seance = reading(Line::new(3, HOSTILE), 60.0);
        let reading = seance.reading().expect("a line");
        let shown = cells(reading, 2.0);
        assert!(!shown.is_empty());
        assert!(
            shown
                .iter()
                .all(|&(character, _)| character == ' ' || character.is_ascii_graphic()),
            "{shown:?}"
        );
        let said = caption(reading);
        assert!(
            said.chars().all(|c| c == ' ' || c.is_ascii_graphic()),
            "{said:?}"
        );
        assert!(said.starts_with("ev?]0;pwned?il"), "{said:?}");
        // Every glyph drawn is one of the bitmap font's own, which has nothing but ASCII.
        let mut painted = frame(960, 540);
        draw(&mut painted, Some(reading), 10.0, 2.0);
        let glyph_items = painted
            .items
            .iter()
            .filter(|item| matches!(item, Item::Glyph { .. }))
            .count();
        assert!(glyph_items > 20, "{glyph_items} glyphs");
    }

    #[test]
    fn the_source_is_what_comes_before_the_first_bracket_or_colon() {
        assert_eq!(source("systemd[1]: Started."), "systemd");
        assert_eq!(source("kernel: nvme0: timeout"), "kernel");
        assert_eq!(source("no separator at all"), "no separator at all");
        assert_eq!(source(&"x".repeat(80)).len(), 24);
        assert_eq!(source("caf\u{e9}[2]: late"), "caf?");
        assert_eq!(source(""), "");
    }

    #[test]
    fn the_caption_names_the_severity_and_the_job() {
        let said = |line: Line| {
            let seance = reading(line, 0.0);
            caption(seance.reading().unwrap())
        };
        assert_eq!(
            said(Line::new(3, "kernel: nvme0 timeout")),
            "kernel | error"
        );
        assert_eq!(
            said(Line::new(6, "systemd[1]: Finished.").with_job(Some(JobResult::Done))),
            "systemd | info | job done"
        );
        assert_eq!(
            said(Line::new(3, "systemd[1]: Failed.").with_job(Some(JobResult::Timeout))),
            "systemd | error | job timeout"
        );
        assert_eq!(said(Line::new(0, "a: b")), "a | emergency");
    }

    #[test]
    fn decoded_characters_are_amber_and_the_rest_scrambled_violet() {
        let seance = reading(Line::new(6, "alpha beta gamma"), 0.25 * 0.6 * 3.0 + 0.1);
        let reading = seance.reading().unwrap();
        assert!(reading.decoded >= 2 && !reading.finished());
        let shown = cells(reading, 1.0);
        for (index, &(character, color)) in shown.iter().enumerate() {
            if index < reading.decoded {
                assert_eq!(character, "alpha beta gamma".chars().nth(index).unwrap());
                assert!(color == AMBER || color == JUST_LANDED, "{index}: {color:?}");
            } else {
                assert_eq!(color, VIOLET, "{index}");
            }
        }
        assert_eq!(
            shown[reading.decoded - 1].1,
            JUST_LANDED,
            "the letter that just landed"
        );
        assert_eq!(shown[5].0, ' ', "a space stays a space");
        // The scramble is a function of the line and the time: steady within a tick, changing
        // between ticks, and a line decoded completely has none of it.
        let again = cells(reading, 1.0 + 0.5 / SCRAMBLE_RATE);
        assert_eq!(shown, again);
        let later = cells(reading, 1.0 + 3.0 / SCRAMBLE_RATE);
        assert!(shown != later);
        let done = reading_of_finished();
        assert!(cells(&done, 3.0).iter().all(|&(_, color)| color == AMBER));
    }

    fn reading_of_finished() -> Reading {
        let seance = reading(Line::new(6, "all done"), 60.0);
        let finished = seance.reading().unwrap().clone();
        assert!(finished.finished());
        finished
    }

    #[test]
    fn a_long_line_shows_a_window_that_follows_the_cursor() {
        assert_eq!(window_start(0, 200), 0);
        assert_eq!(window_start(20, 200), 0);
        assert_eq!(window_start(100, 200), 100 - BEHIND);
        assert_eq!(window_start(200, 200), 200 - WINDOW, "the end of the line");
        assert_eq!(window_start(10, 30), 0, "a short line is shown whole");
        let text: String = (0..100)
            .map(|n| char::from(b'a' + (n % 26) as u8))
            .collect();
        let seance = reading(Line::new(6, &text), 60.0);
        let reading = seance.reading().unwrap();
        let shown = cells(reading, 0.0);
        assert_eq!(shown.len(), WINDOW);
        assert_eq!(shown.last().unwrap().0, text.chars().last().unwrap());
    }

    #[test]
    fn the_ribbon_says_so_before_anything_has_been_read_and_fits_small_frames() {
        let mut empty = frame(960, 540);
        draw(&mut empty, None, 10.0, 0.0);
        assert!(
            empty
                .items
                .iter()
                .any(|item| matches!(item, Item::Glyph { .. }))
        );
        // A frame too narrow to hold a ribbon draws none and does not panic.
        let mut tiny = frame(40, 30);
        draw(&mut tiny, None, 10.0, 0.0);
        assert!(tiny.items.is_empty());
    }
}
