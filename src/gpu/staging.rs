//! A frame's display list as the instances and vertices the GPU draws, in the order the planner
//! chose. Every list is kept from one frame to the next and only grows.

use std::ops::Range;

use super::batch::{Order, Pass, Planner};
use super::{Ball, FieldVertex, Glow, Glyph, Splat, Vertex};
use crate::render::shared::{LINE_DEPTH_BIAS, NONE};
use crate::render::{Color, Frame, Item};

/// One draw call: `range` of the buffer `pass` draws from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Draw {
    pub(super) pass: Pass,
    pub(super) range: Range<u32>,
}

#[derive(Default)]
pub(super) struct Staging {
    pub(super) planner: Planner,
    pub(super) vertices: Vec<Vertex>,
    pub(super) balls: Vec<Ball>,
    pub(super) glows: Vec<Glow>,
    pub(super) glyphs: Vec<Glyph>,
    pub(super) splats: Vec<Splat>,
    pub(super) field_vertices: Vec<FieldVertex>,
    /// The draw calls, in the order they are made.
    pub(super) draws: Vec<Draw>,
    /// The nearest and farthest depth any item reaches, which the shader maps to the depth range.
    pub(super) depth: [f32; 2],
}

fn rgba(color: Color, alpha: u8) -> [u8; 4] {
    [color[0], color[1], color[2], alpha]
}

impl Staging {
    /// Plans `frame`'s items and stages them: `offsets` is where each of its fields starts in
    /// the storage buffer.
    pub(super) fn stage(&mut self, frame: &Frame, offsets: &[u32], order: Order) {
        self.planner
            .plan(&frame.items, frame.width, frame.height, order);
        self.vertices.clear();
        self.balls.clear();
        self.glows.clear();
        self.glyphs.clear();
        self.splats.clear();
        self.field_vertices.clear();
        self.draws.clear();
        self.depth = [f32::INFINITY, f32::NEG_INFINITY];
        for run in &self.planner.runs {
            let start = self.len(run.pass);
            for &index in &self.planner.order[run.items()] {
                push(
                    &frame.items[index as usize],
                    offsets,
                    &mut self.depth,
                    (
                        &mut self.vertices,
                        &mut self.balls,
                        &mut self.glows,
                        &mut self.glyphs,
                        &mut self.splats,
                        &mut self.field_vertices,
                    ),
                );
            }
            self.draws.push(Draw {
                pass: run.pass,
                range: start as u32..self.len(run.pass) as u32,
            });
        }
        if self.depth[0] > self.depth[1] {
            self.depth = [0.0, 1.0];
        }
    }

    /// How many instances or vertices the list `pass` draws from holds so far.
    fn len(&self, pass: Pass) -> usize {
        match pass {
            Pass::Balls => self.balls.len(),
            Pass::Glows => self.glows.len(),
            Pass::Glyphs => self.glyphs.len(),
            Pass::Splats => self.splats.len(),
            Pass::Fields => self.field_vertices.len(),
            _ => self.vertices.len(),
        }
    }

    /// The capacity of every list, which does not grow once the frames have stopped growing.
    #[cfg(test)]
    pub(super) fn capacities(&self) -> [usize; 7] {
        [
            self.vertices.capacity(),
            self.balls.capacity(),
            self.glows.capacity(),
            self.glyphs.capacity(),
            self.splats.capacity(),
            self.field_vertices.capacity(),
            self.draws.capacity(),
        ]
    }
}

/// The lists an item is pushed onto: vertices, balls, glows, glyphs, splats and field vertices.
type Lists<'a> = (
    &'a mut Vec<Vertex>,
    &'a mut Vec<Ball>,
    &'a mut Vec<Glow>,
    &'a mut Vec<Glyph>,
    &'a mut Vec<Splat>,
    &'a mut Vec<FieldVertex>,
);

/// Stages one item, and widens `depth` by the depths it reaches.
fn push(item: &Item, offsets: &[u32], depth: &mut [f32; 2], lists: Lists) {
    let (vertices, balls, glows, glyphs, splats, field_vertices) = lists;
    let mut track = |d: f32| {
        depth[0] = depth[0].min(d);
        depth[1] = depth[1].max(d);
    };
    match *item {
        Item::Triangle(points, color, pick) => {
            for p in points {
                track(p[2]);
                vertices.push(Vertex {
                    position: [p[0], p[1]],
                    depth: p[2],
                    color: rgba(color, 255),
                    pick,
                });
            }
        }
        Item::Line(a, b, color) => {
            for p in [a, b] {
                track(p[2] + LINE_DEPTH_BIAS);
                vertices.push(Vertex {
                    position: [p[0], p[1]],
                    depth: p[2] + LINE_DEPTH_BIAS,
                    color: rgba(color, 255),
                    pick: NONE,
                });
            }
        }
        Item::Beam(a, b, color, weight) => {
            for p in [a, b] {
                vertices.push(Vertex {
                    position: p,
                    depth: 0.0,
                    color: rgba(color, (weight.clamp(0.0, 1.0) * 255.0) as u8),
                    pick: NONE,
                });
            }
        }
        Item::Star(p, color) => vertices.push(Vertex {
            position: [p[0] + 0.5, p[1] + 0.5],
            depth: 0.0,
            color: rgba(color, 255),
            pick: NONE,
        }),
        Item::Sphere {
            center,
            radius,
            depth: world,
            color,
            pick,
            selected,
        } => {
            track(center[2] - world);
            track(center[2] + world);
            balls.push(Ball {
                center,
                radius,
                world,
                color: rgba(color, 255),
                pick,
                selected: selected as u32,
            });
        }
        // The blend unit rounds, which is what a `Puff` does on the CPU; a `Glow` truncates.
        Item::Glow {
            center,
            radius,
            color,
            strength,
        }
        | Item::Puff {
            center,
            radius,
            color,
            strength,
        } => glows.push(Glow {
            center,
            radius,
            strength,
            color: rgba(color, 255),
        }),
        Item::Glyph {
            origin,
            scale,
            bits,
            color,
        } => glyphs.push(Glyph {
            origin,
            scale: scale as f32,
            color: rgba(color, 255),
            bits: [0, 32, 64, 96].map(|shift| (bits >> shift) as u32),
        }),
        Item::Pick {
            center,
            radius,
            depth,
            pick,
        } => {
            track(depth);
            splats.push(Splat {
                center,
                radius,
                depth,
                pick,
            });
        }
        Item::Field { points, uv, field } => {
            for (p, uv) in points.into_iter().zip(uv) {
                track(p[2]);
                field_vertices.push(FieldVertex {
                    position: [p[0], p[1]],
                    depth: p[2],
                    uv,
                    slot: offsets[field as usize],
                });
            }
        }
    }
}
