use std::io;

use crate::terminal::{Popup, SafeText};

use super::pointer::Spot;

/// Terminal rows of the status panel under the scene.
pub(crate) const PANEL_ROWS: u16 = 5;

#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub(crate) columns: u16,
    pub(crate) rows: u16,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Terminal pixels per cell, when the terminal reports its window size.
    pub(crate) cell: Option<[f32; 2]>,
}

impl Layout {
    /// Sizes the scene to the terminal, leaving `PANEL_ROWS` at the bottom when the panel shows.
    pub(crate) fn measure(max_width: u32, panel: bool) -> io::Result<Self> {
        let (columns, rows) = crossterm::terminal::size()?;
        if columns < 40 || rows < 12 {
            return Err(io::Error::other(
                "isotop needs at least 40 columns and 12 rows",
            ));
        }
        let scene_rows = if panel { rows - PANEL_ROWS } else { rows };
        let cell = crossterm::terminal::window_size()
            .ok()
            .filter(|s| s.width > 0 && s.height > 0 && s.columns > 0 && s.rows > 0)
            .map(|s| {
                [
                    s.width as f32 / s.columns as f32,
                    s.height as f32 / s.rows as f32,
                ]
            });
        let [cell_width, cell_height] = cell.unwrap_or([8.0, 16.0]);
        let native_width = columns as f32 * cell_width;
        let width = native_width.min(max_width as f32).max(160.0) as u32;
        let height = (width as f32 * scene_rows as f32 * cell_height / native_width)
            .clamp(100.0, 1440.0) as u32;
        Ok(Self {
            columns,
            rows: scene_rows,
            width,
            height,
            cell,
        })
    }

    /// Frame pixels per terminal cell.
    pub(crate) fn cell_size(&self) -> [f32; 2] {
        [
            self.width as f32 / self.columns as f32,
            self.height as f32 / self.rows as f32,
        ]
    }

    pub(crate) fn cell_of(&self, point: [f32; 2]) -> (u16, u16) {
        let [w, h] = self.cell_size();
        ((point[0] / w) as u16, (point[1] / h) as u16)
    }

    /// Frame position and cell under a mouse report, which arrives in pixels when `pixels` is set.
    pub(crate) fn spot(&self, column: u16, row: u16, pixels: bool) -> Spot {
        match self.cell.filter(|_| pixels) {
            Some([cw, ch]) => {
                let (x, y) = (column as f32, row as f32);
                Spot {
                    frame: [
                        x * self.width as f32 / (self.columns as f32 * cw),
                        y * self.height as f32 / (self.rows as f32 * ch),
                    ],
                    cell: ((x / cw) as u16, (y / ch) as u16),
                }
            }
            None => {
                let [w, h] = self.cell_size();
                Spot {
                    frame: [(column as f32 + 0.5) * w, (row as f32 + 0.5) * h],
                    cell: (column, row),
                }
            }
        }
    }

    pub(crate) fn pick_radius(&self, pixels: bool) -> f32 {
        let [w, h] = self.cell_size();
        if pixels { w.max(h) * 0.6 } else { w.max(h) }
    }
}

/// Screen cells already used by panels and labels, so later labels never overlap them.
#[derive(Default)]
pub(super) struct Board(Vec<[u16; 4]>);

impl Board {
    /// Claims a label's cells if they are free and at least one column clear of every claimed
    /// rectangle on the same rows, so neighbouring labels never run together.
    pub(super) fn claim(&mut self, column: u16, row: u16, width: u16, height: u16) -> bool {
        let rect = [column, row, column + width, row + height];
        let free = self
            .0
            .iter()
            .all(|r| rect[2] < r[0] || r[2] < rect[0] || rect[3] <= r[1] || r[3] <= rect[1]);
        if free {
            self.0.push(rect);
        }
        free
    }

    /// Claims a panel's cells if they are free. Panels may touch each other and labels, but a
    /// label keeps its gap from them.
    pub(super) fn reserve(&mut self, column: u16, row: u16, width: u16, height: u16) -> bool {
        let rect = [column, row, column + width, row + height];
        let free = self
            .0
            .iter()
            .all(|r| rect[2] <= r[0] || r[2] <= rect[0] || rect[3] <= r[1] || r[3] <= rect[1]);
        if free {
            self.0.push(rect);
        }
        free
    }
}

