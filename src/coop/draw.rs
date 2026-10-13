//! Drawing the yard: ground, fence, hedge, feeders, houses and nests, the chickens with their
//! chicks and dust, and the foxes.

use std::f32::consts::TAU;

use crate::geometry::{cuboid, normalize_or_zero, shade};
use crate::model::{Process, Snapshot, bounded, bytes};
use crate::palette::{core_color, kind_color, tint};
use crate::render::{Camera, Color, Frame, NONE, Point, SCALE, Stage, without_apple_prefix};
use crate::simulation::hash::unit_from as fraction;
use crate::simulation::sprout;

use super::layout::{
    BAR_SEATS, Feeder, Flock, HEDGE, HOUSE_HALF, HOUSE_Y, LADDER_BARS, NEST_HALF, PAD_HEIGHT,
    RUNG_STEP, SEAT, TROUGH_DEPTH, bar_height, body_radius, ladder,
};
use super::sim::{Chicken, Role};
use super::{Coop, DUST_RATE, FOX_SECONDS, Fox, plural, puffs};

/// A full nest box: a clutch of round(log2(1 + open files)) eggs up to CLUTCH, then one rotten
/// egg per file deleted while still open, up to ROTTEN. Together they fill its 4 by 6 grid.
pub(super) const CLUTCH: usize = 16;
const ROTTEN: usize = 8;
const EGG_SIZE: f32 = 0.15;
const EGG_SPACING: f32 = 0.24;
const NEST_HEIGHT: f32 = 0.7;
const WALL: f32 = 1.9;
const RIDGE: f32 = 3.1;

const GRASS: Color = [70, 118, 60];
const DIRT: Color = [118, 94, 66];
const WOOD: Color = [156, 120, 82];
const POST: Color = [118, 88, 60];
const LEAVES: Color = [36, 82, 46];
const BARN: Color = [170, 70, 54];
const ROOF: Color = [86, 74, 70];
const DOOR: Color = [56, 38, 30];
const STRAW: Color = [224, 196, 118];
const GRAIN: Color = [246, 214, 128];
const EGGSHELL: Color = [248, 240, 222];
const ROTTEN_SHELL: Color = [104, 122, 58];
const CRACK: Color = [52, 44, 30];
const LOCK_LINE: Color = [236, 214, 160];
const COMB: Color = [216, 46, 46];
const BEAK: Color = [244, 168, 48];
const LEGS: Color = [232, 160, 64];
const PUPIL: Color = [18, 18, 22];
const MUD: Color = [90, 62, 40];
const SHADOW: Color = [40, 56, 36];
const DUST: Color = [176, 140, 96];
const CHICK: Color = [252, 220, 112];
const FOX: Color = [214, 108, 38];
const FOX_WHITE: Color = [240, 234, 226];
const FOX_DARK: Color = [52, 36, 30];
const EYE_GLOW: Color = [255, 184, 56];
const GHOST: Color = [160, 160, 168];

/// Eggs in a flock's clutch for its open files: round(log2(1 + files)), at most CLUTCH.
pub(super) fn clutch(files: u64) -> usize {
    ((files as f64 + 1.0).log2().round() as usize).min(CLUTCH)
}

