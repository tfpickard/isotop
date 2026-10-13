//! Hive: memory as honeycomb frames, threads as bees. Each cgroup (each app on macOS) is a wooden
//! hive on a meadow plot. Each of its three largest processes is a frame lifted out behind it, a
//! fixed 64 by 32 honeycomb whose cells count that process's memory by kind: honey is `Pss_Anon`,
//! pollen `Pss_File`, royal jelly `Pss_Shmem`, capped honey `SwapPss`, and brood is the growth of
//! the resident set since the sample before. One bee crawls the comb for each thread, at a speed
//! set by that thread's CPU.
//!
//! A comb is a field (`render::Field`) on a vertical four-corner quad, repainted only when its
//! cell kinds change. Where the sampler did not split a process's memory, the frame says so:
//! pale nectar from the resident set for a process the rollup has not reached, Pss in one kind for
//! a kernel before 5.3, a sealed frame with no cells for a rollup that could not be read.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};
use std::sync::OnceLock;

use crate::cells::{cell_name, short};
use crate::geometry::cuboid;
use crate::history::{History, Sample};
use crate::layout::Discs;
use crate::model::{Identity, Kind, Measured, Process, Snapshot};
use crate::palette::{CONTAINER, KERNEL, SESSION, SYSTEM, kind_color, tint};
use crate::platform::{self, Source};
use crate::render::{
    Camera, Color, Field, Frame, Item, NONE, Placement, Point, Relief, SCALE, Shading, Stage,
    Title, TourTarget,
};
use crate::simulation::{FixedStep, Rng, hash_identity};
use crate::status::{missing_note, sources_note};
use crate::threads::Thread;
use crate::views::ViewModule;

/// Separates this view's draws of the identity hash from every other view's.
const SALT: u64 = 0x6869_7665_0000_0046;

// ---------------------------------------------------------------------------------------------
// The comb

/// Cells across a comb and down it. Odd rows are shifted half a cell to the right, so a comb is
/// 64 by 32 cells and 64.5 pitches wide.
const COLUMNS: usize = 64;
const ROWS: usize = 32;
const CELLS: usize = COLUMNS * ROWS;
const SQRT_3: f32 = 1.732_050_8;
/// The width and height of a comb in cell pitches (the distance between neighbours in a row).
const COMB_WIDTH: f32 = COLUMNS as f32 + 0.5;
const COMB_HEIGHT: f32 = (ROWS - 1) as f32 * SQRT_3 / 2.0 + 2.0 / SQRT_3;
/// The field's nodes: 8 texels to a cell pitch, across and down, with a node on each edge.
const TEXELS: f32 = 8.0;
const NODES_X: usize = (TEXELS * COMB_WIDTH) as usize + 1;
const NODES_Y: usize = (TEXELS * COMB_HEIGHT) as usize + 1;
/// A cell's circumradius in pitches, and the fraction of its width that its wax wall leaves.
const RADIUS: f32 = 1.0 / SQRT_3;
const WALL: f32 = 0.86;
/// What the comb is made of where there is no cell: the wax foundation.
const WAX: Color = [150, 104, 48];
/// The wax cap that seals the frame of a process whose rollup could not be read.
const SEAL: Color = [214, 184, 120];

/// The pollen colours, which a cell keeps for as long as it is pollen.
const POLLEN: [Color; 6] = [
    [226, 104, 30],
    [204, 72, 34],
    [236, 128, 40],
    [196, 150, 40],
    [214, 88, 60],
    [180, 160, 60],
];
/// The larva's size in four stages, as the share of a fully grown larva.
const LARVA: [f32; 4] = [0.4, 0.6, 0.8, 1.0];
const LARVA_TURNS: usize = 3;

/// A cell's look: the palette index a comb paints it with.
const EMPTY: u8 = 0;
const HONEY: u8 = 1;
const CAPPED: u8 = 2;
const JELLY: u8 = 3;
const NECTAR: u8 = 4;
const POLLEN_FIRST: u8 = 5;
const BROOD_FIRST: u8 = POLLEN_FIRST + POLLEN.len() as u8;
const LOOKS: usize = BROOD_FIRST as usize + LARVA.len() * LARVA_TURNS;

/// Local offsets are kept in 1/64 pitch, and the colour tables cover +-40 of them.
const LOCAL_REACH: i32 = 40;
const LOCAL_SIDE: usize = 2 * LOCAL_REACH as usize + 1;
const NO_CELL: u16 = u16::MAX;

/// The distance from a cell's centre to a point, measured so that it is 1 on the cell's edge: the
/// hexagon's own norm, with the pitch the width of the cell across flats.
fn hex(u: f32, v: f32) -> f32 {
    2.0 * u.abs().max(0.5 * u.abs() + 0.5 * SQRT_3 * v.abs())
}

/// Whether (u, v) lies in the ellipse centred on (cu, cv) with radii (ru, rv).
fn ellipse(u: f32, v: f32, [cu, cv]: [f32; 2], [ru, rv]: [f32; 2]) -> bool {
    let (du, dv) = ((u - cu) / ru, (v - cv) / rv);
    du * du + dv * dv <= 1.0
}

/// The colour of a cell of palette `look` at (u, v) pitches from its centre, from the prototype's
/// `draw_cell`: wall polygons inside one another, offset towards the light, with a gloss spot.
fn paint_cell(look: u8, u: f32, v: f32) -> Color {
    let r = RADIUS;
    let inside = |ox: f32, oy: f32, scale: f32| hex(u - ox * r, v - oy * r) <= WALL * scale;
    let in_wall = hex(u, v) <= WALL;
    if !in_wall {
        return WAX;
    }
    let dark = [70, 46, 24];
    match look {
        EMPTY => {
            if inside(0.05, 0.08, 0.62) {
                [40, 26, 14]
            } else {
                dark
            }
        }
        HONEY | NECTAR => {
            let (wall, body, core, gloss) = if look == HONEY {
                (
                    [168, 92, 10],
                    [246, 178, 36],
                    [255, 214, 92],
                    [255, 240, 190],
                )
            } else {
                (
                    [126, 140, 86],
                    [196, 218, 150],
                    [222, 238, 180],
                    [248, 252, 226],
                )
            };
            if ellipse(u, v, [-0.30 * r, -0.38 * r], [0.16 * r, 0.12 * r]) {
                gloss
            } else if inside(0.1, 0.14, 0.36) {
                core
            } else if inside(0.05, 0.07, 0.74) {
                body
            } else {
                wall
            }
        }
        CAPPED => {
            if inside(-0.1, -0.12, 0.42) {
                [240, 218, 150]
            } else if inside(-0.04, -0.05, 0.8) {
                [222, 192, 112]
            } else {
                [176, 140, 70]
            }
        }
        JELLY => {
            let wall = WALL * r;
            if ellipse(u, v, [-0.22 * r, -0.27 * r], [0.14 * r, 0.11 * r]) {
                [255, 255, 244]
            } else if ellipse(u, v, [0.0, 0.04 * wall], [0.72 * wall, 0.70 * wall]) {
                [238, 234, 220]
            } else {
                dark
            }
        }
        look if look < BROOD_FIRST => {
            let color = POLLEN[usize::from(look - POLLEN_FIRST)];
            if ellipse(u, v, [-0.1 * r, 0.0], [0.2 * r, 0.2 * r]) {
                tint(color, 1.08)
            } else if inside(0.0, 0.02, 0.8) {
                tint(color, 0.85)
            } else {
                dark
            }
        }
        look => {
            let index = usize::from(look - BROOD_FIRST);
            let (stage, turn) = (index / LARVA_TURNS, index % LARVA_TURNS);
            let size = WALL * r * 0.66 * (0.45 + 0.55 * LARVA[stage]);
            if u.hypot(v) > size {
                return dark;
            }
            // A pearly C: a disc with a 60 degree wedge missing, a hollow in the middle and a
            // gloss spot.
            let start = turn as f32 * TAU / LARVA_TURNS as f32;
            let angle = (v.atan2(u) - start).rem_euclid(TAU);
            if ellipse(u, v, [-0.5 * size, -0.6 * size], [0.3 * size, 0.25 * size]) {
                [255, 255, 252]
            } else if u.hypot(v) <= size * 0.45 {
                [196, 188, 176]
            } else if angle <= 300.0_f32.to_radians() {
                [236, 232, 222]
            } else {
                dark
            }
        }
    }
}

/// Where a texel lies in a comb: the cell it belongs to and its offset from the cell's centre,
/// as an index into a colour table of `LOCAL_SIDE` squared entries.
#[derive(Clone, Copy)]
struct Texel {
    cell: u16,
    local: u16,
}

/// What every comb shares, built once on first use: the cell under each texel, the order each
/// kind fills the cells in, and the colour of every look at every offset.
struct Lattice {
    texels: Vec<Texel>,
    brood: Vec<u16>,
    pollen: Vec<u16>,
    jelly: Vec<u16>,
    honey: Vec<u16>,
    tables: Vec<Vec<Color>>,
}

/// The centre of cell `index` in pitches from the comb's top left corner.
fn cell_center(index: usize) -> [f32; 2] {
    let (row, column) = (index / COLUMNS, index % COLUMNS);
    [
        column as f32 + 0.5 + 0.5 * (row % 2) as f32,
        row as f32 * SQRT_3 / 2.0 + 1.0 / SQRT_3,
    ]
}

impl Lattice {
    fn get() -> &'static Lattice {
        static LATTICE: OnceLock<Lattice> = OnceLock::new();
        LATTICE.get_or_init(Lattice::build)
    }

    fn build() -> Lattice {
        let mut texels = Vec::with_capacity(NODES_X * NODES_Y);
        for j in 0..NODES_Y {
            let y = j as f32 * COMB_HEIGHT / (NODES_Y - 1) as f32;
            let guess = ((y - 1.0 / SQRT_3) / (SQRT_3 / 2.0)).round() as i32;
            for i in 0..NODES_X {
                let x = i as f32 * COMB_WIDTH / (NODES_X - 1) as f32;
                let mut best: Option<(f32, usize, [f32; 2])> = None;
                for row in (guess - 1).max(0)..=(guess + 1).min(ROWS as i32 - 1) {
                    let first = (x - 0.5 - 0.5 * (row % 2) as f32).floor() as i32;
                    for column in first.max(0)..=(first + 1).min(COLUMNS as i32 - 1) {
                        let index = row as usize * COLUMNS + column as usize;
                        let [cx, cy] = cell_center(index);
                        let offset = [x - cx, y - cy];
                        let distance = hex(offset[0], offset[1]);
                        if best.is_none_or(|(least, _, _)| distance < least) {
                            best = Some((distance, index, offset));
                        }
                    }
                }
                texels.push(match best {
                    Some((distance, index, [u, v])) if distance <= 1.0 => {
                        let quantize = |offset: f32| {
                            ((offset * 64.0).round() as i32).clamp(-LOCAL_REACH, LOCAL_REACH)
                                + LOCAL_REACH
                        };
                        Texel {
                            cell: index as u16,
                            local: (quantize(v) as usize * LOCAL_SIDE + quantize(u) as usize)
                                as u16,
                        }
                    }
                    _ => Texel {
                        cell: NO_CELL,
                        local: 0,
                    },
                });
            }
        }
        let centers: Vec<[f32; 2]> = (0..CELLS).map(cell_center).collect();
        let middle = [COMB_WIDTH / 2.0, COMB_HEIGHT * 0.58];
        let order = |metric: &dyn Fn(f32, f32) -> f32| {
            let keys: Vec<f32> = centers
                .iter()
                .map(|&[x, y]| metric((x - middle[0]) / 1.55, y - middle[1]))
                .collect();
            let mut cells: Vec<u16> = (0..CELLS as u16).collect();
            cells.sort_by(|&a, &b| {
                keys[usize::from(a)]
                    .total_cmp(&keys[usize::from(b)])
                    .then(a.cmp(&b))
            });
            cells
        };
        let tables = (0..LOOKS as u8)
            .map(|look| {
                (0..LOCAL_SIDE * LOCAL_SIDE)
                    .map(|local| {
                        let u = ((local % LOCAL_SIDE) as i32 - LOCAL_REACH) as f32 / 64.0;
                        let v = ((local / LOCAL_SIDE) as i32 - LOCAL_REACH) as f32 / 64.0;
                        paint_cell(look, u, v)
                    })
                    .collect()
            })
            .collect();
        Lattice {
            texels,
            // The brood is the nest, round the middle of the comb.
            brood: order(&|dx, dy| dx.hypot(dy)),
            // Pollen packs round the brood, and royal jelly lies below it.
            pollen: order(&|dx, dy| dx.hypot(dy)),
            jelly: order(&|dx, dy| dx.hypot(dy) - 0.9 * dy),
            // Honey arches over the nest: cells above count as near, cells below as far.
            honey: order(&|dx, dy| {
                let reach = if dy < 0.0 { dy * 0.42 } else { dy * 2.6 };
                (dx * dx * 0.8 + reach * reach).sqrt()
            }),
            tables,
        }
    }

    /// The look of every cell for `counts`: brood in the middle, pollen round it, royal jelly
    /// below, then honey (capped honey and pale nectar fill the same arch after it). Each kind
    /// takes the free cells nearest its centre in its own order, ties by cell index.
    fn arrange(&self, counts: &Counts, salt: u64) -> Vec<u8> {
        let mut looks = vec![EMPTY; CELLS];
        let mut free = vec![true; CELLS];
        let mut take = |order: &[u16], count: usize, look: &mut dyn FnMut(usize, usize) -> u8| {
            let mut taken = 0;
            for &cell in order {
                if taken == count {
                    break;
                }
                let cell = usize::from(cell);
                if free[cell] {
                    free[cell] = false;
                    looks[cell] = look(taken, cell);
                    taken += 1;
                }
            }
        };
        let brood = counts.brood;
        take(&self.brood, brood, &mut |rank, cell| {
            // The innermost quarter of the brood is the largest larva, the outermost the smallest.
            let stage = LARVA.len() - 1 - (LARVA.len() * rank / brood.max(1)).min(LARVA.len() - 1);
            BROOD_FIRST + (stage * LARVA_TURNS + cell % LARVA_TURNS) as u8
        });
        take(&self.pollen, counts.pollen, &mut |_, cell| {
            POLLEN_FIRST + (Rng::new(cell as u64 ^ salt).next_u64() % POLLEN.len() as u64) as u8
        });
        take(&self.jelly, counts.jelly, &mut |_, _| JELLY);
        take(&self.honey, counts.honey, &mut |_, _| HONEY);
        take(&self.honey, counts.capped, &mut |_, _| CAPPED);
        take(&self.honey, counts.nectar, &mut |_, _| NECTAR);
        looks
    }
}