/// Places a bordered panel beside (or above) an anchor cell, staying inside the scene. The lines
/// are text from the sample (names, commands, a view's notes), so each becomes a `SafeText` here.
pub(super) fn place_panel(
    lines: Vec<String>,
    anchor: Option<(u16, u16)>,
    layout: &Layout,
    above: bool,
    min_width: usize,
) -> Option<Popup> {
    let lines = lines.iter().map(|line| SafeText::new(line)).collect();
    place_lines(lines, anchor, layout, above, min_width)
}

/// `place_panel` for lines already made safe, sparklines among them (`SafeText::braille`). The
/// panel is as wide as its widest line in cells.
pub(super) fn place_lines(
    lines: Vec<SafeText>,
    anchor: Option<(u16, u16)>,
    layout: &Layout,
    above: bool,
    min_width: usize,
) -> Option<Popup> {
    let (columns, rows) = (layout.columns, layout.rows);
    let longest = lines.iter().map(SafeText::width).max().unwrap_or_default();
    let width = (longest + 4).clamp(min_width, 64).min(columns as usize) as u16;
    let height = lines.len() as u16 + 2;
    if height > rows {
        return None;
    }
    let (column, row) = match anchor {
        Some((x, y)) => {
            let column = if x + 4 + width <= columns {
                x + 4
            } else {
                x.saturating_sub(4 + width)
            };
            let row = match (above, y >= height + 2) {
                (true, true) => y - height - 2,
                (true, false) => y + 3,
                (false, _) => y.saturating_sub(height / 2),
            };
            (column, row.min(rows - height))
        }
        None => (columns - width, 0),
    };
    Some(Popup {
        column,
        row,
        width,
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: Layout = Layout {
        columns: 120,
        rows: 40,
        width: 960,
        height: 640,
        cell: None,
    };

    #[test]
    fn a_panel_is_as_wide_as_its_longest_line_in_cells_not_bytes() {
        let ascii = place_panel(
            vec!["abcdefghijklmnopqrstuvwxyz0123".into()],
            None,
            &LAYOUT,
            false,
            10,
        )
        .expect("fits");
        // Thirty ideographs are ninety bytes, and thirty `?` once cleaned.
        let wide = place_panel(vec!["日".repeat(30)], None, &LAYOUT, false, 10).expect("fits");
        assert_eq!(
            wide.width, ascii.width,
            "{} against {}",
            wide.width, ascii.width
        );
        assert_eq!(wide.width, 34, "thirty cells and the border");
        assert_eq!(wide.lines[0].width(), 30);
    }

    #[test]
    fn a_sparkline_counts_a_cell_a_dot_and_text_a_cell_a_character() {
        let lines = vec![SafeText::braille(&"⣿".repeat(40)), SafeText::new("short")];
        let popup = place_lines(lines, None, &LAYOUT, false, 10).expect("fits");
        assert_eq!(popup.width, 44, "forty cells and the border");
        let popup = place_lines(
            vec![SafeText::new(&"x".repeat(200))],
            None,
            &LAYOUT,
            false,
            10,
        )
        .expect("fits");
        assert_eq!(popup.width, 64, "no panel is wider than 64 cells");
    }

    #[test]
    fn labels_on_one_row_keep_a_free_column_between_them() {
        let mut board = Board::default();
        assert!(board.claim(10, 4, 5, 1));
        assert!(!board.claim(15, 4, 5, 1), "touching on the right");
        assert!(!board.claim(5, 4, 5, 1), "touching on the left");
        assert!(board.claim(16, 4, 5, 1), "one free column is enough");
        assert!(board.claim(4, 4, 5, 1), "one free column on the left");
        assert!(board.claim(10, 5, 5, 1), "other rows are not neighbours");
        assert!(!board.claim(12, 4, 2, 1), "overlap is still refused");
    }

    #[test]
    fn labels_keep_a_free_column_from_panels_but_panels_may_touch() {
        let mut board = Board::default();
        assert!(board.reserve(20, 2, 10, 6));
        assert!(board.reserve(30, 2, 5, 6), "panels keep their old rule");
        assert!(!board.reserve(32, 2, 5, 6), "panels still never overlap");
        assert!(!board.claim(35, 4, 6, 1), "label against a panel's edge");
        assert!(!board.claim(14, 4, 6, 1), "label ending at a panel's edge");
        assert!(board.claim(36, 4, 6, 1));
        assert!(board.claim(13, 4, 6, 1));
    }
}
