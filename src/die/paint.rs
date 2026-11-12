//! Drawing the die: the package under it, the heat field (one `Frame::field`, whose owner grid
//! names the core block under each cell), the outlines of the blocks, the marks of the sensors,
//! the throttle flashes and the process points. Everything is triangles, spheres and the field
//! item, so the CPU and the GPU draw it alike; strokes are ribbons, never one-pixel lines.
//!
//! The plan (`floorplan`) is in cells, x to the right and y down. `plane` puts it on the ground
//! with the default camera's quarter turn built in, so the die faces the viewer upright.

use std::f32::consts::FRAC_1_SQRT_2;

use super::floorplan::{COLUMNS, Floorplan, Rect, ROWS, Role};
use super::readings::Coverage;
use crate::palette::{ironbow, mix, tint};
use crate::render::strokes::Width;
use crate::render::{Camera, Color, Field, Frame, NONE, Placement, Point, Relief};

/// World units to a cell of the plan.
pub const CELL: f32 = 0.2;
/// The heights, in world units, of the layers: substrate, die face, strokes over the die, and the
/// process points.
const SUBSTRATE: f32 = -0.5;
const STROKES: f32 = 0.09;
const POINTS: f32 = 0.45;

const LINE: Color = [150, 158, 190];
const LINE_DIM: Color = [92, 98, 126];
const SENSOR: Color = [232, 240, 255];
const FLASH: Color = [110, 240, 255];
const CHOSEN: Color = [255, 255, 255];
const SLAB: Color = [8, 10, 14];
const BOARD: Color = [18, 36, 33];
const PARTS: Color = [58, 66, 62];
/// What a core with no reading at all is drawn in, and its hatching.
const BLANK: Color = [24, 22, 32];
const BLANK_HATCH: Color = [64, 64, 84];
const HATCH: Color = [150, 160, 192];

/// The world point of plan point `(x, y)` (cells) at height `z`.
pub fn plane(x: f32, y: f32, z: f32) -> Point {
    let across = (x - COLUMNS as f32 / 2.0) * CELL;
    let down = (y - ROWS as f32 / 2.0) * CELL;
    [
        (across + down) * FRAC_1_SQRT_2,
        (down - across) * FRAC_1_SQRT_2,
        z,
    ]
}

/// The corners of `rect` at height `z`, clockwise from the top left, closed.
fn ring(rect: Rect, z: f32) -> [Point; 5] {
    [
        plane(rect[0], rect[1], z),
        plane(rect[2], rect[1], z),
        plane(rect[2], rect[3], z),
        plane(rect[0], rect[3], z),
        plane(rect[0], rect[1], z),
    ]
}

/// The frame and camera everything here draws into.
pub struct Pen<'a> {
    pub frame: &'a mut Frame,
    pub camera: &'a Camera,
}

impl Pen<'_> {
    /// A flat rectangle of the plan.
    pub fn flat(&mut self, rect: Rect, z: f32, color: Color) {
        let corners = [
            plane(rect[0], rect[1], z),
            plane(rect[2], rect[1], z),
            plane(rect[2], rect[3], z),
            plane(rect[0], rect[3], z),
        ];
        self.frame.quad(self.camera, corners, color, NONE);
    }

    /// A stroke `pixels` wide along `points`.
    pub fn stroke(&mut self, points: &[Point], pixels: f32, color: Color) {
        self.frame
            .ribbon(self.camera, points, Width::Pixels(pixels), color, NONE, None);
    }

    /// The outline of `rect`.
    pub fn outline(&mut self, rect: Rect, z: f32, pixels: f32, color: Color) {
        self.stroke(&ring(rect, z), pixels, color);
    }

    /// A segment of the plan.
    pub fn segment(&mut self, from: [f32; 2], to: [f32; 2], z: f32, pixels: f32, color: Color) {
        self.stroke(
            &[plane(from[0], from[1], z), plane(to[0], to[1], z)],
            pixels,
            color,
        );
    }
}

