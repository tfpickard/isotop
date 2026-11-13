//! Turning a display list into draw calls without changing a pixel.
//!
//! Painter's order is the contract: a blended item is `c + (t - c) * w` over what is already
//! there, so two overlapping items drawn in the other order give another colour, and a depth-tested
//! item that ties another's depth goes to whichever came later. A draw call draws one kind of item
//! with one pipeline, so a list that alternates kinds (Flow's particles alternate beams and glows)
//! costs a pipeline switch and a draw per item if it is drawn as recorded.
//!
//! The planner moves an item before items of other kinds only when it provably commutes with every
//! one it passes: the screen tiles the item can touch (with a margin for how the GPU rounds its
//! edges) are touched by none of them. Every pixel then sees the same items in the same order as
//! when the list is drawn as recorded, and the frame is the same bit for bit on any backend.
//! Anything the planner cannot bound (a NaN, an item far off screen) touches every tile it could
//! reach or none it could draw on, and an item that overlaps another is never moved past it.

use std::ops::Range;

#[cfg(test)]
use crate::render::Frame;
use crate::render::Item;
use crate::render::shared::{GLYPH_HEIGHT, GLYPH_WIDTH, SELECTION_RING_WIDTH};

/// One pipeline of the GPU path, which is one kind of draw call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Pass {
    Triangles,
    Lines,
    Beams,
    Stars,
    Balls,
    Glows,
    Glyphs,
    Splats,
    Fields,
}

impl Pass {
    /// How many passes there are.
    pub(super) const COUNT: usize = 9;

