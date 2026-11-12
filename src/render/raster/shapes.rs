//! The round primitives of the CPU rasterizer: sphere impostors, pick discs and the soft discs
//! behind `Item::Glow` and `Item::Puff`. Each runs over one band, clipped to its rows.

use super::{Band, first_index, last_index, mix, mix_rounded, reach_of};
use crate::palette::tint;
use crate::render::{Color, Point, shared};

impl Band<'_> {
    /// Writes `pick` over a disc where `depth` is at least as near as what is drawn, as a
    /// sphere's surface tests, leaving colour and depth alone. Samples at integer pixel
    /// coordinates like `ball`.
    pub(super) fn splat(&mut self, center: [f32; 2], radius: f32, depth: f32, pick: u32) {
        let (first, last) = self.span();
        let x0 = first_index(center[0] - radius);
        let x1 = last_index(center[0] + radius, self.width as f32 - 1.0);
        let y0 = first_index(center[1] - radius).max(first);
        let y1 = last_index(center[1] + radius, self.height as f32 - 1.0).min(last);
        for y in y0..=y1 {
            let ny = (y as f32 - center[1]) / radius;
            let row = ny * ny;
            // The sum is at least its row's share, so a row past the radius has no pixel in it.
            if row > 1.0 {
                continue;
            }
            let base = (y as usize - self.first) * self.width;
            for x in x0..=x1 {
                let nx = (x as f32 - center[0]) / radius;
                let i = base + x as usize;
                if nx * nx + row <= 1.0 && depth >= self.depth[i] {
                    self.picks[i] = pick;
                }
            }
        }
    }

    pub(super) fn ball(
        &mut self,
        p: Point,
        r: f32,
        depth: f32,
        color: Color,
        pick: u32,
        selected: bool,
    ) {
        let outer = r + reach_of(selected);
        let (first, last) = self.span();
        let x0 = first_index(p[0] - outer);
        let x1 = last_index(p[0] + outer, self.width as f32 - 1.0);
        let y0 = first_index(p[1] - outer).max(first);
        let y1 = last_index(p[1] + outer, self.height as f32 - 1.0).min(last);
        let ring = (outer / r).powi(2);
        let ring_color = shared::SELECTION_COLOR.map(|c| c as u8);
        for y in y0..=y1 {
            let ny = (y as f32 - p[1]) / r;
            let row = ny * ny;
            // Left of the lighting sum, as it is written: `(-nx * X - ny * Y) + nz * Z`.
            let leaning = ny * shared::SPHERE_LIGHT_Y;
            // Past the radius a row has nothing of the sphere in it, and only a selected one
            // has a ring beyond.
            if !selected && row > 1.0 {
                continue;
            }
            for x in x0..=x1 {
                let nx = (x as f32 - p[0]) / r;
                let d = nx * nx + row;
                if d <= 1.0 {
                    let nz = (1.0 - d).sqrt();
                    let light = (shared::SPHERE_AMBIENT
                        + (-nx * shared::SPHERE_LIGHT_X - leaning + nz * shared::SPHERE_LIGHT_Z)
                            .max(0.0))
                    .min(shared::SPHERE_LIGHT_MAX);
                    self.pixel(x, y, p[2] + nz * depth, tint(color, light), pick);
                } else if selected && d <= ring {
                    self.pixel(x, y, p[2], ring_color, pick);
                }
            }
        }
    }

    pub(super) fn halo(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        self.disc::<false>(p, radius, color, strength);
    }

    pub(super) fn puff(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        self.disc::<true>(p, radius, color, strength);
    }

    /// Blends `color` over the pixels within `radius` of `p` with weight `strength` times the
    /// square of one minus the distance in radii, by `mix` or, when `ROUNDED`, `mix_rounded`.
    ///
    /// The distance is still the C library's `hypot` of the offsets from `p` to the pixel
    /// centre. On glibc 2.39 it equals the double-precision sum of squares and root that could
    /// be vectorized in place of it (it matched on every one of eight thousand million samples),
    /// but another C library, or an older glibc, may round differently, and a bit of difference
    /// in a distance can move a blended level. That is not worth the little the call costs next
    /// to the blend.
    fn disc<const ROUNDED: bool>(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        let (first, last) = self.span();
        // A pixel off the frame is never blended, so the box is cut to it first.
        let x0 = ((p[0] - radius).floor() as i32).max(0);
        let x1 = ((p[0] + radius).ceil() as i32).min(self.width as i32 - 1);
        let y0 = ((p[1] - radius).floor() as i32).max(first);
        let y1 = ((p[1] + radius).ceil() as i32).min(last);
        for y in y0..=y1 {
            let dy = y as f32 + 0.5 - p[1];
            let base = (y as usize - self.first) * self.width;
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - p[0]).hypot(dy) / radius;
                if d < 1.0 {
                    let i = (base + x as usize) * 3;
                    let weight = strength * (1.0 - d) * (1.0 - d);
                    if ROUNDED {
                        mix_rounded(&mut self.pixels[i..i + 3], color, weight);
                    } else {
                        mix(&mut self.pixels[i..i + 3], color, weight);
                    }
                }
            }
        }
    }
}
