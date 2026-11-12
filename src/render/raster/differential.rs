//! Holds the banded rasterizer to the one it replaced, byte for byte.
//!
//! `reference` keeps the old rasterizer verbatim. Each test here draws random frames twice, once
//! through `Frame::reference_rasterize` and once through the code under test, and compares the
//! colour, depth and pick buffers bit for bit (a depth that is NaN equals any other NaN, as the
//! payload of a NaN says nothing). The frames are the ones the golden test never draws: odd
//! sizes, one pixel wide and high, items that are degenerate, NaN, infinite, enormous or off the
//! frame, every kind of `Item` and every band count, so a band edge falls in the middle of
//! things.
//!
//! It runs in a couple of seconds. `ISOTOP_DIFFERENTIAL_SEEDS=n` draws `n` frames of each size
//! instead of the default, for a longer soak:
//! `ISOTOP_DIFFERENTIAL_SEEDS=2000 cargo test --release rasterizer_matches_the_one_it_replaced`.

use super::super::field::{Field, Placement, Relief, Shading};
use super::super::frame::{Backdrop, Buffers, Frame, Item, Sky};
use super::super::{Camera, NONE};
use crate::app::fixtures::all_available;
use crate::views::View;
use crate::{App, Options, demo_snapshot, warm_up};
use clap::Parser;

/// A small deterministic generator (xorshift64*), so a failing seed reproduces.
struct Dice(u64);

impl Dice {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    fn below(&mut self, count: u64) -> u64 {
        self.next() % count.max(1)
    }

    fn chance(&mut self, probability: f32) -> bool {
        self.unit() < probability
    }

    fn color(&mut self) -> [u8; 3] {
        let bits = self.next();
        [bits as u8, (bits >> 8) as u8, (bits >> 16) as u8]
    }

    fn pick(&mut self) -> u32 {
        match self.below(4) {
            0 => NONE,
            1 => 0,
            _ => self.below(1000) as u32,
        }
    }

    /// A depth: mostly a small range, so that items overwrite each other both ways, now and
    /// then a value nothing compares well with.
    fn depth(&mut self) -> f32 {
        match self.below(60) {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => f32::NEG_INFINITY,
            3 => 0.0,
            4 => -0.0,
            _ => self.between(-5.0, 5.0),
        }
    }
}

/// How wild the coordinates of a generated item may be.
#[derive(Clone, Copy)]
enum Reach {
    /// Anything, including NaN, infinities and values that overflow an edge function.
    Wild,
    /// Finite, within a few frame sizes: the items whose cost grows with their size.
    Tame,
}

/// What a frame is drawn at, and the rows of the bands it will be drawn in.
struct Plan {
    width: u32,
    height: u32,
    /// Rows per band for each band count the test uses: item edges are snapped to them.
    seams: Vec<f32>,
}

impl Plan {
    /// A coordinate along an axis of `extent` pixels.
    fn coordinate(&self, dice: &mut Dice, extent: f32, vertical: bool, reach: Reach) -> f32 {
        let wild = matches!(reach, Reach::Wild);
        match dice.below(100) {
            0..=59 => dice.between(-0.2 * extent, 1.2 * extent),
            60..=69 => (dice.unit() * extent).round() + [0.0, 0.5, -0.5][dice.below(3) as usize],
            70..=79 if vertical && !self.seams.is_empty() => {
                let seam = self.seams[dice.below(self.seams.len() as u64) as usize];
                let rows = dice.below(5) as f32 * seam;
                rows + [-1.0, -0.5, -0.001, 0.0, 0.001, 0.5, 1.0][dice.below(7) as usize]
            }
            80..=86 => dice.between(-4.0 * extent, 5.0 * extent),
            87..=90 if wild => dice.between(-1.0e5, 1.0e5),
            91..=92 if wild => dice.between(-1.0e12, 1.0e12),
            93 if wild => dice.between(-1.0e30, 1.0e30),
            94 if wild => {
                [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX][dice.below(4) as usize]
            }
            95 => 0.0,
            96 => dice.between(-1.0e-6, 1.0e-6),
            _ => dice.between(0.0, extent),
        }
    }