/// The package under the die: a substrate slightly larger than it, a few capacitors along its
/// edges, and the die's slab, which stands a little above it. All of it is decoration.
pub fn package(pen: &mut Pen) {
    pen.flat([-6.0, -6.0, COLUMNS as f32 + 6.0, ROWS as f32 + 6.0], SUBSTRATE, BOARD);
    for row in [-4.2_f32, ROWS as f32 + 1.7] {
        for column in 0..22 {
            let x = 6.0 + column as f32 * 8.6;
            pen.flat([x, row, x + 4.0, row + 2.5], SUBSTRATE + 0.02, PARTS);
        }
    }
    // The slab's sides, from the substrate up to the die's face.
    let (right, bottom) = (COLUMNS as f32, ROWS as f32);
    let edge = |from: [f32; 2], to: [f32; 2]| {
        [
            plane(from[0], from[1], SUBSTRATE + 0.04),
            plane(to[0], to[1], SUBSTRATE + 0.04),
            plane(to[0], to[1], 0.0),
            plane(from[0], from[1], 0.0),
        ]
    };
    for (from, to) in [
        ([0.0, 0.0], [right, 0.0]),
        ([right, 0.0], [right, bottom]),
        ([right, bottom], [0.0, bottom]),
        ([0.0, bottom], [0.0, 0.0]),
    ] {
        pen.frame.quad(pen.camera, edge(from, to), SLAB, NONE);
    }
}

/// How a core block's colour is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// The ironbow colour of the field: a sensor speaks for the block.
    Measured,
    /// The ironbow colour of the model, hatched: no sensor does.
    Model,
    /// No sensor and no load reading: dark and hatched, never cold.
    Blank,
}

impl Look {
    pub fn of(coverage: Coverage, proxy: Option<f32>) -> Look {
        match (coverage, proxy) {
            (Coverage::Unknown, Some(_)) => Look::Model,
            (Coverage::Unknown, None) => Look::Blank,
            _ => Look::Measured,
        }
    }
}

/// Colours the field's nodes from `shown`: each cell's level on the scale through ironbow, a
/// diagonal hatch over the blocks no sensor speaks for, and a blank grey for the block with no
/// reading at all. `level` maps a value to 0 to 1; `cell_block` is the block under each cell
/// (`u16::MAX` for none) and `looks` each block's `Look`; `picks` is each block's pick value.
pub fn colour_field(
    field: &mut Field,
    shown: &[f32],
    level: impl Fn(f32) -> f32,
    cell_block: &[u16],
    looks: &[Look],
    picks: &[u32],
) {
    let owners = field.owners.get_or_insert_with(Vec::new);
    owners.clear();
    for (cell, &value) in shown.iter().enumerate() {
        let (column, row) = (cell % COLUMNS, cell / COLUMNS);
        let block = cell_block[cell];
        let base = ironbow(level(value));
        let hatched = (column + row) % 6 == 0;
        let color = match looks.get(usize::from(block)) {
            Some(Look::Model) if hatched => mix(base, HATCH, 0.5),
            Some(Look::Blank) => {
                if hatched {
                    BLANK_HATCH
                } else {
                    BLANK
                }
            }
            _ => base,
        };
        field.colors[cell] = color;
        owners.push(picks.get(usize::from(block)).copied().unwrap_or(NONE));
    }
    field.revision += 1;
}

/// Draws `field` over the die.
pub fn field(pen: &mut Pen, field: &Field) {
    let corners = [
        plane(0.5, 0.5, 0.0),
        plane(COLUMNS as f32 - 0.5, 0.5, 0.0),
        plane(COLUMNS as f32 - 0.5, ROWS as f32 - 0.5, 0.0),
        plane(0.5, ROWS as f32 - 0.5, 0.0),
    ];
    pen.frame
        .field(pen.camera, Placement::Corners(corners), field, Relief::Flat);
}

/// The block outlines, the sensors' marks, and what is chosen. `chosen` is the block of the
/// selected or hovered thing.
pub fn outlines(pen: &mut Pen, plan: &Floorplan, coverage: &[Coverage], chosen: Option<usize>) {
    for part in &plan.context {
        pen.outline(part.rect, STROKES, 1.2, LINE_DIM);
    }
    for group in &plan.groups {
        pen.outline(group.rect, STROKES, 1.4, LINE);
    }
    for (index, block) in plan.blocks.iter().enumerate() {
        let color = match block.role {
            Role::Performance | Role::Plain => LINE,
            Role::Efficiency | Role::LowPower => LINE_DIM,
        };
        pen.outline(block.rect, STROKES, 1.3, color);
        // A cross at the hot spot of each core that has a sensor of its own.
        if matches!(coverage.get(index), Some(Coverage::Own(_))) {
            let [x, y] = block.hot;
            pen.segment([x - 1.0, y], [x + 1.0, y], STROKES * 1.2, 1.3, SENSOR);
            pen.segment([x, y - 1.0], [x, y + 1.0], STROKES * 1.2, 1.3, SENSOR);
        }
    }
    pen.outline([0.0, 0.0, COLUMNS as f32, ROWS as f32], STROKES, 1.6, LINE_DIM);
    if let Some(block) = chosen.and_then(|index| plan.blocks.get(index)) {
        pen.outline(block.rect, STROKES * 1.6, 2.6, CHOSEN);
    }
}

