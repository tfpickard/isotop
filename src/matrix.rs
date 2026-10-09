//! Matrix: the systemd journal decoded out of digital rain. The screen is a log, newest line at
//! the bottom, written horizontally in its severity's colour; but each character only resolves
//! when a falling column of rain passes over it, with a white flash that settles into a steady
//! glow. Every new line brings a shower down onto its own characters. Between the lines, the rain
//! is cmatrix: green streams of flickering glyphs with white heads.

use std::collections::VecDeque;

use crate::glyphs::{self, HEIGHT, WIDTH};
use crate::journal::Line;
use crate::render::{Color, Frame, Item};

/// Seconds after arriving that a character shows even if no rain has passed over it.
const UNVEIL: f32 = 3.0;
/// Seconds over which a struck character's flash settles to its steady colour.
const FLASH: f32 = 0.35;
/// Steady brightness of decoded text, as a fraction of its severity's colour.
const STEADY: f32 = 0.85;
/// Rows per second of ambient rain, and of the faster showers that decode new lines.
const AMBIENT: [f32; 2] = [8.0, 16.0];
const SHOWER: [f32; 2] = [14.0, 26.0];
/// Ambient streams per text column.
const DENSITY: f32 = 0.5;
/// Indent of a wrapped line's continuation rows.
const INDENT: usize = 2;
const RAIN: Color = [70, 230, 95];
const HEAD: Color = [210, 255, 220];

/// Full-strength colour of a syslog severity: red for errors and worse, amber for warnings,
/// Matrix green for notices and information, dim teal for debugging.
fn level(priority: u8) -> Color {
    match priority {
        0..=3 => [255, 72, 60],
        4 => [255, 190, 60],
        5 | 6 => [80, 255, 95],
        _ => [70, 160, 150],
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t) as u8)
}

/// One screen row of the log: a journal line, or a wrapped continuation of one.
struct Row {
    text: Vec<char>,
    priority: u8,
    arrived: f32,
    /// When rain last passed over each character; negative infinity if it never has.
    struck: Vec<f32>,
}

/// A falling stream: head row (fractional, may be above the screen), rows per second, length,
/// and for a shower the row it decodes, where it stops and drains away.
struct Stream {
    column: usize,
    head: f32,
    speed: f32,
    length: usize,
    target: Option<usize>,
}

impl Stream {
    /// The lowest row this stream reaches.
    fn floor(&self, rows: usize) -> usize {
        self.target.unwrap_or(rows)
    }
}

#[derive(Default)]
pub struct Rain {
    pending: VecDeque<Line>,
    rows: VecDeque<Row>,
    streams: Vec<Stream>,
    latest: Option<String>,
    /// Text columns, rows and glyph scale of the current frame size.
    grid: (usize, usize, u32),
    last: Option<f32>,
    release: f32,
    random: u64,
    greeted: bool,
    /// Lines dropped because they arrived faster than the log could show them.
    pub skipped: u64,
}

impl Rain {
    /// Queues a journal line to be written into the log.
    pub fn push(&mut self, line: Line) {
        self.latest = Some(line.text.clone());
        self.pending.push_back(line);
        self.trim();
    }

    /// Drops the oldest queued lines beyond what two screens can show. Until the first frame sizes
    /// the grid, nothing is dropped, so the startup history survives to fill the screen.
    fn trim(&mut self) {
        if self.grid.1 == 0 {
            return;
        }
        let capacity = self.grid.1.max(16) * 2;
        while self.pending.len() > capacity {
            self.pending.pop_front();
            self.skipped += 1;
        }
    }

