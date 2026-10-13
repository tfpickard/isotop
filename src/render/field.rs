//! The field primitive: a grid of colours drawn in world space, either flat (optionally embossed)
//! or as a lit relief, sampled bilinearly, with an owner grid that picks through to the process
//! behind each cell. One item, `Item::Field`, carries it, and the CPU rasterizer and the wgpu
//! backend draw the same pixels from it.
//!
//! A view keeps a [`Field`] and records it every frame with [`Frame::field`]. The frame shades
//! it in Rust, per node, into the colours both backends sample, and keeps the result for the
//! next frame (recycled through `Buffers`), so a field whose `(key, revision)` has not changed
//! is not shaded again and, on the GPU, not uploaded again.
//!
//! **The sampling formula is the parity contract.** Both backends compute, at each pixel's
//! sample point (its centre, as `Item::Triangle` does), `uv` in node units by linear
//! interpolation across the triangle, then:
//!
//! - `uv` is clamped to `[0, columns - 1] x [0, rows - 1]`;
//! - the colour is `floor(bilerp(c00, c10, c01, c11; fx, fy) + 0.5)` per channel in f32, where
//!   `bilerp` lerps `a + (b - a) * t` along x on both rows and then along y, `(i, j)` is the
//!   cell `floor(uv)` (the last node's cell has `fx = fy = 0`) and `(fx, fy) = uv - (i, j)`;
//! - the pick is the owner of the nearest node, `floor(uv + 0.5)`, or `NONE` without owners.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "ouija draws flat fields of colour; heights, owners, shading and displaced relief have no view yet"
    )
)]

use std::sync::atomic::{AtomicU64, Ordering};

use super::{Camera, Color, Frame, Item, NONE, Point, shared};
use crate::geometry::{dot, normalize};
use crate::medium::{Grid, mix};

/// The most vertices per side of a displaced field's mesh unless a view asks for another count.
pub const MESH: usize = 80;

/// Lit shading baked into a field's node colours. A node's colour is scaled by
/// `ambient + diffuse * max(normal . light, 0)`, with `light` the direction `render::LIGHT`,
/// and a white highlight of `specular * max(normal . half, 0) ^ shininess` is added, with `half`
/// the vector between the light and `Camera::viewer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shading {
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    pub shininess: f32,
}

/// Where a field's nodes sit in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// Node (i, j) sits at `grid.world(i, j)` at height `z`; relief lifts along +z. Only the
    /// grid's origin and cell size are read: the field's own `columns` and `rows` say how many
    /// nodes there are.
    Grid { grid: Grid, z: f32 },
    /// The world corners of nodes (0, 0), (columns - 1, 0), (columns - 1, rows - 1) and
    /// (0, rows - 1), in that order, mapped bilinearly over the field: vertical frames and
    /// tilted panels. Relief lifts along the normal `(c1 - c0) x (c3 - c0)`, so list the corners
    /// counter-clockwise as seen from the lit side. A parallelogram maps affinely, so a flat
    /// field (two triangles) is exact on one; any other quad needs `Relief::Displaced`.
    Corners([Point; 4]),
}

/// How a field is cut into items.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Relief {
    /// Two triangles for the whole quad. Heights, if present, affect shading only.
    Flat,
    /// A mesh of at most `mesh` vertices per side (clamped to the grid), lifted by `scale` times
    /// the bilinearly sampled height. Never one item per cell; colour and shading still come
    /// from the full-resolution grid.
    Displaced { scale: f32, mesh: usize },
}

impl Relief {
    /// A relief of the default mesh size.
    pub fn displaced(scale: f32) -> Self {
        Self::Displaced { scale, mesh: MESH }
    }
}

/// A grid of nodes a view draws with `Frame::field`. It is laid out row-major, node (i, j) at
/// index `j * columns + i`, as `medium::Grid` is, and sized for up to 512 x 512 nodes. A field
/// with fewer than two columns or rows, or a vector of the wrong length, draws nothing.
#[derive(Clone, Debug)]
pub struct Field {
    pub columns: usize,
    pub rows: usize,
    /// The colour at each node, before shading.
    pub colors: Vec<Color>,
    /// Height of each node in world units. With `shading` it shapes the light; with
    /// `Relief::Displaced` it lifts the surface.
    pub heights: Option<Vec<f32>>,
    /// The pick of each node (an index into `frame.identities`, a thing pick, or `NONE`). The
    /// pixel takes the owner of the nearest node; without owners every pixel picks `NONE`.
    pub owners: Option<Vec<u32>>,
    /// Without shading the colours are drawn as they are.
    pub shading: Option<Shading>,
    /// Stable per field, and distinct from every other field the app draws (hash a per-view salt
    /// with the field's own id): what the frame recognises it by from one frame to the next.
    pub key: u64,
    /// The caller bumps it whenever `colors`, `heights`, `owners` or `shading` change.
    pub revision: u64,
}

impl Field {
    /// A black field of `columns` by `rows` nodes with no heights, owners or shading.
    pub fn new(key: u64, columns: usize, rows: usize) -> Self {
        Self {
            columns,
            rows,
            colors: vec![[0; 3]; columns * rows],
            heights: None,
            owners: None,
            shading: None,
            key,
            revision: 0,
        }
    }

    fn drawable(&self) -> bool {
        let nodes = self.columns * self.rows;
        self.columns >= 2
            && self.rows >= 2
            && self.colors.len() == nodes
            && self.heights.as_ref().is_none_or(|h| h.len() == nodes)
            && self.owners.as_ref().is_none_or(|o| o.len() == nodes)
    }
}

/// What the shading of a field depends on besides its data: the placement's steps along the
/// columns and the rows and its normal, the height scale, the shading itself, and the viewer
/// where there is a highlight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Lighting {
    steps: [Point; 3],
    scale: f32,
    shading: Shading,
    viewer: Option<Point>,
}

/// What a `ShadedField` was built from. Equal stamps mean equal texels and owners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stamp {
    revision: u64,
    light: Option<Lighting>,
}

/// A field as both backends sample it: the shaded node colours packed as little-endian RGBA
/// words (`r | g << 8 | b << 16 | 255 << 24`, which is also how the GPU reads them) and the
/// owners. `Frame::fields` holds one for each field a frame draws.
pub struct ShadedField {
    pub(crate) key: u64,
    pub(crate) columns: usize,
    pub(crate) rows: usize,
    pub(crate) texels: Vec<u32>,
    pub(crate) owners: Option<Vec<u32>>,
    pub(crate) stamp: Stamp,
    /// Names these exact texels and owners: taken afresh from `BUILDS` whenever they are
    /// computed, and 0 until then. The GPU rewrites a slot only when the build it holds is not
    /// this one, so a field that was not shaded again is not uploaded again, and one that is
    /// rebuilt, even from a recycled frame that was lost, always is.
    pub(crate) build: u64,
}

static BUILDS: AtomicU64 = AtomicU64::new(1);

impl ShadedField {
    fn empty(key: u64) -> Self {
        Self {
            key,
            columns: 0,
            rows: 0,
            texels: Vec::new(),
            owners: None,
            stamp: Stamp {
                revision: 0,
                light: None,
            },
            build: 0,
        }
    }