    /// A point on the pixel grid or a pixel centre, the places where an exact comparison is
    /// decided by a single bit: a circle that grazes a pixel, a vertex on a pixel centre.
    fn grid(&self, dice: &mut Dice) -> [f32; 2] {
        let snap = |dice: &mut Dice, extent: f32| {
            (dice.between(-3.0, extent + 3.0)).floor() + [0.0, 0.5][dice.below(2) as usize]
        };
        [
            snap(dice, self.width as f32),
            snap(dice, self.height as f32),
        ]
    }

    fn x(&self, dice: &mut Dice, reach: Reach) -> f32 {
        self.coordinate(dice, self.width as f32, false, reach)
    }

    fn y(&self, dice: &mut Dice, reach: Reach) -> f32 {
        self.coordinate(dice, self.height as f32, true, reach)
    }

    fn point(&self, dice: &mut Dice, reach: Reach) -> [f32; 3] {
        [self.x(dice, reach), self.y(dice, reach), dice.depth()]
    }

    fn triangle(&self, dice: &mut Dice) -> Item {
        let reach = if dice.chance(0.1) {
            Reach::Wild
        } else {
            Reach::Tame
        };
        if dice.chance(0.2) {
            // Vertices on pixel centres and corners, some far apart: the narrowed rows of a long
            // thin triangle end exactly on them.
            let reach = [2.0, 8.0, 40.0, 150.0][dice.below(4) as usize];
            let [x, y] = self.grid(dice);
            let vertex = |dice: &mut Dice| {
                [
                    x + (dice.between(-reach, reach)).floor() + [0.0, 0.5][dice.below(2) as usize],
                    y + (dice.between(-reach, reach)).floor() + [0.0, 0.5][dice.below(2) as usize],
                    dice.depth(),
                ]
            };
            let points = [vertex(dice), vertex(dice), vertex(dice)];
            return Item::Triangle(points, dice.color(), dice.pick());
        }
        let a = self.point(dice, reach);
        let (b, c) = match dice.below(10) {
            // Collinear, to the last bit or nearly.
            0 => {
                let b = self.point(dice, reach);
                let t = dice.between(-1.0, 2.0);
                let c = [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    dice.depth(),
                ];
                (b, c)
            }
            // A repeated vertex.
            1 => (a, self.point(dice, reach)),
            // A sliver or a small triangle near the first vertex.
            2..=5 => {
                let size = [0.5, 2.0, 6.0, 24.0][dice.below(4) as usize];
                let near = |dice: &mut Dice| {
                    [
                        a[0] + dice.between(-size, size),
                        a[1] + dice.between(-size, size),
                        dice.depth(),
                    ]
                };
                (near(dice), near(dice))
            }
            // A long thin stroke, as a ribbon piece is.
            6 | 7 => {
                let tip = [
                    a[0] + dice.between(-70.0, 70.0),
                    a[1] + dice.between(-70.0, 70.0),
                ];
                let width = dice.between(0.2, 3.0);
                (
                    [tip[0], tip[1], dice.depth()],
                    [tip[0] + width, tip[1] - width, dice.depth()],
                )
            }
            _ => (self.point(dice, reach), self.point(dice, reach)),
        };
        // Either winding.
        let points = if dice.chance(0.5) {
            [a, b, c]
        } else {
            [a, c, b]
        };
        Item::Triangle(points, dice.color(), dice.pick())
    }