impl Coop {
    pub(super) fn paint(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        order: &[usize],
        snapshot: &Snapshot,
        alpha: f32,
    ) -> usize {
        let camera = stage.camera;
        let time = stage.time;
        let grows: Vec<f32> = order
            .iter()
            .map(|&index| sprout(stage.growth(processes[index].id)))
            .collect();
        let frame = &mut *stage.frame;
        let (low, high) = (self.yard.low, self.yard.high);
        draw_ground(frame, camera, low, high, self.band_top);
        draw_hedge(frame, camera, low, high);
        let psi_memory = snapshot.pressure[1];
        for (k, eye) in self.yard.eyes.iter().enumerate() {
            // Blinking is decorative.
            let blink = (time * 0.7 + k as f32 * 2.3).sin() > 0.93;
            if blink {
                continue;
            }
            let tangent = if eye[0] < low[0] || eye[0] > high[0] {
                [0.0, 1.0]
            } else {
                [1.0, 0.0]
            };
            for side in [-1.0, 1.0] {
                let p = [
                    eye[0] + tangent[0] * 0.22 * side,
                    eye[1] + tangent[1] * 0.22 * side,
                    0.75,
                ];
                frame.glow(
                    camera,
                    p,
                    (0.35 * camera.zoom).max(3.0),
                    EYE_GLOW,
                    0.55 + 0.4 * bounded(psi_memory, 20.0),
                );
                frame.world_sphere(camera, p, 0.07, [255, 220, 120]);
            }
        }
        draw_fence(frame, camera, low, high);
        for feeder in &self.feeders {
            draw_feeder(frame, camera, feeder);
        }
        for flock in &self.flocks {
            draw_house(frame, camera, flock);
            draw_clutch(frame, camera, flock);
        }
        let nest_notes: Vec<String> = self.flocks.iter().map(nest_note).collect();
        for chicken in &self.yard.chickens {
            if chicken.role == Role::Brooding {
                // A straw pad for a brooding hen beside the nest box; decorative.
                let [x, y] = chicken.target;
                let half = chicken.radius * 0.9;
                cuboid(
                    frame,
                    camera,
                    [x - half, y - half, 0.0],
                    [x + half, y + half, PAD_HEIGHT - 0.01],
                    STRAW,
                );
            }
        }

        let orders = self.yard.flock_orders(self.flocks.len());
        let mut shown = 0;
        for (k, &index) in order.iter().enumerate() {
            let process = processes[index];
            let Some(chicken) = self.yard.chickens.get(k).filter(|c| c.id == process.id) else {
                continue;
            };
            let mut ground = [
                chicken.previous[0] + (chicken.position[0] - chicken.previous[0]) * alpha,
                chicken.previous[1] + (chicken.position[1] - chicken.previous[1]) * alpha,
            ];
            if chicken.role == Role::Frozen {
                ground = chicken.position;
            }
            let r = body_radius(process.memory as f32) * grows[k];
            let selected = stage.selected == Some(process.id);
            let body = draw_chicken(
                frame,
                camera,
                chicken,
                ground,
                r,
                kind_color(process),
                index as u32,
                selected,
                time,
            );
            if let Some(read) = process.read_rate.filter(|&rate| rate > DUST_RATE) {
                for puff in 0..puffs(read) {
                    // The swirl is decorative; the number of puffs is the measurement.
                    let angle = puff as f32 / puffs(read) as f32 * TAU + time * 1.7;
                    let p = [
                        ground[0] + angle.cos() * r * 1.1,
                        ground[1] + angle.sin() * r * 1.1,
                        0.15 + 0.2 * (time * 2.0 + puff as f32).sin().abs(),
                    ];
                    frame.glow(camera, p, (0.55 * r * camera.zoom).max(3.0), DUST, 0.55);
                }
            }
            draw_chicks(frame, camera, chicken, ground, body, r, time);
            stage.positions.insert(process.id, body);
            let flock = &self.flocks[chicken.flock];
            let phi = match orders[chicken.flock] {
                Some((phi, foraging)) => format!("phi {phi:.2} ({foraging} foraging)"),
                None => "(none foraging)".into(),
            };
            let mut notes = vec![format!("flock {}: {phi}", flock.label)];
            notes.extend(chicken.notes.iter().cloned());
            notes.push(nest_notes[chicken.flock].clone());
            stage.notes.insert(process.id, notes);
            shown += 1;
        }
        // A faint line from each hen queueing for a lock to the hen holding it.
        for chicken in &self.yard.chickens {
            if chicken.role != Role::Waiting {
                continue;
            }
            if let Some(holder) = chicken.holder.filter(|&holder| holder != chicken.id)
                && let (Some(&from), Some(&to)) = (
                    stage.positions.get(&chicken.id),
                    stage.positions.get(&holder),
                )
            {
                frame.beam(camera, from, to, LOCK_LINE, 0.22);
            }
        }
        for fox in &self.foxes {
            draw_fox(frame, camera, fox, time);
        }
        let mut ranked: Vec<&Flock> = self.flocks.iter().collect();
        ranked.sort_by(|a, b| b.members.cmp(&a.members).then(a.key.cmp(&b.key)));
        let chosen = stage
            .selected
            .and_then(|id| self.yard.find(id))
            .map(|c| c.flock);
        for (rank, flock) in ranked.iter().enumerate() {
            let selected = chosen.is_some_and(|f| self.flocks[f].key == flock.key);
            if rank < 10 || selected {
                stage.places.push((
                    [flock.center[0], flock.center[1] + HOUSE_Y, RIDGE + 0.3],
                    without_apple_prefix(&flock.label).to_owned(),
                ));
            }
        }
        for x in [low[0], high[0]] {
            for y in [low[1], high[1]] {
                stage.bounds.push([x, y, 0.0]);
                stage.bounds.push([x, y, 3.0]);
            }
        }
        shown
    }
}