// ---------------------------------------------------------------------------------------------
// What a frame shows

/// What the sampler could say about a process's memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Pss split into anonymous, file and shared memory, with swap.
    Split,
    /// Pss without the split (a kernel before 5.3), with swap.
    Pss,
    /// Only the resident set (or the footprint on macOS): no rollup, or one not reached yet.
    Unsplit,
    /// A rollup that could not be read: a wax cap and no cells.
    Sealed,
}

/// A frame's bytes by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Parts {
    honey: u64,
    pollen: u64,
    jelly: u64,
    capped: u64,
    nectar: u64,
}

/// A process's memory as its frame draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Measure {
    mode: Mode,
    parts: Parts,
    /// The resident set's growth since the sample before, in bytes: the brood.
    growth: u64,
}

impl Measure {
    /// Reads `process` as the sample says. A source the platform lacks comes first, as the rollup
    /// of each process then says nothing; after it, `Pending` and `Unreadable` differ.
    fn of(process: &Process, snapshot: &Snapshot, before: Option<u64>) -> Measure {
        let growth = before.map_or(0, |before| process.memory.saturating_sub(before));
        let unsplit = Measure {
            mode: Mode::Unsplit,
            parts: Parts {
                nectar: process.memory,
                ..Parts::default()
            },
            growth,
        };
        if snapshot.missing.contains(Source::MemoryDetail) {
            return unsplit;
        }
        match &process.memory_detail {
            Measured::Known(rollup) => match (rollup.pss_anon, rollup.pss_file, rollup.pss_shmem) {
                (Some(honey), Some(pollen), Some(jelly))
                    if !snapshot.missing.contains(Source::MemorySplit) =>
                {
                    Measure {
                        mode: Mode::Split,
                        parts: Parts {
                            honey,
                            pollen,
                            jelly,
                            capped: rollup.swap_pss,
                            nectar: 0,
                        },
                        growth,
                    }
                }
                _ => Measure {
                    mode: Mode::Pss,
                    parts: Parts {
                        capped: rollup.swap_pss,
                        nectar: rollup.pss,
                        ..Parts::default()
                    },
                    growth,
                },
            },
            Measured::Unreadable => Measure {
                mode: Mode::Sealed,
                parts: Parts::default(),
                growth: 0,
            },
            Measured::Pending => unsplit,
        }
    }

    /// The bytes the frame counts: Pss and swap, or the resident set.
    fn bytes(&self) -> u64 {
        let parts = &self.parts;
        parts.honey + parts.pollen + parts.jelly + parts.capped + parts.nectar
    }
}

/// A frame's cells by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    brood: usize,
    honey: usize,
    pollen: usize,
    jelly: usize,
    capped: usize,
    nectar: usize,
}

/// The cells of `bytes` at `unit` bytes a cell, rounded to the nearest, a half up.
fn cells(bytes: u64, unit: u64) -> usize {
    ((bytes + unit / 2) / unit) as usize
}

impl Counts {
    /// Each component rounded on its own, so five of them add at most three cells to the exact
    /// total. The brood is carved out of the honey of a split frame and out of the pale nectar of
    /// any other, and never exceeds what it is carved from.
    fn of(measure: &Measure, unit: u64) -> Counts {
        if measure.mode == Mode::Sealed {
            return Counts::default();
        }
        let parts = &measure.parts;
        let mut counts = Counts {
            brood: 0,
            honey: cells(parts.honey, unit),
            pollen: cells(parts.pollen, unit),
            jelly: cells(parts.jelly, unit),
            capped: cells(parts.capped, unit),
            nectar: cells(parts.nectar, unit),
        };
        let source = if measure.mode == Mode::Split {
            &mut counts.honey
        } else {
            &mut counts.nectar
        };
        counts.brood = cells(measure.growth, unit).min(*source);
        *source -= counts.brood;
        counts
    }
}

/// The smallest unit, 64 KiB. Units are powers of two from here.
const UNIT_FLOOR: u64 = 64 * 1024;
/// The most cells the largest frame may have, and the fewest it keeps before the unit shrinks.
const BAND_TOP: u64 = 2000;
const BAND_BOTTOM: u64 = 700;

/// The unit for `largest`, the biggest frame drawn, when the scene is at `current`: the unit
/// holds while 700 units <= largest <= 2,000 units (or while it is the smallest unit and the
/// frame has fewer cells than that), and otherwise becomes the smallest power of two that keeps the
/// largest frame within 2,000 cells. With no frame the unit holds.
fn next_unit(current: u64, largest: Option<u64>) -> u64 {
    let Some(largest) = largest else {
        return current;
    };
    let fits = largest <= BAND_TOP * current;
    if fits && (largest >= BAND_BOTTOM * current || current == UNIT_FLOOR) {
        return current;
    }
    let mut unit = UNIT_FLOOR;
    while largest > BAND_TOP * unit {
        unit *= 2;
    }
    unit
}

/// A unit as the legend writes it: a whole number of KiB, MiB or GiB.
fn unit_text(unit: u64) -> String {
    match unit {
        unit if unit >= 1 << 30 => format!("{} GiB", unit >> 30),
        unit if unit >= 1 << 20 => format!("{} MiB", unit >> 20),
        unit => format!("{} KiB", unit >> 10),
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
}

// ---------------------------------------------------------------------------------------------
// Bees

/// Bees a frame draws at most.
const MAX_BEES: usize = 200;
/// A thread below this share of one core (percent) is idle.
const IDLE_BELOW: f32 = 3.0;
/// The queen walks no faster than this, in cells a second.
const QUEEN_SPEED: f32 = 0.6;
/// How much larger the queen is drawn.
const QUEEN_SIZE: f32 = 1.37;
/// A bee's length in cell pitches, and the speed at which an idle bee walks back to its seat.
const BEE_LENGTH: f32 = 2.8;
const HOMING_SPEED: f32 = 2.0;
/// Bees keep this far from the edge of the comb, in pitches.
const MARGIN: f32 = 0.7;
/// The bee simulation steps at 20 Hz.
const STEP: f32 = 0.05;

/// A bee's speed in cells a second from its thread's CPU in percent of one core: 0.25 plus
/// nine times the square root of the share. An idle thread has none.
fn bee_speed(cpu: f32) -> f32 {
    if cpu < IDLE_BELOW {
        return 0.0;
    }
    0.25 + 9.0 * (cpu / 100.0).clamp(0.0, 1.0).sqrt()
}

/// What a frame wants of one bee: who it is, and what the sampler says it does.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Wish {
    key: u64,
    /// Percent of one core; None for a bee with no per-thread reading.
    cpu: Option<f32>,
    queen: bool,
}

/// The bee a listed thread keeps for its whole life: a hash of its tid and start, which the
/// system does not reuse together.
fn thread_key(thread: &Thread) -> u64 {
    Rng::new(thread.start ^ (u64::from(thread.tid) << 32)).next_u64()
}

/// The bees of `process`: at most 200, the listed threads first (the leader leads the list), then
/// bees for the threads the list does not hold, which have no reading. When the leader is not
/// listed, the first of those is the queen, who stands with the idle bees.
fn wishes(process: &Process) -> Vec<Wish> {
    let wanted = (process.threads as usize).min(MAX_BEES);
    let mut wishes = Vec::with_capacity(wanted);
    if let Measured::Known(list) = &process.thread_list {
        wishes.extend(list.threads.iter().take(wanted).map(|thread| Wish {
            key: thread_key(thread),
            cpu: thread.cpu,
            queen: thread.leader,
        }));
    }
    let listed = wishes.len();
    wishes.extend((listed..wanted).map(|index| Wish {
        key: index as u64,
        cpu: None,
        queen: false,
    }));
    if listed < wanted && !wishes.iter().any(|wish| wish.queen) {
        wishes[listed].queen = true;
    }
    wishes
}

/// One bee on a comb, in cell pitches from the comb's top left corner.
#[derive(Clone, Debug)]
struct Bee {
    key: u64,
    queen: bool,
    cpu: Option<f32>,
    position: [f32; 2],
    heading: f32,
    before: [f32; 2],
    before_heading: f32,
    turn: f32,
    speed: f32,
    /// Where it stands when it is idle: beside the nest, and the queen in it.
    home: [f32; 2],
    random: Rng,
}

impl Bee {
    fn new(process: Identity, wish: &Wish) -> Bee {
        let mut random = Rng::for_identity(process, SALT ^ wish.key);
        let (nest_x, nest_y) = (COMB_WIDTH / 2.0, COMB_HEIGHT * 0.58);
        let seat = |random: &mut Rng| {
            [
                (nest_x - 13.8 + 3.6 * random.normal()).clamp(1.0, COMB_WIDTH - 1.0),
                (nest_y - 4.7 + 2.2 * random.normal()).clamp(1.0, COMB_HEIGHT - 1.0),
            ]
        };
        let seat = seat(&mut random);
        let home = if wish.queen {
            [nest_x + 0.9, nest_y + 0.4]
        } else {
            seat
        };
        let wander = [
            1.0 + random.unit() * (COMB_WIDTH - 2.0),
            1.0 + random.unit() * (COMB_HEIGHT - 2.0),
        ];
        let heading = random.unit() * TAU;
        let busy = wish.cpu.is_some_and(|cpu| cpu >= IDLE_BELOW);
        let position = if busy { wander } else { home };
        Bee {
            key: wish.key,
            queen: wish.queen,
            cpu: wish.cpu,
            position,
            heading,
            before: position,
            before_heading: heading,
            turn: 0.0,
            speed: 0.0,
            home,
            random,
        }
    }

    /// The speed its thread's CPU asks for, in cells a second; 0 for an idle bee and for one with
    /// no reading. The queen is slow.
    fn wanted(&self) -> f32 {
        let speed = self.cpu.map_or(0.0, bee_speed);
        if self.queen {
            speed.min(QUEEN_SPEED)
        } else {
            speed
        }
    }

    /// One step of `STEP` seconds. A busy bee walks a wandering path at its speed and turns back
    /// at the edge of the comb; an idle one walks home and stands still there. The random stream
    /// advances once a step whatever the bee does.
    fn step(&mut self) {
        self.before = self.position;
        self.before_heading = self.heading;
        let noise = self.random.normal();
        let wanted = self.wanted();
        if wanted > 0.0 {
            self.speed += (wanted - self.speed) * 0.25;
            self.turn += noise * 1.4 * STEP * 4.0;
            self.turn *= 0.92;
            self.heading += self.turn * STEP * 2.2;
            self.position[0] += self.heading.cos() * self.speed * STEP;
            self.position[1] += self.heading.sin() * self.speed * STEP;
            let (low, high) = (
                [MARGIN, MARGIN],
                [COMB_WIDTH - MARGIN, COMB_HEIGHT - MARGIN],
            );
            if self.position[0] < low[0] || self.position[0] > high[0] {
                self.heading = PI - self.heading;
            }
            if self.position[1] < low[1] || self.position[1] > high[1] {
                self.heading = -self.heading;
            }
            for axis in 0..2 {
                self.position[axis] = self.position[axis].clamp(low[axis], high[axis]);
            }
            return;
        }
        let to = [
            self.home[0] - self.position[0],
            self.home[1] - self.position[1],
        ];
        let distance = to[0].hypot(to[1]);
        if distance <= 0.05 {
            self.position = self.home;
            self.speed = 0.0;
        } else {
            let walk = (HOMING_SPEED * STEP).min(distance);
            self.heading = to[1].atan2(to[0]);
            self.position[0] += to[0] / distance * walk;
            self.position[1] += to[1] / distance * walk;
            self.speed = HOMING_SPEED;
        }
    }

    /// Where the bee is `alpha` of the way from its last step to the next, and which way it faces.
    fn pose(&self, alpha: f32) -> ([f32; 2], f32) {
        let position = [
            self.before[0] + (self.position[0] - self.before[0]) * alpha,
            self.before[1] + (self.position[1] - self.before[1]) * alpha,
        ];
        let turned = (self.heading - self.before_heading + PI).rem_euclid(TAU) - PI;
        (position, self.before_heading + turned * alpha)
    }
}

/// The bees of one frame, in the order of their keys.
#[derive(Debug, Default)]
struct Swarm {
    bees: BTreeMap<u64, Bee>,
}

impl Swarm {
    /// Brings the swarm to `wishes`: a bee for each wish, keeping those that stay, with the CPU and
    /// the queen's mark the sample now says.
    fn update(&mut self, process: Identity, wishes: &[Wish]) {
        let keys: HashSet<u64> = wishes.iter().map(|wish| wish.key).collect();
        self.bees.retain(|key, _| keys.contains(key));
        for wish in wishes {
            let bee = self
                .bees
                .entry(wish.key)
                .or_insert_with(|| Bee::new(process, wish));
            bee.cpu = wish.cpu;
            bee.queen = wish.queen;
        }
    }