    pub(super) fn of(item: &Item) -> Self {
        match item {
            Item::Sphere { .. } => Self::Balls,
            Item::Glow { .. } | Item::Puff { .. } => Self::Glows,
            Item::Glyph { .. } => Self::Glyphs,
            Item::Triangle(..) => Self::Triangles,
            Item::Line(..) => Self::Lines,
            Item::Beam(..) => Self::Beams,
            Item::Star(..) => Self::Stars,
            Item::Pick { .. } => Self::Splats,
            Item::Field { .. } => Self::Fields,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Side of a screen tile in pixels, as a shift. Smaller tiles find more items that commute, and
/// cost more to mark for the large ones.
const TILE_SHIFT: u32 = 3;

/// A list whose recorded order already makes this many draw calls or fewer is drawn as recorded:
/// the planner would save little, and a frame of one big mesh pays nothing for it.
const REORDER_FROM: usize = 16;

/// How many pixels past the geometry the GPU may still touch: a triangle's covered pixels have
/// their centres inside it, a line or a point is rasterized within a pixel or so of it, and an
/// impostor's quad is its own bound. Two pixels is the widest of these, with room to spare.
const MARGIN: f32 = 2.0;

/// How many pixels past its radius the vertex shader puts the corners of an impostor's quad:
/// `glow_vertex` and `splat_vertex` use `center + CORNERS[index] * (radius + 1.0)` and
/// `ball_vertex` `(radius + SELECTION_RING_WIDTH + 1.0)`, in `shaders.wgsl`. `bounds` must cover
/// that quad, so it is written from this constant, and `the_quads_are_the_ones_the_shader_draws`
/// holds the constant to the shader source.
const QUAD_PAD: f32 = 1.0;

/// Half the side of the quad the GPU draws for an impostor of `radius` whose shader reaches
/// `radius + pad`. A radius that is not positive is not drawn by the CPU rasterizer to any
/// further than one pixel from the centre, but the GPU draws the quad as the shader gives it: a
/// reach below zero mirrors the corners, so the quad is the same size either way, and a fragment
/// is kept whatever sign the distance over the radius has. The bound takes the magnitude of the
/// radius and pads it, which holds the quad for every sign (`|r + pad| <= |r| + pad`) and the
/// CPU's pixels as well, and is the radius plus the pad, as it always was, for a positive one.
/// A radius that is NaN or infinite gives a bound that is not finite: the item is unbounded.
fn quad_reach(radius: f32, pad: f32) -> f32 {
    radius.abs() + pad
}

/// A run of items of one kind the GPU draws in one call: `order[start..end]` of the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub(super) pass: Pass,
    pub(super) start: u32,
    pub(super) end: u32,
}

impl Run {
    pub(super) fn items(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

/// The pixels an item can change, as `[x0, y0, x1, y1]`, or None when they cannot be bounded (a
/// coordinate that is not a number), in which case the item counts as touching the whole frame.
pub(super) fn bounds(item: &Item) -> Option<[f32; 4]> {
    let around = |x: f32, y: f32, reach: f32| [x - reach, y - reach, x + reach, y + reach];
    let hull = |points: &[[f32; 2]], margin: f32| {
        // `min` and `max` skip a NaN, so a point that is not a number is refused first.
        if points.iter().flatten().any(|c| !c.is_finite()) {
            return [f32::NAN; 4];
        }
        let mut rectangle = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for p in points {
            rectangle = [
                rectangle[0].min(p[0]),
                rectangle[1].min(p[1]),
                rectangle[2].max(p[0]),
                rectangle[3].max(p[1]),
            ];
        }
        [
            rectangle[0] - margin,
            rectangle[1] - margin,
            rectangle[2] + margin,
            rectangle[3] + margin,
        ]
    };
    let rectangle = match *item {
        Item::Triangle(points, ..) | Item::Field { points, .. } => {
            hull(&points.map(|p| [p[0], p[1]]), MARGIN)
        }
        Item::Line(a, b, _) => hull(&[[a[0], a[1]], [b[0], b[1]]], MARGIN),
        Item::Beam(a, b, ..) => hull(&[a, b], MARGIN),
        Item::Star(p, _) => around(p[0] + 0.5, p[1] + 0.5, MARGIN),
        Item::Sphere { center, radius, .. } => around(
            center[0],
            center[1],
            quad_reach(radius, SELECTION_RING_WIDTH + QUAD_PAD) + MARGIN,
        ),
        Item::Glow { center, radius, .. }
        | Item::Puff { center, radius, .. }
        | Item::Pick { center, radius, .. } => {
            around(center[0], center[1], quad_reach(radius, QUAD_PAD) + MARGIN)
        }
        Item::Glyph { origin, scale, .. } => {
            let scale = scale as f32;
            [
                origin[0] - MARGIN,
                origin[1] - MARGIN,
                origin[0] + GLYPH_WIDTH * scale + MARGIN,
                origin[1] + GLYPH_HEIGHT * scale + MARGIN,
            ]
        }
    };
    rectangle.iter().all(|c| c.is_finite()).then_some(rectangle)
}

/// How a frame's items are laid out in draw calls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Order {
    /// Exactly as recorded: adjacent items of one kind share a draw call, nothing else does.
    Recorded,
    /// Items that commute with the ones they pass join the latest draw call of their kind, so
    /// the frame is the same one in fewer calls. Lists with few calls already are left alone.
    Commuting,
}

/// Plans the draw calls of a display list, keeping its buffers from one frame to the next.
#[derive(Default)]
pub(super) struct Planner {
    /// Per screen tile, one more than the index of the latest run that can touch it, 0 for none.
    tiles: Vec<u32>,
    columns: usize,
    /// The run each item went into.
    run_of: Vec<u32>,
    /// How many items each run holds, then where each starts.
    starts: Vec<u32>,
    /// The item indices, run after run, recorded order within a run.
    pub(super) order: Vec<u32>,
    pub(super) runs: Vec<Run>,
    /// How many draw calls the list would be as recorded.
    pub(super) recorded_runs: usize,
}

impl Planner {
    /// Plans `items` for a frame of `width` by `height` pixels.
    pub(super) fn plan(&mut self, items: &[Item], width: u32, height: u32, order: Order) {
        self.runs.clear();
        self.order.clear();
        self.recorded_runs = items
            .iter()
            .map(Pass::of)
            .fold((None, 0), |(last, count), pass| {
                (Some(pass), count + usize::from(last != Some(pass)))
            })
            .1;
        if order == Order::Recorded || self.recorded_runs <= REORDER_FROM {
            self.record(items);
        } else {
            self.commute(items, width, height);
        }
    }

    /// The identity order, adjacent items of one kind in one run.
    fn record(&mut self, items: &[Item]) {
        self.order.extend(0..items.len() as u32);
        for (index, item) in items.iter().enumerate() {
            let pass = Pass::of(item);
            match self.runs.last_mut() {
                Some(run) if run.pass == pass => run.end += 1,
                _ => self.runs.push(Run {
                    pass,
                    start: index as u32,
                    end: index as u32 + 1,
                }),
            }
        }
    }

    fn commute(&mut self, items: &[Item], width: u32, height: u32) {
        let columns = (width as usize).div_ceil(1 << TILE_SHIFT).max(1);
        let rows = (height as usize).div_ceil(1 << TILE_SHIFT).max(1);
        self.columns = columns;
        self.tiles.clear();
        self.tiles.resize(columns * rows, 0);
        self.run_of.clear();
        let mut latest = [None::<u32>; Pass::COUNT];
        for item in items {
            let pass = Pass::of(item);
            let reach = tile_reach(item, width, height, columns, rows);
            // The latest run any of the item's tiles is touched by: it may not go before it.
            let mut barrier = 0;
            if let Some([x0, y0, x1, y1]) = reach {
                for y in y0..=y1 {
                    let row = &self.tiles[y * columns + x0..=y * columns + x1];
                    barrier = row.iter().copied().fold(barrier, u32::max);
                }
            }
            // `barrier` is a run index plus one, so a run at `barrier - 1` or later will do.
            let run = match latest[pass.index()] {
                Some(run) if run + 1 >= barrier => run,
                _ => {
                    self.runs.push(Run {
                        pass,
                        start: 0,
                        end: 0,
                    });
                    let run = self.runs.len() as u32 - 1;
                    latest[pass.index()] = Some(run);
                    run
                }
            };
            if let Some([x0, y0, x1, y1]) = reach {
                for y in y0..=y1 {
                    for tile in &mut self.tiles[y * columns + x0..=y * columns + x1] {
                        *tile = (*tile).max(run + 1);
                    }
                }
            }
            self.run_of.push(run);
        }
        // A stable counting sort of the items by run.
        self.starts.clear();
        self.starts.resize(self.runs.len() + 1, 0);
        for &run in &self.run_of {
            self.starts[run as usize + 1] += 1;
        }
        for index in 1..self.starts.len() {
            self.starts[index] += self.starts[index - 1];
        }
        for (index, run) in self.runs.iter_mut().enumerate() {
            run.start = self.starts[index];
            run.end = self.starts[index + 1];
        }
        self.order.resize(items.len(), 0);
        for (item, &run) in self.run_of.iter().enumerate() {
            let slot = &mut self.starts[run as usize];
            self.order[*slot as usize] = item as u32;
            *slot += 1;
        }
    }

    /// The capacity of every list the planner keeps, which does not grow once the frames have
    /// stopped growing.
    #[cfg(test)]
    pub(super) fn capacities(&self) -> [usize; 5] {
        [
            self.tiles.capacity(),
            self.run_of.capacity(),
            self.starts.capacity(),
            self.order.capacity(),
            self.runs.capacity(),
        ]
    }

    /// The frame's items in drawn order, for tests.
    #[cfg(test)]
    pub(super) fn planned(&self, frame: &Frame) -> Vec<Item> {
        self.order
            .iter()
            .map(|&index| frame.items[index as usize])
            .collect()
    }
}

/// The tiles `[x0, y0, x1, y1]` (inclusive) an item can touch, None for an item that touches
/// none. An item that cannot be bounded touches the whole frame.
fn tile_reach(
    item: &Item,
    width: u32,
    height: u32,
    columns: usize,
    rows: usize,
) -> Option<[usize; 4]> {
    let Some([x0, y0, x1, y1]) = bounds(item) else {
        return Some([0, 0, columns - 1, rows - 1]);
    };
    if x1 < 0.0 || y1 < 0.0 || x0 >= width as f32 || y0 >= height as f32 {
        return None;
    }
    let tile = |at: f32, last: usize| (at.max(0.0) as usize >> TILE_SHIFT).min(last);
    Some([
        tile(x0, columns - 1),
        tile(y0, rows - 1),
        tile(x1, columns - 1),
        tile(y1, rows - 1),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Backdrop, Buffers, Sky};
    use crate::simulation::Rng;

    fn sky() -> Sky {
        Sky {
            backdrop: Backdrop::Dusk,
            haze: [0.0; 3],
            time: 0.0,
        }
    }

    fn glow(x: f32, y: f32) -> Item {
        Item::Glow {
            center: [x, y],
            radius: 3.0,
            color: [200, 120, 60],
            strength: 0.7,
        }
    }

    fn beam(x: f32, y: f32) -> Item {
        Item::Beam([x - 4.0, y], [x + 4.0, y + 3.0], [60, 140, 220], 0.6)
    }

    /// A random display list of every kind but a field, dense enough on a small frame that most
    /// items overlap several others.
    fn random_items(seed: u64, count: usize, width: f32, height: f32) -> Vec<Item> {
        let mut random = Rng::new(seed);
        let at = |random: &mut Rng| [random.unit() * width, random.unit() * height];
        let color = |random: &mut Rng| [0, 1, 2].map(|_| (random.unit() * 255.0) as u8);
        // One radius in five is negative, from just under zero to a few pixels, which no view
        // records but the GPU would still draw as the shader's quad.
        let signed = |random: &mut Rng, widest: f32| {
            let radius = 2.0 + random.unit() * widest;
            if random.below(5) == 0 {
                -radius * random.unit() * 1.5
            } else {
                radius
            }
        };
        (0..count)
            .map(|_| {
                let [x, y] = at(&mut random);
                let depth = random.unit() * 10.0;
                let kind = random.below(11);
                let pick = random.below(5) as u32;
                match kind {
                    0..=1 => Item::Triangle(
                        [
                            [x, y, depth],
                            [
                                x + random.signed() * 20.0,
                                y + random.signed() * 20.0,
                                depth,
                            ],
                            [
                                x + random.signed() * 20.0,
                                y + random.signed() * 20.0,
                                depth,
                            ],
                        ],
                        color(&mut random),
                        pick,
                    ),
                    2 => Item::Line(
                        [x, y, depth],
                        [
                            x + random.signed() * 25.0,
                            y + random.signed() * 25.0,
                            depth,
                        ],
                        color(&mut random),
                    ),
                    3..=4 => Item::Beam(
                        [x, y],
                        [x + random.signed() * 25.0, y + random.signed() * 25.0],
                        color(&mut random),
                        random.unit(),
                    ),
                    5 => Item::Star([x.floor(), y.floor()], color(&mut random)),
                    6 => Item::Sphere {
                        center: [x, y, depth],
                        radius: signed(&mut random, 6.0),
                        depth: 1.0,
                        color: color(&mut random),
                        pick,
                        selected: random.below(4) == 0,
                    },
                    7..=8 => {
                        let item = Item::Glow {
                            center: [x, y],
                            radius: signed(&mut random, 7.0),
                            color: color(&mut random),
                            strength: random.unit(),
                        };
                        if random.below(2) == 0 {
                            item
                        } else if let Item::Glow {
                            center,
                            radius,
                            color,
                            strength,
                        } = item
                        {
                            Item::Puff {
                                center,
                                radius,
                                color,
                                strength,
                            }
                        } else {
                            item
                        }
                    }
                    9 => Item::Glyph {
                        origin: [x.floor(), y.floor()],
                        scale: 1 + random.below(2) as u32,
                        bits: u128::from(random.next_u64()) << 64 | u128::from(random.next_u64()),
                        color: color(&mut random),
                    },
                    _ => Item::Pick {
                        center: [x, y],
                        radius: signed(&mut random, 6.0),
                        depth,
                        pick,
                    },
                }
            })
            .collect()
    }

    fn frame_of(items: Vec<Item>, width: u32, height: u32) -> Frame {
        let mut frame = Frame::new(width, height, sky(), Buffers::default());
        frame.identities = (0..5)
            .map(|pid| crate::model::Identity { pid, start: 1 })
            .collect();
        frame.items = items;
        frame
    }

    /// What the batching of adjacent items of one kind made of a list before the planner.
    fn adjacent_runs(items: &[Item]) -> Vec<(Pass, Range<usize>)> {
        let mut batches: Vec<(Pass, Range<usize>)> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let pass = Pass::of(item);
            match batches.last_mut() {
                Some((last, range)) if *last == pass => range.end = index + 1,
                _ => batches.push((pass, index..index + 1)),
            }
        }
        batches
    }

    #[test]
    fn recorded_order_is_the_adjacent_batching_that_came_before_the_planner() {
        let mut planner = Planner::default();
        for seed in 0..20 {
            let items = random_items(seed, 300, 96.0, 64.0);
            planner.plan(&items, 96, 64, Order::Recorded);
            let planned: Vec<_> = planner
                .runs
                .iter()
                .map(|run| (run.pass, run.items()))
                .collect();
            assert_eq!(planned, adjacent_runs(&items));
            assert_eq!(planner.recorded_runs, planned.len());
            assert!(planner.order.iter().copied().eq(0..items.len() as u32));
        }
    }

    /// The rectangle the GPU draws an item in, written from the vertex shaders and not from
    /// `bounds`: the quad of an impostor is its centre and `radius + pad` on every side, in
    /// whichever direction that reaches, and the others are `bounds` less its margin.
    fn drawn_rectangle(item: &Item) -> Option<[f32; 4]> {
        let quad = |center: [f32; 2], radius: f32, pad: f32| {
            let reach = (radius + pad).abs();
            [
                center[0] - reach,
                center[1] - reach,
                center[0] + reach,
                center[1] + reach,
            ]
        };
        match *item {
            Item::Glow { center, radius, .. }
            | Item::Puff { center, radius, .. }
            | Item::Pick { center, radius, .. } => Some(quad(center, radius, 1.0)),
            Item::Sphere { center, radius, .. } => Some(quad(
                [center[0], center[1]],
                radius,
                SELECTION_RING_WIDTH + 1.0,
            )),
            _ => bounds(item),
        }
    }

    /// Whether two items can change a pixel in common, by the rectangles the GPU draws them in
    /// and nothing coarser than that. An unbounded item overlaps everything.
    fn overlap(a: &Item, b: &Item) -> bool {
        match (bounds(a), bounds(b), drawn_rectangle(a), drawn_rectangle(b)) {
            (Some(_), Some(_), Some(a), Some(b)) => {
                a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
            }
            _ => true,
        }
    }

    #[test]
    fn the_bound_of_an_item_holds_the_quad_the_gpu_draws_for_it() {
        for seed in 0..40 {
            for item in random_items(seed, 200, 96.0, 64.0) {
                let (Some(bound), Some(quad)) = (bounds(&item), drawn_rectangle(&item)) else {
                    continue;
                };
                assert!(
                    bound[0] <= quad[0]
                        && bound[1] <= quad[1]
                        && bound[2] >= quad[2]
                        && bound[3] >= quad[3],
                    "{item:?}: bound {bound:?} does not hold quad {quad:?}"
                );
            }
        }
        for radius in [-30.0, -10.0, -3.5, -0.5, 0.0, 4.0] {
            let glow = Item::Glow {
                center: [50.0, 50.0],
                radius,
                color: [1; 3],
                strength: 1.0,
            };
            let [x0, y0, x1, y1] = bounds(&glow).expect("a finite radius is bounded");
            assert!(x0 < x1 && y0 < y1, "radius {radius} gave an inverted bound");
        }
        for radius in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let pick = Item::Pick {
                center: [5.0, 5.0],
                radius,
                depth: 1.0,
                pick: 0,
            };
            assert!(bounds(&pick).is_none(), "radius {radius} cannot be bounded");
        }
    }

    #[test]
    fn the_quads_are_the_ones_the_shader_draws() {
        let source = include_str!("../shaders.wgsl");
        for expression in [
            format!("glow.center + CORNERS[index] * (glow.radius + {QUAD_PAD:?})"),
            format!("splat.center + CORNERS[index] * (splat.radius + {QUAD_PAD:?})"),
            format!("ball.radius + SELECTION_RING_WIDTH + {QUAD_PAD:?}"),
        ] {
            assert!(
                source.contains(&expression),
                "shaders.wgsl no longer has `{expression}`, which `bounds` is written from"
            );
        }
    }

    #[test]
    fn a_glow_of_negative_radius_stays_behind_the_beam_it_covers() {
        // Pairs of beams and glows far from the place (enough calls for the planner to reorder),
        // then a glow, a beam and a glow of radius -10 at (60, 60): the GPU draws the glow's quad
        // (nine pixels either side), so it must not join the earlier glow call and be drawn
        // before the beam.
        let mut items: Vec<Item> = (0..30)
            .flat_map(|index| {
                [
                    beam(20.0 + index as f32 * 20.0, 300.0),
                    glow(20.0 + index as f32 * 20.0, 300.0),
                ]
            })
            .collect();
        // A glow there makes the beam a call of its own, after the glows so far.
        items.push(glow(60.0, 60.0));
        items.push(beam(60.0, 60.0));
        items.push(Item::Glow {
            center: [60.0, 60.0],
            radius: -10.0,
            color: [9; 3],
            strength: 0.5,
        });
        let mut planner = Planner::default();
        planner.plan(&items, 640, 400, Order::Commuting);
        let position = |index: u32| planner.order.iter().position(|&i| i == index).unwrap();
        let (covered, negative) = (items.len() as u32 - 2, items.len() as u32 - 1);
        assert!(
            position(covered) < position(negative),
            "the glow of radius -10 was drawn before the beam it covers"
        );
    }

    #[test]
    fn commuting_never_moves_an_item_past_one_it_overlaps() {
        let mut planner = Planner::default();
        for seed in 0..60 {
            let (width, height) = [(96, 64), (400, 300), (33, 17)][seed as usize % 3];
            let items = random_items(seed, 400, width as f32, height as f32);
            planner.plan(&items, width, height, Order::Commuting);
            let mut position = vec![usize::MAX; items.len()];
            for (at, &index) in planner.order.iter().enumerate() {
                assert_eq!(
                    position[index as usize],
                    usize::MAX,
                    "an item is drawn once"
                );
                position[index as usize] = at;
            }
            assert!(
                position.iter().all(|&at| at != usize::MAX),
                "every item is drawn"
            );
            for i in 0..items.len() {
                for j in i + 1..items.len() {
                    if overlap(&items[i], &items[j]) {
                        assert!(
                            position[i] < position[j],
                            "seed {seed}: items {i} and {j} overlap and were swapped"
                        );
                    }
                }
            }
            // The runs partition the order, each of one kind, none next to one of its own kind.
            let mut end = 0;
            for run in &planner.runs {
                assert_eq!(run.start, end);
                assert!(run.end > run.start);
                end = run.end;
                assert!(
                    planner.order[run.items()]
                        .iter()
                        .all(|&index| Pass::of(&items[index as usize]) == run.pass)
                );
            }
            assert_eq!(end as usize, items.len());
            assert!(planner.runs.len() <= planner.recorded_runs);
        }
    }

    #[test]
    fn a_frame_drawn_in_the_planned_order_is_the_frame_drawn_as_recorded() {
        let mut planner = Planner::default();
        let mut saved = 0;
        for seed in 0..40 {
            let (width, height) = [(96, 64), (160, 120)][seed as usize % 2];
            let items = random_items(seed, 500, width as f32, height as f32);
            let mut recorded = frame_of(items, width, height);
            planner.plan(&recorded.items, width, height, Order::Commuting);
            let mut reordered = frame_of(planner.planned(&recorded), width, height);
            saved += planner.recorded_runs - planner.runs.len();
            recorded.rasterize();
            reordered.rasterize();
            assert!(
                recorded.pixels == reordered.pixels,
                "seed {seed}: pixels differ"
            );
            assert!(
                recorded.picks == reordered.picks,
                "seed {seed}: picks differ"
            );
            assert!(
                recorded.depth() == reordered.depth(),
                "seed {seed}: depth differs"
            );
        }
        assert!(
            saved > 0,
            "the lists were dense, but some draw calls still merge"
        );
    }

    #[test]
    fn particles_that_alternate_beams_and_glows_take_two_calls_not_hundreds() {
        // A grid of 24 px apart: each particle's beam and glow overlap, no two particles do.
        let items: Vec<Item> = (0..400)
            .flat_map(|index| {
                let (x, y) = (
                    (index % 20) as f32 * 24.0 + 12.0,
                    (index / 20) as f32 * 24.0 + 12.0,
                );
                [beam(x, y), glow(x, y)]
            })
            .collect();
        let mut planner = Planner::default();
        planner.plan(&items, 480, 480, Order::Commuting);
        assert_eq!(planner.recorded_runs, 800);
        assert_eq!(planner.runs.len(), 2);
        assert_eq!(planner.runs[0].pass, Pass::Beams);
        assert_eq!(planner.runs[1].pass, Pass::Glows);
    }

    #[test]
    fn items_that_overlap_keep_their_calls() {
        // The same particle drawn glow, beam, glow, beam, ...: every one overlaps the one before.
        let items: Vec<Item> = (0..100)
            .map(|index| {
                if index % 2 == 0 {
                    glow(50.0, 50.0)
                } else {
                    beam(50.0, 50.0)
                }
            })
            .collect();
        let mut planner = Planner::default();
        planner.plan(&items, 100, 100, Order::Commuting);
        assert_eq!(planner.runs.len(), 100);
    }

    #[test]
    fn an_item_that_cannot_be_bounded_stays_where_it_was() {
        let mut items: Vec<Item> = (0..60)
            .flat_map(|index| {
                let (x, y) = (
                    (index % 10) as f32 * 40.0 + 20.0,
                    (index / 10) as f32 * 40.0 + 20.0,
                );
                [beam(x, y), glow(x, y)]
            })
            .collect();
        items.insert(40, Item::Beam([f32::NAN, 0.0], [1.0, 1.0], [9; 3], 1.0));
        let mut planner = Planner::default();
        planner.plan(&items, 400, 400, Order::Commuting);
        let position = |index: u32| planner.order.iter().position(|&i| i == index).unwrap();
        for other in (0..items.len() as u32).filter(|&i| i != 40) {
            assert_eq!(
                position(other) < position(40),
                other < 40,
                "item {other} crossed the unbounded one"
            );
        }
    }

    #[test]
    fn items_off_the_frame_touch_no_tile_and_join_the_latest_call_of_their_kind() {
        let mut items: Vec<Item> = (0..40)
            .flat_map(|index| {
                [
                    beam(30.0 + index as f32 * 8.0, 40.0),
                    glow(30.0 + index as f32 * 8.0, 40.0),
                ]
            })
            .collect();
        items.push(glow(-500.0, -500.0));
        items.push(beam(9000.0, 20.0));
        let mut planner = Planner::default();
        planner.plan(&items, 400, 100, Order::Commuting);
        assert!(planner.runs.len() <= planner.recorded_runs);
        let mut seen = vec![false; items.len()];
        for &index in &planner.order {
            seen[index as usize] = true;
        }
        assert!(seen.into_iter().all(|drawn| drawn));
    }

    #[test]
    fn planning_again_keeps_every_buffer_it_grew() {
        let mut planner = Planner::default();
        let items = random_items(3, 2000, 320.0, 200.0);
        planner.plan(&items, 320, 200, Order::Commuting);
        let grown = planner.capacities();
        for seed in 3..8 {
            planner.plan(
                &random_items(seed, 1900, 320.0, 200.0),
                320,
                200,
                Order::Commuting,
            );
            assert_eq!(
                planner.capacities(),
                grown,
                "no list grew after the first frame"
            );
        }
    }
}