/// Grass inside and around the fence, with packed earth under the feeders. Decorative.
fn draw_ground(frame: &mut Frame, camera: &Camera, low: [f32; 2], high: [f32; 2], band_top: f32) {
    let margin = HEDGE + 2.5;
    let (x0, x1) = (low[0] - margin, high[0] + margin);
    let (y0, y1) = (low[1] - margin, high[1] + margin);
    let columns = ((x1 - x0) / 1.5).ceil().clamp(8.0, 48.0) as usize;
    let rows = ((y1 - y0) / 1.5).ceil().clamp(8.0, 48.0) as usize;
    let (dx, dy) = ((x1 - x0) / columns as f32, (y1 - y0) / rows as f32);
    for j in 0..rows {
        for i in 0..columns {
            let (ax, ay) = (x0 + i as f32 * dx, y0 + j as f32 * dy);
            let cell = (j * 64 + i) as u32;
            let swirl = (ax * 0.21).sin() * (ay * 0.17).cos();
            let inside = ax + dx > low[0] && ax < high[0] && ay + dy > low[1] && ay < high[1];
            let lush = if inside { 1.0 } else { 0.82 };
            let color = tint(GRASS, lush * (0.9 + 0.1 * swirl + 0.12 * fraction(cell)));
            frame.quad(
                camera,
                [
                    [ax, ay, 0.0],
                    [ax + dx, ay, 0.0],
                    [ax + dx, ay + dy, 0.0],
                    [ax, ay + dy, 0.0],
                ],
                color,
                NONE,
            );
        }
    }
    let (top, bottom) = (band_top + 0.3, low[1]);
    let depth = top - bottom;
    if depth > 0.0 {
        let rows = (depth / 1.5).ceil().clamp(1.0, 24.0) as usize;
        let columns = ((high[0] - low[0]) / 1.5).ceil().clamp(4.0, 48.0) as usize;
        let (dx, dy) = ((high[0] - low[0]) / columns as f32, depth / rows as f32);
        for j in 0..rows {
            for i in 0..columns {
                let (ax, ay) = (low[0] + i as f32 * dx, bottom + j as f32 * dy);
                let color = tint(DIRT, 0.9 + 0.15 * fraction((j * 64 + i) as u32 + 9000));
                frame.quad(
                    camera,
                    [
                        [ax, ay, 0.01],
                        [ax + dx, ay, 0.01],
                        [ax + dx, ay + dy, 0.01],
                        [ax, ay + dy, 0.01],
                    ],
                    color,
                    NONE,
                );
            }
        }
    }
}

/// Posts and two rails around the yard; the fence is the reflecting boundary.
fn draw_fence(frame: &mut Frame, camera: &Camera, low: [f32; 2], high: [f32; 2]) {
    let corners = [
        [low[0], low[1]],
        [high[0], low[1]],
        [high[0], high[1]],
        [low[0], high[1]],
    ];
    for side in 0..4 {
        let (a, b) = (corners[side], corners[(side + 1) % 4]);
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        let posts = (length / 2.4).ceil().max(1.0) as usize;
        for k in 0..posts {
            let t = k as f32 / posts as f32;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            cuboid(
                frame,
                camera,
                [p[0] - 0.09, p[1] - 0.09, 0.0],
                [p[0] + 0.09, p[1] + 0.09, 1.15],
                POST,
            );
        }
        for height in [0.45, 0.9] {
            frame.quad(
                camera,
                [
                    [a[0], a[1], height],
                    [b[0], b[1], height],
                    [b[0], b[1], height + 0.1],
                    [a[0], a[1], height + 0.1],
                ],
                WOOD,
                NONE,
            );
        }
    }
}

/// Dark shrubs in a double row outside the fence. Decorative and never pickable.
fn draw_hedge(frame: &mut Frame, camera: &Camera, low: [f32; 2], high: [f32; 2]) {
    for (layer, offset) in [(0_u32, HEDGE + 0.9), (1, HEDGE - 0.2)] {
        let (a, b) = (
            [low[0] - offset, low[1] - offset],
            [high[0] + offset, high[1] + offset],
        );
        let perimeter = 2.0 * (b[0] - a[0] + b[1] - a[1]);
        let count = (perimeter / 1.6).ceil() as u32;
        for k in 0..count {
            let seed = k * 2 + layer;
            let mut d = (k as f32 + 0.5 * layer as f32) / count as f32 * perimeter;
            let width = b[0] - a[0];
            let height = b[1] - a[1];
            let p = if d < width {
                [a[0] + d, b[1]]
            } else {
                d -= width;
                if d < height {
                    [b[0], b[1] - d]
                } else {
                    d -= height;
                    if d < width {
                        [b[0] - d, a[1]]
                    } else {
                        [a[0], a[1] + d - width]
                    }
                }
            };
            let radius = 0.85 + 0.45 * fraction(seed);
            let color = tint(LEAVES, 0.8 + 0.4 * fraction(seed + 7777));
            frame.world_sphere(camera, [p[0], p[1], radius * 0.8], radius, color);
        }
    }
}

fn draw_feeder(frame: &mut Frame, camera: &Camera, feeder: &Feeder) {
    let [x, y] = feeder.center;
    let half = [feeder.length() * 0.5, TROUGH_DEPTH * 0.5];
    let color = core_color(feeder.kind);
    cuboid(
        frame,
        camera,
        [x - half[0], y - half[1], 0.0],
        [x + half[0], y + half[1], 0.45],
        tint(color, 0.75),
    );
    // Grain brightness is how busy the CPU was, as the lanes of the cores view.
    frame.quad(
        camera,
        [
            [x - half[0] + 0.1, y - half[1] + 0.1, 0.46],
            [x + half[0] - 0.1, y - half[1] + 0.1, 0.46],
            [x + half[0] - 0.1, y + half[1] - 0.1, 0.46],
            [x - half[0] + 0.1, y + half[1] - 0.1, 0.46],
        ],
        tint(GRAIN, 0.25 + 0.85 * feeder.busy),
        NONE,
    );
}