    fn item(&self, dice: &mut Dice, fields: u32) -> Item {
        let tame = Reach::Tame;
        match dice.below(100) {
            0..=44 => self.triangle(dice),
            45..=52 => {
                let a = self.point(dice, tame);
                let b = if dice.chance(0.5) {
                    [
                        a[0] + dice.between(-40.0, 40.0),
                        a[1] + dice.between(-40.0, 40.0),
                        dice.depth(),
                    ]
                } else {
                    self.point(dice, tame)
                };
                if dice.chance(0.03) {
                    Item::Line([f32::NAN, a[1], a[2]], b, dice.color())
                } else {
                    Item::Line(a, b, dice.color())
                }
            }
            53..=62 => Item::Sphere {
                center: if dice.chance(0.4) {
                    let [x, y] = self.grid(dice);
                    [x, y, dice.depth()]
                } else {
                    self.point(dice, tame)
                },
                radius: [0.0, 0.4, 1.0, 3.0, 9.0, 25.0, -2.0, f32::NAN][dice.below(8) as usize]
                    + if dice.chance(0.3) { 2.0 } else { 0.0 },
                depth: dice.between(-3.0, 3.0),
                color: dice.color(),
                pick: dice.pick(),
                selected: dice.chance(0.3),
            },
            63..=70 => Item::Glow {
                center: self.circle_center(dice),
                radius: self.radius(dice),
                color: dice.color(),
                strength: self.strength(dice),
            },
            71..=77 => Item::Puff {
                center: self.circle_center(dice),
                radius: self.radius(dice),
                color: dice.color(),
                strength: self.strength(dice),
            },
            78..=82 => {
                let a = [self.x(dice, tame), self.y(dice, tame)];
                let b = [
                    a[0] + dice.between(-90.0, 90.0),
                    a[1] + dice.between(-90.0, 90.0),
                ];
                Item::Beam(a, b, dice.color(), dice.between(-0.5, 1.5))
            }
            83..=85 => Item::Star(
                [
                    dice.between(-3.0, self.width as f32 + 3.0),
                    dice.between(-3.0, self.height as f32 + 3.0),
                ],
                dice.color(),
            ),
            86..=88 => Item::Glyph {
                origin: [
                    dice.between(-20.0, self.width as f32 + 5.0).trunc(),
                    dice.between(-20.0, self.height as f32 + 5.0).trunc(),
                ],
                scale: 1 + dice.below(4) as u32,
                bits: u128::from(dice.next()) << 40 | u128::from(dice.next()),
                color: dice.color(),
            },
            89..=93 => Item::Pick {
                center: self.circle_center(dice),
                radius: [0.0, 0.5, 2.0, 7.0, 20.0, 5.0, f32::NAN][dice.below(7) as usize],
                depth: dice.between(-5.0, 5.0),
                pick: dice.pick(),
            },
            _ if fields > 0 => {
                let reach = if dice.chance(0.1) { Reach::Wild } else { tame };
                let points = [
                    self.point(dice, reach),
                    self.point(dice, reach),
                    self.point(dice, reach),
                ];
                let uv =
                    std::array::from_fn(|_| [dice.between(-2.0, 12.0), dice.between(-2.0, 9.0)]);
                Item::Field {
                    points,
                    uv,
                    field: dice.below(u64::from(fields)) as u32,
                }
            }
            _ => self.triangle(dice),
        }
    }

    fn circle_center(&self, dice: &mut Dice) -> [f32; 2] {
        if dice.chance(0.4) {
            self.grid(dice)
        } else {
            [self.x(dice, Reach::Tame), self.y(dice, Reach::Tame)]
        }
    }

    fn radius(&self, dice: &mut Dice) -> f32 {
        match dice.below(12) {
            10 => 5.0,
            11 => 12.0,
            0 => 0.0,
            1 => -3.0,
            2 => f32::NAN,
            3 => dice.between(0.0, 1.5),
            4..=7 => dice.between(1.0, 40.0),
            _ => dice.between(20.0, 150.0),
        }
    }

    fn strength(&self, dice: &mut Dice) -> f32 {
        match dice.below(12) {
            0 => f32::NAN,
            1 => 3.0,
            2 => -0.5,
            _ => dice.between(0.0, 1.0),
        }
    }
}

