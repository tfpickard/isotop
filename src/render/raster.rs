use std::f32::consts::TAU;

use super::frame::{BACKGROUND, Frame, Item};
use super::{Color, NONE, Point, shared};
use crate::glyphs;
use crate::palette::tint;

impl Frame {
    /// Rasterizes the recorded items on the CPU into pixels, depth and picking buffers.
    pub fn rasterize(&mut self) {
        let n = (self.width * self.height) as usize;
        // Every pixel is painted by the sky, so only the length matters here.
        self.pixels.resize(n * 3, 0);
        self.depth.clear();
        self.depth.resize(n, f32::NEG_INFINITY);
        self.picks.clear();
        self.picks.resize(n, NONE);
        self.paint_sky();
        let items = std::mem::take(&mut self.items);
        let fields = std::mem::take(&mut self.fields);
        for item in &items {
            match *item {
                Item::Triangle([a, b, c], color, pick) => self.triangle(a, b, c, color, pick),
                Item::Line(a, b, color) => self.segment(a, b, color),
                Item::Sphere {
                    center,
                    radius,
                    depth,
                    color,
                    pick,
                    selected,
                } => self.ball(center, radius, depth, color, pick, selected),
                Item::Glow {
                    center,
                    radius,
                    color,
                    strength,
                } => self.halo(center, radius, color, strength),
                Item::Puff {
                    center,
                    radius,
                    color,
                    strength,
                } => self.puff(center, radius, color, strength.clamp(0.0, 1.0)),
                Item::Beam(a, b, color, weight) => self.ray(a, b, color, weight),
                Item::Star([x, y], color) => {
                    self.pixel(x as i32, y as i32, BACKGROUND, color, NONE)
                }
                Item::Glyph {
                    origin,
                    scale,
                    bits,
                    color,
                } => self.glyph(origin, scale, bits, color),
                Item::Pick {
                    center,
                    radius,
                    depth,
                    pick,
                } => self.splat(center, radius, depth, pick),
                Item::Field { points, uv, field } => {
                    self.field_triangle(points, uv, &fields[field as usize]);
                }
            }
        }
        self.items = items;
        self.fields = fields;
    }

    fn glyph(&mut self, origin: [f32; 2], scale: u32, bits: u128, color: Color) {
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

    fn paint_sky(&mut self) {
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

    /// Two-pixel annotation line drawn over the finished frame, ignoring depth.
    pub fn stroke(&mut self, a: [f32; 2], b: [f32; 2], color: Color) {
        let steps = (a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .min(4000.0)
            .ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let x = (a[0] + (b[0] - a[0]) * t) as i32;
            let y = (a[1] + (b[1] - a[1]) * t) as i32;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1)] {
                self.blend(x + dx, y + dy, color, 1.0);
            }
        }
    }

    /// Annotation circle over the finished frame.
    pub fn circle(&mut self, center: [f32; 2], radius: f32, color: Color) {
        let point = |i: usize| {
            let angle = i as f32 / 40.0 * TAU;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        };
        for i in 0..40 {
            self.stroke(point(i), point(i + 1), color);
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: Color, weight: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 3;
        for (channel, &target) in self.pixels[i..i + 3].iter_mut().zip(&color) {
            *channel = (*channel as f32 + (target as f32 - *channel as f32) * weight) as u8;
        }
    }

    /// `blend` rounded to the nearest level, as the GPU's blend unit rounds.
    fn blend_rounded(&mut self, x: i32, y: i32, color: Color, weight: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 3;
        for (channel, &target) in self.pixels[i..i + 3].iter_mut().zip(&color) {
            *channel = (*channel as f32 + (target as f32 - *channel as f32) * weight + 0.5) as u8;
        }
    }

    pub(super) fn pixel(&mut self, x: i32, y: i32, depth: f32, color: Color, pick: u32) {
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

    fn triangle(&mut self, a: Point, b: Point, c: Point, color: Color, pick: u32) {
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
                    self.pixel(x, y, w0 * a[2] + w1 * b[2] + w2 * c[2], color, pick);
                }
            }
        }
    }

    fn segment(&mut self, a: Point, b: Point, color: Color) {
        let steps = (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            self.pixel(
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
    fn splat(&mut self, center: [f32; 2], radius: f32, depth: f32, pick: u32) {
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

    fn ball(&mut self, p: Point, r: f32, depth: f32, color: Color, pick: u32, selected: bool) {
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
                    self.pixel(x, y, p[2] + nz * depth, tint(color, light), pick);
                } else if selected && d <= (outer / r).powi(2) {
                    self.pixel(x, y, p[2], shared::SELECTION_COLOR.map(|c| c as u8), pick);
                }
            }
        }
    }

    fn halo(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        self.disc(p, radius, color, strength, Self::blend);
    }

    fn puff(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        self.disc(p, radius, color, strength, Self::blend_rounded);
    }

    /// Blends `color` over the pixels within `radius` of `p` with weight `strength` times the
    /// square of one minus the distance in radii, by `blend`.
    fn disc(
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

    fn ray(&mut self, a: [f32; 2], b: [f32; 2], color: Color, weight: f32) {
        let steps = (a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .min(8000.0)
            .ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let x = (a[0] + (b[0] - a[0]) * t) as i32;
            let y = (a[1] + (b[1] - a[1]) * t) as i32;
            self.blend(x, y, color, weight);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::frame::{Backdrop, Buffers, sky_of};
    #[test]
    fn stacked_puffs_round_to_the_nearest_level_where_glows_truncate() {
        // Over black, a centred half-strength disc of 255 blends to 127.5, then 191.5, then 223.5.
        let mut puffs = Frame::new(4, 4, sky_of(Backdrop::Void), Buffers::default());
        let mut glows = Frame::new(4, 4, sky_of(Backdrop::Void), Buffers::default());
        puffs.pixels = vec![0; 48];
        glows.pixels = vec![0; 48];
        let (mut rounded, mut truncated) = ([0u8; 3], [0u8; 3]);
        for k in 0..3 {
            puffs.puff([1.5, 1.5], 2.0, [255; 3], 0.5);
            glows.halo([1.5, 1.5], 2.0, [255; 3], 0.5);
            rounded[k] = puffs.pixels[(4 + 1) * 3];
            truncated[k] = glows.pixels[(4 + 1) * 3];
        }
        assert_eq!(
            rounded,
            [128, 192, 224],
            "a puff rounds to the nearest level"
        );
        assert_eq!(
            truncated,
            [127, 191, 223],
            "a glow truncates, as it always has"
        );
        // A strength above one is a full blend, never an overshoot.
        let mut frame = Frame::new(4, 4, sky_of(Backdrop::Void), Buffers::default());
        frame.items = vec![Item::Puff {
            center: [1.5, 1.5],
            radius: 2.0,
            color: [90; 3],
            strength: 3.0,
        }];
        frame.rasterize();
        assert_eq!(frame.pixels[(4 + 1) * 3], 90);
    }
}