    fn step(&mut self) {
        for bee in self.bees.values_mut() {
            bee.step();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The world

/// Hives shown at most, frames to a hive, and the ratio by which a hive must outweigh a shown one
/// to replace it (as the quotient of two integers, 5/4).
const MAX_HIVES: usize = 8;
const FRAMES: usize = 3;
const REPLACE: (u64, u64) = (5, 4);
/// The radius of the disc a hive and its frames take on the ground.
const HIVE_RADIUS: f32 = 1.75;

/// The prototype draws in a 2:1 projection. One of its screen units across is `ACROSS` of a world
/// unit along the frame's across axis, and one up is `UP` of a world unit of height, which keeps
/// every proportion on screen at the default camera. Its ground units are world units.
const ACROSS: f32 = FRAC_1_SQRT_2;
const UP: f32 = 0.866;
/// Where the lid of a hive tops out, in the prototype's units.
const HIVE_TOP: f32 = 1.72;

/// A frame measures 4.4 by 2.04 of those units; its bars and lugs are the prototype's.
const FRAME_WIDTH: f32 = 4.4 * ACROSS;
const FRAME_HEIGHT: f32 = 2.04 * UP;
const SIDE_BAR: f32 = 0.085 * ACROSS;
const TOP_BAR: f32 = 0.13 * UP;
const BOTTOM_BAR: f32 = 0.07 * UP;
const LUG: f32 = 0.16 * ACROSS;
const INNER_WIDTH: f32 = FRAME_WIDTH - 2.0 * SIDE_BAR;
const INNER_HEIGHT: f32 = FRAME_HEIGHT - TOP_BAR - BOTTOM_BAR;
/// World units of one cell pitch across the comb and up it.
const PITCH_ACROSS: f32 = INNER_WIDTH / COMB_WIDTH;
const PITCH_UP: f32 = INNER_HEIGHT / COMB_HEIGHT;
/// How far a bar stands in front of the comb and behind it.
const BAR_FRONT: f32 = 0.035;
const BAR_BACK: f32 = 0.04;

/// The world axes of a frame: along its top bar, and out of its face towards the default viewer.
const ACROSS_AXIS: [f32; 2] = [FRAC_1_SQRT_2, -FRAC_1_SQRT_2];
const FRONT_AXIS: [f32; 2] = [FRAC_1_SQRT_2, FRAC_1_SQRT_2];

/// The frame of slot `slot` stands this far behind its hive and to the side of it (the
/// prototype's `frame_rect`), and rises to this height.
fn frame_base(hive: [f32; 2], slot: usize) -> Point {
    let j = slot as f32;
    let (back, side) = (0.5 + 0.4 * j, 0.1 * j);
    [
        hive[0] - back + side,
        hive[1] - back - side,
        (HIVE_TOP + 0.5 + 0.78 * j) * UP,
    ]
}

/// The top of the third frame, which the camera fit must leave room for.
const FRAME_TOP: f32 = (HIVE_TOP + 0.5 + 0.78 * (FRAMES - 1) as f32) * UP + FRAME_HEIGHT;

/// The world point `across` from a frame's centre line, `up` from its bottom edge and `front`
/// out of the comb's plane.
fn at(base: Point, across: f32, up: f32, front: f32) -> Point {
    [
        base[0] + ACROSS_AXIS[0] * across + FRONT_AXIS[0] * front,
        base[1] + ACROSS_AXIS[1] * across + FRONT_AXIS[1] * front,
        base[2] + up,
    ]
}

/// The point of the comb at (x, y) cell pitches from its top left corner.
fn comb_point(base: Point, x: f32, y: f32) -> Point {
    at(
        base,
        -INNER_WIDTH / 2.0 + x * PITCH_ACROSS,
        FRAME_HEIGHT - TOP_BAR - y * PITCH_UP,
        0.0,
    )
}

/// The corners of the comb, top left first and counter-clockwise as the viewer sees them.
fn comb_corners(base: Point) -> [Point; 4] {
    [
        comb_point(base, 0.0, 0.0),
        comb_point(base, COMB_WIDTH, 0.0),
        comb_point(base, COMB_WIDTH, COMB_HEIGHT),
        comb_point(base, 0.0, COMB_HEIGHT),
    ]
}

/// How much darker a frame is the further back it stands.
fn dim(slot: usize) -> f32 {
    1.0 - 0.12 * slot as f32
}

/// The colour of a hive's trim: that of the kind of its largest process.
fn kind_base(kind: Kind) -> Color {
    match kind {
        Kind::Kernel => KERNEL,
        Kind::System => SYSTEM,
        Kind::Session => SESSION,
        Kind::Container => CONTAINER,
    }
}

/// A frame's comb: the field it is drawn as, and what that field was last painted from.
struct Comb {
    field: Field,
    looks: Vec<u8>,
    counts: Option<Counts>,
    owner: u32,
}

impl Comb {
    fn new(id: Identity) -> Comb {
        let mut field = Field::new(hash_identity(id, SALT), NODES_X, NODES_Y);
        field.owners = Some(vec![NONE; NODES_X * NODES_Y]);
        Comb {
            field,
            looks: Vec::new(),
            counts: None,
            owner: NONE,
        }
    }

    /// Brings the field to `looks`, dimmed as `slot` says and owned by the process at `owner` in
    /// the frame's identities, bumping its revision only for what changed.
    fn update(&mut self, lattice: &Lattice, looks: Vec<u8>, owner: u32, slot: usize) {
        let mut changed = false;
        if looks != self.looks {
            let palette = &lattice.tables;
            for (color, texel) in self.field.colors.iter_mut().zip(&lattice.texels) {
                *color = if texel.cell == NO_CELL {
                    WAX
                } else {
                    palette[usize::from(looks[usize::from(texel.cell)])][usize::from(texel.local)]
                };
            }
            self.looks = looks;
            changed = true;
        }
        if owner != self.owner {
            if let Some(owners) = &mut self.field.owners {
                owners.fill(owner);
            }
            self.owner = owner;
            changed = true;
        }
        let shading = (slot > 0).then(|| Shading {
            ambient: dim(slot),
            diffuse: 0.0,
            specular: 0.0,
            shininess: 1.0,
        });
        if shading != self.field.shading {
            self.field.shading = shading;
            changed = true;
        }
        if changed {
            self.field.revision += 1;
        }
    }
}

/// A cgroup's processes in the slice.
struct Group<'a> {
    name: &'a str,
    memory: u64,
    members: Vec<&'a Process>,
}

/// One frame to draw this frame.
struct Standing<'a> {
    process: &'a Process,
    slot: usize,
    base: Point,
    measure: Measure,
    /// The process's index in the frame's identities, which its pixels pick.
    index: u32,
}

/// What the legend counts, as of the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Summary {
    hives: usize,
    hidden_hives: usize,
    frames: usize,
    unsplit: usize,
    counted: usize,
    bees: usize,
    unread: usize,
}

/// The shown hives' names and the centres of their discs.
type Centers = Vec<(String, [f32; 2])>;

/// The corners of the box the camera fits, and the discs they stand round.
struct Envelope {
    centers: Centers,
    points: Vec<Point>,
}

/// A shown hive.
struct Placed<'a> {
    group: &'a Group<'a>,
    center: [f32; 2],
    /// How many frames it stands.
    framed: usize,
    /// The colour of its trim: that of the kind of its largest process.
    trim: Color,
}

/// What one frame draws: the hives shown, the frames they stand and the hives left out.
struct Plan<'a> {
    hives: Vec<Placed<'a>>,
    standing: Vec<Standing<'a>>,
    centers: Centers,
    hidden: usize,
}

/// Hive: the view's state.
pub struct Hive {
    /// What one cell counts, in bytes.
    unit: u64,
    /// The hives shown, by name, in name order. A shown hive stays until a hive outweighs it by
    /// a quarter.
    shown: Vec<String>,
    discs: Discs,
    /// The process in each slot of each shown hive's frames.
    slots: BTreeMap<String, [Option<Identity>; FRAMES]>,
    combs: BTreeMap<Identity, Comb>,
    swarms: BTreeMap<Identity, Swarm>,
    clock: FixedStep,
    /// The fit box and the discs it was made from.
    envelope: Option<Envelope>,
    summary: Summary,
}

impl Default for Hive {
    fn default() -> Self {
        Hive {
            unit: UNIT_FLOOR,
            shown: Vec::new(),
            discs: Discs::default(),
            slots: BTreeMap::new(),
            combs: BTreeMap::new(),
            swarms: BTreeMap::new(),
            clock: FixedStep::new(STEP, 10, 0.5),
            envelope: None,
            summary: Summary::default(),
        }
    }
}

/// The memory of each process at the sample before the shown one, for its brood.
fn previous_sample(history: &History, elapsed: f64) -> Option<&Sample> {
    let mut last = history.recent(elapsed, 2);
    let (first, second) = (last.next(), last.next());
    match (first, second) {
        (Some(older), Some(newer)) if newer.elapsed == elapsed => Some(older),
        (Some(_), Some(newer)) => Some(newer),
        (Some(only), None) if only.elapsed < elapsed => Some(only),
        _ => None,
    }
}

impl Hive {
    /// Keeps the hives shown now, fills empty places with the heaviest hives left and lets a hive
    /// that outweighs the lightest shown one by a quarter take its place.
    fn choose(&mut self, groups: &BTreeMap<&str, Group>) {
        self.shown.retain(|name| groups.contains_key(name.as_str()));
        let mut ranked: Vec<&Group> = groups.values().collect();
        ranked.sort_by(|a, b| b.memory.cmp(&a.memory).then(a.name.cmp(b.name)));
        let weight = |name: &str| groups[name].memory;
        for group in &ranked {
            if self.shown.len() < MAX_HIVES && !self.shown.iter().any(|name| name == group.name) {
                self.shown.push(group.name.to_owned());
            }
        }
        for group in &ranked {
            if self.shown.iter().any(|name| name == group.name) {
                continue;
            }
            let weakest = self
                .shown
                .iter()
                .enumerate()
                .min_by(|a, b| weight(a.1).cmp(&weight(b.1)).then_with(|| b.1.cmp(a.1)))
                .map(|(index, name)| (index, weight(name)));
            match weakest {
                Some((index, least)) if group.memory * REPLACE.1 > least * REPLACE.0 => {
                    self.shown[index] = group.name.to_owned();
                }
                _ => break,
            }
        }
        self.shown.sort();
    }

    /// Gives each of the hive's three largest processes a slot, keeping those it had.
    fn seat(&mut self, hive: &str, members: &[&Process]) -> [Option<Identity>; FRAMES] {
        let mut top: Vec<&Process> = members.to_vec();
        top.sort_by(|a, b| b.memory.cmp(&a.memory).then(a.id.cmp(&b.id)));
        top.truncate(FRAMES);
        let slots = self.slots.entry(hive.to_owned()).or_default();
        for slot in slots.iter_mut() {
            if slot.is_some_and(|id| !top.iter().any(|process| process.id == id)) {
                *slot = None;
            }
        }
        for process in top {
            if slots.contains(&Some(process.id)) {
                continue;
            }
            if let Some(slot) = slots.iter_mut().find(|slot| slot.is_none()) {
                *slot = Some(process.id);
            }
        }
        *slots
    }