/// The inspector line about a flock's nest, with the counts behind the capped clutch and what
/// they leave out.
fn nest_note(flock: &Flock) -> String {
    let nest = &flock.nest;
    let read = flock.members - nest.unreadable - nest.pending;
    if read == 0 {
        let why = if nest.unreadable > 0 {
            "unreadable (permissions)"
        } else {
            "not read yet"
        };
        return format!("{} nest: no eggs, open files {why}", flock.label);
    }
    let qualifier = if nest.partial {
        "about "
    } else if read < flock.members {
        "at least "
    } else {
        ""
    };
    let mut line = format!(
        "{} nest: {} for {qualifier}{}",
        flock.label,
        plural(clutch(nest.files) as u64, "egg", "eggs"),
        plural(nest.files, "open file", "open files")
    );
    let mut left_out = Vec::new();
    if nest.unreadable > 0 {
        left_out.push(format!("{} unreadable", nest.unreadable));
    }
    if nest.pending > 0 {
        left_out.push(format!("{} not read yet", nest.pending));
    }
    if !left_out.is_empty() {
        line.push_str(&format!(
            " ({} of {} members)",
            left_out.join(", "),
            flock.members
        ));
    }
    let deleted = nest.deleted.len() as u64;
    if deleted > 0 {
        let at_least = if nest.partial || read < flock.members {
            "at least "
        } else {
            ""
        };
        line.push_str(&format!(
            ", {} rotten for {at_least}{} still open ({} held)",
            deleted.min(ROTTEN as u64),
            plural(deleted, "deleted file", "deleted files"),
            bytes(nest.deleted_bytes())
        ));
    }
    line
}

/// Where an egg lies in the nest box, a grid four across and six along: the clutch fills it from
/// the back (south, away from the camera) and the rotten eggs from the front, where they show.
/// CLUTCH and ROTTEN fill four and two rows, so the two never meet.
fn egg_spot(nest: [f32; 2], index: usize, rotten: bool) -> [f32; 2] {
    let (column, row) = (index % 4, index / 4);
    let row = if rotten { 5 - row } else { row };
    [
        nest[0] + (column as f32 - 1.5) * EGG_SPACING,
        nest[1] + (row as f32 - 2.5) * EGG_SPACING,
    ]
}

/// The flock's eggs in its nest box: the clutch for its open files, then a cracked, discoloured
/// egg for each file deleted while still open.
fn draw_clutch(frame: &mut Frame, camera: &Camera, flock: &Flock) {
    let nest = flock.nest();
    let eggs = clutch(flock.nest.files);
    let rotten = flock.nest.deleted.len().min(ROTTEN);
    for index in 0..eggs {
        let [x, y] = egg_spot(nest, index, false);
        let position = [x, y, NEST_HEIGHT + 0.02 + EGG_SIZE];
        frame.world_sphere(camera, position, EGG_SIZE / SCALE, EGGSHELL);
    }
    for index in 0..rotten {
        let [x, y] = egg_spot(nest, index, true);
        let position = [x, y, NEST_HEIGHT + 0.02 + EGG_SIZE];
        frame.world_sphere(camera, position, EGG_SIZE / SCALE, ROTTEN_SHELL);
        let top = position[2] + EGG_SIZE * 0.75;
        let crack = [
            [x - 0.11, y - 0.02, top - 0.04],
            [x - 0.04, y + 0.03, top],
            [x + 0.02, y - 0.03, top],
            [x + 0.11, y + 0.02, top - 0.04],
        ];
        for pair in crack.windows(2) {
            frame.line(camera, pair[0], pair[1], CRACK);
        }
    }
}