/// A frame of the given plan with `count` random items, drawn from `seed`.
fn random_frame(plan: &Plan, seed: u64, count: usize) -> Frame {
    let mut dice = Dice::new(seed);
    let backdrop = [
        Backdrop::Dusk,
        Backdrop::Space,
        Backdrop::Sea,
        Backdrop::Void,
        Backdrop::Twilight,
    ][dice.below(5) as usize];
    let haze = if dice.chance(0.4) {
        [
            dice.between(0.0, 40.0),
            dice.between(0.0, 40.0),
            dice.between(0.0, 40.0),
        ]
    } else {
        [0.0; 3]
    };
    let sky = Sky {
        backdrop,
        haze,
        time: dice.between(0.0, 50.0),
    };
    let mut frame = Frame::new(plan.width, plan.height, sky, Buffers::default());
    // Fields of several sizes, with and without owners and shading, registered through the
    // public call; the two items it records are kept as items like any other.
    let camera = Camera::default();
    for _ in 0..dice.below(4) {
        let (columns, rows) = (2 + dice.below(8) as usize, 2 + dice.below(6) as usize);
        let mut field = Field::new(dice.next(), columns, rows);
        for color in &mut field.colors {
            *color = dice.color();
        }
        if dice.chance(0.6) {
            field.owners = Some((0..columns * rows).map(|_| dice.pick()).collect());
        }
        if dice.chance(0.5) {
            field.heights = Some(
                (0..columns * rows)
                    .map(|_| dice.between(-1.0, 1.0))
                    .collect(),
            );
            field.shading = Some(Shading {
                ambient: 0.35,
                diffuse: 0.65,
                specular: if dice.chance(0.5) { 0.4 } else { 0.0 },
                shininess: 12.0,
            });
        }
        let corners = [
            [0.0, 0.0, 0.0],
            [columns as f32, 0.0, 0.0],
            [columns as f32, rows as f32, 0.0],
            [0.0, rows as f32, 0.0],
        ];
        frame.field(&camera, Placement::Corners(corners), &field, Relief::Flat);
    }
    let fields = frame.fields.len() as u32;
    for _ in 0..count {
        let item = plan.item(&mut dice, fields);
        frame.items.push(item);
    }
    frame
}

/// A frame's three buffers, kept to compare with another run's.
struct Drawn {
    width: usize,
    pixels: Vec<u8>,
    picks: Vec<u32>,
    depth: Vec<f32>,
}

impl Drawn {
    fn of(frame: &Frame) -> Self {
        Self {
            width: frame.width as usize,
            pixels: frame.pixels.clone(),
            picks: frame.picks.clone(),
            depth: frame.depth().to_vec(),
        }
    }

    /// Whether this is `old` to the bit, naming the first difference.
    fn matches(&self, old: &Self) -> Result<(), String> {
        let at = |i: usize| format!("pixel ({}, {})", i % self.width, i / self.width);
        if self.pixels.len() != old.pixels.len() {
            return Err(format!(
                "{} pixel bytes against {}",
                self.pixels.len(),
                old.pixels.len()
            ));
        }
        if let Some(i) = (0..self.pixels.len()).find(|&i| self.pixels[i] != old.pixels[i]) {
            return Err(format!(
                "colour differs at {}: {:?} against {:?}",
                at(i / 3),
                &self.pixels[i / 3 * 3..i / 3 * 3 + 3],
                &old.pixels[i / 3 * 3..i / 3 * 3 + 3]
            ));
        }
        if let Some(i) = (0..self.picks.len()).find(|&i| self.picks[i] != old.picks[i]) {
            return Err(format!(
                "pick differs at {}: {} against {}",
                at(i),
                self.picks[i],
                old.picks[i]
            ));
        }
        let unequal = |i: usize| {
            let (n, o) = (self.depth[i], old.depth[i]);
            !(n.to_bits() == o.to_bits() || (n.is_nan() && o.is_nan()))
        };
        if let Some(i) = (0..self.depth.len()).find(|&i| unequal(i)) {
            return Err(format!(
                "depth differs at {}: {:?} against {:?}",
                at(i),
                self.depth[i],
                old.depth[i]
            ));
        }
        Ok(())
    }
}

/// Whether two frames' buffers are the same, naming the first difference.
fn same(new: &Frame, old: &Frame) -> Result<(), String> {
    Drawn::of(new).matches(&Drawn::of(old))
}

const SIZES: [(u32, u32); 14] = [
    (1, 1),
    (1, 9),
    (9, 1),
    (2, 2),
    (3, 5),
    (7, 64),
    (17, 13),
    (31, 64),
    (64, 31),
    (100, 70),
    (257, 129),
    (333, 200),
    (96, 1),
    (480, 270),
];

fn frames_per_size() -> u64 {
    std::env::var("ISOTOP_DIFFERENTIAL_SEEDS")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(10)
}