    /// The box around the shown hives' discs, from the ground to the top of the third frame;
    /// made again only when the discs change.
    fn fit(&mut self, centers: &Centers) -> Vec<Point> {
        if let Some(envelope) = &self.envelope
            && envelope.centers == *centers
        {
            return envelope.points.clone();
        }
        let reach = HIVE_RADIUS + 0.2;
        let points: Vec<Point> = centers
            .iter()
            .flat_map(|(_, [x, y])| {
                [-reach, reach].into_iter().flat_map(move |dx| {
                    [-reach, reach]
                        .into_iter()
                        .flat_map(move |dy| [0.0, FRAME_TOP].map(|z| [x + dx, y + dy, z]))
                })
            })
            .collect();
        self.envelope = Some(Envelope {
            centers: centers.clone(),
            points: points.clone(),
        });
        points
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing

/// Darkens or lightens the faces of a timber by the way they face.
const FACE_FRONT: f32 = 0.9;
const FACE_TOP: f32 = 1.08;
const FACE_BACK: f32 = 0.5;
const FACE_LEFT: f32 = 0.72;
const FACE_RIGHT: f32 = 0.62;

/// A timber of a frame: the box from `across[0]` to `across[1]` of its centre line, `up[0]` to
/// `up[1]` of its bottom edge and `front[0]` to `front[1]` out of the comb's plane (the three
/// ranges, in that order), without its bottom. Its faces pick `pick`.
fn timber(
    frame: &mut Frame,
    camera: &Camera,
    base: Point,
    [[a0, a1], [u0, u1], [f0, f1]]: [[f32; 2]; 3],
    color: Color,
    pick: u32,
) {
    let p = |a, u, f| at(base, a, u, f);
    let faces = [
        (
            [p(a0, u0, f1), p(a1, u0, f1), p(a1, u1, f1), p(a0, u1, f1)],
            FACE_FRONT,
        ),
        (
            [p(a0, u0, f0), p(a1, u0, f0), p(a1, u1, f0), p(a0, u1, f0)],
            FACE_BACK,
        ),
        (
            [p(a0, u1, f0), p(a1, u1, f0), p(a1, u1, f1), p(a0, u1, f1)],
            FACE_TOP,
        ),
        (
            [p(a0, u0, f0), p(a0, u0, f1), p(a0, u1, f1), p(a0, u1, f0)],
            FACE_LEFT,
        ),
        (
            [p(a1, u0, f0), p(a1, u0, f1), p(a1, u1, f1), p(a1, u1, f0)],
            FACE_RIGHT,
        ),
    ];
    for (corners, shade) in faces {
        frame.quad(camera, corners, tint(color, shade), pick);
    }
}

const WOOD: Color = [196, 156, 104];

/// The wooden frame round a comb: top bar with its lugs, side bars, bottom bar and the tag of the
/// process's kind colour on the top bar.
fn draw_wood(
    frame: &mut Frame,
    camera: &Camera,
    base: Point,
    slot: usize,
    tag: Color,
    pick: u32,
    selected: bool,
) {
    let wood = tint(WOOD, dim(slot));
    let depth = [-BAR_BACK, BAR_FRONT];
    let half = FRAME_WIDTH / 2.0;
    let top = FRAME_HEIGHT - TOP_BAR;
    let mut bar = |across: [f32; 2], up: [f32; 2], wood: Color| {
        timber(frame, camera, base, [across, up, depth], wood, pick);
    };
    bar(
        [-half - LUG, half + LUG],
        [top, FRAME_HEIGHT],
        tint(wood, 1.06),
    );
    bar([-half, -half + SIDE_BAR], [BOTTOM_BAR, top], wood);
    bar([half - SIDE_BAR, half], [BOTTOM_BAR, top], wood);
    bar([-half, half], [0.0, BOTTOM_BAR], tint(wood, 0.92));
    timber(
        frame,
        camera,
        base,
        [
            [-half + 0.05 * ACROSS, -half + 0.42 * ACROSS],
            [FRAME_HEIGHT - 0.095 * UP, FRAME_HEIGHT - 0.035 * UP],
            [BAR_FRONT, BAR_FRONT + 0.004],
        ],
        tint(tag, dim(slot)),
        pick,
    );
    if selected {
        let color = crate::render::shared::SELECTION_COLOR.map(|c| c as u8);
        let corners = [
            at(base, -half - LUG, 0.0, BAR_FRONT + 0.01),
            at(base, half + LUG, 0.0, BAR_FRONT + 0.01),
            at(base, half + LUG, FRAME_HEIGHT, BAR_FRONT + 0.01),
            at(base, -half - LUG, FRAME_HEIGHT, BAR_FRONT + 0.01),
        ];
        for k in 0..4 {
            frame.hairline(camera, corners[k], corners[(k + 1) % 4], color, 1.6);
        }
    }
}

/// A point `bias` closer to the viewer along the line of sight, which changes only its depth.
fn lifted(camera: &Camera, point: Point, bias: f32) -> Point {
    let viewer = camera.viewer();
    [
        point[0] + viewer[0] * bias,
        point[1] + viewer[1] * bias,
        point[2] + viewer[2] * bias,
    ]
}

/// How far a bee floats in front of the comb, and the depth between one bee and the next and
/// between the layers of one, so no two triangles of a frame lie at the same depth.
const BEE_LIFT: f32 = 0.02;
const BEE_STEP: f32 = 0.0008;
const LAYER_STEP: f32 = 0.0001;
/// A bee at least this many pixels long gets wings.
const WINGS_FROM: f32 = 6.0;

/// How the bees of one frame are lit, picked and marked.
struct Coat {
    /// The frame's dimming, which its bees share.
    shade: f32,
    pick: u32,
    /// The stage's time, which sets the flap of the wings.
    time: f32,
    /// The queen's mark: the colour of the process's kind.
    mark: Color,
}

/// Draws one bee of at most eight triangles, with its place in the frame's order `rank`: an
/// abdomen of two, a stripe, a thorax and a head, the queen's mark, and two wings when it is
/// long enough on screen. Every triangle picks `coat.pick`.
fn draw_bee(
    frame: &mut Frame,
    camera: &Camera,
    base: Point,
    bee: &Bee,
    alpha: f32,
    rank: usize,
    coat: &Coat,
) {
    let ([x, y], heading) = bee.pose(alpha);
    let length = BEE_LENGTH * if bee.queen { QUEEN_SIZE } else { 1.0 };
    let (sin, cos) = heading.sin_cos();
    let world = |u: f32, v: f32| {
        let (u, v) = (u * length, v * length);
        comb_point(base, x + u * cos - v * sin, y + u * sin + v * cos)
    };
    let floor = BEE_LIFT + rank as f32 * BEE_STEP;
    let (pick, time, mark) = (coat.pick, coat.time, coat.mark);
    let color = |color: Color| tint(color, coat.shade);
    let mut layer = 0.0;
    let mut triangle = |corners: [Point; 3], paint: Color| {
        let corners = corners.map(|corner| lifted(camera, corner, floor + layer * LAYER_STEP));
        frame.facet(camera, corners, color(paint), pick);
        layer += 1.0;
    };
    let on_screen = length * PITCH_ACROSS * SCALE * camera.zoom;
    if on_screen >= WINGS_FROM {
        // The wings spread and close as the bee flaps while it walks.
        let flap = if bee.speed > 0.5 {
            (time * 40.0 + (bee.key % 64) as f32).sin()
        } else {
            0.0
        };
        let spread = 0.55 + 0.35 * flap;
        for side in [-1.0_f32, 1.0] {
            let angle = PI + side * spread;
            let tip = (angle.cos() * 0.52, angle.sin() * 0.52);
            triangle(
                [
                    world(0.06, side * 0.05),
                    world(0.18, side * 0.08),
                    world(0.06 + tip.0, side * 0.08 + tip.1),
                ],
                [220, 232, 255],
            );
        }
    }
    let tail = if bee.queen { 0.58 } else { 0.54 };
    let abdomen = [
        world(-tail, 0.0),
        world(-0.18, 0.19),
        world(0.18, 0.0),
        world(-0.18, -0.19),
    ];
    triangle([abdomen[0], abdomen[1], abdomen[2]], [214, 150, 40]);
    triangle([abdomen[0], abdomen[2], abdomen[3]], [214, 150, 40]);
    triangle(
        [world(-0.34, 0.15), world(-0.34, -0.15), world(-0.12, 0.0)],
        [30, 20, 10],
    );
    triangle(
        [world(0.02, 0.13), world(0.02, -0.13), world(0.26, 0.0)],
        [120, 82, 40],
    );
    triangle(
        [world(0.24, 0.09), world(0.24, -0.09), world(0.40, 0.0)],
        [30, 22, 16],
    );
    if bee.queen {
        triangle(
            [world(0.07, 0.06), world(0.07, -0.06), world(0.17, 0.0)],
            mark,
        );
    }
}

/// The meadow plot the hives stand on: a slab of quads, each a little different, with sides.
fn draw_ground(frame: &mut Frame, camera: &Camera, centers: &[[f32; 2]]) {
    let reach = HIVE_RADIUS + 0.1;
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for center in centers {
        for axis in 0..2 {
            low[axis] = low[axis].min(center[axis] - reach);
            high[axis] = high[axis].max(center[axis] + reach);
        }
    }
    if low[0] > high[0] {
        return;
    }
    let (first, last) = (low.map(f32::floor), high.map(f32::ceil));
    let tile = |x: f32, y: f32| {
        let cell = ((x as i32 as u64) << 32) ^ (y as i32 as u64 & 0xffff_ffff);
        let noise = Rng::new(cell ^ SALT).unit();
        tint([30, 46, 40], 0.84 + 0.14 * noise)
    };
    let mut x = first[0];
    while x < last[0] {
        let mut y = first[1];
        while y < last[1] {
            let color = tile(x, y);
            frame.quad(
                camera,
                [
                    [x, y, 0.0],
                    [x + 1.0, y, 0.0],
                    [x + 1.0, y + 1.0, 0.0],
                    [x, y + 1.0, 0.0],
                ],
                color,
                NONE,
            );
            y += 1.0;
        }
        x += 1.0;
    }
    let depth = 0.35;
    let sides = [
        ([first[0], first[1]], [last[0], first[1]], [26, 28, 40]),
        ([last[0], first[1]], [last[0], last[1]], [18, 20, 30]),
        ([last[0], last[1]], [first[0], last[1]], [26, 28, 40]),
        ([first[0], last[1]], [first[0], first[1]], [18, 20, 30]),
    ];
    for ([ax, ay], [bx, by], color) in sides {
        frame.quad(
            camera,
            [
                [ax, ay, 0.0],
                [bx, by, 0.0],
                [bx, by, -depth],
                [ax, ay, -depth],
            ],
            color,
            NONE,
        );
    }
}

/// A box of the hive, whose heights are in the prototype's units.
fn block(frame: &mut Frame, camera: &Camera, low: [f32; 3], high: [f32; 3], color: Color) {
    cuboid(
        frame,
        camera,
        [low[0], low[1], low[2] * UP],
        [high[0], high[1], high[2] * UP],
        color,
    );
}

/// A wooden hive on its stand with a lid, trimmed in `kind`: the prototype's `draw_hive`.
fn draw_hive(frame: &mut Frame, camera: &Camera, [hx, hy]: [f32; 2], kind: Color) {
    let (ax, ay) = (0.66, 0.84);
    let (x0, x1, y0, y1) = (hx - ax, hx + ax, hy - ay, hy + ay);
    let wood = [176, 132, 84];
    // The shadow the hive casts, then its stand: three legs and a rail.
    block(
        frame,
        camera,
        [x0 - 0.2, y0 - 0.2, 0.0],
        [x1 + 0.3, y1 + 0.3, 0.01],
        [12, 16, 20],
    );
    for (lx, ly) in [
        (x1 - 0.1, y1 - 0.1),
        (x1 - 0.1, y0 + 0.1),
        (x0 + 0.1, y1 - 0.1),
    ] {
        let (low, high) = ([lx - 0.06, ly - 0.06, 0.01], [lx + 0.06, ly + 0.06, 0.3]);
        block(frame, camera, low, high, [52, 42, 36]);
    }
    block(
        frame,
        camera,
        [x0 - 0.05, y0 - 0.05, 0.3],
        [x1 + 0.05, y1 + 0.05, 0.38],
        [70, 56, 44],
    );
    // The landing board on the +y side.
    block(
        frame,
        camera,
        [x0 + 0.2, y1, 0.34],
        [x1 - 0.2, y1 + 0.3, 0.4],
        [120, 92, 62],
    );
    let mut z = 0.38;
    for height in [0.72, 0.46] {
        block(frame, camera, [x0, y0, z], [x1, y1, z + height], wood);
        let (low, high) = (
            [x0 - 0.015, y0 - 0.015, z + height - 0.1],
            [x1 + 0.015, y1 + 0.015, z + height],
        );
        block(frame, camera, low, high, kind);
        z += height;
    }
    // The entrance slot at the bottom of the brood box, on the +y face.
    let slot = |x: f32, up: f32| [x, y1 + 0.006, up * UP];
    let corners = [
        slot(hx - 0.38, 0.4),
        slot(hx + 0.38, 0.4),
        slot(hx + 0.38, 0.47),
        slot(hx - 0.38, 0.47),
    ];
    frame.quad(camera, corners, [16, 10, 6], NONE);
    block(
        frame,
        camera,
        [x0 - 0.07, y0 - 0.07, z],
        [x1 + 0.07, y1 + 0.07, z + 0.14],
        [138, 146, 160],
    );
}

/// A faint column of the hive's colour between its lid and the lifted frames. Puffs round alike
/// on both rasterizers however many stack.
fn draw_beam(frame: &mut Frame, camera: &Camera, [hx, hy]: [f32; 2], kind: Color) {
    for step in 0..4 {
        let rise = step as f32 / 3.0;
        let z = (HIVE_TOP + 0.5 * rise) * UP;
        let center = frame.project(camera, [hx, hy, z]);
        frame.items.push(Item::Puff {
            center: [center[0], center[1]],
            radius: 1.1 * SCALE * camera.zoom,
            color: kind,
            strength: 0.05 + 0.04 * rise,
        });
    }
}

// ---------------------------------------------------------------------------------------------
// The view

/// The slice's processes by cgroup, each as the sample says it and not as the scene eased it:
/// cells follow the sample, so a comb is repainted when a sample arrives and not on every frame
/// between.
fn grouped<'a>(processes: &[&'a Process], snapshot: &'a Snapshot) -> BTreeMap<&'a str, Group<'a>> {
    let sampled: HashMap<Identity, &Process> =
        snapshot.processes.iter().map(|p| (p.id, p)).collect();
    let mut groups = BTreeMap::new();
    for &process in processes {
        let process = sampled.get(&process.id).copied().unwrap_or(process);
        let group = groups.entry(cell_name(process)).or_insert_with(|| Group {
            name: cell_name(process),
            memory: 0,
            members: Vec::new(),
        });
        group.memory = group.memory.saturating_add(process.memory);
        group.members.push(process);
    }
    groups
}