    /// Whether `field` shaded under `stamp` is already what this holds.
    fn current(&self, field: &Field, stamp: &Stamp) -> bool {
        self.build != 0
            && self.key == field.key
            && self.columns == field.columns
            && self.rows == field.rows
            && self.stamp == *stamp
    }

    /// Shades `field` into the texels, unless they are current, and takes in its owners when
    /// the revision moved.
    fn refresh(&mut self, field: &Field, stamp: Stamp) {
        if self.current(field, &stamp) {
            return;
        }
        let same_data = self.build != 0
            && self.columns == field.columns
            && self.rows == field.rows
            && self.stamp.revision == stamp.revision;
        self.texels.clear();
        self.texels.reserve(field.columns * field.rows);
        match stamp.light {
            None => self.texels.extend(field.colors.iter().map(|&c| pack(c))),
            Some(light) => shade(field, &light, &mut self.texels),
        }
        if !same_data {
            match (&field.owners, &mut self.owners) {
                (Some(owners), Some(held)) => held.clone_from(owners),
                (Some(owners), held @ None) => *held = Some(owners.clone()),
                (None, held) => *held = None,
            }
        }
        self.columns = field.columns;
        self.rows = field.rows;
        self.stamp = stamp;
        self.build = BUILDS.fetch_add(1, Ordering::Relaxed);
    }

    /// Words this field takes in the GPU storage buffer: the header, the colours and any owners.
    fn words(&self) -> usize {
        let nodes = self.columns * self.rows;
        shared::FIELD_HEADER_WORDS as usize + nodes * (1 + usize::from(self.owners.is_some()))
    }
}