#[test]
fn rasterizer_matches_the_one_it_replaced() {
    for (index, &(width, height)) in SIZES.iter().enumerate() {
        let counts = [1usize, 2, 3, 5, 8, 13];
        let plan = Plan {
            width,
            height,
            seams: counts
                .iter()
                .map(|&bands| height.div_ceil(bands as u32) as f32)
                .collect(),
        };
        for frame_number in 0..frames_per_size() {
            let seed = (index as u64) << 32 | frame_number;
            let items = if width * height > 20_000 { 150 } else { 60 };
            let mut old = random_frame(&plan, seed, items);
            old.reference_rasterize();
            for &bands in &counts {
                for threads in [1, 3] {
                    let mut new = random_frame(&plan, seed, items);
                    new.rasterize_banded(bands, threads);
                    if let Err(difference) = same(&new, &old) {
                        panic!(
                            "{width}x{height}, seed {seed}, {bands} bands on {threads} threads: {difference}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn the_default_rasterizer_matches_the_one_it_replaced() {
    // What `rasterize` itself picks for the machine's cores, on frames large enough to split.
    for (index, &(width, height)) in [(960u32, 540u32), (1021, 523)].iter().enumerate() {
        let plan = Plan {
            width,
            height,
            seams: Vec::new(),
        };
        for frame_number in 0..frames_per_size().min(4) {
            let seed = 0xD1FF << 32 | (index as u64) << 16 | frame_number;
            let mut old = random_frame(&plan, seed, 400);
            old.reference_rasterize();
            let mut new = random_frame(&plan, seed, 400);
            new.rasterize();
            if let Err(difference) = same(&new, &old) {
                panic!("{width}x{height}, seed {seed}: {difference}");
            }
        }
    }
}

#[test]
fn an_empty_frame_rasterizes_to_its_sky() {
    for (width, height) in [(0, 0), (0, 5), (5, 0)] {
        let mut frame = Frame::new(
            width,
            height,
            super::super::frame::sky_of(Backdrop::Dusk),
            Buffers::default(),
        );
        frame.rasterize();
        assert!(frame.pixels.is_empty() && frame.picks.is_empty());
    }
}

#[test]
fn the_random_frames_draw_every_kind_of_item() {
    // A comparison of two empty frames proves nothing, so check that each kind of item, drawn
    // alone, changes the frame in most of the random frames.
    let plan = Plan {
        width: 257,
        height: 129,
        seams: Vec::new(),
    };
    let kinds = [
        "triangle", "line", "sphere", "glow", "puff", "beam", "star", "glyph", "pick", "field",
    ];
    let mut changed = [0u32; 10];
    for seed in 0..40 {
        let mut bare = random_frame(&plan, seed, 150);
        bare.items.clear();
        bare.reference_rasterize();
        for (index, drawn) in changed.iter_mut().enumerate() {
            let mut frame = random_frame(&plan, seed, 150);
            frame.items.retain(|item| item_kind(item) == index);
            frame.reference_rasterize();
            if same(&frame, &bare).is_err() {
                *drawn += 1;
            }
        }
    }
    for (index, name) in kinds.iter().enumerate() {
        assert!(
            changed[index] >= 20,
            "{name}s change only {} of 40 random frames",
            changed[index]
        );
    }
}

/// One item of a kind whose first or last row is `edge`, reaching down from it for a `direction`
/// of 1 and up for -1, drawn alone at `width` by `height`.
fn seam_frame(width: u32, height: u32, kind: usize, edge: f32, direction: f32) -> Frame {
    let mut frame = Frame::new(
        width,
        height,
        super::super::frame::sky_of(Backdrop::Dusk),
        Buffers::default(),
    );
    let mut field = Field::new(0x5EA4, 4, 3);
    for (index, color) in field.colors.iter_mut().enumerate() {
        *color = [30 + 20 * index as u8, 200 - 15 * index as u8, 90];
    }
    field.owners = Some((0..12).collect());
    let corners = [
        [0.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [4.0, 3.0, 0.0],
        [0.0, 3.0, 0.0],
    ];
    frame.field(
        &Camera::default(),
        Placement::Corners(corners),
        &field,
        Relief::Flat,
    );
    frame.items.clear();
    // A pick disc only writes where something is drawn under it. Nothing else is drawn over a
    // backdrop: a star, which is behind everything, is seen only over the sky.
    if kind == 9 {
        let (far, near) = (width as f32 + 5.0, height as f32 + 5.0);
        for points in [
            [[-5.0, -5.0, -1.0], [far, -5.0, -1.0], [far, near, -1.0]],
            [[-5.0, -5.0, -1.0], [far, near, -1.0], [-5.0, near, -1.0]],
        ] {
            frame.items.push(Item::Triangle(points, [40, 60, 90], 1));
        }
    }
    let reach = 9.0 * direction;
    let color = [220, 90, 40];
    frame.items.push(match kind {
        0 => Item::Triangle(
            [
                [4.0, edge, 1.0],
                [20.0, edge + reach, 1.0],
                [8.0, edge + reach, 1.0],
            ],
            color,
            7,
        ),
        1 => Item::Line([4.0, edge, 0.5], [20.0, edge + reach, 0.5], color),
        2 | 3 => Item::Sphere {
            center: [
                12.0,
                edge + 6.0 * direction + if kind == 3 { 2.0 * direction } else { 0.0 },
                0.0,
            ],
            radius: 5.0,
            depth: 2.0,
            color,
            pick: 3,
            selected: kind == 3,
        },
        4 => Item::Glow {
            center: [12.0, edge + 5.0 * direction],
            radius: 5.0,
            color,
            strength: 0.8,
        },
        5 => Item::Puff {
            center: [12.0, edge + 5.0 * direction],
            radius: 5.0,
            color,
            strength: 0.8,
        },
        6 => Item::Beam([4.0, edge], [20.0, edge + reach], color, 0.7),
        7 => Item::Star([12.0, edge], color),
        8 => Item::Glyph {
            origin: [4.0, if direction > 0.0 { edge } else { edge - 13.0 }],
            scale: 1,
            bits: (1 << 98) - 1,
            color,
        },
        9 => Item::Pick {
            center: [12.0, edge + 5.0 * direction],
            radius: 5.0,
            depth: 3.0,
            pick: 9,
        },
        _ => Item::Field {
            points: [
                [4.0, edge, 1.0],
                [20.0, edge + reach, 1.0],
                [8.0, edge + reach, 1.0],
            ],
            uv: [[0.0, 0.0], [3.0, 0.0], [0.0, 2.0]],
            field: 0,
        },
    });
    frame
}

#[test]
fn an_item_that_ends_on_a_band_seam_is_drawn_whole() {
    // Items whose first or last row falls on the seam between two bands, a fraction before it or
    // a fraction after it, one of each kind, as the reference draws them.
    let (width, height) = (28u32, 32u32);
    for kind in 0..11 {
        for direction in [1.0, -1.0] {
            for bands in [2usize, 3, 4, 8] {
                let rows = height.div_ceil(bands as u32) as f32;
                for seam in (1..bands).map(|index| rows * index as f32) {
                    for offset in [
                        -1.5, -1.0, -0.75, -0.5, -0.25, -0.001, 0.0, 0.001, 0.25, 0.5, 0.75, 1.0,
                        1.5,
                    ] {
                        let edge = seam + offset;
                        let mut old = seam_frame(width, height, kind, edge, direction);
                        old.reference_rasterize();
                        let mut new = seam_frame(width, height, kind, edge, direction);
                        new.rasterize_banded(bands, 1);
                        if let Err(difference) = same(&new, &old) {
                            panic!(
                                "kind {kind}, edge {edge}, {} the seam, {bands} bands: {difference}",
                                if direction > 0.0 { "below" } else { "above" }
                            );
                        }
                    }
                }
            }
        }
    }
}

/// The frame a headless demo run of `name` records at `width`, before it is rasterized.
fn recorded(name: &str, width: u32, processes: u32) -> Frame {
    let options = Options::parse_from([
        "isotop",
        "--demo",
        "--time",
        "12",
        "--renderer",
        "cpu",
        "--width",
        &width.to_string(),
        "--processes",
        &processes.to_string(),
        "--limit",
        &processes.to_string(),
        "--output",
        "speed.png",
        "--view",
        name,
    ]);
    let view = View::all()
        .find(|view| view.name() == name)
        .expect("a registered view");
    let mut app = App::new(
        view,
        demo_snapshot(
            view.info(),
            options.time - options.sample_seconds(),
            options.time,
            options.processes as usize,
        ),
        all_available(),
    );
    warm_up(&mut app, &options, None);
    app.animation = options.time as f32;
    app.draw(
        options.width,
        options.headless_height(),
        options.limit as usize,
    )
}

/// The process's CPU time so far, in milliseconds.
fn cpu_ms() -> f64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid, writable `timespec`, which is all `clock_gettime` needs.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut now) };
    now.tv_sec as f64 * 1000.0 + now.tv_nsec as f64 / 1.0e6
}

/// The least CPU time (every thread's) and the least wall time, in milliseconds, of `runs` calls
/// of `draw`: the least is the run the rest of the machine disturbed least, and CPU time is
/// the one that a busy machine disturbs less than the clock does.
fn best_of(runs: u32, mut draw: impl FnMut()) -> (f64, f64) {
    let (mut cpu, mut wall) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..runs {
        let (started, clock) = (cpu_ms(), std::time::Instant::now());
        draw();
        cpu = cpu.min(cpu_ms() - started);
        wall = wall.min(clock.elapsed().as_secs_f64() * 1000.0);
    }
    (cpu, wall)
}

/// The least CPU time, see `best_of`.
fn best(runs: u32, draw: impl FnMut()) -> f64 {
    best_of(runs, draw).0
}

/// Draws the frames of real views at 1920 pixels both ways, checks they agree to the bit and
/// prints the times. Not a check of speed (no assertion could hold on a shared machine): run it
/// to measure, in release, with
/// `cargo test --release the_rasterizer_is_faster -- --ignored --nocapture`, where
/// `ISOTOP_SPEED_VIEWS=city,coop` picks the views, `ISOTOP_SPEED_PROCESSES=1000` the population
/// and `ISOTOP_SPEED_RUNS` the runs the least is taken of (default 7).
#[test]
#[ignore = "a measurement, not a check: run it by hand in release"]
fn the_rasterizer_is_faster_than_the_one_it_replaced() {
    let views = std::env::var("ISOTOP_SPEED_VIEWS").unwrap_or_else(|_| {
        "orbit,city,coop,ripple,plumbing,anthill,horizon,murmuration,ouija".into()
    });
    let var = |name: &str, default: u32| {
        std::env::var(name)
            .ok()
            .and_then(|text| text.parse().ok())
            .unwrap_or(default)
    };
    let (processes, runs) = (
        var("ISOTOP_SPEED_PROCESSES", 192),
        var("ISOTOP_SPEED_RUNS", 7),
    );
    println!(
        "{:<12} {:>8} {:>9} {:>9} {:>9} {:>9}  (ms at 1920x1080, {processes} processes, {} cores; \
         old and 1 thread are CPU time, threads wall and CPU)",
        "view",
        "items",
        "old",
        "1 thread",
        "wall",
        "cpu",
        super::cores()
    );
    for name in views.split(',') {
        let mut frame = recorded(name, 1920, processes);
        if let Ok(only) = std::env::var("ISOTOP_SPEED_ONLY") {
            // Just one rasterizer, `runs` times, for a profiler to watch: `old`, `single` or a
            // number of bands drawn on every core.
            for _ in 0..runs {
                match only.as_str() {
                    "old" => frame.reference_rasterize(),
                    "single" => frame.rasterize_banded(1, 1),
                    bands => frame.rasterize_banded(
                        bands.parse().expect("old, single or a number of bands"),
                        super::cores(),
                    ),
                }
            }
            continue;
        }
        let old = best(runs, || frame.reference_rasterize());
        let reference = Drawn::of(&frame);
        let single = best(runs, || frame.rasterize_banded(1, 1));
        if let Err(difference) = Drawn::of(&frame).matches(&reference) {
            panic!("{name} on one thread: {difference}");
        }
        let (threaded_cpu, threaded) = best_of(runs, || frame.rasterize());
        if let Err(difference) = Drawn::of(&frame).matches(&reference) {
            panic!("{name} on threads: {difference}");
        }
        println!(
            "{name:<12} {:>8} {old:>9.2} {single:>9.2} {threaded:>9.2} {threaded_cpu:>9.2}",
            frame.items.len()
        );
        if let Ok(counts) = std::env::var("ISOTOP_SPEED_BANDS") {
            for bands in counts.split(',').filter_map(|text| text.parse().ok()) {
                let (cpu, wall) = best_of(runs, || frame.rasterize_banded(bands, super::cores()));
                let one = best(runs, || frame.rasterize_banded(bands, 1));
                println!(
                    "    {bands:>3} bands: {} threads wall {wall:>7.2} cpu {cpu:>7.2}, one thread {one:>7.2}",
                    super::cores()
                );
            }
        }
        if std::env::var_os("ISOTOP_SPEED_KINDS").is_some() {
            // Where the time goes: the frame with only the sky, then each kind of item alone
            // (its time less the sky's), old and new, one thread.
            let every = frame.items.clone();
            frame.items.clear();
            let sky = (
                best(runs, || frame.reference_rasterize()),
                best(runs, || frame.rasterize_banded(1, 1)),
            );
            println!("    sky only {:>9.2} {:>9.2}", sky.0, sky.1);
            for (kind, label) in [
                "triangle", "line", "sphere", "glow", "puff", "beam", "star", "glyph", "pick",
                "field",
            ]
            .iter()
            .enumerate()
            {
                frame.items = every
                    .iter()
                    .copied()
                    .filter(|item| item_kind(item) == kind)
                    .collect();
                if frame.items.is_empty() {
                    continue;
                }
                let old = best(runs, || frame.reference_rasterize()) - sky.0;
                let new = best(runs, || frame.rasterize_banded(1, 1)) - sky.1;
                println!(
                    "    {label:<8} {:>6} {old:>9.2} {new:>9.2}",
                    frame.items.len()
                );
            }
            frame.items = every;
        }
    }
}

/// Which of the ten kinds of item this is, in the order `Item` declares them.
fn item_kind(item: &Item) -> usize {
    match item {
        Item::Triangle(..) => 0,
        Item::Line(..) => 1,
        Item::Sphere { .. } => 2,
        Item::Glow { .. } => 3,
        Item::Puff { .. } => 4,
        Item::Beam(..) => 5,
        Item::Star(..) => 6,
        Item::Glyph { .. } => 7,
        Item::Pick { .. } => 8,
        Item::Field { .. } => 9,
    }
}

/// Writes the triangles, spheres and glows of the real views' frames to the directory in
/// `ISOTOP_DUMP_DIR`, one file a view, for a stand-alone benchmark of the primitives: little-endian
/// words, a header of the width, the height and the number of records, then for a triangle the
/// kind 0, nine coordinates, the colour and the pick, for a sphere the kind 1 and its fields, for
/// a glow the kind 2.
#[test]
#[ignore = "writes files for a benchmark: run it by hand"]
fn dump_the_primitives_of_real_views() {
    let dir = std::env::var("ISOTOP_DUMP_DIR").expect("a directory to write to");
    let views = std::env::var("ISOTOP_SPEED_VIEWS")
        .unwrap_or_else(|_| "orbit,city,coop,ripple,plumbing,anthill,horizon,murmuration".into());
    for name in views.split(',') {
        let frame = recorded(name, 1920, 192);
        let mut words: Vec<u32> = Vec::new();
        let colour = |c: [u8; 3]| u32::from(c[0]) | u32::from(c[1]) << 8 | u32::from(c[2]) << 16;
        for item in &frame.items {
            match *item {
                Item::Triangle(points, color, pick) => {
                    words.push(0);
                    words.extend(points.iter().flatten().map(|v| v.to_bits()));
                    words.extend([colour(color), pick]);
                }
                Item::Sphere {
                    center,
                    radius,
                    depth,
                    color,
                    pick,
                    selected,
                } => {
                    words.push(1);
                    words.extend(center.iter().map(|v| v.to_bits()));
                    words.extend([
                        radius.to_bits(),
                        depth.to_bits(),
                        colour(color),
                        pick,
                        u32::from(selected),
                    ]);
                }
                Item::Glow {
                    center,
                    radius,
                    color,
                    strength,
                } => {
                    words.push(2);
                    words.extend(center.iter().map(|v| v.to_bits()));
                    words.extend([radius.to_bits(), colour(color), strength.to_bits()]);
                }
                _ => {}
            }
        }
        let mut bytes: Vec<u8> = Vec::new();
        for word in [frame.width, frame.height, words.len() as u32] {
            bytes.extend(word.to_le_bytes());
        }
        for word in words {
            bytes.extend(word.to_le_bytes());
        }
        std::fs::write(format!("{dir}/{name}.bin"), bytes).expect("the file is written");
    }
}