impl Hive {
    /// Chooses the hives, places them and gives each its three frames, with what each frame
    /// shows of its process.
    fn plan<'a>(
        &mut self,
        groups: &'a BTreeMap<&'a str, Group<'a>>,
        processes: &[&Process],
        history: &History,
        snapshot: &Snapshot,
    ) -> Plan<'a> {
        self.choose(groups);
        let needs: Vec<(String, f32)> = self
            .shown
            .iter()
            .map(|name| (name.clone(), HIVE_RADIUS))
            .collect();
        let discs = self.discs.arrange(&needs);
        let centers: Centers = self
            .shown
            .iter()
            .filter_map(|name| discs.get(name).map(|&center| (name.clone(), center)))
            .collect();
        self.slots
            .retain(|name, _| self.shown.iter().any(|shown| shown == name));

        let before = previous_sample(history, snapshot.elapsed);
        let mut hives = Vec::new();
        let mut standing = Vec::new();
        for (name, center) in &centers {
            let group = &groups[name.as_str()];
            let seats = self.seat(name, &group.members);
            let mut framed = 0;
            for (slot, id) in seats.iter().enumerate() {
                let Some(process) =
                    id.and_then(|id| group.members.iter().copied().find(|p| p.id == id))
                else {
                    continue;
                };
                let previous = before
                    .and_then(|sample| sample.reading(process.id))
                    .map(|reading| reading.memory);
                let index = processes
                    .iter()
                    .position(|p| p.id == process.id)
                    .map_or(NONE, |index| index as u32);
                standing.push(Standing {
                    process,
                    slot,
                    base: frame_base(*center, slot),
                    measure: Measure::of(process, snapshot, previous),
                    index,
                });
                framed += 1;
            }
            let trim = group
                .members
                .iter()
                .max_by(|a, b| a.memory.cmp(&b.memory).then(b.id.cmp(&a.id)))
                .map_or(SYSTEM, |process| kind_base(process.kind));
            hives.push(Placed {
                group,
                center: *center,
                framed,
                trim,
            });
        }
        Plan {
            hives,
            standing,
            hidden: groups.len() - centers.len(),
            centers,
        }
    }

    /// Brings the unit, the combs and the swarms to the frames standing: a comb is repainted
    /// only when its cell counts, its owner or its dimming change, and a swarm keeps the bees
    /// that stay.
    fn settle(&mut self, standing: &[Standing]) {
        let lattice = Lattice::get();
        let largest = standing
            .iter()
            .filter(|frame| frame.measure.mode != Mode::Sealed)
            .map(|frame| frame.measure.bytes())
            .max();
        self.unit = next_unit(self.unit, largest);
        let live: HashSet<Identity> = standing.iter().map(|frame| frame.process.id).collect();
        self.combs.retain(|id, _| live.contains(id));
        self.swarms.retain(|id, _| live.contains(id));
        for frame in standing {
            let id = frame.process.id;
            if frame.measure.mode == Mode::Sealed {
                self.combs.remove(&id);
                self.swarms.remove(&id);
                continue;
            }
            let comb = self.combs.entry(id).or_insert_with(|| Comb::new(id));
            let counts = Counts::of(&frame.measure, self.unit);
            let looks = if comb.counts == Some(counts) {
                comb.looks.clone()
            } else {
                lattice.arrange(&counts, hash_identity(id, SALT))
            };
            comb.counts = Some(counts);
            comb.update(lattice, looks, frame.index, frame.slot);
            self.swarms
                .entry(id)
                .or_default()
                .update(id, &wishes(frame.process));
        }
    }

    /// Steps the bees at 20 Hz of the stage's clock and returns how far the frame lies into the
    /// next step.
    fn advance(&mut self, time: f32) -> f32 {
        let swarms = &mut self.swarms;
        self.clock.advance(time, |_| {
            for swarm in swarms.values_mut() {
                swarm.step();
            }
        })
    }

    /// Draws one frame: its wood, its comb and its bees, and gives the app what it needs of the
    /// process it stands for.
    fn paint(&self, stage: &mut Stage, frame: &Standing, alpha: f32, summary: &mut Summary) {
        let camera = stage.camera;
        let (process, base, slot, pick) = (frame.process, frame.base, frame.slot, frame.index);
        let id = process.id;
        let tag = kind_color(process);
        let selected = stage.highlight.contains(&id);
        draw_wood(stage.frame, camera, base, slot, tag, pick, selected);
        let corners = comb_corners(base);
        if frame.measure.mode == Mode::Sealed {
            let cap = tint(SEAL, dim(slot));
            stage.frame.quad(camera, corners, cap, pick);
            let middle = at(base, 0.0, FRAME_HEIGHT * 0.5, 0.0);
            stage.places.push((middle, SEALED.to_owned()));
            stage.callout.push((id, SEALED.to_owned()));
        } else {
            summary.unsplit += usize::from(frame.measure.mode != Mode::Split);
            if let Some(comb) = self.combs.get(&id) {
                let placement = Placement::Corners(corners);
                stage
                    .frame
                    .field(camera, placement, &comb.field, Relief::Flat);
            }
            if let Some(swarm) = self.swarms.get(&id) {
                let coat = Coat {
                    shade: dim(slot) + 0.1_f32.min(1.0 - dim(slot)),
                    pick,
                    time: stage.time,
                    mark: tag,
                };
                for (rank, bee) in swarm.bees.values().enumerate() {
                    draw_bee(stage.frame, camera, base, bee, alpha, rank, &coat);
                }
                summary.bees += swarm.bees.len();
                summary.unread += swarm.bees.values().filter(|bee| bee.cpu.is_none()).count();
                let overflow = (process.threads as usize).saturating_sub(swarm.bees.len());
                if overflow > 0 {
                    let corner = at(base, FRAME_WIDTH / 2.0, FRAME_HEIGHT + 0.05, 0.0);
                    stage.places.push((corner, format!("+{overflow} bees")));
                }
            }
        }
        let center = at(base, 0.0, FRAME_HEIGHT * 0.5, 0.0);
        stage.positions.insert(id, center);
        stage.tour.push(TourTarget {
            id,
            position: center,
            weight: process.memory as f32,
        });
        stage.notes.insert(id, self.notes(frame));
    }
}

impl ViewModule for Hive {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let groups = grouped(processes, snapshot);
        let plan = self.plan(&groups, processes, stage.history, snapshot);
        self.settle(&plan.standing);
        let alpha = self.advance(stage.time);