fn pack(color: Color) -> u32 {
    u32::from_le_bytes([color[0], color[1], color[2], 255])
}

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn plus(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn times(a: Point, scale: f32) -> Point {
    a.map(|v| v * scale)
}

impl Placement {
    /// The world displacement of one step along the columns and along the rows, and the unit
    /// normal relief lifts along.
    fn steps(self, columns: usize, rows: usize) -> [Point; 3] {
        match self {
            Self::Grid { grid, .. } => [
                [grid.cell, 0.0, 0.0],
                [0.0, grid.cell, 0.0],
                [0.0, 0.0, 1.0],
            ],
            Self::Corners([c0, c1, _, c3]) => {
                let along = |to: Point, count: usize| {
                    [0, 1, 2].map(|k| (to[k] - c0[k]) / (count - 1) as f32)
                };
                let (u, v) = (along(c1, columns), along(c3, rows));
                [u, v, normalize(cross(u, v))]
            }
        }
    }

    /// The world position of node coordinates (u, v), before relief.
    fn position(self, columns: usize, rows: usize, u: f32, v: f32) -> Point {
        match self {
            Self::Grid { grid, z } => {
                let [x, y] = grid.world(u, v);
                [x, y, z]
            }
            Self::Corners([c0, c1, c2, c3]) => {
                let s = u / (columns - 1) as f32;
                let t = v / (rows - 1) as f32;
                [0, 1, 2].map(|k| {
                    let near = mix(c0[k], c1[k], s);
                    let far = mix(c3[k], c2[k], s);
                    mix(near, far, t)
                })
            }
        }
    }
}

/// The node coordinates clamped to the field, the cell they fall in, that cell's far corner
/// (the same node at the last row and column) and the fractions across it.
fn cell(columns: usize, rows: usize, u: f32, v: f32) -> ([usize; 2], [usize; 2], [f32; 2]) {
    let u = u.clamp(0.0, (columns - 1) as f32);
    let v = v.clamp(0.0, (rows - 1) as f32);
    let (i, j) = (u as usize, v as usize);
    (
        [i, j],
        [(i + 1).min(columns - 1), (j + 1).min(rows - 1)],
        [u - i as f32, v - j as f32],
    )
}

fn height_at(heights: &[f32], columns: usize, rows: usize, u: f32, v: f32) -> f32 {
    let ([i, j], [i1, j1], [fx, fy]) = cell(columns, rows, u, v);
    let top = mix(heights[j * columns + i], heights[j * columns + i1], fx);
    let bottom = mix(heights[j1 * columns + i], heights[j1 * columns + i1], fx);
    mix(top, bottom, fy)
}

/// Shades every node of `field` as `light` says into `texels`: the normal of node (i, j) comes
/// from the central differences of its heights (one-sided at the border), as the cross product
/// of the surface's tangents along the columns and the rows.
fn shade(field: &Field, light: &Lighting, texels: &mut Vec<u32>) {
    let Lighting {
        steps: [along_columns, along_rows, normal],
        scale,
        shading,
        viewer,
    } = *light;
    let toward = normalize(shared::LIGHT);
    // The tangents are U + hx N along the columns and V + hy N along the rows, with U and V the
    // steps and N the normal, so their cross product is U x V + hy (U x N) + hx (N x V).
    let flat = cross(along_columns, along_rows);
    let across_rows = cross(along_columns, normal);
    let across_columns = cross(normal, along_rows);
    let half = viewer.map(|viewer| normalize(plus(toward, viewer)));
    let glossy = half.is_some() && shading.specular > 0.0;
    let reflect = |slope: [f32; 2]| {
        let surface = plus(
            flat,
            plus(
                times(across_rows, slope[1]),
                times(across_columns, slope[0]),
            ),
        );
        let length = dot(surface, surface).sqrt().max(1e-6);
        let lit = shading.ambient + shading.diffuse * (dot(surface, toward) / length).max(0.0);
        let glint = half.filter(|_| glossy).map_or(0.0, |half| {
            255.0
                * shading.specular
                * (dot(surface, half) / length)
                    .max(0.0)
                    .powf(shading.shininess)
        });
        (lit, glint)
    };
    let tint = |color: Color, (lit, glint): (f32, f32)| {
        pack(color.map(|c| (c as f32 * lit + glint + 0.5).clamp(0.0, 255.0) as u8))
    };
    let (columns, rows) = (field.columns, field.rows);
    let Some(heights) = &field.heights else {
        let level = reflect([0.0, 0.0]);
        texels.extend(field.colors.iter().map(|&color| tint(color, level)));
        return;
    };
    for j in 0..rows {
        let (j0, j1) = (j.saturating_sub(1), (j + 1).min(rows - 1));
        for i in 0..columns {
            let (i0, i1) = (i.saturating_sub(1), (i + 1).min(columns - 1));
            let at = |i: usize, j: usize| heights[j * columns + i] * scale;
            let slope = [
                (at(i1, j) - at(i0, j)) / (i1 - i0) as f32,
                (at(i, j1) - at(i, j0)) / (j1 - j0) as f32,
            ];
            texels.push(tint(field.colors[j * columns + i], reflect(slope)));
        }
    }
}

impl Frame {
    /// Draws `field` placed in the world, as `relief` says. The field is shaded here, in Rust,
    /// per node, into the colours both backends sample; a field the last frame also drew with
    /// the same `(key, revision)`, and the same viewer if it has a highlight, is not shaded
    /// again. A field recorded twice in a frame under one stamp is stored once.
    pub fn field(&mut self, camera: &Camera, placement: Placement, field: &Field, relief: Relief) {
        if !field.drawable() {
            return;
        }
        let (columns, rows) = (field.columns, field.rows);
        let steps = placement.steps(columns, rows);
        let scale = match relief {
            Relief::Flat => 1.0,
            Relief::Displaced { scale, .. } => scale,
        };
        let stamp = Stamp {
            revision: field.revision,
            light: field.shading.map(|shading| Lighting {
                steps,
                scale,
                shading,
                viewer: (shading.specular > 0.0).then(|| camera.viewer()),
            }),
        };
        let index = self.take_field(field, stamp);
        let index = u32::try_from(index).expect("a frame draws fewer than 2^32 fields");
        match relief {
            Relief::Flat => self.flat_field(camera, placement, field, index),
            Relief::Displaced { scale, mesh } => {
                self.displaced_field(camera, placement, field, (scale, mesh), steps[2], index);
            }
        }
    }

    /// The index in `self.fields` of `field` shaded under `stamp`: this frame's own if it holds
    /// one, else the last frame's, refreshed, else a new one.
    fn take_field(&mut self, field: &Field, stamp: Stamp) -> usize {
        if let Some(index) = self
            .fields
            .iter()
            .position(|held| held.current(field, &stamp))
        {
            return index;
        }
        let earlier = &mut self.earlier_fields;
        let found = earlier
            .iter()
            .position(|held| held.current(field, &stamp))
            .or_else(|| earlier.iter().position(|held| held.key == field.key));
        let mut held = match found {
            Some(index) => earlier.swap_remove(index),
            None => ShadedField::empty(field.key),
        };
        held.refresh(field, stamp);
        self.fields.push(held);
        self.fields.len() - 1
    }

    fn flat_field(&mut self, camera: &Camera, placement: Placement, field: &Field, index: u32) {
        let (columns, rows) = (field.columns, field.rows);
        let (right, bottom) = ((columns - 1) as f32, (rows - 1) as f32);
        let corners = [[0.0, 0.0], [right, 0.0], [right, bottom], [0.0, bottom]];
        let points =
            corners.map(|[u, v]| self.project(camera, placement.position(columns, rows, u, v)));
        for (a, b, c) in [(0, 1, 2), (0, 2, 3)] {
            self.items.push(Item::Field {
                points: [points[a], points[b], points[c]],
                uv: [corners[a], corners[b], corners[c]],
                field: index,
            });
        }
    }

    /// A mesh of `mesh` vertices per side at most, evenly spread over the node coordinates and
    /// lifted along `normal` by the scaled height at each.
    fn displaced_field(
        &mut self,
        camera: &Camera,
        placement: Placement,
        field: &Field,
        (scale, mesh): (f32, usize),
        normal: Point,
        index: u32,
    ) {
        let (columns, rows) = (field.columns, field.rows);
        let across = mesh.clamp(2, columns);
        let down = mesh.clamp(2, rows);
        let mut vertices = Vec::with_capacity(across * down);
        for b in 0..down {
            let v = (rows - 1) as f32 * b as f32 / (down - 1) as f32;
            for a in 0..across {
                let u = (columns - 1) as f32 * a as f32 / (across - 1) as f32;
                let lift = field.heights.as_ref().map_or(0.0, |heights| {
                    height_at(heights, columns, rows, u, v) * scale
                });
                let world = plus(placement.position(columns, rows, u, v), times(normal, lift));
                vertices.push((self.project(camera, world), [u, v]));
            }
        }
        self.items.reserve(2 * (across - 1) * (down - 1));
        for b in 0..down - 1 {
            for a in 0..across - 1 {
                let corner = |a: usize, b: usize| vertices[b * across + a];
                let [p00, p10, p11, p01] = [
                    corner(a, b),
                    corner(a + 1, b),
                    corner(a + 1, b + 1),
                    corner(a, b + 1),
                ];
                for (p, q, r) in [(p00, p10, p11), (p00, p11, p01)] {
                    self.items.push(Item::Field {
                        points: [p.0, q.0, r.0],
                        uv: [p.1, q.1, r.1],
                        field: index,
                    });
                }
            }
        }
    }

    /// Rasterizes one field triangle on the CPU. Each row's span comes from where the scanline
    /// crosses the triangle's edges, each crossing computed from the edge's endpoints in a fixed
    /// order (top first), so two triangles that share an edge agree on where it is and no pixel
    /// centre falls between them; both include pixels centred on it. `uv` and depth are linear in
    /// screen space, so each is evaluated at the row's first pixel centre and stepped.
    pub(super) fn field_triangle(
        &mut self,
        points: [Point; 3],
        uv: [[f32; 2]; 3],
        field: &ShadedField,
    ) {
        let [a, b, c] = points;
        let (bx, by) = ((b[0] - a[0]) as f64, (b[1] - a[1]) as f64);
        let (cx, cy) = ((c[0] - a[0]) as f64, (c[1] - a[1]) as f64);
        let determinant = bx * cy - cx * by;
        if determinant.abs() < 0.001 {
            return;
        }
        // The plane through the three values: f(x, y) = f_a + gx * (x - a_x) + gy * (y - a_y).
        let plane = |f: [f32; 3]| {
            let (f1, f2) = ((f[1] - f[0]) as f64, (f[2] - f[0]) as f64);
            [
                (f1 * cy - f2 * by) / determinant,
                (f2 * bx - f1 * cx) / determinant,
            ]
        };
        let planes = [
            (uv[0][0], plane([uv[0][0], uv[1][0], uv[2][0]])),
            (uv[0][1], plane([uv[0][1], uv[1][1], uv[2][1]])),
            (a[2], plane([a[2], b[2], c[2]])),
        ];
        let edges = [(a, b), (b, c), (c, a)].map(|(p, q)| Edge::new(p, q));
        let top = a[1].min(b[1]).min(c[1]);
        let bottom = a[1].max(b[1]).max(c[1]);
        let first = ((top - 0.5).ceil() as i64).max(0);
        let last = ((bottom - 0.5).floor() as i64).min(self.height as i64 - 1);
        let limit = [(field.columns - 1) as f32, (field.rows - 1) as f32];
        let width = self.width as usize;
        for y in first..=last {
            let center = y as f32 + 0.5;
            let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
            for edge in edges.iter().flatten() {
                if let Some(x) = edge.crossing(center) {
                    low = low.min(x);
                    high = high.max(x);
                }
            }
            let start = ((low - 0.5).ceil() as i64).max(0);
            let end = ((high - 0.5).floor() as i64).min(width as i64 - 1);
            if start > end {
                continue;
            }
            let [u, v, depth] = planes.map(|(origin, [gx, gy])| {
                let at = origin as f64
                    + gx * (start as f64 + 0.5 - a[0] as f64)
                    + gy * (center as f64 - a[1] as f64);
                (at as f32, gx as f32)
            });
            let (first_pixel, count) = (
                y as usize * width + start as usize,
                (end - start + 1) as usize,
            );
            let span = first_pixel..first_pixel + count;
            sample_span(
                field,
                limit,
                [u, v],
                depth,
                &mut self.depth[span.clone()],
                &mut self.pixels[span.start * 3..span.end * 3],
                &mut self.picks[span],
            );
        }
    }
}

/// An edge of a field triangle that crosses scanlines: its top end first, the x at the top and
/// the change in x per row.
struct Edge {
    top: f32,
    bottom: f32,
    x: f32,
    slope: f32,
}

impl Edge {
    /// None for a horizontal edge, which no scanline crosses but its neighbours' ends cover.
    fn new(a: Point, b: Point) -> Option<Self> {
        let (top, bottom) = if a[1] <= b[1] { (a, b) } else { (b, a) };
        (bottom[1] > top[1]).then(|| Self {
            top: top[1],
            bottom: bottom[1],
            x: top[0],
            slope: (bottom[0] - top[0]) / (bottom[1] - top[1]),
        })
    }

    fn crossing(&self, y: f32) -> Option<f32> {
        (self.top..=self.bottom)
            .contains(&y)
            .then_some(self.x + (y - self.top) * self.slope)
    }
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// One scanline span of a field triangle: for each pixel, if its depth is at least what is
/// drawn there, the sampled colour, the owner of the nearest node and the depth. `u`, `v` and
/// `depth` are each the value at the first pixel centre and the step per pixel.
fn sample_span(
    field: &ShadedField,
    limit: [f32; 2],
    [(mut u, du), (mut v, dv)]: [(f32, f32); 2],
    (mut depth, ddepth): (f32, f32),
    depths: &mut [f32],
    pixels: &mut [u8],
    picks: &mut [u32],
) {
    let columns = field.columns;
    let texel = |i: usize, j: usize| {
        let word = field.texels[j * columns + i];
        [
            (word & 255) as f32,
            ((word >> 8) & 255) as f32,
            ((word >> 16) & 255) as f32,
            0.0,
        ]
    };
    let mut held = ([usize::MAX; 2], [[0.0; 4]; 4]);
    let pixels = pixels.as_chunks_mut::<3>().0;
    for ((nearer, pixel), pick) in depths.iter_mut().zip(pixels).zip(picks) {
        let (here, here_u, here_v) = (depth, u, v);
        depth += ddepth;
        u += du;
        v += dv;
        if here < *nearer {
            continue;
        }
        *nearer = here;
        let ([i, j], [i1, j1], [fx, fy]) = cell(columns, field.rows, here_u, here_v);
        if held.0 != [i, j] {
            held = (
                [i, j],
                [texel(i, j), texel(i1, j), texel(i, j1), texel(i1, j1)],
            );
        }
        let [c00, c10, c01, c11] = held.1;
        let color = lerp4(lerp4(c00, c10, fx), lerp4(c01, c11, fx), fy);
        *pixel = [0, 1, 2].map(|k| (color[k] + 0.5) as u8);
        *pick = field.owners.as_ref().map_or(NONE, |owners| {
            let u = here_u.clamp(0.0, limit[0]);
            let v = here_v.clamp(0.0, limit[1]);
            let nearest = [
                ((u + 0.5) as usize).min(columns - 1),
                ((v + 0.5) as usize).min(field.rows - 1),
            ];
            owners[nearest[1] * columns + nearest[0]]
        });
    }
}

/// Where each field of a frame sits in the GPU storage buffer, and which slots must be written.
/// The slots follow one another in the order the frame records its fields, each holding the
/// header, the colours and the owners (`shared::FIELD_HEADER_WORDS`). A slot is written when the
/// field at that place is not the build the slot holds (or the buffer grew), so a frame whose
/// fields are as the last frame's, shaded or recycled, writes nothing.
#[derive(Default)]
pub struct StorageLayout {
    slots: Vec<Slot>,
}

#[derive(Clone, Copy, PartialEq)]
struct Slot {
    build: u64,
    offset: u32,
}

/// What `StorageLayout::plan` decides for a frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StoragePlan {
    /// The word offset of each field's slot, by index into `Frame::fields`.
    pub offsets: Vec<u32>,
    /// The indices of the fields whose slots must be written.
    pub rewrite: Vec<usize>,
    /// The size in words the buffer needs to grow to, and every slot is then rewritten.
    pub grow: Option<usize>,
}

impl StorageLayout {
    /// Lays out `fields` in a buffer of `capacity` words. A frame with no fields changes
    /// nothing: the buffer keeps what it holds.
    pub fn plan(&mut self, fields: &[ShadedField], capacity: usize) -> StoragePlan {
        if fields.is_empty() {
            return StoragePlan::default();
        }
        let mut offsets = Vec::with_capacity(fields.len());
        let mut slots = Vec::with_capacity(fields.len());
        let mut end = 0;
        for field in fields {
            offsets.push(u32::try_from(end).expect("the storage buffer fits 2^32 words"));
            slots.push(Slot {
                build: field.build,
                offset: end as u32,
            });
            end += field.words();
        }
        let grow = (end > capacity).then(|| end.next_power_of_two());
        let rewrite = (0..fields.len())
            .filter(|&i| grow.is_some() || self.slots.get(i) != Some(&slots[i]))
            .collect();
        self.slots = slots;
        StoragePlan {
            offsets,
            rewrite,
            grow,
        }
    }

    /// Forgets what the buffer holds, so the next plan rewrites every slot.
    pub fn forget(&mut self) {
        self.slots.clear();
    }

    /// The words of the header written at the start of a field's slot.
    pub fn header(field: &ShadedField) -> [u32; shared::FIELD_HEADER_WORDS as usize] {
        [
            field.columns as u32,
            field.rows as u32,
            u32::from(field.owners.is_some()),
            0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Backdrop, Buffers, ORIGIN_Y, SCALE, Sky};
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    use std::time::Instant;

    const WIDTH: u32 = 360;
    const HEIGHT: u32 = 240;

    fn frame_of(width: u32, height: u32, buffers: Buffers) -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Void,
            haze: [0.0; 3],
            time: 0.0,
        };
        Frame::new(width, height, sky, buffers)
    }

    fn frame() -> Frame {
        frame_of(WIDTH, HEIGHT, Buffers::default())
    }

    /// Looks straight down with the world axes along the screen's, `pixels` pixels to a world
    /// unit, and world (x, y) `anchor` at screen position (`at`, `at`) in pixels.
    fn overhead(anchor: [f32; 2], at: f32, pixels: f32) -> Camera {
        Camera {
            center: [
                anchor[0] - (at - WIDTH as f32 / 2.0) / pixels,
                anchor[1] - (at - HEIGHT as f32 * ORIGIN_Y) / pixels,
            ],
            zoom: pixels / SCALE,
            rotation: -FRAC_PI_4,
            pitch: FRAC_PI_2,
            ..Camera::default()
        }
    }

    fn unit_grid(columns: usize, rows: usize) -> Placement {
        Placement::Grid {
            grid: Grid {
                origin: [0.0, 0.0],
                cell: 1.0,
                columns,
                rows,
            },
            z: 0.0,
        }
    }

    fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
        let i = (y * frame.width + x) as usize * 3;
        [frame.pixels[i], frame.pixels[i + 1], frame.pixels[i + 2]]
    }

    fn pick(frame: &Frame, x: u32, y: u32) -> u32 {
        frame.picks[(y * frame.width + x) as usize]
    }

    fn field_items(frame: &Frame) -> usize {
        frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Field { .. }))
            .count()
    }

    /// A field whose node (i, j) is coloured `(i * step, j * step, 40)`.
    fn gradient(columns: usize, rows: usize, step: u8) -> Field {
        let mut field = Field::new(7, columns, rows);
        for j in 0..rows {
            for i in 0..columns {
                field.colors[j * columns + i] = [i as u8 * step, j as u8 * step, 40];
            }
        }
        field
    }

    /// The colour a field's nodes blend to at node coordinates (u, v), computed in f64 from the
    /// definition, to check the rasterizer against.
    fn blended(field: &Field, u: f64, v: f64) -> [f64; 3] {
        let (i, j) = (u.floor() as usize, v.floor() as usize);
        let (fx, fy) = (u - i as f64, v - j as f64);
        let at = |i: usize, j: usize| {
            field.colors[j.min(field.rows - 1) * field.columns + i.min(field.columns - 1)]
                .map(f64::from)
        };
        let (c00, c10, c01, c11) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        [0, 1, 2].map(|k| {
            let top = c00[k] * (1.0 - fx) + c10[k] * fx;
            let bottom = c01[k] * (1.0 - fx) + c11[k] * fx;
            top * (1.0 - fy) + bottom * fy
        })
    }

    #[test]
    fn a_flat_field_shows_node_colours_at_nodes_and_blends_between_them() {
        // Node (i, j) is at the centre of pixel (20 + 16 i, 20 + 16 j). The outer nodes sit on
        // the field's border, where a pixel centre is a tie, so the checks stay inside it.
        let field = gradient(5, 4, 20);
        let camera = overhead([0.0, 0.0], 20.5, 16.0);
        let mut frame = frame();
        frame.field(&camera, unit_grid(5, 4), &field, Relief::Flat);
        frame.rasterize();
        for j in 1..3 {
            for i in 1..4 {
                let color = pixel(&frame, 20 + 16 * i, 20 + 16 * j);
                assert_eq!(color, [i as u8 * 20, j as u8 * 20, 40], "node ({i}, {j})");
            }
        }
        // A quarter, and the middle, of the way from node (1, 1) to node (2, 1), and down to (1, 2).
        assert_eq!(pixel(&frame, 36 + 4, 36), [25, 20, 40]);
        assert_eq!(pixel(&frame, 36 + 8, 36), [30, 20, 40]);
        assert_eq!(pixel(&frame, 36 + 8, 36 + 8), [30, 30, 40]);
        assert_eq!(pixel(&frame, 36 + 12, 36 + 4), [35, 25, 40]);
        // Beyond the border is the sky.
        assert_eq!(pixel(&frame, 20 + 16 * 4 + 6, 36), [0, 0, 0]);
        assert_eq!(pixel(&frame, 36, 20 - 6), [0, 0, 0]);
    }

    #[test]
    fn field_picks_name_the_owner_of_the_nearest_node() {
        let mut field = gradient(5, 4, 20);
        field.owners = Some((0..20).map(|node| 1000 + node).collect());
        let camera = overhead([0.0, 0.0], 20.5, 16.0);
        let mut frame = frame();
        frame.field(&camera, unit_grid(5, 4), &field, Relief::Flat);
        frame.rasterize();
        let owner = |i: u32, j: u32| 1000 + j * 5 + i;
        // Node (2, 1) is at pixel (52, 36); seven pixels short of halfway is still its own.
        assert_eq!(pick(&frame, 52, 36), owner(2, 1));
        assert_eq!(pick(&frame, 52 + 7, 36), owner(2, 1));
        assert_eq!(pick(&frame, 52 - 7, 36), owner(2, 1));
        assert_eq!(pick(&frame, 52, 36 + 7), owner(2, 1));
        // Nine pixels along, the next node is nearer.
        assert_eq!(pick(&frame, 52 + 9, 36), owner(3, 1));
        assert_eq!(pick(&frame, 52 - 9, 36), owner(1, 1));
        assert_eq!(pick(&frame, 52, 36 + 9), owner(2, 2));
        assert_eq!(pick(&frame, 52 + 9, 36 + 9), owner(3, 2));
        assert_eq!(pick(&frame, 10, 10), NONE);
    }

    #[test]
    fn a_field_without_owners_picks_none() {
        let field = gradient(5, 4, 20);
        let camera = overhead([0.0, 0.0], 20.5, 16.0);
        let mut frame = frame();
        // A pickable triangle under the whole frame, which the field covers where it lies.
        frame.items.push(Item::Triangle(
            [
                [0.0, 0.0, -1.0],
                [WIDTH as f32 * 2.0, 0.0, -1.0],
                [0.0, HEIGHT as f32 * 2.0, -1.0],
            ],
            [200, 0, 0],
            5,
        ));
        frame.field(&camera, unit_grid(5, 4), &field, Relief::Flat);
        frame.rasterize();
        assert_eq!(pixel(&frame, 52, 36), [40, 20, 40]);
        assert_eq!(
            pick(&frame, 52, 36),
            NONE,
            "the field clears the pick below it"
        );
        assert_eq!(
            pick(&frame, 300, 200),
            5,
            "outside the field the triangle still picks"
        );
    }

    #[test]
    fn a_field_placed_by_four_corners_maps_its_corner_nodes_to_those_corners() {
        // A vertical panel facing the camera, wider than tall, seen from above and the side.
        let corners = [
            [-30.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            [30.0, 0.0, 40.0],
            [-30.0, 0.0, 40.0],
        ];
        let mut field = Field::new(3, 4, 3);
        for (node, color) in field.colors.iter_mut().enumerate() {
            *color = [
                (node * 37 % 256) as u8,
                (node * 91 % 256) as u8,
                (node * 53 % 256) as u8,
            ];
        }
        let camera = Camera {
            zoom: 2.0,
            center: [0.0, 0.0],
            ..Camera::default()
        };
        let mut frame = frame();
        frame.field(&camera, Placement::Corners(corners), &field, Relief::Flat);
        frame.rasterize();
        let screen = corners.map(|corner| frame.project(&camera, corner));
        // Every pixel well inside the quad shows the colour its (s, t) over the corners gives.
        let (across, up) = (
            [screen[1][0] - screen[0][0], screen[1][1] - screen[0][1]],
            [screen[3][0] - screen[0][0], screen[3][1] - screen[0][1]],
        );
        let determinant = (across[0] * up[1] - across[1] * up[0]) as f64;
        let mut checked = 0;
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let (dx, dy) = (x as f32 + 0.5 - screen[0][0], y as f32 + 0.5 - screen[0][1]);
                let s = (dx * up[1] - dy * up[0]) as f64 / determinant;
                let t = (across[0] * dy - across[1] * dx) as f64 / determinant;
                if !(0.01..=0.99).contains(&s) || !(0.01..=0.99).contains(&t) {
                    continue;
                }
                let want = blended(&field, s * 3.0, t * 2.0);
                let got = pixel(&frame, x, y).map(f64::from);
                for k in 0..3 {
                    assert!(
                        (got[k] - want[k]).abs() <= 1.0,
                        "pixel ({x}, {y}) at ({s:.3}, {t:.3}): {got:?} against {want:?}"
                    );
                }
                checked += 1;
            }
        }
        assert!(
            checked > 5000,
            "only {checked} pixels were inside the panel"
        );
        // The pixel just inside each corner is nearest that corner's own node colour.
        let centre = [
            screen.iter().map(|p| p[0]).sum::<f32>() / 4.0,
            screen.iter().map(|p| p[1]).sum::<f32>() / 4.0,
        ];
        let nodes = [0, 3, 11, 8];
        for (corner, node) in nodes.into_iter().enumerate() {
            let inward = |at: f32, to: f32| at + (to - at) * 0.04;
            let (x, y) = (
                inward(screen[corner][0], centre[0]),
                inward(screen[corner][1], centre[1]),
            );
            let got = pixel(&frame, x as u32, y as u32).map(i32::from);
            let distance = |color: Color| -> i32 {
                (0..3).map(|k| (i32::from(color[k]) - got[k]).pow(2)).sum()
            };
            let nearest = (0..12)
                .min_by_key(|&other| distance(field.colors[other]))
                .expect("a field has nodes");
            assert_eq!(nearest, node, "corner {corner} shows node {node}'s colour");
        }
    }

    #[test]
    fn a_relief_field_hides_what_lies_behind_its_ridges() {
        // A ridge 12 high across the middle of a 40-unit square, seen from low in front.
        let mut field = Field::new(4, 41, 41);
        field.colors.fill([30, 160, 60]);
        let mut heights = vec![0.0; 41 * 41];
        for j in 19..=21 {
            heights[j * 41..(j + 1) * 41].fill(12.0);
        }
        field.heights = Some(heights);
        let camera = Camera {
            center: [20.0, 20.0],
            zoom: 4.0,
            rotation: -FRAC_PI_4,
            pitch: crate::render::LOWEST_PITCH,
            ..Camera::default()
        };
        let marker = |y: f32| [[10.0, y, 0.5], [30.0, y, 0.5], [20.0, y + 6.0, 0.5]];
        let red = [250, 20, 20];
        let draw = |relief: Relief| {
            let mut frame = frame();
            // One marker behind the ridge, one in front of it, both a little above the ground.
            frame.facet(&camera, marker(4.0), red, 0);
            frame.facet(&camera, marker(30.0), red, 0);
            frame.field(&camera, unit_grid(41, 41), &field, relief);
            frame.rasterize();
            let seen = |y: f32| {
                let at = frame.project(&camera, [20.0, y + 2.0, 0.5]);
                pixel(&frame, at[0] as u32, at[1] as u32)
            };
            (seen(4.0), seen(30.0))
        };
        let (behind, front) = draw(Relief::Flat);
        assert_eq!(
            (behind, front),
            (red, red),
            "on flat ground both markers show"
        );
        let (behind, front) = draw(Relief::displaced(1.0));
        assert_eq!(
            behind,
            [30, 160, 60],
            "the ridge covers the marker behind it"
        );
        assert_eq!(front, red, "the marker in front of the ridge still shows");
    }

    #[test]
    fn relief_mesh_size_is_capped_whatever_the_grid_size() {
        let camera = overhead([0.0, 0.0], 20.0, 1.0);
        let count = |columns: usize, rows: usize, relief: Relief| {
            let mut frame = frame();
            let field = Field::new(1, columns, rows);
            frame.field(&camera, unit_grid(columns, rows), &field, relief);
            field_items(&frame)
        };
        let side = |vertices: usize| 2 * (vertices - 1);
        assert_eq!(
            count(1024, 1024, Relief::displaced(1.0)),
            side(80) * (80 - 1)
        );
        assert_eq!(count(1024, 1024, Relief::displaced(1.0)), 2 * 79 * 79);
        assert_eq!(count(1024, 16, Relief::displaced(1.0)), 2 * 79 * 15);
        assert_eq!(
            count(
                1024,
                1024,
                Relief::Displaced {
                    scale: 1.0,
                    mesh: 5
                }
            ),
            32
        );
        assert_eq!(
            count(3, 3, Relief::displaced(1.0)),
            8,
            "clamped to the grid"
        );
        assert_eq!(count(1024, 1024, Relief::Flat), 2, "flat is two triangles");
        assert_eq!(count(384, 384, Relief::Flat), 2);
    }

    /// The texel of the middle node of a 9 x 9 field of grey sloping `rise` per node along x.
    fn lit_middle(rise: f32, shading: Shading, scale: f32) -> [u8; 3] {
        let mut field = Field::new(2, 9, 9);
        field.colors.fill([200, 200, 200]);
        field.heights = Some((0..81).map(|node| rise * (node % 9) as f32).collect());
        field.shading = Some(shading);
        let mut frame = frame();
        let relief = if scale == 1.0 {
            Relief::Flat
        } else {
            Relief::Displaced { scale, mesh: 9 }
        };
        frame.field(&Camera::default(), unit_grid(9, 9), &field, relief);
        let texel = frame.fields[0].texels[4 * 9 + 4];
        let [r, g, b, a] = texel.to_le_bytes();
        assert_eq!(a, 255);
        [r, g, b]
    }

    #[test]
    fn shading_darkens_slopes_that_face_away_from_the_light() {
        let matte = Shading {
            ambient: 0.3,
            diffuse: 0.9,
            specular: 0.0,
            shininess: 1.0,
        };
        // LIGHT leans towards -x, so ground that falls away towards -x faces it.
        let towards = lit_middle(0.5, matte, 1.0)[0];
        let level = lit_middle(0.0, matte, 1.0)[0];
        let away = lit_middle(-0.5, matte, 1.0)[0];
        assert!(
            towards > level && level > away,
            "{towards} > {level} > {away}"
        );
        let light = normalize(crate::render::LIGHT);
        let expected = |normal: Point| {
            let normal = normalize(normal);
            (200.0 * (0.3 + 0.9 * dot(normal, light).max(0.0)) + 0.5) as u8
        };
        assert_eq!(level, expected([0.0, 0.0, 1.0]));
        assert_eq!(towards, expected([-0.5, 0.0, 1.0]));
        assert_eq!(away, expected([0.5, 0.0, 1.0]));
        // Facing wholly away leaves the ambient light.
        assert_eq!(lit_middle(-5.0, matte, 1.0), [60, 60, 60]);
        // A displaced field is lit as the surface it is: its heights scaled.
        assert_eq!(lit_middle(1.0, matte, 0.5), lit_middle(0.5, matte, 1.0));
    }

    #[test]
    fn a_highlight_brightens_the_node_that_mirrors_the_light_to_the_viewer() {
        let glossy = Shading {
            ambient: 0.2,
            diffuse: 0.5,
            specular: 0.8,
            shininess: 24.0,
        };
        let matte = Shading {
            specular: 0.0,
            ..glossy
        };
        let make = |shading| {
            let mut field = Field::new(2, 3, 3);
            field.colors.fill([100, 100, 100]);
            field.shading = Some(shading);
            field
        };
        let shine = |camera: &Camera, shading| {
            let mut frame = frame();
            frame.field(camera, unit_grid(3, 3), &make(shading), Relief::Flat);
            frame.fields[0].texels[4]
        };
        let overhead_view = Camera {
            pitch: FRAC_PI_2,
            ..Camera::default()
        };
        let low_view = Camera {
            pitch: crate::render::LOWEST_PITCH,
            ..Camera::default()
        };
        assert_ne!(
            shine(&overhead_view, glossy),
            shine(&low_view, glossy),
            "the highlight moves with the viewer"
        );
        assert_eq!(
            shine(&overhead_view, matte),
            shine(&low_view, matte),
            "without a highlight the viewer does not matter"
        );
        assert!(
            shine(&overhead_view, glossy).to_le_bytes()[0]
                > shine(&overhead_view, matte).to_le_bytes()[0]
        );
    }

    #[test]
    fn an_unchanged_field_is_not_reshaded_or_reuploaded() {
        let camera = overhead([0.0, 0.0], 20.0, 8.0);
        let mut field = gradient(8, 8, 10);
        field.heights = Some((0..64).map(|node| (node % 8) as f32 * 0.1).collect());
        field.owners = Some((0..64).collect());
        field.shading = Some(Shading {
            ambient: 0.4,
            diffuse: 0.7,
            specular: 0.0,
            shininess: 1.0,
        });
        let mut layout = StorageLayout::default();
        let mut buffers = Buffers::default();
        let mut last_build = 0;
        // Records the field in a frame built from `buffers`, and returns whether it was shaded
        // since the last call and which slots the GPU would write.
        let mut record = |camera: &Camera, field: &Field, buffers: &mut Buffers| {
            let mut frame = frame_of(WIDTH, HEIGHT, std::mem::take(buffers));
            frame.field(camera, unit_grid(8, 8), field, Relief::Flat);
            let build = frame.fields[0].build;
            let plan = layout.plan(&frame.fields, 1 << 20);
            *buffers = frame.release();
            let shaded = build != last_build;
            last_build = build;
            (shaded, plan.rewrite)
        };
        assert_eq!(
            record(&camera, &field, &mut buffers),
            (true, vec![0]),
            "the first frame shades and writes the field"
        );
        // The same field under a still camera, and under one that panned and zoomed.
        assert_eq!(record(&camera, &field, &mut buffers), (false, vec![]));
        let moved = overhead([3.0, 2.0], 80.0, 5.0);
        assert_eq!(record(&moved, &field, &mut buffers), (false, vec![]));
        // A new revision shades and writes it again, once.
        field.revision += 1;
        assert_eq!(record(&camera, &field, &mut buffers), (true, vec![0]));
        assert_eq!(record(&camera, &field, &mut buffers), (false, vec![]));
        // Turning the camera does not change a field with no highlight ...
        let turned = Camera {
            rotation: 0.4,
            ..camera.clone()
        };
        assert_eq!(record(&turned, &field, &mut buffers), (false, vec![]));
        // ... but does change one that has.
        field.shading = field.shading.map(|shading| Shading {
            specular: 0.5,
            shininess: 8.0,
            ..shading
        });
        field.revision += 1;
        assert_eq!(record(&camera, &field, &mut buffers), (true, vec![0]));
        assert_eq!(
            record(&moved, &field, &mut buffers),
            (false, vec![]),
            "panning keeps the viewer"
        );
        assert_eq!(
            record(&turned, &field, &mut buffers),
            (true, vec![0]),
            "turning moves it"
        );
        // A frame that lost the last frame's fields does not trust the key: it shades the field
        // again, and the GPU writes it again.
        assert_eq!(
            record(&turned, &field, &mut Buffers::default()),
            (true, vec![0])
        );
    }

    #[test]
    fn the_storage_plan_rewrites_only_the_slots_that_changed_and_grows_when_short() {
        let mut buffers = Buffers::default();
        let mut layout = StorageLayout::default();
        // Records `fields` in a frame that recycles the last one's and plans their storage in a
        // buffer of `capacity` words.
        let mut plan = |layout: &mut StorageLayout, fields: &[Field], capacity: usize| {
            let camera = overhead([0.0, 0.0], 20.0, 8.0);
            let mut frame = frame_of(WIDTH, HEIGHT, std::mem::take(&mut buffers));
            for field in fields {
                frame.field(&camera, unit_grid(4, 4), field, Relief::Flat);
            }
            let plan = layout.plan(&frame.fields, capacity);
            buffers = frame.release();
            plan
        };
        let (mut first, mut second) = (gradient(4, 4, 10), gradient(4, 4, 20));
        second.key = 8;
        let nodes = 16;
        let words = shared::FIELD_HEADER_WORDS as usize + nodes;
        let both = plan(&mut layout, &[first.clone(), second.clone()], 4);
        assert_eq!(both.offsets, [0, words as u32]);
        assert_eq!(both.rewrite, [0, 1]);
        assert_eq!(both.grow, Some((2 * words).next_power_of_two()));
        // With room, nothing moves and nothing is written ...
        let again = plan(&mut layout, &[first.clone(), second.clone()], 1 << 10);
        assert_eq!(again.rewrite, Vec::<usize>::new());
        assert_eq!(again.grow, None);
        // ... until one changes, which rewrites its own slot only.
        second.revision = 1;
        let changed = plan(&mut layout, &[first.clone(), second.clone()], 1 << 10);
        assert_eq!(changed.rewrite, [1]);
        // A frame with no fields leaves the buffer as it is, but drops the fields it recycled,
        // so those that return are shaded again and written again.
        assert_eq!(plan(&mut layout, &[], 1 << 10), StoragePlan::default());
        let back = plan(&mut layout, &[first.clone(), second], 1 << 10);
        assert_eq!(back.rewrite, [0, 1]);
        // Owners add words to a slot.
        first.owners = Some(vec![NONE; nodes]);
        first.revision = 1;
        let owned = plan(&mut layout, &[first], 1 << 10);
        assert_eq!(owned.rewrite, [0]);
        layout.forget();
        let forgotten = plan(&mut layout, &[gradient(4, 4, 5)], 1 << 10);
        assert_eq!(
            forgotten.rewrite,
            [0],
            "a forgotten layout writes everything"
        );
    }

    #[test]
    fn the_two_triangles_of_a_flat_field_leave_no_gap_on_their_diagonal() {
        // A 100 x 100 pixel square on the pixel grid: its diagonal runs through pixel centres.
        let mut field = Field::new(5, 2, 2);
        field.colors.fill([255, 255, 255]);
        let square = Placement::Grid {
            grid: Grid {
                origin: [0.0, 0.0],
                cell: 100.0,
                columns: 2,
                rows: 2,
            },
            z: 0.0,
        };
        let mut frame = frame();
        frame.field(
            &overhead([0.0, 0.0], 10.0, 1.0),
            square,
            &field,
            Relief::Flat,
        );
        frame.rasterize();
        let covered = (10..110)
            .flat_map(|y| (10..110).map(move |x| (x, y)))
            .filter(|&(x, y)| pixel(&frame, x, y) == [255, 255, 255])
            .count();
        assert_eq!(covered, 100 * 100, "every pixel of the square is drawn");
        // And a wider rectangle whose diagonal misses the centres.
        let mut frame = frame_of(WIDTH, HEIGHT, Buffers::default());
        let wide = Placement::Corners([
            [0.0, 0.0, 0.0],
            [137.0, 0.0, 0.0],
            [137.0, 61.0, 0.0],
            [0.0, 61.0, 0.0],
        ]);
        frame.field(&overhead([0.0, 0.0], 10.0, 1.0), wide, &field, Relief::Flat);
        frame.rasterize();
        let covered = (0..HEIGHT)
            .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| pixel(&frame, x, y) == [255, 255, 255])
            .count();
        assert_eq!(covered, 137 * 61);
    }

    #[test]
    fn a_displaced_field_without_heights_draws_what_the_flat_one_does() {
        let mut field = gradient(6, 5, 30);
        field.owners = Some((0..30).collect());
        // A quarter-pixel off the grid, so that no border passes through a pixel centre.
        let camera = overhead([0.0, 0.0], 20.25, 16.0);
        let draw = |relief| {
            let mut frame = frame();
            frame.field(&camera, unit_grid(6, 5), &field, relief);
            frame.rasterize();
            frame
        };
        let flat = draw(Relief::Flat);
        let meshed = draw(Relief::displaced(3.0));
        assert!(field_items(&meshed) == 2 * 5 * 4);
        let mut differing = 0;
        for (a, b) in flat.pixels.iter().zip(&meshed.pixels) {
            differing += usize::from(a.abs_diff(*b) > 1);
        }
        assert_eq!(differing, 0, "the colours agree to a level");
        let owners_differ = flat
            .picks
            .iter()
            .zip(&meshed.picks)
            .filter(|(a, b)| a != b)
            .count();
        assert!(owners_differ <= 3, "{owners_differ} picks differ");
    }

    #[test]
    fn a_512_by_512_field_renders_every_pixel_it_covers() {
        let mut field = Field::new(6, 512, 512);
        field.colors = (0..512 * 512)
            .map(|node| [(node % 512 / 2) as u8, (node / 512 / 2) as u8, 90])
            .collect();
        field.heights = Some(
            (0..512 * 512)
                .map(|node| ((node % 37) as f32).sin())
                .collect(),
        );
        field.owners = Some((0..512 * 512).collect());
        field.shading = Some(Shading {
            ambient: 0.5,
            diffuse: 0.6,
            specular: 0.2,
            shininess: 12.0,
        });
        let corners = [
            [-170.0, -100.0, 0.0],
            [170.0, -100.0, 0.0],
            [170.0, 100.0, 0.0],
            [-170.0, 100.0, 0.0],
        ];
        let camera = Camera {
            center: [0.0, 0.0],
            zoom: 1.0 / SCALE,
            rotation: -FRAC_PI_4,
            pitch: FRAC_PI_2,
            ..Camera::default()
        };
        let mut frame = frame();
        frame.field(
            &camera,
            Placement::Corners(corners),
            &field,
            Relief::displaced(0.0),
        );
        frame.rasterize();
        let covered = frame.picks.iter().filter(|&&owner| owner != NONE).count();
        assert_eq!(covered, 340 * 200, "every pixel of the quad has an owner");
        let at = frame.project(&camera, [0.0, 0.0, 0.0]);
        // The node at the middle owns the middle pixel.
        let owner = pick(&frame, at[0] as u32, at[1] as u32) as usize;
        assert!((owner % 512).abs_diff(256) <= 2 && (owner / 512).abs_diff(256) <= 2);
    }

    #[test]
    #[ignore = "a timing; run with `cargo test --release a_screen_filling_field -- --ignored --nocapture`"]
    fn a_screen_filling_field_rasterizes_within_budget() {
        // A 384 x 384 field stretched over nearly all of a 1920 x 1080 frame, looking straight
        // down so that the triangles of the mesh are as large as they get.
        let (width, height) = (1920, 1080);
        let mut field = Field::new(9, 384, 384);
        for (node, color) in field.colors.iter_mut().enumerate() {
            let (i, j) = (node % 384, node / 384);
            *color = [
                (i * 255 / 383) as u8,
                (j * 255 / 383) as u8,
                ((i + j) % 256) as u8,
            ];
        }
        field.heights = Some(
            (0..384 * 384)
                .map(|node| ((node % 384) as f32 * 0.07).sin() * ((node / 384) as f32 * 0.05).cos())
                .collect(),
        );
        field.owners = Some((0..384 * 384).map(|node| node as u32 % 192).collect());
        field.shading = Some(Shading {
            ambient: 0.4,
            diffuse: 0.7,
            specular: 0.3,
            shininess: 16.0,
        });
        let corners = Placement::Corners([
            [-940.0, -520.0, 0.0],
            [940.0, -520.0, 0.0],
            [940.0, 520.0, 0.0],
            [-940.0, 520.0, 0.0],
        ]);
        let overhead = Camera {
            center: [0.0, 0.0],
            zoom: 1.0 / SCALE,
            rotation: -FRAC_PI_4,
            pitch: FRAC_PI_2,
            ..Camera::default()
        };
        // The same field laid on the ground and seen at an angle, its relief high enough to
        // fold over itself, which is more overdraw than a flat view of it.
        let ground = Placement::Grid {
            grid: Grid {
                origin: [-192.0, -192.0],
                cell: 1.0,
                columns: 384,
                rows: 384,
            },
            z: 0.0,
        };
        let tilted = Camera {
            center: [0.0, 0.0],
            zoom: 3.0,
            rotation: 0.3,
            pitch: 0.7,
            ..Camera::default()
        };
        // Records the field into a fresh frame, then rasterizes it: returns what each took, in
        // milliseconds, and how many pixels the field covers.
        let run = |buffers: &mut Buffers, camera: &Camera, placement, relief, draw: bool| {
            let mut frame = frame_of(width, height, std::mem::take(buffers));
            let start = Instant::now();
            if draw {
                frame.field(camera, placement, &field, relief);
            }
            let recorded = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            frame.rasterize();
            let rasterized = start.elapsed().as_secs_f64() * 1000.0;
            let covered = frame.picks.iter().filter(|&&pick| pick != NONE).count();
            *buffers = frame.release();
            (recorded, rasterized, covered)
        };
        let measure = |camera: &Camera, placement, relief: Relief| {
            // The first frame shades the field; the ones after reuse it.
            let mut buffers = Buffers::default();
            let (first, _, covered) = run(&mut buffers, camera, placement, relief, true);
            let best = |draw: bool, pick: fn((f64, f64, usize)) -> f64| {
                let mut buffers = Buffers::default();
                run(&mut buffers, camera, placement, relief, true);
                (0..9)
                    .map(|_| pick(run(&mut buffers, camera, placement, relief, draw)))
                    .fold(f64::INFINITY, f64::min)
            };
            (
                first,
                best(true, |run| run.0),
                best(true, |run| run.1),
                best(false, |run| run.1),
                covered,
            )
        };
        let (first, record, flat, sky, covered) = measure(&overhead, corners, Relief::Flat);
        println!(
            "field budget, 384 x 384 nodes, 1920 x 1080, {:.0} % of the frame: sky alone {sky:.2} ms, flat {flat:.2} ms (target 20)",
            100.0 * covered as f64 / (width * height) as f64
        );
        println!(
            "  recording and shading the first frame {first:.2} ms, an unchanged one {record:.3} ms"
        );
        let (_, _, relief, _, covered) = measure(&overhead, corners, Relief::displaced(6.0));
        println!(
            "  relief with mesh {MESH}, from above, {:.0} % of the frame: {relief:.2} ms (target 35)",
            100.0 * covered as f64 / (width * height) as f64
        );
        let (_, _, tilted_flat, _, covered) = measure(&tilted, ground, Relief::Flat);
        let (_, _, tilted_relief, _, _) = measure(&tilted, ground, Relief::displaced(60.0));
        println!(
            "  on the ground at an angle, {:.0} % of the frame: flat {tilted_flat:.2} ms, relief {tilted_relief:.2} ms",
            100.0 * covered as f64 / (width * height) as f64
        );
        if !cfg!(debug_assertions) {
            assert!(flat <= 20.0, "flat took {flat:.2} ms, over 20 ms");
            assert!(relief <= 35.0, "relief took {relief:.2} ms, over 35 ms");
        }
    }
}
