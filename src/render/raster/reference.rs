//! The rasterizer as it stood before the banded one replaced it, kept only so the differential
//! test can hold the new one to it byte for byte. It is the old code verbatim: single threaded,
//! every pixel of a triangle's box evaluated, `hypot` per pixel of a halo. Only the names change
//! (`reference_` before each method) and the annotation functions `stroke` and `circle`, which
//! run on a finished frame, are left out. Do not tidy it, speed it up or "fix" it: its whole
//! value is that it is the old behaviour.

use super::super::field::ShadedField;
use super::super::frame::{BACKGROUND, Frame, Item};
use super::super::{Color, NONE, Point, shared};
use crate::glyphs;
use crate::palette::tint;

impl Frame {
    /// Rasterizes the recorded items on the CPU into pixels, depth and picking buffers.
    pub(super) fn reference_rasterize(&mut self) {
        let n = (self.width * self.height) as usize;
        // Every pixel is painted by the sky, so only the length matters here.
        self.pixels.resize(n * 3, 0);
        self.depth.clear();
        self.depth.resize(n, f32::NEG_INFINITY);
        self.picks.clear();
        self.picks.resize(n, NONE);
        self.reference_paint_sky();
        let items = std::mem::take(&mut self.items);
        let fields = std::mem::take(&mut self.fields);
        for item in &items {
            match *item {
                Item::Triangle([a, b, c], color, pick) => {
                    self.reference_triangle(a, b, c, color, pick)
                }
                Item::Line(a, b, color) => self.reference_segment(a, b, color),
                Item::Sphere {
                    center,
                    radius,
                    depth,
                    color,
                    pick,
                    selected,
                } => self.reference_ball(center, radius, depth, color, pick, selected),
                Item::Glow {
                    center,
                    radius,
                    color,
                    strength,
                } => self.reference_halo(center, radius, color, strength),
                Item::Puff {
                    center,
                    radius,
                    color,
                    strength,
                } => self.reference_puff(center, radius, color, strength.clamp(0.0, 1.0)),
                Item::Beam(a, b, color, weight) => self.reference_ray(a, b, color, weight),
                Item::Star([x, y], color) => {
                    self.reference_pixel(x as i32, y as i32, BACKGROUND, color, NONE)
                }
                Item::Glyph {
                    origin,
                    scale,
                    bits,
                    color,
                } => self.reference_glyph(origin, scale, bits, color),
                Item::Pick {
                    center,
                    radius,
                    depth,
                    pick,
                } => self.reference_splat(center, radius, depth, pick),
                Item::Field { points, uv, field } => {
                    self.reference_field_triangle(points, uv, &fields[field as usize]);
                }
            }
        }
        self.items = items;
        self.fields = fields;
    }

    fn reference_glyph(&mut self, origin: [f32; 2], scale: u32, bits: u128, color: Color) {
        for row in 0..glyphs::HEIGHT {
            for column in 0..glyphs::WIDTH {
                if bits >> (row * glyphs::WIDTH + column) & 1 == 0 {
                    continue;
                }
                for dy in 0..scale {
                    let y = origin[1] as i64 + (row * scale + dy) as i64;
                    if !(0..self.height as i64).contains(&y) {
                        continue;
                    }
                    for dx in 0..scale {
                        let x = origin[0] as i64 + (column * scale + dx) as i64;
                        if (0..self.width as i64).contains(&x) {
                            let i = (y as usize * self.width as usize + x as usize) * 3;
                            self.pixels[i..i + 3].copy_from_slice(&color);
                        }
                    }
                }
            }
        }
    }

    fn reference_paint_sky(&mut self) {
        let (w, h) = (self.width as usize, self.height as usize);
        let sky = self.sky;
        let columns: Vec<f32> = (0..w)
            .map(|x| sky.profile(x as f32 / w as f32, 0.0))
            .collect();
        for y in 0..h {
            let v = y as f32 / h as f32;
            let base = sky.base(v);
            let row = sky.profile(v, 1.9);
            let line = &mut self.pixels[y * w * 3..(y + 1) * w * 3];
            if sky.hazy() {
                for (pixel, column) in line.as_chunks_mut::<3>().0.iter_mut().zip(&columns) {
                    let density = row * column;
                    for k in 0..3 {
                        pixel[k] = (base[k] + sky.haze[k] * density).min(255.0) as u8;
                    }
                }
            } else {
                let color = base.map(|c| c as u8);
                for pixel in line.as_chunks_mut::<3>().0 {
                    *pixel = color;
                }
            }
        }
    }