        // The meadow, the hives and the beams above them, then the frames and their bees.
        let camera = stage.camera;
        let ground: Vec<[f32; 2]> = plan.centers.iter().map(|&(_, center)| center).collect();
        draw_ground(stage.frame, camera, &ground);
        for hive in &plan.hives {
            draw_beam(stage.frame, camera, hive.center, hive.trim);
        }
        for hive in &plan.hives {
            draw_hive(stage.frame, camera, hive.center, hive.trim);
        }
        let mut summary = Summary {
            hives: plan.hives.len(),
            hidden_hives: plan.hidden,
            frames: plan.standing.len(),
            counted: plan
                .hives
                .iter()
                .map(|hive| hive.group.members.len() - hive.framed)
                .sum(),
            ..Summary::default()
        };
        for frame in &plan.standing {
            self.paint(stage, frame, alpha, &mut summary);
        }
        stage
            .tour
            .sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
        for hive in &plan.hives {
            let more = hive.group.members.len() - hive.framed;
            let name = short(hive.group.name);
            stage.titles.push(Title {
                at: [hive.center[0], hive.center[1], 0.0],
                reach: HIVE_RADIUS,
                text: if more > 0 {
                    format!("{name} +{more} more")
                } else {
                    name
                },
            });
        }
        stage.bounds.extend(self.fit(&plan.centers));
        self.summary = summary;
        plan.standing.len()
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let summary = &self.summary;
        let mut text = format!(
            " 1 cell = {} | {} hives (+{}), {} frames",
            unit_text(self.unit),
            summary.hives,
            summary.hidden_hives,
            summary.frames,
        );
        if summary.unsplit > 0 {
            text.push_str(&format!(", {} unsplit", summary.unsplit));
        }
        text.push_str(&format!(
            ", {} counted, {} bees",
            summary.counted, summary.bees
        ));
        if summary.unread > 0 {
            text.push_str(&format!(" ({} without thread CPU)", summary.unread));
        }
        text.extend(missing_note(
            snapshot,
            Source::MemoryDetail,
            &format!("cells = {}", platform::MEMORY_LABEL),
        ));
        text.extend(missing_note(
            snapshot,
            Source::MemorySplit,
            "cells = Pss and SwapPss",
        ));
        text.extend(sources_note(snapshot));
        text.push_str(&format!(
            " | hive = {} | frame = process (top 3 by {}) | honey = Pss_Anon, pollen = Pss_File, royal jelly = Pss_Shmem, capped = SwapPss, brood = {} growth | bee = thread, speed = thread CPU, queen = leader",
            HIVE_NOUN,
            platform::MEMORY_LABEL,
            platform::MEMORY_LABEL,
        ));
        text
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// What a hive stands for in the legend: a cgroup on Linux, an app on macOS.
#[cfg(target_os = "macos")]
const HIVE_NOUN: &str = "app";
#[cfg(not(target_os = "macos"))]
const HIVE_NOUN: &str = "cgroup";

/// What the frame of a process whose rollup could not be read says.
const SEALED: &str = "sealed: smaps_rollup unreadable";

impl Hive {
    /// The inspector lines of a frame: its bytes by kind, its brood, its threads and the busiest
    /// listed one, whose name is raw process text for the popup to clean.
    fn notes(&self, frame: &Standing) -> Vec<String> {
        let process = frame.process;
        let (measure, parts) = (&frame.measure, &frame.measure.parts);
        let mut lines = Vec::new();
        match measure.mode {
            Mode::Split => lines.push(format!(
                "honey {} | pollen {} | royal jelly {} | capped {}",
                mib(parts.honey),
                mib(parts.pollen),
                mib(parts.jelly),
                mib(parts.capped),
            )),
            Mode::Pss => lines.push(format!(
                "Pss {} | capped {}",
                mib(parts.nectar),
                mib(parts.capped)
            )),
            Mode::Unsplit => lines.push(format!(
                "{} {} (unsplit)",
                platform::MEMORY_LABEL,
                mib(parts.nectar)
            )),
            Mode::Sealed => {
                lines.push(SEALED.to_owned());
                return lines;
            }
        }
        if measure.growth > 0 {
            lines.push(format!(
                "brood {} ({} growth)",
                mib(measure.growth),
                platform::MEMORY_LABEL
            ));
        }
        let bees = self
            .swarms
            .get(&process.id)
            .map_or(0, |swarm| swarm.bees.len());
        lines.push(format!("{} threads, {bees} bees", process.threads));
        if let Measured::Known(list) = &process.thread_list
            && let Some((busiest, cpu)) = list
                .threads
                .iter()
                .filter_map(|thread| thread.cpu.map(|cpu| (thread, cpu)))
                .max_by(|a, b| a.1.total_cmp(&b.1))
        {
            lines.push(format!("busiest thread: {} {cpu:.0}%", busiest.name));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{churned, rescaled};
    use crate::model::testing::process;
    use crate::model::{Rollup, demo_with};
    use crate::render::{Camera, Frame, RenderInput, Scene};
    use crate::terminal::clean_text;
    use crate::threads::ThreadList;
    use crate::views::View;
    use std::sync::Arc;

    const KIB: u64 = 1 << 10;
    const MIB: u64 = 1 << 20;

    fn view() -> View {
        View::named("hive").expect("hive is registered")
    }

    fn hive(scene: &Scene) -> &Hive {
        scene.module::<Hive>(view()).expect("hive has drawn")
    }

    /// The `smaps_rollup` of the Linux 6.8 fixture in `platform/linux/memory.rs`, in bytes.
    fn fixture() -> Rollup {
        Rollup {
            rss: 34816 * KIB,
            pss: 12288 * KIB,
            pss_anon: Some(8192 * KIB),
            pss_file: Some(3584 * KIB),
            pss_shmem: Some(512 * KIB),
            shared_clean: 16384 * KIB,
            shared_dirty: 128 * KIB,
            private_clean: 9728 * KIB,
            private_dirty: 8576 * KIB,
            swap: 2048 * KIB,
            swap_pss: 1024 * KIB,
        }
    }

    /// A split rollup of `anon`, `file`, `shmem` and `swap` bytes.
    fn rollup(anon: u64, file: u64, shmem: u64, swap: u64) -> Rollup {
        Rollup {
            pss: anon + file + shmem,
            pss_anon: Some(anon),
            pss_file: Some(file),
            pss_shmem: Some(shmem),
            swap_pss: swap,
            ..fixture()
        }
    }

    /// A process of `cgroup` with `memory` resident and `detail` for its rollup.
    fn framed(pid: u32, cgroup: &str, memory: u64, detail: Measured<Rollup>) -> Process {
        Process {
            memory,
            cgroup: cgroup.into(),
            memory_detail: detail,
            ..process(pid, 1)
        }
    }

    fn snapshot_of(processes: Vec<Process>, elapsed: f64) -> Snapshot {
        Snapshot {
            processes,
            elapsed,
            ..Snapshot::default()
        }
    }

    /// The thread list of a process whose threads use `cpus` percent of a core, the first one
    /// the leader.
    fn listed(cpus: &[Option<f32>], complete: bool) -> Measured<ThreadList> {
        let threads: Vec<Thread> = cpus
            .iter()
            .enumerate()
            .map(|(index, &cpu)| Thread {
                tid: 100 + index as u32,
                start: 7 * index as u64,
                name: format!("worker-{index}"),
                cpu,
                state: 'S',
                leader: index == 0,
            })
            .collect();
        Measured::Known(ThreadList {
            threads: Arc::from(threads),
            complete,
        })
    }

    /// The input of a 640 by 360 frame of the hive view.
    fn input<'a>(snapshot: &'a Snapshot, camera: &'a Camera, time: f32) -> RenderInput<'a> {
        RenderInput {
            width: 640,
            height: 360,
            ..RenderInput::small(snapshot, view(), camera, time)
        }
    }

    /// Draws `snapshot` at `time` in a 640 by 360 frame with a camera that fits the hives.
    fn draw(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> (Frame, Camera) {
        let mut camera = Camera::default();
        scene.render(&input(snapshot, &camera, time));
        scene.fit(&mut camera, 640, 360);
        let frame = scene.render(&input(snapshot, &camera, time));
        (frame, camera)
    }

    fn legend(scene: &Scene, snapshot: &Snapshot) -> String {
        hive(scene).legend(snapshot)
    }

    // -----------------------------------------------------------------------------------------
    // Cells and units

    #[test]
    fn cell_counts_follow_a_smaps_rollup_fixture() {
        let rollup = fixture();
        let process = Process {
            memory: rollup.rss,
            memory_detail: Measured::Known(rollup),
            ..process(1, 0)
        };
        let measure = Measure::of(&process, &Snapshot::default(), None);
        assert_eq!(measure.mode, Mode::Split);
        // The frame counts Pss_Anon, Pss_File, Pss_Shmem and SwapPss (8, 3.5, 0.5 and 1 MiB),
        // not the 34 MiB resident set.
        assert_eq!(measure.bytes(), 13 * MIB);
        assert_eq!(
            Counts::of(&measure, 64 * KIB),
            Counts {
                brood: 0,
                honey: 128,
                pollen: 56,
                jelly: 8,
                capped: 16,
                nectar: 0
            }
        );
        // Each component rounds to the nearest cell on its own, a half up: 8, 4, 1 and 1 cells
        // for 13 MiB, three over, which is the most five components can add.
        let coarse = Counts::of(&measure, MIB);
        assert_eq!(
            (coarse.honey, coarse.pollen, coarse.jelly, coarse.capped),
            (8, 4, 1, 1)
        );
        // However the components round, the largest frame (at most 2,000 cells) fits the grid.
        let unit = 4 * MIB;
        let up = Measure {
            mode: Mode::Split,
            parts: Parts {
                honey: 399 * unit + unit / 2,
                pollen: 399 * unit + unit / 2,
                jelly: 399 * unit + unit / 2,
                capped: 399 * unit + unit / 2,
                nectar: 0,
            },
            growth: 0,
        };
        let counts = Counts::of(&up, unit);
        let total = counts.honey + counts.pollen + counts.jelly + counts.capped;
        assert_eq!(total, 4 * 400);
        assert!(total + 3 <= CELLS);
    }

    #[test]
    fn the_unit_is_the_smallest_power_of_two_that_keeps_the_largest_frame_under_two_thousand_cells()
    {
        let floor = UNIT_FLOOR;
        assert_eq!(floor, 64 * KIB);
        assert_eq!(next_unit(floor, Some(2000 * floor)), floor);
        assert_eq!(next_unit(floor, Some(2000 * floor + 1)), 2 * floor);
        assert_eq!(next_unit(floor, Some(1 << 30)), MIB);
        for largest in [
            1,
            700 * KIB,
            100 * MIB,
            2000 * floor,
            2000 * floor + 1,
            1 << 30,
            3 << 30,
            100 << 30,
            1 << 40,
        ] {
            // From a unit that is wrong in either direction.
            for current in [floor, MIB, 1 << 36] {
                let unit = next_unit(current, Some(largest));
                let expected = (0..)
                    .map(|power| floor << power)
                    .find(|&unit| largest <= 2000 * unit)
                    .expect("some power of two is large enough");
                let held = unit == current;
                assert!(unit.is_power_of_two() && unit >= floor);
                assert!(largest <= 2000 * unit, "{largest} bytes at {unit}");
                if !held {
                    assert_eq!(unit, expected, "{largest} bytes from {current}");
                }
                assert!(unit == floor || largest > 2000 * (unit / 2) || held);
            }
        }
        assert_eq!(unit_text(64 * KIB), "64 KiB");
        assert_eq!(unit_text(4 * MIB), "4 MiB");
        assert_eq!(unit_text(2 << 30), "2 GiB");
    }

    #[test]
    fn the_unit_holds_while_the_largest_frame_stays_in_its_band() {
        let unit = MIB;
        for largest in [700 * MIB, 1000 * MIB, 2000 * MIB] {
            assert_eq!(next_unit(unit, Some(largest)), unit);
        }
        // Just below 700 cells the frame is small for its unit and it shrinks; just above 2,000
        // it no longer fits and it grows.
        assert_eq!(next_unit(unit, Some(700 * MIB - 1)), 512 * KIB);
        assert_eq!(next_unit(unit, Some(2000 * MIB + 1)), 2 * MIB);
        assert_eq!(next_unit(unit, Some(10 * MIB)), UNIT_FLOOR);
        // The smallest unit never shrinks, so a small frame keeps it.
        for largest in [0, 10, 699 * UNIT_FLOOR] {
            assert_eq!(next_unit(UNIT_FLOOR, Some(largest)), UNIT_FLOOR);
        }
        // With no frame drawn the unit stays where it was.
        assert_eq!(next_unit(8 * MIB, None), 8 * MIB);
        // A frame that grows through 2,000 cells doubles the unit once, and then rides in the
        // new band down to 1,400 cells of it without moving again.
        let mut unit = UNIT_FLOOR;
        let mut changes = 0;
        for cells in (800..=2400)
            .step_by(100)
            .chain((1400..=2400).rev().step_by(100))
        {
            let next = next_unit(unit, Some(cells * 64 * KIB));
            changes += usize::from(next != unit);
            unit = next;
        }
        assert_eq!(unit, 128 * KIB);
        assert_eq!(changes, 1);
    }

    // -----------------------------------------------------------------------------------------
    // What a frame shows

    #[test]
    fn rss_growth_since_the_last_reading_becomes_brood_and_reverts_when_growth_stops() {
        let split = Measured::Known(rollup(100 * MIB, 20 * MIB, 4 * MIB, 0));
        let id = framed(1, "/a.service", 0, Measured::Pending).id;
        let mut scene = Scene::new();
        let mut seen = Vec::new();
        // The resident set holds, grows by 12 MiB, holds, and then shrinks.
        for (step, memory) in [150, 150, 162, 162, 150].into_iter().enumerate() {
            let at = step as f64 + 1.0;
            let process = framed(1, "/a.service", memory * MIB, split.clone());
            draw(&mut scene, &snapshot_of(vec![process], at), at as f32);
            let comb = &hive(&scene).combs[&id];
            let brooded = comb
                .looks
                .iter()
                .filter(|&&look| look >= BROOD_FIRST)
                .count();
            seen.push((comb.counts.expect("counted"), brooded));
        }
        // The frame is 124 MiB, under 2,000 cells of 64 KiB: honey is 1,600 cells.
        let (honey, growth) = (1600, 192);
        assert_eq!(
            seen[0].0.brood, 0,
            "the first reading has no reading before it"
        );
        assert_eq!(seen[1].0.brood, 0);
        assert_eq!(seen[2].0.brood, growth);
        assert_eq!(
            seen[2].0.honey,
            honey - growth,
            "brood is carved out of honey"
        );
        assert_eq!(seen[2].1, growth, "and painted as larvae");
        assert_eq!(seen[3].0.brood, 0, "with no growth the cells revert");
        assert_eq!((seen[3].0.honey, seen[3].1), (honey, 0));
        assert_eq!(seen[4].0.brood, 0, "shrinking is not brood");

        // Growth is capped at what it is carved from: all of the honey, and not a cell more.
        let mut scene = Scene::new();
        for (step, memory) in [150, 750].into_iter().enumerate() {
            let at = step as f64 + 1.0;
            let process = framed(1, "/a.service", memory * MIB, split.clone());
            draw(&mut scene, &snapshot_of(vec![process], at), at as f32);
        }
        let counts = hive(&scene).combs[&id].counts.expect("counted");
        assert_eq!((counts.brood, counts.honey), (1600, 0));
        assert_eq!((counts.pollen, counts.jelly), (320, 64));

        // A frame with no split carves it out of its pale nectar.
        let mut scene = Scene::new();
        for (step, memory) in [100, 110].into_iter().enumerate() {
            let at = step as f64 + 1.0;
            let process = framed(1, "/a.service", memory * MIB, Measured::Pending);
            draw(&mut scene, &snapshot_of(vec![process], at), at as f32);
        }
        let counts = hive(&scene).combs[&id].counts.expect("counted");
        assert_eq!((counts.brood, counts.nectar), (160, 1760 - 160));
    }

    #[test]
    fn brood_is_right_on_the_first_frame_after_a_view_switch() {
        let split = Measured::Known(rollup(100 * MIB, 20 * MIB, 4 * MIB, 0));
        let at = |memory: u64, elapsed: f64| {
            let process = framed(1, "/a.service", memory * MIB, split.clone());
            snapshot_of(vec![process], elapsed)
        };
        let (before, now) = (at(150, 1.0), at(162, 2.0));
        let mut scene = Scene::new();
        // City is shown while both samples arrive, and the scene records them all the same.
        for (snapshot, time) in [(&before, 1.0), (&now, 2.0)] {
            let camera = Camera::default();
            scene.render(&RenderInput::small(snapshot, View::at(0), &camera, time));
        }
        draw(&mut scene, &now, 2.0);
        let id = now.processes[0].id;
        assert_eq!(hive(&scene).combs[&id].counts.expect("counted").brood, 192);
    }

    #[test]
    fn an_unreadable_rollup_shows_a_sealed_empty_frame() {
        let sealed = framed(1, "/a.service", 64 << 30, Measured::Unreadable);
        let open = framed(
            2,
            "/a.service",
            32 * MIB,
            Measured::Known(rollup(20 * MIB, 8 * MIB, MIB, 0)),
        );
        let (sealed_id, open_id) = (sealed.id, open.id);
        let sealed = Process {
            threads: 4,
            thread_list: listed(&[Some(90.0); 4], true),
            ..sealed
        };
        let open = Process {
            threads: 3,
            thread_list: listed(&[Some(90.0); 3], true),
            ..open
        };
        let measure = Measure::of(&sealed, &Snapshot::default(), Some(0));
        assert_eq!(measure.mode, Mode::Sealed);
        assert_eq!(Counts::of(&measure, UNIT_FLOOR), Counts::default());

        let snapshot = snapshot_of(vec![sealed, open], 5.0);
        let mut scene = Scene::new();
        let (mut frame, camera) = draw(&mut scene, &snapshot, 5.0);
        let state = hive(&scene);
        assert!(!state.combs.contains_key(&sealed_id), "no cells");
        assert!(!state.swarms.contains_key(&sealed_id), "no bees");
        assert!(state.combs.contains_key(&open_id));
        assert_eq!(state.unit, UNIT_FLOOR, "a sealed frame sets no unit");
        assert!(
            scene.positions.contains_key(&sealed_id),
            "it is still a frame"
        );
        assert_eq!(scene.visible, 2);
        let said = |lines: &[(Identity, String)]| {
            lines
                .iter()
                .any(|(id, text)| *id == sealed_id && text == SEALED)
        };
        assert!(said(&scene.callout));
        assert!(
            scene
                .places
                .iter()
                .any(|(_, text)| text == "sealed: smaps_rollup unreadable")
        );
        let text = legend(&scene, &snapshot);
        assert!(text.contains(", 2 frames"), "{text}");
        assert!(
            text.contains(", 3 bees"),
            "the sealed frame's four bees are not drawn: {text}"
        );

        // Its comb is one wax cap, picked as the process.
        frame.rasterize();
        let index = scene
            .positions
            .iter()
            .find(|(id, _)| **id == sealed_id)
            .map(|(_, &point)| point)
            .expect("a position");
        let [x, y, _] = frame.project(&camera, index);
        let at = (y as usize * 640 + x as usize) * 3;
        assert_eq!(frame.pixels[at..at + 3], tint(SEAL, 1.0));
        let sealed_index = frame.identities.iter().position(|id| *id == sealed_id);
        assert_eq!(
            frame.picks[y as usize * 640 + x as usize],
            sealed_index.unwrap() as u32
        );
    }

    #[test]
    fn a_missing_memory_detail_source_draws_unsplit_frames_not_sealed_ones() {
        // macOS has no rollup: every process is pending or unreadable, and the source is missing.
        let processes = vec![
            framed(1, "/a.service", 80 * MIB, Measured::Pending),
            framed(2, "/a.service", 40 * MIB, Measured::Unreadable),
        ];
        let mut snapshot = snapshot_of(processes, 3.0);
        snapshot.missing.insert(Source::MemoryDetail);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 3.0);
        let state = hive(&scene);
        assert_eq!(state.summary.unsplit, 2);
        assert!(!scene.places.iter().any(|(_, text)| text == SEALED));
        for process in &snapshot.processes {
            let comb = &state.combs[&process.id];
            let counts = comb.counts.expect("counted");
            assert_eq!(counts.nectar, cells(process.memory, state.unit));
            assert_eq!(
                (counts.honey, counts.pollen, counts.jelly, counts.capped),
                (0, 0, 0, 0)
            );
        }
        let text = legend(&scene, &snapshot);
        let note = format!(" | no memory detail: cells = {}", platform::MEMORY_LABEL);
        assert!(text.contains(&note), "{text}");
        assert!(text.contains(", 2 unsplit"), "{text}");
    }

    #[test]
    fn a_rollup_without_the_split_draws_pss_as_one_kind() {
        let before_split = Rollup {
            pss_anon: None,
            pss_file: None,
            pss_shmem: None,
            ..fixture()
        };
        let process = framed(1, "/a.service", 34 * MIB, Measured::Known(before_split));
        let mut snapshot = snapshot_of(vec![process], 3.0);
        snapshot.missing.insert(Source::MemorySplit);
        let measure = Measure::of(&snapshot.processes[0], &snapshot, None);
        assert_eq!(measure.mode, Mode::Pss);
        // Pss as pale nectar and SwapPss as capped honey: 12 and 1 MiB.
        assert_eq!(
            Counts::of(&measure, 64 * KIB),
            Counts {
                nectar: 192,
                capped: 16,
                ..Counts::default()
            }
        );
        assert_eq!(measure.bytes(), 13 * MIB);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 3.0);
        assert_eq!(hive(&scene).summary.unsplit, 1);
        let text = legend(&scene, &snapshot);
        assert!(
            text.contains(" | no memory split: cells = Pss and SwapPss"),
            "{text}"
        );
        // The kernel's missing split is also what the rollup says when the source was not noted.
        snapshot.missing = Default::default();
        assert_eq!(
            Measure::of(&snapshot.processes[0], &snapshot, None).mode,
            Mode::Pss
        );
    }

    #[test]
    fn a_pending_rollup_draws_an_unsplit_frame() {
        let process = framed(1, "/a.service", 64 * MIB, Measured::Pending);
        let id = process.id;
        let snapshot = snapshot_of(vec![process], 3.0);
        let measure = Measure::of(&snapshot.processes[0], &snapshot, None);
        assert_eq!(measure.mode, Mode::Unsplit);
        assert_eq!(
            measure.bytes(),
            64 * MIB,
            "the resident set, which is measured"
        );
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 3.0);
        let comb = &hive(&scene).combs[&id];
        assert_eq!(comb.counts.expect("counted").nectar, 1024);
        // A process the rollup has not reached gets no honey, pollen, jelly or capped cells: it
        // is told apart by none of the kinds it was not measured in.
        assert!(
            comb.looks
                .iter()
                .all(|&look| look == EMPTY || look == NECTAR || look >= BROOD_FIRST)
        );
        let text = legend(&scene, &snapshot);
        assert!(text.contains(", 1 unsplit"), "{text}");
        assert!(!text.contains("| no memory"), "nothing is missing: {text}");
    }