    /// The most recent journal line, for the status strip.
    pub fn latest(&self) -> Option<&str> {
        self.latest.as_deref()
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    fn uniform(&mut self) -> f32 {
        // SplitMix64: deterministic, so a demo frame at a fixed time is repeatable.
        self.random = self.random.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.random;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, range: [f32; 2]) -> f32 {
        range[0] + (range[1] - range[0]) * self.uniform()
    }

    /// Glyph scale and the text grid for a frame size.
    fn layout(width: u32, height: u32) -> (usize, usize, u32) {
        let scale = (height / 540).max(1);
        let columns = (width / (WIDTH * scale)).max(8) as usize;
        let rows = (height / (HEIGHT * scale)).max(2) as usize;
        (columns, rows, scale)
    }

    /// Moves every stored time by `delta`, when the clock jumps (pausing switches clocks).
    fn rebase(&mut self, delta: f32) {
        self.release += delta;
        for row in &mut self.rows {
            row.arrived += delta;
            for struck in &mut row.struck {
                *struck += delta;
            }
        }
    }

    /// Writes the next queued line into the log and sends a shower down onto its characters.
    fn write(&mut self, line: Line, time: f32) {
        let (columns, rows, _) = self.grid;
        let wrapped = wrap(&line.text, columns);
        // Rows that would scroll straight off the screen are never shown, so skip them.
        let hidden = wrapped.len().saturating_sub(rows);
        for text in wrapped.into_iter().skip(hidden) {
            if self.rows.len() == rows {
                self.rows.pop_front();
            }
            let row = self.rows.len();
            for (column, &character) in text.iter().enumerate() {
                if character != ' ' {
                    let lead = 1.0 + self.uniform() * 10.0;
                    let speed = self.between(SHOWER);
                    let length = 2 + (self.uniform() * 5.0) as usize;
                    self.streams.push(Stream {
                        column,
                        head: row as f32 - lead,
                        speed,
                        length,
                        target: Some(row),
                    });
                }
            }
            self.rows.push_back(Row {
                struck: vec![f32::NEG_INFINITY; text.len()],
                text,
                priority: line.priority,
                arrived: time,
            });
        }
    }

    fn advance(&mut self, time: f32) {
        if !self.greeted {
            self.greeted = true;
            let user = std::env::var("USER").unwrap_or_else(|_| "Neo".into());
            for text in [
                "Follow the white rabbit.".into(),
                "The Matrix has you...".into(),
                format!("Wake up, {user}..."),
            ] {
                self.pending.push_front(Line::new(6, text));
            }
        }
        let mut dt = self.last.map_or(0.0, |last| time - last);
        if !(0.0..=1.0).contains(&dt) {
            self.rebase(dt);
            dt = 0.0;
        }
        self.last = Some(time);
        while time >= self.release
            && let Some(line) = self.pending.pop_front()
        {
            self.write(line, time);
            // Lines arrive one at a time, faster when they queue up. Each release follows the
            // previous one rather than this frame, so a slow frame rate still keeps pace; a burst
            // after a long gap is limited to one second's worth, the slowest frame interval.
            let interval = 0.5 / (1.0 + self.pending.len() as f32 / 3.0);
            self.release = self.release.max(time - 1.0) + interval;
        }
        let (columns, rows, _) = self.grid;
        let wanted = (columns as f32 * DENSITY) as usize;
        let ambient = self.streams.iter().filter(|s| s.target.is_none()).count();
        for _ in ambient..wanted {
            let column = (self.uniform() * columns as f32) as usize;
            let head = -self.uniform() * rows as f32 * 0.6;
            let speed = self.between(AMBIENT);
            let length = 8 + (self.uniform() * 16.0) as usize;
            self.streams.push(Stream {
                column: column.min(columns - 1),
                head,
                speed,
                length,
                target: None,
            });
        }
        for stream in &mut self.streams {
            let before = stream.head.floor() as isize;
            stream.head += stream.speed * dt;
            let last = (stream.head.floor() as isize).min(stream.floor(rows) as isize);
            for row in before + 1..=last {
                if let Some(row) = usize::try_from(row).ok().and_then(|r| self.rows.get_mut(r))
                    && let Some(struck) = row.struck.get_mut(stream.column)
                {
                    *struck = time;
                }
            }
        }
        self.streams
            .retain(|s| s.head - (s.length as f32) < s.floor(rows) as f32);
    }

    /// Advances the rain to `time` and records it into the frame.
    pub fn draw(&mut self, frame: &mut Frame, time: f32) {
        let grid = Self::layout(frame.width, frame.height);
        if grid != self.grid {
            // Rewrap what is on screen for the new width; it decodes again under a fresh shower.
            let mut lines: Vec<Line> = Vec::new();
            for row in &self.rows {
                if row.text.starts_with(&[' '; INDENT]) && !lines.is_empty() {
                    let last = lines.last_mut().expect("not empty");
                    last.text.extend(row.text[INDENT..].iter());
                } else {
                    lines.push(Line::new(row.priority, row.text.iter().collect::<String>()));
                }
            }
            self.rows.clear();
            self.streams.clear();
            self.grid = grid;
            self.trim();
            for line in lines {
                self.write(line, time);
            }
        }
        self.advance(time);
        let (_, rows, scale) = grid;
        let cell = [(WIDTH * scale) as f32, (HEIGHT * scale) as f32];
        let mut occupied = vec![false; grid.0 * rows];
        for (index, row) in self.rows.iter().enumerate() {
            let base = level(row.priority);
            let steady = base.map(|c| (c as f32 * STEADY) as u8);
            for (column, (&character, &struck)) in row.text.iter().zip(&row.struck).enumerate() {
                if character == ' ' {
                    continue;
                }
                let unveiled = row.arrived + UNVEIL;
                if struck == f32::NEG_INFINITY && time < unveiled {
                    continue;
                }
                occupied[index * grid.0 + column] = true;
                let revealed = if struck.is_finite() { struck } else { unveiled };
                let flash = (-(time - revealed).max(0.0) / FLASH).exp();
                frame.items.push(Item::Glyph {
                    origin: [column as f32 * cell[0], index as f32 * cell[1]],
                    scale,
                    bits: glyphs::glyph(character),
                    color: mix(steady, HEAD, flash),
                });
            }
        }
        for stream in &self.streams {
            let head = stream.head.floor() as isize;
            for behind in 0..stream.length as isize {
                let Ok(row) = usize::try_from(head - behind) else {
                    break;
                };
                if row >= rows || row > stream.floor(rows) || occupied[row * grid.0 + stream.column]
                {
                    continue;
                }
                let origin = [stream.column as f32 * cell[0], row as f32 * cell[1]];
                let color = if behind == 0 {
                    if stream.target.is_none() {
                        frame.items.push(Item::Glow {
                            center: [origin[0] + cell[0] * 0.5, origin[1] + cell[1] * 0.5],
                            radius: cell[1] * 1.4,
                            color: RAIN,
                            strength: 0.3,
                        });
                    }
                    HEAD
                } else {
                    let fade = 1.0 - behind as f32 / stream.length as f32;
                    RAIN.map(|c| (c as f32 * 0.6 * fade) as u8)
                };
                frame.items.push(Item::Glyph {
                    origin,
                    scale,
                    bits: glyphs::glyph(flicker(stream.column, row, time)),
                    color,
                });
            }
        }
    }
}

/// A rain glyph that changes several times a second, out of step with its neighbours.
fn flicker(column: usize, row: usize, time: f32) -> char {
    let tick = (time * 6.0 + column as f32 * 0.37) as u64;
    let hash = ((column as u64 * 0x9e37_79b9) ^ (row as u64 * 0x85eb_ca6b) ^ tick)
        .wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    (b'!' + ((hash >> 59) ^ (hash >> 33)) as u8 % 94) as char
}

/// Splits a line into rows of at most `width` characters, breaking after the last space that fits
/// (or mid-word when none does), and indenting continuation rows.
fn wrap(text: &str, width: usize) -> Vec<Vec<char>> {
    let characters: Vec<char> = text.chars().collect();
    let mut rows = Vec::new();
    let mut start = 0;
    let mut indent = 0;
    while start < characters.len() || rows.is_empty() {
        let room = width.saturating_sub(indent).max(1);
        let mut end = (start + room).min(characters.len());
        if end < characters.len()
            && let Some(space) = characters[start..end]
                .iter()
                .rposition(|&c| c == ' ')
                .filter(|&at| at > 0)
        {
            end = start + space + 1;
        }
        let mut row = vec![' '; indent];
        row.extend(&characters[start..end]);
        rows.push(row);
        start = end;
        indent = INDENT.min(width.saturating_sub(1));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Backdrop, Buffers, Sky};

    fn frame() -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Void,
            haze: [0.0; 3],
            time: 0.0,
        };
        Frame::new(560, 280, sky, Buffers::default())
    }

    fn rain() -> Rain {
        Rain {
            greeted: true,
            ..Rain::default()
        }
    }

    /// Runs the rain from `from` to `to` seconds and returns the last frame.
    fn run(rain: &mut Rain, from: f32, to: f32) -> Frame {
        let mut time = from;
        loop {
            let mut frame = frame();
            rain.draw(&mut frame, time);
            if time >= to {
                return frame;
            }
            time = (time + 0.05).min(to);
        }
    }

    fn glyph_at(frame: &Frame, column: usize, row: usize) -> Option<(u128, Color)> {
        let origin = [(column as u32 * WIDTH) as f32, (row as u32 * HEIGHT) as f32];
        frame.items.iter().find_map(|item| match *item {
            Item::Glyph {
                origin: at,
                bits,
                color,
                ..
            } if at == origin => Some((bits, color)),
            _ => None,
        })
    }

    /// Whether a character on the first row has finished flashing at `time`.
    fn settled(rain: &Rain, column: usize, time: f32) -> bool {
        let row = &rain.rows[0];
        let struck = row.struck[column];
        let revealed = if struck.is_finite() {
            struck
        } else {
            row.arrived + UNVEIL
        };
        time - revealed > 10.0 * FLASH
    }

    #[test]
    fn a_line_is_decoded_horizontally_in_its_level_colour() {
        let mut rain = rain();
        run(&mut rain, 0.0, 0.0);
        rain.push(Line::new(4, "disk: low space"));
        let end = UNVEIL + 2.0;
        let frame = run(&mut rain, 0.0, end);
        let steady = level(4).map(|c| (c as f32 * STEADY) as u8);
        let mut checked = 0;
        for (column, character) in "disk: low space".chars().enumerate() {
            if character == ' ' {
                continue;
            }
            let (bits, color) = glyph_at(&frame, column, 0).expect("character is drawn");
            assert_eq!(bits, glyphs::glyph(character), "column {column}");
            if settled(&rain, column, end) {
                let off = (0..3).map(|k| color[k].abs_diff(steady[k])).max().unwrap();
                assert!(off <= 1, "column {column}: {color:?} is not {steady:?}");
                checked += 1;
            }
        }
        assert!(checked > 0, "every character was still flashing");
    }

    #[test]
    fn characters_stay_hidden_until_rain_passes_over_them() {
        let mut rain = rain();
        run(&mut rain, 0.0, 0.0);
        rain.push(Line::new(6, "x".repeat(40)));
        let frame = run(&mut rain, 0.0, 0.05);
        let row = &rain.rows[0];
        for (column, &struck) in row.struck.iter().enumerate() {
            let drawn =
                glyph_at(&frame, column, 0).is_some_and(|(bits, _)| bits == glyphs::glyph('x'));
            if struck == f32::NEG_INFINITY {
                assert!(!drawn, "column {column} showed before any rain reached it");
            }
        }
        let later = run(&mut rain, 0.05, UNVEIL + 0.1);
        assert!((0..40).all(|c| glyph_at(&later, c, 0).is_some()));
    }

    #[test]
    fn errors_settle_red_and_information_green() {
        for (priority, red) in [(2, true), (6, false)] {
            let mut rain = rain();
            run(&mut rain, 0.0, 0.0);
            rain.push(Line::new(priority, "abcdefghij"));
            let end = UNVEIL + 3.0;
            let frame = run(&mut rain, 0.0, end);
            let calm: Vec<usize> = (0..10).filter(|&c| settled(&rain, c, end)).collect();
            assert!(!calm.is_empty(), "every character was still flashing");
            for column in calm {
                let (_, color) = glyph_at(&frame, column, 0).unwrap();
                assert_eq!(color[0] > color[1], red, "priority {priority}: {color:?}");
            }
        }
    }

    #[test]
    fn long_lines_wrap_at_spaces_with_indented_continuations() {
        let rows: Vec<String> = wrap("alpha beta gamma delta", 12)
            .into_iter()
            .map(|row| row.into_iter().collect())
            .collect();
        assert_eq!(rows, ["alpha beta ", "  gamma ", "  delta"]);
        assert_eq!(wrap("", 12), vec![Vec::<char>::new()]);
    }

    #[test]
    fn the_log_scrolls_and_keeps_only_a_screen_of_rows() {
        let mut rain = rain();
        run(&mut rain, 0.0, 0.0);
        let rows = rain.grid.1;
        for k in 0..rows + 5 {
            rain.push(Line::new(6, format!("line {k}")));
        }
        run(&mut rain, 0.0, 60.0);
        assert_eq!(rain.rows.len(), rows);
        let last: String = rain.rows[rows - 1].text.iter().collect();
        assert_eq!(last, format!("line {}", rows + 4));
    }

    #[test]
    fn an_empty_journal_still_rains() {
        let mut rain = rain();
        let frame = run(&mut rain, 0.0, 3.0);
        assert!(rain.rows.is_empty());
        assert!(frame.items.iter().any(|i| matches!(i, Item::Glyph { .. })));
    }

    #[test]
    fn startup_history_waits_for_the_screen_size_before_trimming() {
        let mut rain = rain();
        for k in 0..200 {
            rain.push(Line::new(6, format!("line {k}")));
        }
        assert_eq!((rain.pending(), rain.skipped), (200, 0));
        let mut first = frame();
        rain.draw(&mut first, 0.0);
        let kept = rain.grid.1 * 2;
        assert_eq!(rain.pending() + rain.rows.len(), kept);
        assert_eq!(rain.skipped as usize, 200 - kept);
    }

    #[test]
    fn a_slow_frame_releases_every_overdue_line() {
        let mut rain = rain();
        run(&mut rain, 0.0, 0.0);
        for k in 0..6 {
            rain.push(Line::new(6, format!("line {k}")));
        }
        // At one frame per second, the first line goes out at once and the queue then releases at
        // 0.19, 0.40, 0.65 and 0.98 s, so five lines are due by the second frame.
        run(&mut rain, 0.0, 0.0);
        let mut late = frame();
        rain.draw(&mut late, 1.0);
        assert_eq!(rain.rows.len(), 5);
    }

    #[test]
    fn a_giant_line_only_writes_what_the_screen_can_show() {
        let mut rain = rain();
        run(&mut rain, 0.0, 0.0);
        let (columns, rows, _) = rain.grid;
        rain.push(Line::new(6, "z".repeat(columns * rows * 20)));
        run(&mut rain, 0.0, 0.0);
        assert_eq!(rain.rows.len(), rows);
        assert!(rain.streams.len() <= columns * rows + columns);
    }

    #[test]
    fn a_flood_is_capped_and_counted() {
        let mut rain = rain();
        run(&mut rain, 0.0, 0.0);
        for k in 0..1000 {
            rain.push(Line::new(6, format!("line {k}")));
        }
        assert_eq!(rain.pending() as u64 + rain.skipped, 1000);
        assert!(rain.pending() <= rain.grid.1.max(16) * 2);
        assert_eq!(rain.latest(), Some("line 999"));
    }
}