/// The henhouse with a pitched roof and door, its nest box, and its perch ladders.
fn draw_house(frame: &mut Frame, camera: &Camera, flock: &Flock) {
    let [cx, cy] = [flock.center[0], flock.center[1] + HOUSE_Y];
    let [hx, hy] = HOUSE_HALF;
    cuboid(
        frame,
        camera,
        [cx - hx, cy - hy, 0.0],
        [cx + hx, cy + hy, WALL],
        BARN,
    );
    let eave = 0.18;
    for side in [-1.0_f32, 1.0] {
        frame.quad(
            camera,
            [
                [cx - hx - eave, cy, RIDGE],
                [cx + hx + eave, cy, RIDGE],
                [cx + hx + eave, cy + side * (hy + eave), WALL - 0.1],
                [cx - hx - eave, cy + side * (hy + eave), WALL - 0.1],
            ],
            tint(ROOF, if side > 0.0 { 1.15 } else { 0.85 }),
            NONE,
        );
    }
    for side in [-1.0_f32, 1.0] {
        frame.facet(
            camera,
            [
                [cx + side * hx, cy - hy, WALL],
                [cx + side * hx, cy + hy, WALL],
                [cx + side * hx, cy, RIDGE - 0.05],
            ],
            shade(BARN, [side, 0.0]),
            NONE,
        );
    }
    frame.quad(
        camera,
        [
            [cx - 0.3, cy + hy + 0.01, 0.0],
            [cx + 0.3, cy + hy + 0.01, 0.0],
            [cx + 0.3, cy + hy + 0.01, 0.85],
            [cx - 0.3, cy + hy + 0.01, 0.85],
        ],
        DOOR,
        NONE,
    );
    let nest = flock.nest();
    let [nx, ny] = NEST_HALF;
    cuboid(
        frame,
        camera,
        [nest[0] - nx, nest[1] - ny, 0.0],
        [nest[0] + nx, nest[1] + ny, NEST_HEIGHT - 0.05],
        tint(WOOD, 0.85),
    );
    frame.quad(
        camera,
        [
            [nest[0] - nx + 0.08, nest[1] - ny + 0.08, NEST_HEIGHT],
            [nest[0] + nx - 0.08, nest[1] - ny + 0.08, NEST_HEIGHT],
            [nest[0] + nx - 0.08, nest[1] + ny - 0.08, NEST_HEIGHT],
            [nest[0] - nx + 0.08, nest[1] + ny - 0.08, NEST_HEIGHT],
        ],
        STRAW,
        NONE,
    );
    let seats = flock.span.max(1);
    let per_ladder = BAR_SEATS * LADDER_BARS;
    for g in 0..seats.div_ceil(per_ladder) {
        let cell = ladder(g);
        let bars = (seats - g * per_ladder)
            .div_ceil(BAR_SEATS)
            .min(LADDER_BARS);
        let [lx, ly] = [flock.center[0] + cell[0], flock.center[1] + cell[1]];
        let reach = SEAT * BAR_SEATS as f32 * 0.5;
        let front = ly + RUNG_STEP * 1.5 + 0.3;
        let back = ly + RUNG_STEP * (1.5 - (bars as f32 - 1.0)) - 0.25;
        let top = bar_height(bars - 1);
        for side in [-1.0_f32, 1.0] {
            let x = lx + side * (reach + 0.05);
            frame.quad(
                camera,
                [
                    [x, front, 0.0],
                    [x, back, top + 0.05],
                    [x, back, top + 0.2],
                    [x, front, 0.15],
                ],
                POST,
                NONE,
            );
            frame.quad(
                camera,
                [
                    [x, back - 0.06, 0.0],
                    [x, back + 0.06, 0.0],
                    [x, back + 0.06, top + 0.15],
                    [x, back - 0.06, top + 0.15],
                ],
                tint(POST, 0.85),
                NONE,
            );
        }
        for bar in 0..bars {
            let y = ly + RUNG_STEP * (1.5 - bar as f32);
            let z = bar_height(bar);
            frame.quad(
                camera,
                [
                    [lx - reach, y - 0.1, z],
                    [lx + reach, y - 0.1, z],
                    [lx + reach, y + 0.1, z],
                    [lx - reach, y + 0.1, z],
                ],
                WOOD,
                NONE,
            );
            frame.quad(
                camera,
                [
                    [lx - reach, y + 0.1, z - 0.14],
                    [lx + reach, y + 0.1, z - 0.14],
                    [lx + reach, y + 0.1, z],
                    [lx - reach, y + 0.1, z],
                ],
                tint(WOOD, 0.8),
                NONE,
            );
        }
    }
}

/// A part of a chicken: pickable at full size when large enough to draw true to size, otherwise
/// an unclamped, unpickable sphere, so small parts never swell to the minimum body size.
fn part(frame: &mut Frame, camera: &Camera, p: Point, radius: f32, color: Color, pick: u32) {
    let pixels = radius * camera.zoom;
    if pick != NONE && pixels >= crate::render::MIN_BODY_PX {
        frame.sphere(camera, p, radius, color, pick, false);
    } else if pixels >= 0.45 {
        frame.world_sphere(camera, p, radius / SCALE, color);
    }
}

fn offset(base: Point, forward: [f32; 2], f: f32, side: f32, up: f32) -> Point {
    [
        base[0] + forward[0] * f - forward[1] * side,
        base[1] + forward[1] * f + forward[0] * side,
        base[2] + up,
    ]
}