    // -----------------------------------------------------------------------------------------
    // The comb

    #[test]
    fn brood_sits_in_the_centre_with_pollen_around_it_and_honey_above() {
        let counts = Counts {
            brood: 60,
            pollen: 150,
            jelly: 40,
            honey: 400,
            capped: 30,
            nectar: 0,
        };
        let looks = Lattice::get().arrange(&counts, 7);
        assert_eq!(
            looks,
            Lattice::get().arrange(&counts, 7),
            "ties break by cell index"
        );
        let of = |keep: &dyn Fn(u8) -> bool| -> Vec<[f32; 2]> {
            (0..CELLS)
                .filter(|&cell| keep(looks[cell]))
                .map(cell_center)
                .collect()
        };
        let brood = of(&|look| look >= BROOD_FIRST);
        let pollen = of(&|look| (POLLEN_FIRST..BROOD_FIRST).contains(&look));
        let jelly = of(&|look| look == JELLY);
        let honey = of(&|look| look == HONEY);
        let capped = of(&|look| look == CAPPED);
        assert_eq!(
            [
                brood.len(),
                pollen.len(),
                jelly.len(),
                honey.len(),
                capped.len()
            ],
            [60, 150, 40, 400, 30],
            "every cell is placed"
        );
        let mean = |cells: &[[f32; 2]]| {
            let n = cells.len() as f32;
            [
                cells.iter().map(|c| c[0]).sum::<f32>() / n,
                cells.iter().map(|c| c[1]).sum::<f32>() / n,
            ]
        };
        let nest = [COMB_WIDTH / 2.0, COMB_HEIGHT * 0.58];
        let middle = mean(&brood);
        assert!((middle[0] - nest[0]).abs() < 0.5 && (middle[1] - nest[1]).abs() < 0.5);
        // Pollen lies outside every brood cell, in the metric of the nest, and round it.
        let away = |[x, y]: [f32; 2]| ((x - nest[0]) / 1.55).hypot(y - nest[1]);
        let farthest = |cells: &[[f32; 2]]| cells.iter().map(|&c| away(c)).fold(0.0, f32::max);
        let nearest = |cells: &[[f32; 2]]| cells.iter().map(|&c| away(c)).fold(f32::MAX, f32::min);
        assert!(nearest(&pollen) >= farthest(&brood));
        let around = mean(&pollen);
        assert!((around[0] - nest[0]).abs() < 1.0 && (around[1] - nest[1]).abs() < 1.0);
        // Royal jelly is below the brood, and honey above it.
        assert!(mean(&jelly)[1] > middle[1] + 2.0);
        assert!(mean(&honey)[1] < middle[1] - 2.0);
        // Capped honey is on the rim of the honey's arch.
        let arch = |[x, y]: [f32; 2]| {
            let (dx, dy) = ((x - nest[0]) / 1.55, y - nest[1]);
            let reach = if dy < 0.0 { dy * 0.42 } else { dy * 2.6 };
            (dx * dx * 0.8 + reach * reach).sqrt()
        };
        let reach = |cells: &[[f32; 2]], pick: fn(f32, f32) -> f32, start: f32| {
            cells.iter().map(|&c| arch(c)).fold(start, pick)
        };
        assert!(reach(&capped, f32::min, f32::MAX) >= reach(&honey, f32::max, 0.0));
        // The larvae of the nest's middle are grown and those of its edge are young.
        let stage = |cell: usize| usize::from(looks[cell] - BROOD_FIRST) / LARVA_TURNS;
        let mut nest_cells: Vec<usize> = (0..CELLS)
            .filter(|&cell| looks[cell] >= BROOD_FIRST)
            .collect();
        nest_cells.sort_by(|&a, &b| away(cell_center(a)).total_cmp(&away(cell_center(b))));
        assert_eq!(stage(nest_cells[0]), LARVA.len() - 1);
        assert_eq!(stage(nest_cells[nest_cells.len() - 1]), 0);
    }

    // -----------------------------------------------------------------------------------------
    // Bees

    #[test]
    fn bees_are_capped_at_two_hundred_per_frame_and_the_queen_is_the_leader() {
        let cpus = vec![Some(40.0); 300];
        let big = Process {
            threads: 500,
            thread_list: listed(&cpus, false),
            ..framed(1, "/a.service", 64 * MIB, Measured::Pending)
        };
        let wanted = wishes(&big);
        assert_eq!(wanted.len(), MAX_BEES);
        assert_eq!(wanted.iter().filter(|wish| wish.queen).count(), 1);
        assert!(wanted[0].queen && wanted.iter().all(|wish| wish.cpu.is_some()));
        // A small process has a bee for each thread, and the unlisted ones come last.
        let small = Process {
            threads: 5,
            thread_list: listed(&[Some(40.0), Some(10.0)], false),
            ..framed(2, "/a.service", MIB, Measured::Pending)
        };
        let small = wishes(&small);
        assert_eq!(small.len(), 5);
        assert_eq!(small.iter().filter(|wish| wish.cpu.is_some()).count(), 2);
        assert!(small[0].queen);
        // No list, no thread: a process always has the bees its thread count says, up to the cap.
        let unlisted = Process {
            threads: 7,
            ..framed(3, "/a.service", MIB, Measured::Pending)
        };
        assert_eq!(wishes(&unlisted).len(), 7);
        let none = Process {
            threads: 0,
            ..framed(4, "/a.service", MIB, Measured::Pending)
        };
        assert!(wishes(&none).is_empty());

        // The overflow is labelled, and the legend counts the bees drawn.
        let id = big.id;
        let snapshot = snapshot_of(vec![big], 4.0);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 4.0);
        assert_eq!(hive(&scene).swarms[&id].bees.len(), MAX_BEES);
        assert!(scene.places.iter().any(|(_, text)| text == "+300 bees"));
        assert!(legend(&scene, &snapshot).contains(", 200 bees"));

        // The queen walks no faster than 0.6 cells a second even when her thread is all CPU.
        let leader = Process {
            threads: 3,
            thread_list: listed(&[Some(100.0), Some(100.0), Some(100.0)], true),
            ..framed(5, "/a.service", MIB, Measured::Pending)
        };
        let mut swarm = Swarm::default();
        swarm.update(leader.id, &wishes(&leader));
        let queen = swarm
            .bees
            .values()
            .find(|bee| bee.queen)
            .expect("a queen")
            .key;
        let mut walked = [0.0_f32; 2];
        for _ in 0..200 {
            let before: BTreeMap<u64, [f32; 2]> = swarm
                .bees
                .iter()
                .map(|(&key, bee)| (key, bee.position))
                .collect();
            swarm.step();
            for (key, bee) in &swarm.bees {
                let was = before[key];
                let step = (bee.position[0] - was[0]).hypot(bee.position[1] - was[1]);
                if *key == queen {
                    assert!(step <= QUEEN_SPEED * STEP + 1e-4, "the queen moved {step}");
                    walked[0] += step;
                } else {
                    walked[1] = walked[1].max(step);
                }
            }
        }
        assert!(walked[0] > 0.0, "the queen does walk, slowly");
        assert!(
            walked[1] > QUEEN_SPEED * STEP * 3.0,
            "the workers are faster"
        );
    }

    /// The distance each bee walked in `steps` steps, by the thread index it belongs to.
    fn walk(swarm: &mut Swarm, keys: &[u64], steps: usize) -> Vec<f32> {
        let mut walked = vec![0.0; keys.len()];
        for _ in 0..steps {
            let before: Vec<[f32; 2]> = keys.iter().map(|key| swarm.bees[key].position).collect();
            swarm.step();
            for (index, key) in keys.iter().enumerate() {
                let (was, now) = (before[index], swarm.bees[key].position);
                walked[index] += (now[0] - was[0]).hypot(now[1] - was[1]);
            }
        }
        walked
    }

    #[test]
    fn idle_bees_stand_still_and_busy_bees_move_with_thread_cpu() {
        assert_eq!(bee_speed(0.0), 0.0);
        assert_eq!(bee_speed(2.99), 0.0);
        assert!((bee_speed(3.0) - (0.25 + 9.0 * 0.03_f32.sqrt())).abs() < 1e-6);
        assert!((bee_speed(100.0) - 9.25).abs() < 1e-6);
        assert!(bee_speed(25.0) < bee_speed(50.0) && bee_speed(50.0) < bee_speed(100.0));

        // The leader (queen) is busy too, but slow; the others span idle to flat out.
        let cpus = [
            Some(50.0),
            Some(0.0),
            Some(2.9),
            Some(3.0),
            Some(25.0),
            Some(100.0),
        ];
        let process = Process {
            threads: cpus.len() as u32,
            thread_list: listed(&cpus, true),
            ..framed(1, "/a.service", MIB, Measured::Pending)
        };
        let Measured::Known(list) = &process.thread_list else {
            unreachable!("a list");
        };
        let keys: Vec<u64> = list.threads.iter().map(thread_key).collect();
        let mut swarm = Swarm::default();
        swarm.update(process.id, &wishes(&process));
        let start: Vec<[f32; 2]> = keys.iter().map(|key| swarm.bees[key].position).collect();
        let walked = walk(&mut swarm, &keys, 100);
        // Below 3 % a bee stays where it is, in the cluster beside the brood.
        for idle in [1, 2] {
            assert_eq!(walked[idle], 0.0);
            assert_eq!(swarm.bees[&keys[idle]].position, start[idle]);
        }
        // Above it a bee walks at 0.25 + 9 sqrt(CPU fraction) cells a second, a little less for
        // the quarter second it takes to get up to speed.
        for (index, cpu) in [(3, 3.0), (4, 25.0), (5, 100.0)] {
            let expected = bee_speed(cpu) * 100.0 * STEP;
            assert!(
                walked[index] > 0.85 * expected && walked[index] < 1.02 * expected,
                "thread {index} at {cpu} % walked {} of {expected}",
                walked[index]
            );
        }
        assert!(walked[3] < walked[4] && walked[4] < walked[5]);
        // The queen's 50 % would be 6.7 cells a second, and she walks 0.6.
        assert!(walked[0] <= QUEEN_SPEED * 100.0 * STEP + 1e-3);
    }