/// The outline of a block that was throttled `age` seconds ago, blinking three times a second.
pub fn flash(pen: &mut Pen, rect: Rect, age: f32) {
    let blink = 0.5 + 0.5 * (std::f32::consts::TAU * 3.0 * age).cos();
    pen.outline(rect, STROKES * 2.0, 2.0 + 2.2 * blink, FLASH);
}

/// One process: a small sphere on the block it last ran on, bright where it is busy.
pub fn point(pen: &mut Pen, at: [f32; 2], radius: f32, color: Color, pick: u32, selected: bool) {
    let center = plane(at[0], at[1], POINTS);
    pen.frame
        .sphere(pen.camera, center, radius, color, pick, selected);
}

/// The world position of a process point (what `Stage::positions` records).
pub fn point_position(at: [f32; 2]) -> Point {
    plane(at[0], at[1], POINTS)
}

/// The colour of a process point: its kind's colour, brighter the busier it is.
pub fn point_color(kind: Color, cpu: f32) -> Color {
    tint(kind, 0.55 + 0.45 * (cpu / 100.0).clamp(0.0, 1.0).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_faces_the_default_camera_with_x_to_the_right_and_y_down() {
        // The default camera turns the world a quarter: across = (x - y) / sqrt 2, along =
        // (x + y) / sqrt 2. A plan point to the right of another is to the right on screen, and
        // one further down the plan is nearer the viewer.
        let (centre, right, down) = (plane(96.0, 48.0, 0.0), plane(106.0, 48.0, 0.0), plane(96.0, 58.0, 0.0));
        let across = |p: Point| (p[0] - p[1]) * FRAC_1_SQRT_2;
        let along = |p: Point| (p[0] + p[1]) * FRAC_1_SQRT_2;
        assert!(across(right) - across(centre) > 1.9 && (along(right) - along(centre)).abs() < 1e-4);
        assert!(along(down) - along(centre) > 1.9 && (across(down) - across(centre)).abs() < 1e-4);
        assert_eq!(centre, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_block_is_hatched_only_where_no_sensor_speaks_for_it() {
        assert_eq!(Look::of(Coverage::Own(60.0), Some(0.5)), Look::Measured);
        assert_eq!(Look::of(Coverage::Ccd(0, 60.0), None), Look::Measured);
        assert_eq!(Look::of(Coverage::Package(60.0), Some(0.5)), Look::Measured);
        assert_eq!(Look::of(Coverage::Unknown, Some(0.0)), Look::Model, "an idle core is modelled, not blank");
        assert_eq!(Look::of(Coverage::Unknown, None), Look::Blank);
    }

    #[test]
    fn the_field_takes_ironbow_colours_hatches_unmeasured_blocks_and_picks_through_to_blocks() {
        let mut field = Field::new(1, COLUMNS, ROWS);
        let shown = vec![60.0; COLUMNS * ROWS];
        // Block 0 covers the left half, block 1 the right; neither is outside.
        let cell_block: Vec<u16> = (0..COLUMNS * ROWS)
            .map(|cell| u16::from(cell % COLUMNS >= COLUMNS / 2))
            .collect();
        let looks = [Look::Measured, Look::Model];
        let picks = [7, 9];
        colour_field(&mut field, &shown, |value| (value - 30.0) / 75.0, &cell_block, &looks, &picks);
        let plain = ironbow(0.4);
        let at = |column: usize, row: usize| field.colors[row * COLUMNS + column];
        assert_eq!(at(10, 1), plain, "a measured block is plain ironbow");
        assert_ne!(at(102, 0), plain, "a modelled block is hatched on every sixth diagonal");
        assert_eq!(at(101, 0), plain, "and plain between");
        let owners = field.owners.as_ref().expect("owners");
        assert_eq!((owners[10], owners[100 + COLUMNS]), (7, 9));
        assert_eq!(field.revision, 1);
        // A blank block is dark whatever the field says, and a cell in no block picks nothing.
        let looks = [Look::Blank, Look::Blank];
        colour_field(&mut field, &vec![200.0; COLUMNS * ROWS], |value| value, &cell_block, &looks, &[]);
        assert!(field.colors.iter().all(|color| *color == BLANK || *color == BLANK_HATCH));
        assert!(field.owners.as_ref().expect("owners").iter().all(|&pick| pick == NONE));
    }
}