/// Draws one chicken in the pose of its role and returns its body centre: a plump body of
/// three spheres, neck, head with comb, beak, wattle and eyes, wings, a tail fan and legs.
#[expect(clippy::too_many_arguments, reason = "split in #78 (A19)")]
#[expect(clippy::too_many_lines, reason = "split in #78 (A19)")]
fn draw_chicken(
    frame: &mut Frame,
    camera: &Camera,
    chicken: &Chicken,
    ground: [f32; 2],
    r: f32,
    plumage: Color,
    pick: u32,
    selected: bool,
    time: f32,
) -> Point {
    let forward = [chicken.theta.cos(), chicken.theta.sin()];
    let phase = (chicken.id.pid % 97) as f32;
    let seated = chicken.role == Role::Roosting || chicken.perch > 0.0;
    let walking = chicken.role == Role::Foraging || chicken.walking();
    // Hopping up to the perch over the last unit of the walk.
    let lift = if seated {
        let away = (chicken.target[0] - ground[0]).hypot(chicken.target[1] - ground[1]);
        chicken.perch * (1.0 - away).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let perched = seated && lift > 0.0;
    let zombie = chicken.role == Role::Zombie;
    // Body height, then the head's offset forward and up, in body radii.
    let (height, head_forward, head_up, neck) = match chicken.role {
        Role::Zombie => (r * 0.8, 1.05, -0.3, false),
        Role::Frozen => (lift + r * 0.72, 0.85, 0.45, false),
        // Brooding: settled low on the straw pad, feathers fluffed out.
        Role::Brooding if perched => (lift + r * 0.4, 0.8, 0.4, false),
        _ if perched => (lift + r * 0.6, 0.7, 0.5, false),
        Role::Feeding if chicken.arrived => {
            // Pecking is decorative: the head bobs into the trough.
            let peck = (time * 7.0 + phase).sin().abs();
            (r * 1.3, 1.15, 0.25 - 0.45 * peck, true)
        }
        _ => (r * 1.3, 0.85, 0.95, true),
    };
    let up = if zombie { -1.0 } else { 1.0 };
    let body = [ground[0], ground[1], height];
    if !perched && pick != NONE {
        // A soft shadow grounds the bird; decorative.
        let shade = r * 1.05;
        for k in 0..6 {
            let (a, b) = (k as f32 / 6.0 * TAU, (k + 1) as f32 / 6.0 * TAU);
            frame.facet(
                camera,
                [
                    [ground[0], ground[1], 0.015],
                    [
                        ground[0] + shade * a.cos(),
                        ground[1] + shade * a.sin(),
                        0.015,
                    ],
                    [
                        ground[0] + shade * b.cos(),
                        ground[1] + shade * b.sin(),
                        0.015,
                    ],
                ],
                SHADOW,
                NONE,
            );
        }
    }
    if chicken.role == Role::Stuck {
        let puddle = r * 1.8;
        for k in 0..10 {
            let (a, b) = (k as f32 / 10.0 * TAU, (k + 1) as f32 / 10.0 * TAU);
            let wobble = |angle: f32| puddle * (0.85 + 0.15 * (angle * 3.0 + phase).sin());
            frame.facet(
                camera,
                [
                    [ground[0], ground[1], 0.02],
                    [
                        ground[0] + wobble(a) * a.cos(),
                        ground[1] + wobble(a) * 0.8 * a.sin(),
                        0.02,
                    ],
                    [
                        ground[0] + wobble(b) * b.cos(),
                        ground[1] + wobble(b) * 0.8 * b.sin(),
                        0.02,
                    ],
                ],
                if k % 2 == 0 { MUD } else { tint(MUD, 0.88) },
                NONE,
            );
        }
    }
    if zombie {
        for side in [-0.3, 0.3] {
            let hip = offset(body, forward, -0.1 * r, side * r, 0.5 * r);
            let knee = offset(body, forward, -0.15 * r, side * 1.2 * r, 1.25 * r);
            let foot = offset(body, forward, 0.05 * r, side * 1.3 * r, 1.6 * r);
            frame.line(camera, hip, knee, LEGS);
            frame.line(camera, knee, foot, LEGS);
        }
    } else if !perched && chicken.role != Role::Frozen {
        let stride = if walking {
            (time * (4.0 + 3.0 * chicken.speed) + phase).sin() * 0.35 * r
        } else {
            0.0
        };
        for (side, sign) in [(-0.3, 1.0), (0.3, -1.0)] {
            let hip = offset(body, forward, 0.0, side * r, -0.5 * r);
            let foot = [
                ground[0] + forward[0] * stride * sign - forward[1] * side * r,
                ground[1] + forward[1] * stride * sign + forward[0] * side * r,
                0.0,
            ];
            frame.line(camera, hip, foot, LEGS);
        }
    }
    let feathers = tint(plumage, 0.72);
    for side in [-0.3_f32, 0.0, 0.3] {
        frame.facet(
            camera,
            [
                offset(body, forward, -0.6 * r, (side - 0.25) * r, up * 0.2 * r),
                offset(body, forward, -0.6 * r, (side + 0.25) * r, up * 0.2 * r),
                offset(body, forward, -1.05 * r, side * 1.1 * r, up * 1.05 * r),
            ],
            feathers,
            pick,
        );
    }
    frame.sphere(camera, body, r, plumage, pick, selected);
    part(
        frame,
        camera,
        offset(body, forward, -0.5 * r, 0.0, up * 0.15 * r),
        0.72 * r,
        tint(plumage, 0.92),
        pick,
    );
    part(
        frame,
        camera,
        offset(body, forward, 0.45 * r, 0.0, up * 0.05 * r),
        0.75 * r,
        tint(plumage, 1.05),
        pick,
    );
    for side in [-1.0_f32, 1.0] {
        frame.facet(
            camera,
            [
                offset(body, forward, 0.3 * r, side * 0.9 * r, up * 0.3 * r),
                offset(body, forward, -0.75 * r, side * 0.8 * r, up * 0.4 * r),
                offset(body, forward, -0.35 * r, side * 0.82 * r, -up * 0.2 * r),
            ],
            tint(plumage, 0.8),
            pick,
        );
    }
    let head = offset(body, forward, head_forward * r, 0.0, up * head_up * r);
    let head_radius = 0.45 * r;
    if neck {
        part(
            frame,
            camera,
            offset(body, forward, 0.65 * r, 0.0, (head_up * 0.5 + 0.1) * r),
            0.38 * r,
            plumage,
            pick,
        );
    }
    part(frame, camera, head, head_radius, tint(plumage, 1.08), pick);
    let comb_radius = 0.18 * r;
    if comb_radius * camera.zoom >= 1.2 {
        for (along, rise) in [(0.35, 0.8), (-0.05, 0.95), (-0.45, 0.75)] {
            part(
                frame,
                camera,
                offset(
                    head,
                    forward,
                    along * head_radius,
                    0.0,
                    up * rise * head_radius,
                ),
                comb_radius * if along.abs() < 0.1 { 1.0 } else { 0.8 },
                COMB,
                NONE,
            );
        }
    } else {
        frame.facet(
            camera,
            [
                offset(
                    head,
                    forward,
                    0.5 * head_radius,
                    0.0,
                    up * 0.5 * head_radius,
                ),
                offset(
                    head,
                    forward,
                    -0.7 * head_radius,
                    0.0,
                    up * 0.5 * head_radius,
                ),
                offset(
                    head,
                    forward,
                    -0.1 * head_radius,
                    0.0,
                    up * 1.5 * head_radius,
                ),
            ],
            COMB,
            NONE,
        );
    }
    let tip = offset(
        head,
        forward,
        1.8 * head_radius,
        0.0,
        -up * 0.15 * head_radius,
    );
    frame.facet(
        camera,
        [
            offset(
                head,
                forward,
                0.8 * head_radius,
                0.0,
                up * 0.25 * head_radius,
            ),
            offset(
                head,
                forward,
                0.8 * head_radius,
                0.0,
                -up * 0.35 * head_radius,
            ),
            tip,
        ],
        BEAK,
        NONE,
    );
    frame.facet(
        camera,
        [
            offset(head, forward, 0.8 * head_radius, -0.3 * head_radius, 0.0),
            offset(head, forward, 0.8 * head_radius, 0.3 * head_radius, 0.0),
            tip,
        ],
        tint(BEAK, 0.85),
        NONE,
    );
    part(
        frame,
        camera,
        offset(
            head,
            forward,
            0.75 * head_radius,
            0.0,
            -up * 0.75 * head_radius,
        ),
        0.25 * head_radius,
        COMB,
        NONE,
    );
    for side in [-0.6, 0.6] {
        part(
            frame,
            camera,
            offset(
                head,
                forward,
                0.55 * head_radius,
                side * head_radius,
                up * 0.25 * head_radius,
            ),
            0.16 * head_radius,
            PUPIL,
            NONE,
        );
    }
    body
}

/// Chicks for threads: in a file behind a moving hen along its trail, tucked beside a still one.
fn draw_chicks(
    frame: &mut Frame,
    camera: &Camera,
    chicken: &Chicken,
    ground: [f32; 2],
    body: Point,
    r: f32,
    time: f32,
) {
    if chicken.chicks == 0 {
        return;
    }
    let size = 0.2;
    let moving = chicken.role == Role::Foraging || chicken.walking();
    let forward = [chicken.theta.cos(), chicken.theta.sin()];
    let mut cursor = ground;
    let mut points = chicken.trail.iter();
    let mut travelled = 0.0;
    for k in 0..chicken.chicks {
        let p = if moving {
            let wanted = r + 0.45 + k as f32 * 0.6;
            let mut spot = None;
            while travelled < wanted {
                let Some(&next) = points.next() else {
                    break;
                };
                let step = (next[0] - cursor[0]).hypot(next[1] - cursor[1]);
                if travelled + step >= wanted && step > 1e-6 {
                    let t = (wanted - travelled) / step;
                    let at = [
                        cursor[0] + (next[0] - cursor[0]) * t,
                        cursor[1] + (next[1] - cursor[1]) * t,
                    ];
                    travelled = wanted;
                    cursor = at;
                    spot = Some(at);
                    break;
                }
                travelled += step;
                cursor = next;
            }
            let at = spot.unwrap_or_else(|| {
                // A short trail: the rest of the brood bunches up at its end.
                let angle = k as f32 * 2.4;
                [
                    cursor[0] + 0.15 * angle.cos(),
                    cursor[1] + 0.15 * angle.sin(),
                ]
            });
            let hop = 0.05 * (time * 9.0 + k as f32).sin().abs();
            [at[0], at[1], size + hop]
        } else {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let along = (k / 2) as f32;
            let base = [body[0], body[1], (body[2] - r * 0.45).max(size)];
            offset(
                base,
                forward,
                0.35 * r - along * 0.25,
                side * (0.75 * r + 0.15 + along * 0.12),
                0.0,
            )
        };
        let look = if moving { forward } else { [0.0, 1.0] };
        part(frame, camera, p, size, CHICK, NONE);
        part(
            frame,
            camera,
            offset(p, look, 0.15, 0.0, 0.14),
            0.11,
            tint(CHICK, 1.05),
            NONE,
        );
    }
}

/// The fox: in through the hedge to its victim, then away with a grey ghost in its mouth.
fn draw_fox(frame: &mut Frame, camera: &Camera, fox: &Fox, time: f32) {
    let u = (time - fox.start) / FOX_SECONDS;
    if !(0.0..1.0).contains(&u) {
        return;
    }
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let lerp =
        |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let (at, heading, carrying) = if u < 0.42 {
        (
            lerp(fox.entry, fox.prey, ease(u / 0.42)),
            normalize_or_zero([fox.prey[0] - fox.entry[0], fox.prey[1] - fox.entry[1]]),
            false,
        )
    } else if u < 0.55 {
        (
            fox.prey,
            normalize_or_zero([fox.prey[0] - fox.entry[0], fox.prey[1] - fox.entry[1]]),
            true,
        )
    } else {
        (
            lerp(fox.prey, fox.exit, ease((u - 0.55) / 0.45)),
            normalize_or_zero([fox.exit[0] - fox.prey[0], fox.exit[1] - fox.prey[1]]),
            true,
        )
    };
    let forward = if heading == [0.0, 0.0] {
        [1.0, 0.0]
    } else {
        heading
    };
    let running = !(0.42..0.55).contains(&u);
    let size = 1.4;
    let gallop = if running { (time * 14.0).sin() } else { 0.0 };
    let body = [at[0], at[1], size * 0.95 + 0.08 * gallop.abs()];
    for (along, side) in [(0.55, -0.25), (0.55, 0.25), (-0.55, -0.25), (-0.55, 0.25)] {
        let swing = gallop * 0.3 * if along > 0.0 { 1.0 } else { -1.0 };
        let hip = offset(body, forward, along * size, side * size, -0.3 * size);
        let foot = offset(
            [at[0], at[1], 0.0],
            forward,
            (along + swing) * size,
            side * size,
            0.0,
        );
        frame.line(camera, hip, foot, FOX_DARK);
    }
    let mut tail = offset(body, forward, -0.95 * size, 0.0, 0.05 * size);
    for (k, girth) in [0.24_f32, 0.34, 0.4, 0.36, 0.26].into_iter().enumerate() {
        let color = if k == 4 { FOX_WHITE } else { FOX };
        frame.world_sphere(camera, tail, girth * size / SCALE, color);
        let sway = 0.06 * gallop * size * k as f32;
        tail = offset(
            tail,
            forward,
            -0.3 * size,
            sway,
            (0.1 + 0.03 * k as f32) * size,
        );
    }
    for (along, girth) in [(-0.5_f32, 0.48_f32), (0.0, 0.52), (0.5, 0.55)] {
        frame.world_sphere(
            camera,
            offset(body, forward, along * size, 0.0, 0.0),
            girth * size / SCALE,
            FOX,
        );
    }
    frame.world_sphere(
        camera,
        offset(body, forward, 0.8 * size, 0.0, -0.12 * size),
        0.32 * size / SCALE,
        FOX_WHITE,
    );
    let head = offset(body, forward, 1.2 * size, 0.0, 0.45 * size);
    frame.world_sphere(camera, head, 0.4 * size / SCALE, FOX);
    let snout = offset(head, forward, 0.85 * size, 0.0, -0.15 * size);
    for side in [-0.22, 0.22] {
        frame.facet(
            camera,
            [
                offset(head, forward, 0.2 * size, side * size, 0.08 * size),
                offset(head, forward, 0.2 * size, 0.0, -0.28 * size),
                snout,
            ],
            tint(FOX, 0.9),
            NONE,
        );
        frame.facet(
            camera,
            [
                offset(head, forward, 0.2 * size, side * size, 0.08 * size),
                offset(head, forward, 0.2 * size, 0.0, 0.2 * size),
                snout,
            ],
            FOX,
            NONE,
        );
        frame.facet(
            camera,
            [
                offset(head, forward, -0.12 * size, side * 0.8 * size, 0.25 * size),
                offset(head, forward, 0.12 * size, side * 1.5 * size, 0.25 * size),
                offset(head, forward, 0.0, side * 1.3 * size, 0.85 * size),
            ],
            FOX_DARK,
            NONE,
        );
    }
    frame.world_sphere(camera, snout, 0.08 * size / SCALE, FOX_DARK);
    if carrying && let Some(radius) = fox.ghost {
        let mouth = offset(head, forward, 0.7 * size + radius * 0.8, 0.0, -0.35 * size);
        frame.world_sphere(camera, mouth, radius / SCALE, GHOST);
    }
}