    #[test]
    fn bees_without_a_thread_cpu_reading_stand_still_and_are_counted() {
        // Three listed threads, one of them on its first observation, and seven the list does
        // not reach: eight bees have no reading.
        let cpus = [Some(80.0), None, Some(60.0)];
        let cut = Process {
            threads: 10,
            thread_list: listed(&cpus, false),
            ..framed(1, "/a.service", 64 * MIB, Measured::Pending)
        };
        // A pending list and an unreadable one give every bee no reading.
        let pending = Process {
            threads: 6,
            ..framed(2, "/b.service", 48 * MIB, Measured::Pending)
        };
        let unreadable = Process {
            threads: 4,
            thread_list: Measured::Unreadable,
            ..framed(3, "/c.service", 32 * MIB, Measured::Pending)
        };
        for process in [&cut, &pending, &unreadable] {
            let mut swarm = Swarm::default();
            swarm.update(process.id, &wishes(process));
            let blind: Vec<u64> = swarm
                .bees
                .values()
                .filter(|bee| bee.cpu.is_none())
                .map(|bee| bee.key)
                .collect();
            let start: Vec<[f32; 2]> = blind.iter().map(|key| swarm.bees[key].position).collect();
            let walked = walk(&mut swarm, &blind, 100);
            assert!(walked.iter().all(|&distance| distance == 0.0), "{walked:?}");
            for (key, was) in blind.iter().zip(start) {
                assert_eq!(swarm.bees[key].position, was);
                assert_eq!(swarm.bees[key].wanted(), 0.0);
            }
        }
        // A queen the list does not hold is one of the bees that stand still.
        let headless = Process {
            threads: 3,
            thread_list: Measured::Known(ThreadList {
                threads: Arc::from(vec![Thread {
                    tid: 500,
                    start: 1,
                    name: "t".into(),
                    cpu: Some(70.0),
                    state: 'R',
                    leader: false,
                }]),
                complete: false,
            }),
            ..framed(4, "/d.service", MIB, Measured::Pending)
        };
        let wanted = wishes(&headless);
        assert_eq!(wanted.iter().filter(|wish| wish.queen).count(), 1);
        assert!(
            wanted
                .iter()
                .filter(|wish| wish.queen)
                .all(|wish| wish.cpu.is_none())
        );

        let snapshot = snapshot_of(vec![cut, pending, unreadable], 4.0);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 4.0);
        assert_eq!(hive(&scene).summary.unread, 8 + 6 + 4);
        let text = legend(&scene, &snapshot);
        assert!(text.contains(", 20 bees (18 without thread CPU)"), "{text}");
    }

    #[test]
    fn bees_follow_the_same_path_at_ten_and_sixty_frames_per_second() {
        let cpus: Vec<Option<f32>> = (0..12).map(|i| Some(5.0 + 8.0 * i as f32)).collect();
        let process = Process {
            threads: cpus.len() as u32,
            thread_list: listed(&cpus, true),
            ..framed(1, "/a.service", MIB, Measured::Pending)
        };
        let state = |hive: &Hive| -> Vec<(u64, [f32; 2], f32, f32)> {
            hive.swarms[&process.id]
                .bees
                .values()
                .map(|bee| (bee.key, bee.position, bee.heading, bee.speed))
                .collect()
        };
        let run = |fps: u32| {
            let mut hive = Hive::default();
            hive.swarms
                .entry(process.id)
                .or_default()
                .update(process.id, &wishes(&process));
            // The same clock, counting the steps it takes, says which step a frame has reached.
            let mut mirror = FixedStep::new(STEP, 10, 0.5);
            let mut steps = 0_u32;
            let mut states = BTreeMap::new();
            for frame in 0..=11 * fps {
                let time = 100.0 + frame as f32 / fps as f32;
                hive.advance(time);
                mirror.advance(time, |_| steps += 1);
                states.insert(steps, state(&hive));
            }
            states
        };
        let (slow, fast) = (run(10), run(60));
        let mut compared = 0;
        for (steps, bees) in &slow {
            if let Some(other) = fast.get(steps) {
                assert_eq!(bees, other, "after {steps} steps");
                compared += 1;
            }
        }
        assert!(compared >= 100, "compared {compared} states");
        assert!(slow.keys().max() >= Some(&215));
    }

    // -----------------------------------------------------------------------------------------
    // Hives and frames

    /// A cgroup of `memory` bytes in all, for choosing hives.
    fn group(name: &str, memory: u64) -> Group<'_> {
        Group {
            name,
            memory,
            members: Vec::new(),
        }
    }

    #[test]
    fn a_shown_hive_is_replaced_only_by_a_challenger_that_outweighs_it_by_a_quarter() {
        let names: Vec<String> = ('a'..='j').map(String::from).collect();
        let weigh = |extra: &[(usize, u64)]| {
            let mut groups = BTreeMap::new();
            for (index, name) in names.iter().enumerate() {
                let memory = extra
                    .iter()
                    .find(|(i, _)| *i == index)
                    .map_or(100 * MIB, |&(_, memory)| memory);
                groups.insert(name.as_str(), group(name, memory));
            }
            groups
        };
        let mut hive = Hive::default();
        // Ten hives, eight shown: the heaviest first, and ties by name.
        let mut groups = weigh(&[(8, 90 * MIB), (9, 80 * MIB)]);
        hive.choose(&groups);
        assert_eq!(hive.shown, ["a", "b", "c", "d", "e", "f", "g", "h"]);
        // 125 % of the lightest shown hive is not enough, more than that is.
        groups = weigh(&[(8, 125 * MIB), (9, 80 * MIB)]);
        hive.choose(&groups);
        assert!(!hive.shown.iter().any(|name| name == "i"));
        groups = weigh(&[(8, 125 * MIB + 1), (9, 80 * MIB)]);
        hive.choose(&groups);
        assert_eq!(hive.shown.len(), MAX_HIVES);
        assert!(hive.shown.iter().any(|name| name == "i"));
        // The challenger that was shown stays when it falls back to a tie, and a hive that is
        // gone is dropped and its place filled by the heaviest left.
        groups = weigh(&[(8, 100 * MIB), (9, 100 * MIB)]);
        groups.remove("a");
        hive.choose(&groups);
        assert!(hive.shown.iter().any(|name| name == "i"));
        assert!(!hive.shown.iter().any(|name| name == "a"));
        assert_eq!(hive.shown.len(), MAX_HIVES);
    }

    #[test]
    fn each_hive_shows_its_three_largest_processes_and_keeps_their_slots() {
        let members: Vec<Process> = [(1, 50), (2, 40), (3, 30), (4, 20), (5, 40)]
            .into_iter()
            .map(|(pid, memory)| framed(pid, "/a.service", memory * MIB, Measured::Pending))
            .collect();
        let refs: Vec<&Process> = members.iter().collect();
        let id = |pid: u32| members[pid as usize - 1].id;
        let mut hive = Hive::default();
        // Largest first, and a tie by identity: 50, then the two 40s, pid 2 before pid 5.
        assert_eq!(
            hive.seat("a", &refs),
            [Some(id(1)), Some(id(2)), Some(id(5))]
        );
        // The 40 of pid 2 exits and the 30 moves up into the top three: it takes the free slot
        // and the others stay where they stand.
        let rest: Vec<&Process> = refs.iter().copied().filter(|p| p.id != id(2)).collect();
        assert_eq!(
            hive.seat("a", &rest),
            [Some(id(1)), Some(id(3)), Some(id(5))]
        );
        // The largest process falling out makes room for the next, in the lowest free slot.
        let rest: Vec<&Process> = rest.iter().copied().filter(|p| p.id != id(1)).collect();
        assert_eq!(
            hive.seat("a", &rest),
            [Some(id(4)), Some(id(3)), Some(id(5))]
        );
    }

    /// The demo workload at `time` as the hive row asks for it.
    fn demo(time: f64) -> Snapshot {
        demo_with(time, 192, view().info().demo)
    }

    /// Every shown hive's centre, and the hive and slot of every frame.
    type Places = (
        BTreeMap<String, [f32; 2]>,
        HashMap<Identity, (String, usize)>,
    );

    /// The places the hives and frames keep, after a frame was drawn.
    fn places(scene: &Scene) -> Places {
        let state = hive(scene);
        let centers = state
            .envelope
            .as_ref()
            .expect("a frame was drawn")
            .centers
            .iter()
            .cloned()
            .collect();
        let slots = state
            .slots
            .iter()
            .flat_map(|(name, slots)| {
                slots
                    .iter()
                    .enumerate()
                    .filter_map(move |(slot, id)| id.map(|id| (id, (name.clone(), slot))))
            })
            .collect();
        (centers, slots)
    }

    #[test]
    fn hives_and_frames_keep_their_places_when_cpu_memory_or_population_change() {
        let base = demo(12.0);
        let (changed, exited) = churned(&base);
        assert!(!exited.is_empty());
        for after in [rescaled(&base), changed] {
            let mut scene = Scene::new();
            draw(&mut scene, &base, 12.0);
            let (hives, frames) = places(&scene);
            let positions = scene.positions.clone();
            let mut after = after;
            after.elapsed = 13.0;
            draw(&mut scene, &after, 13.0);
            let (hives_after, frames_after) = places(&scene);
            // A hive that is shown before and after has not moved.
            let kept: Vec<_> = hives
                .keys()
                .filter(|name| hives_after.contains_key(*name))
                .collect();
            assert!(kept.len() >= MAX_HIVES - 2, "{} hives stayed", kept.len());
            for name in kept {
                assert_eq!(hives[name], hives_after[name], "hive {name}");
            }
            // A frame that is shown before and after keeps its slot, and so its place.
            let mut stayed = 0;
            for (id, slot) in &frames {
                if let Some(now) = frames_after.get(id) {
                    assert_eq!(slot, now, "frame {id:?}");
                    assert_eq!(positions[id], scene.positions[id]);
                    stayed += 1;
                }
            }
            assert!(stayed >= 12, "{stayed} frames stayed");
        }
    }

    #[test]
    fn hives_are_labelled_and_only_frames_are_drawn_for_processes() {
        let cgroups = ["/system.slice/nginx.service", "/user.slice/session-1.scope"];
        let mut processes = Vec::new();
        for (index, cgroup) in cgroups.iter().enumerate() {
            for member in 0..5 {
                let pid = 10 * (index as u32 + 1) + member;
                let memory = (50 - 5 * u64::from(member)) * MIB;
                processes.push(framed(pid, cgroup, memory, Measured::Pending));
            }
        }
        let snapshot = snapshot_of(processes, 4.0);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 4.0);
        let labels: Vec<&str> = scene
            .titles
            .iter()
            .map(|title| title.text.as_str())
            .collect();
        assert_eq!(labels.len(), 2);
        assert!(labels.contains(&"nginx +2 more"), "{labels:?}");
        assert!(labels.contains(&"session-1 +2 more"), "{labels:?}");
        // Three frames of five in each hive: nothing is drawn for the other two.
        assert_eq!(scene.positions.len(), 6);
        assert_eq!(scene.visible, 6);
        assert_eq!(scene.collapsed, 4);
        assert_eq!(scene.tour.len(), 6);
        let text = legend(&scene, &snapshot);
        assert!(
            text.starts_with(" 1 cell = 64 KiB | 2 hives (+0), 6 frames, 6 unsplit, 4 counted, "),
            "{text}"
        );
    }

    #[test]
    fn the_inspector_lines_carry_mebibytes_the_thread_count_and_the_busiest_thread_raw() {
        let cpus = [Some(10.0), Some(80.0)];
        let mut process = Process {
            threads: 2,
            thread_list: listed(&cpus, true),
            ..framed(1, "/a.service", 40 * MIB, Measured::Known(fixture()))
        };
        if let Measured::Known(list) = &mut process.thread_list {
            let mut threads = list.threads.to_vec();
            threads[1].name = "bad\x1b[2Jname".into();
            list.threads = Arc::from(threads);
        }
        let id = process.id;
        let snapshot = snapshot_of(vec![process], 4.0);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 4.0);
        let lines = &scene.notes[&id];
        assert_eq!(
            lines[0],
            "honey 8.0 MiB | pollen 3.5 MiB | royal jelly 0.5 MiB | capped 1.0 MiB"
        );
        assert!(lines.contains(&"2 threads, 2 bees".to_owned()), "{lines:?}");
        let busiest = lines.last().expect("a line");
        assert!(
            busiest.starts_with("busiest thread: bad\x1b[2Jname 80%"),
            "{busiest:?}"
        );
        // The name is stored raw, and the popup cleans it.
        assert!(!clean_text(busiest, 80).contains('\x1b'));
    }

    #[test]
    fn the_legend_follows_the_conventions_order_and_names_missing_sources() {
        let mut view = Hive {
            unit: 4 * MIB,
            summary: Summary {
                hives: 8,
                hidden_hives: 4,
                frames: 24,
                unsplit: 2,
                counted: 168,
                bees: 107,
                unread: 5,
            },
            ..Hive::default()
        };
        let mut snapshot = Snapshot::default();
        snapshot.missing.insert(Source::MemoryDetail);
        snapshot.missing.insert(Source::MemorySplit);
        snapshot.sources.insert(Source::Threads);
        let label = platform::MEMORY_LABEL;
        let noun = HIVE_NOUN;
        let expected = [
            " 1 cell = 4 MiB | 8 hives (+4), 24 frames, 2 unsplit, 168 counted, 107 bees (5 without thread CPU)",
            &format!(" | no memory detail: cells = {label}"),
            " | no memory split: cells = Pss and SwapPss",
            " | sources: threads",
            &format!(
                " | hive = {noun} | frame = process (top 3 by {label}) | honey = Pss_Anon, pollen = Pss_File, royal jelly = Pss_Shmem, capped = SwapPss, brood = {label} growth | bee = thread, speed = thread CPU, queen = leader"
            ),
        ]
        .concat();
        assert_eq!(view.legend(&snapshot), expected);
        // Zero counts of unsplit frames and blind bees are left out, and nothing missing says nothing.
        view.summary.unsplit = 0;
        view.summary.unread = 0;
        let text = view.legend(&Snapshot::default());
        assert!(
            text.starts_with(
                " 1 cell = 4 MiB | 8 hives (+4), 24 frames, 168 counted, 107 bees | hive = "
            ),
            "{text}"
        );
        assert!(
            !text.contains("| no ") && !text.contains("sources:"),
            "{text}"
        );
    }

    #[test]
    fn the_demo_shows_brood_at_the_reference_time() {
        let mut scene = Scene::new();
        let time = view().info().contract.time;
        // The sample before, as the headless replay records it, and the sample drawn.
        draw(&mut scene, &demo(time - 1.0), (time - 1.0) as f32);
        let snapshot = demo(time);
        draw(&mut scene, &snapshot, time as f32);
        let state = hive(&scene);
        assert_eq!(
            state.summary.hives, MAX_HIVES,
            "the demo's districts fill the apiary"
        );
        assert_eq!(state.summary.frames, MAX_HIVES * FRAMES);
        let brooding = state
            .combs
            .values()
            .filter(|comb| comb.counts.is_some_and(|counts| counts.brood > 0))
            .count();
        assert!(brooding >= 1, "{brooding} frames carry brood at {time} s");
        // The frames of processes the rollup was never taken for (the demo's kernel threads,
        // which have no address space to read one from) are the unsplit ones; the rest are split.
        let pending = snapshot
            .processes
            .iter()
            .filter(|p| scene.positions.contains_key(&p.id))
            .filter(|p| p.memory_detail == Measured::Pending)
            .count();
        assert_eq!(state.summary.unsplit, pending);
        assert!(pending > 0 && pending < state.summary.frames);
    }

    #[test]
    fn hive_renders_the_same_frame_twice_from_the_same_inputs() {
        let render = || {
            let mut scene = Scene::new();
            draw(&mut scene, &demo(11.0), 11.0);
            let (mut frame, _) = draw(&mut scene, &demo(12.0), 12.0);
            frame.rasterize();
            (
                frame.pixels.clone(),
                frame.picks.clone(),
                scene.positions.len(),
            )
        };
        let (first, second) = (render(), render());
        assert!(first.0 == second.0, "pixels differ");
        assert!(first.1 == second.1, "picks differ");
        assert!(first.2 > 0);
    }
}