    fn reference_blend(&mut self, x: i32, y: i32, color: Color, weight: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 3;
        for (channel, &target) in self.pixels[i..i + 3].iter_mut().zip(&color) {
            *channel = (*channel as f32 + (target as f32 - *channel as f32) * weight) as u8;
        }
    }

    /// `blend` rounded to the nearest level, as the GPU's blend unit rounds.
    fn reference_blend_rounded(&mut self, x: i32, y: i32, color: Color, weight: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 3;
        for (channel, &target) in self.pixels[i..i + 3].iter_mut().zip(&color) {
            *channel = (*channel as f32 + (target as f32 - *channel as f32) * weight + 0.5) as u8;
        }
    }

    fn reference_pixel(&mut self, x: i32, y: i32, depth: f32, color: Color, pick: u32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = y as usize * self.width as usize + x as usize;
        if depth < self.depth[i] {
            return;
        }
        self.depth[i] = depth;
        self.picks[i] = pick;
        self.pixels[i * 3..i * 3 + 3].copy_from_slice(&color);
    }

    fn reference_triangle(&mut self, a: Point, b: Point, c: Point, color: Color, pick: u32) {
        let edge = |a: Point, b: Point, x: f32, y: f32| {
            (x - a[0]) * (b[1] - a[1]) - (y - a[1]) * (b[0] - a[0])
        };
        let area = edge(a, b, c[0], c[1]);
        if area.abs() < 0.001 {
            return;
        }
        let min_x = a[0].min(b[0]).min(c[0]).floor().max(0.0) as i32;
        let max_x = a[0].max(b[0]).max(c[0]).ceil().min(self.width as f32 - 1.0) as i32;
        let min_y = a[1].min(b[1]).min(c[1]).floor().max(0.0) as i32;
        let max_y = a[1]
            .max(b[1])
            .max(c[1])
            .ceil()
            .min(self.height as f32 - 1.0) as i32;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let w0 = edge(b, c, x as f32 + 0.5, y as f32 + 0.5) / area;
                let w1 = edge(c, a, x as f32 + 0.5, y as f32 + 0.5) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    self.reference_pixel(x, y, w0 * a[2] + w1 * b[2] + w2 * c[2], color, pick);
                }
            }
        }
    }

    fn reference_segment(&mut self, a: Point, b: Point, color: Color) {
        let steps = (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            self.reference_pixel(
                (a[0] + (b[0] - a[0]) * t) as i32,
                (a[1] + (b[1] - a[1]) * t) as i32,
                a[2] + (b[2] - a[2]) * t + shared::LINE_DEPTH_BIAS,
                color,
                NONE,
            );
        }
    }

    /// Writes `pick` over a disc where `depth` is at least as near as what is drawn, as a
    /// sphere's surface tests, leaving colour and depth alone. Samples at integer pixel
    /// coordinates like `ball`.
    fn reference_splat(&mut self, center: [f32; 2], radius: f32, depth: f32, pick: u32) {
        let x0 = (center[0] - radius).floor().max(0.0) as i32;
        let x1 = (center[0] + radius).ceil().min(self.width as f32 - 1.0) as i32;
        let y0 = (center[1] - radius).floor().max(0.0) as i32;
        let y1 = (center[1] + radius).ceil().min(self.height as f32 - 1.0) as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let nx = (x as f32 - center[0]) / radius;
                let ny = (y as f32 - center[1]) / radius;
                let i = y as usize * self.width as usize + x as usize;
                if nx * nx + ny * ny <= 1.0 && depth >= self.depth[i] {
                    self.picks[i] = pick;
                }
            }
        }
    }

    fn reference_ball(
        &mut self,
        p: Point,
        r: f32,
        depth: f32,
        color: Color,
        pick: u32,
        selected: bool,
    ) {
        let outer = if selected {
            r + shared::SELECTION_RING_WIDTH
        } else {
            r + shared::SPHERE_EDGE
        };
        let x0 = (p[0] - outer).floor().max(0.0) as i32;
        let x1 = (p[0] + outer).ceil().min(self.width as f32 - 1.0) as i32;
        let y0 = (p[1] - outer).floor().max(0.0) as i32;
        let y1 = (p[1] + outer).ceil().min(self.height as f32 - 1.0) as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let nx = (x as f32 - p[0]) / r;
                let ny = (y as f32 - p[1]) / r;
                let d = nx * nx + ny * ny;
                if d <= 1.0 {
                    let nz = (1.0 - d).sqrt();
                    let light = (shared::SPHERE_AMBIENT
                        + (-nx * shared::SPHERE_LIGHT_X - ny * shared::SPHERE_LIGHT_Y
                            + nz * shared::SPHERE_LIGHT_Z)
                            .max(0.0))
                    .min(shared::SPHERE_LIGHT_MAX);
                    self.reference_pixel(x, y, p[2] + nz * depth, tint(color, light), pick);
                } else if selected && d <= (outer / r).powi(2) {
                    self.reference_pixel(
                        x,
                        y,
                        p[2],
                        shared::SELECTION_COLOR.map(|c| c as u8),
                        pick,
                    );
                }
            }
        }
    }

    fn reference_halo(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        self.reference_disc(p, radius, color, strength, Self::reference_blend);
    }

    fn reference_puff(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        self.reference_disc(p, radius, color, strength, Self::reference_blend_rounded);
    }

    /// Blends `color` over the pixels within `radius` of `p` with weight `strength` times the
    /// square of one minus the distance in radii, by `blend`.
    fn reference_disc(
        &mut self,
        p: [f32; 2],
        radius: f32,
        color: Color,
        strength: f32,
        blend: fn(&mut Self, i32, i32, Color, f32),
    ) {
        let x0 = (p[0] - radius).floor() as i32;
        let x1 = (p[0] + radius).ceil() as i32;
        let y0 = (p[1] - radius).floor() as i32;
        let y1 = (p[1] + radius).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - p[0]).hypot(y as f32 + 0.5 - p[1]) / radius;
                if d < 1.0 {
                    blend(self, x, y, color, strength * (1.0 - d) * (1.0 - d));
                }
            }
        }
    }

    fn reference_ray(&mut self, a: [f32; 2], b: [f32; 2], color: Color, weight: f32) {
        let steps = (a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .min(8000.0)
            .ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let x = (a[0] + (b[0] - a[0]) * t) as i32;
            let y = (a[1] + (b[1] - a[1]) * t) as i32;
            self.reference_blend(x, y, color, weight);
        }
    }

    /// Rasterizes one field triangle on the CPU. Each row's span comes from where the scanline
    /// crosses the triangle's edges, each crossing computed from the edge's endpoints in a fixed
    /// order (top first), so two triangles that share an edge agree on where it is and no pixel
    /// centre falls between them; both include pixels centred on it. `uv` and depth are linear in
    /// screen space, so each is evaluated at the row's first pixel centre and stepped.
    fn reference_field_triangle(
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
        let edges = [(a, b), (b, c), (c, a)].map(|(p, q)| ReferenceEdge::new(p, q));
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
            reference_sample_span(
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

/// The node coordinates clamped to the field, the cell they fall in, that cell's far corner
/// (the same node at the last row and column) and the fractions across it.
fn reference_cell(
    columns: usize,
    rows: usize,
    u: f32,
    v: f32,
) -> ([usize; 2], [usize; 2], [f32; 2]) {
    let u = u.clamp(0.0, (columns - 1) as f32);
    let v = v.clamp(0.0, (rows - 1) as f32);
    let (i, j) = (u as usize, v as usize);
    (
        [i, j],
        [(i + 1).min(columns - 1), (j + 1).min(rows - 1)],
        [u - i as f32, v - j as f32],
    )
}

/// An edge of a field triangle that crosses scanlines: its top end first, the x at the top and
/// the change in x per row.
struct ReferenceEdge {
    top: f32,
    bottom: f32,
    x: f32,
    slope: f32,
}

impl ReferenceEdge {
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

fn reference_lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
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
fn reference_sample_span(
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
        let ([i, j], [i1, j1], [fx, fy]) = reference_cell(columns, field.rows, here_u, here_v);
        if held.0 != [i, j] {
            held = (
                [i, j],
                [texel(i, j), texel(i1, j), texel(i, j1), texel(i1, j1)],
            );
        }
        let [c00, c10, c01, c11] = held.1;
        let color = reference_lerp4(
            reference_lerp4(c00, c10, fx),
            reference_lerp4(c01, c11, fx),
            fy,
        );
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
