use std::cell::RefCell;
use std::f32::consts::TAU;
use std::sync::{Mutex, OnceLock};

use super::field::ShadedField;
use super::frame::{BACKGROUND, Frame, Item, Sky};
use super::{Color, NONE, Point, shared};
use crate::glyphs;

mod shapes;
mod triangle;

#[cfg(test)]
mod differential;
#[cfg(test)]
mod reference;

/// A frame of fewer pixels than this is drawn on the calling thread, in one band: splitting it
/// costs the threads, which allocate, and a list of items for each band, so it has to save more
/// than that. (It is also what keeps a steady-state frame of 480 pixels, which the allocation
/// ratchet counts, at the one allocation it always took.)
const THREADED_MIN_PIXELS: usize = 256 * 1024;
/// Bands a thread is dealt on average. More than one, so that a band that is heavy (content
/// gathers in the middle of a frame) does not leave the other threads idle at the end.
const BANDS_PER_THREAD: usize = 6;
/// The fewest rows in a band: an item that touches several bands is set up in each.
const MIN_BAND_ROWS: usize = 16;

thread_local! {
    /// The items each band of the last frame this thread drew in bands was dealt, by index.
    static LISTS: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
}

/// The threads a frame is split across: the cores this process may run on.
fn cores() -> usize {
    static CORES: OnceLock<usize> = OnceLock::new();
    *CORES.get_or_init(|| std::thread::available_parallelism().map_or(1, usize::from))
}

/// `v.floor().max(0.0) as i32` without the call to `floorf` (the baseline x86-64 target has no
/// rounding instruction): from 0 up truncation is the floor, below it the maximum wins, and a
/// NaN gives 0 either way.
#[inline]
pub(super) fn first_index(v: f32) -> i32 {
    v.max(0.0) as i32
}

/// `v.ceil().min(last) as i32` without the call to `ceilf`, for a `last` that is a whole number
/// no more than 2^24: the ceiling commutes with the minimum, and the ceiling of what is left is
/// its truncation, one higher if anything was cut off.
#[inline]
pub(super) fn last_index(v: f32, last: f32) -> i32 {
    let v = v.min(last);
    let truncated = v as i32;
    truncated + i32::from(v > truncated as f32)
}

/// The rows of a frame of `height` rows that a line from `a` to `b` can reach, given the rows
/// they are on: the positions along it are truncated, so a couple of pixels past either end
/// cover every rounding, if both are finite and small enough for that to hold; else every row.
fn line_rows(a: f32, b: f32, height: usize) -> (i32, i32) {
    const REACH: f32 = 1_048_576.0;
    if !(a.abs() <= REACH && b.abs() <= REACH) {
        return (0, height as i32 - 1);
    }
    (
        first_index(a.min(b) - 2.0),
        last_index(a.max(b) + 2.0, height as f32 - 1.0),
    )
}

/// How far past its radius a sphere's box reaches.
pub(super) fn reach_of(selected: bool) -> f32 {
    if selected {
        shared::SELECTION_RING_WIDTH
    } else {
        shared::SPHERE_EDGE
    }
}

impl Item {
    /// The first and last rows, of a frame `height` rows high, that drawing this item can
    /// change, and the first after the last if it changes none. It is a promise made for the
    /// band lists: each primitive clips to its own rows by the same expressions, and this only
    /// has to name at least those, so the cases that are awkward (lines, glyphs, NaN) say all of
    /// them.
    fn rows(&self, height: usize) -> (i32, i32) {
        let last = height as f32 - 1.0;
        match *self {
            Self::Triangle([a, b, c], ..) => (
                first_index(a[1].min(b[1]).min(c[1])),
                last_index(a[1].max(b[1]).max(c[1]), last),
            ),
            Self::Field {
                points: [a, b, c], ..
            } => (
                first_index(a[1].min(b[1]).min(c[1]) - 0.5),
                last_index(a[1].max(b[1]).max(c[1]), last),
            ),
            Self::Line(a, b, _) => line_rows(a[1], b[1], height),
            Self::Beam(a, b, ..) => line_rows(a[1], b[1], height),
            Self::Sphere {
                center,
                radius,
                selected,
                ..
            } => {
                let outer = radius + reach_of(selected);
                (
                    first_index(center[1] - outer),
                    last_index(center[1] + outer, last),
                )
            }
            Self::Glow { center, radius, .. } | Self::Puff { center, radius, .. } => (
                ((center[1] - radius).floor() as i32).max(0),
                ((center[1] + radius).ceil() as i32).min(height as i32 - 1),
            ),
            Self::Pick { center, radius, .. } => (
                first_index(center[1] - radius),
                last_index(center[1] + radius, last),
            ),
            Self::Star([_, y], _) => (y as i32, y as i32),
            Self::Glyph { origin, scale, .. } => {
                // The font cell's rows in the frame, unless the scale is one whose arithmetic
                // `glyph` does in `u32` and might wrap.
                if scale > 4096 {
                    return (0, height as i32 - 1);
                }
                let top = (origin[1] as i64).clamp(-(1 << 40), 1 << 40);
                let rows = i64::from(glyphs::HEIGHT) * i64::from(scale);
                (
                    top.clamp(0, height as i64) as i32,
                    (top + rows - 1).clamp(-1, height as i64 - 1) as i32,
                )
            }
        }
    }
}

/// Whole rows of a frame's three buffers. Every primitive draws into one of these, clipped to
/// its rows, and a pixel's value depends on the item and the pixel alone, never on the band
/// that holds it, so any split into bands gives the same frame as one band holding every row
/// (`differential` holds the rasterizer to that).
pub(super) struct Band<'a> {
    pub(super) width: usize,
    /// The height of the whole frame, which the sky and the bounds of a primitive are measured
    /// against.
    pub(super) height: usize,
    /// The frame row of this band's first row.
    pub(super) first: usize,
    /// How many rows the band holds.
    pub(super) rows: usize,
    pub(super) pixels: &'a mut [u8],
    pub(super) depth: &'a mut [f32],
    pub(super) picks: &'a mut [u32],
}

impl Frame {
    /// Rasterizes the recorded items on the CPU into pixels, depth and picking buffers. Large
    /// frames are cut into bands of rows, drawn by one thread each; the result does not depend
    /// on how many.
    pub fn rasterize(&mut self) {
        let pixels = self.width as usize * self.height as usize;
        let threads = if pixels < THREADED_MIN_PIXELS {
            1
        } else {
            cores()
        };
        let bands = if threads == 1 {
            1
        } else {
            threads * BANDS_PER_THREAD
        };
        self.rasterize_banded(bands.min(self.height as usize / MIN_BAND_ROWS), threads);
    }

    /// `rasterize` with the frame cut into `bands` bands (at least one, at most one a row), drawn
    /// by up to `threads` threads.
    pub(super) fn rasterize_banded(&mut self, bands: usize, threads: usize) {
        let (width, height) = (self.width as usize, self.height as usize);
        let n = width * height;
        self.pixels.resize(n * 3, 0);
        self.depth.resize(n, 0.0);
        self.picks.resize(n, NONE);
        if n == 0 {
            return;
        }
        let Self {
            sky,
            items,
            fields,
            pixels,
            depth,
            picks,
            ..
        } = self;
        // The haze is a product of one factor per column and one per row.
        let columns: Vec<f32> = (0..width)
            .map(|x| sky.profile(x as f32 / width as f32, 0.0))
            .collect();
        let rows = height.div_ceil(bands.clamp(1, height));
        if rows >= height {
            let mut band = Band {
                width,
                height,
                first: 0,
                rows: height,
                pixels,
                depth,
                picks,
            };
            band.draw(sky, &columns, items, fields, None);
            return;
        }
        let bands: Vec<Band> = pixels
            .chunks_mut(rows * width * 3)
            .zip(depth.chunks_mut(rows * width))
            .zip(picks.chunks_mut(rows * width))
            .enumerate()
            .map(|(index, ((pixels, depth), picks))| Band {
                width,
                height,
                first: index * rows,
                rows: depth.len() / width,
                pixels,
                depth,
                picks,
            })
            .collect();
        let threads = threads.clamp(1, bands.len());
        // The lists of items of the bands are kept by the thread that draws the frame, so that
        // a steady-state frame does not grow them again.
        LISTS.with_borrow_mut(|lists| {
            lists.resize_with(bands.len(), Vec::new);
            lists.iter_mut().for_each(Vec::clear);
            // Which items each band draws, in the order of the list: those whose rows reach it.
            for (index, item) in items.iter().enumerate() {
                let (first, last) = item.rows(height);
                let (first, last) = (first.max(0), last.min(height as i32 - 1));
                if first <= last {
                    for list in &mut lists[first as usize / rows..=last as usize / rows] {
                        list.push(index as u32);
                    }
                }
            }
            let lists: &[Vec<u32>] = lists;
            let queue = Mutex::new(bands.into_iter().zip(lists));
            let work = || {
                loop {
                    let next = queue
                        .lock()
                        .expect("no band panics while it holds the queue")
                        .next();
                    let Some((mut band, list)) = next else { break };
                    band.draw(sky, &columns, items, fields, Some(list));
                }
            };
            if threads == 1 {
                work();
                return;
            }
            std::thread::scope(|scope| {
                for _ in 1..threads {
                    // A thread the system will not give is no loss: the bands are dealt from
                    // the queue, so the ones that are running take the rest.
                    let _ = std::thread::Builder::new().spawn_scoped(scope, work);
                }
                work();
            });
        });
    }
}

impl Band<'_> {
    /// Paints the sky into the band, clears depth and picks, and draws every item over it in
    /// order.
    fn draw(
        &mut self,
        sky: &Sky,
        columns: &[f32],
        items: &[Item],
        fields: &[ShadedField],
        list: Option<&[u32]>,
    ) {
        self.depth.fill(f32::NEG_INFINITY);
        self.picks.fill(NONE);
        self.paint_sky(sky, columns);
        match list {
            Some(list) => {
                for &index in list {
                    self.item(&items[index as usize], fields);
                }
            }
            None => {
                for item in items {
                    self.item(item, fields);
                }
            }
        }
    }

    /// Draws one item over what the band holds.
    fn item(&mut self, item: &Item, fields: &[ShadedField]) {
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
                self.pixel(x as i32, y as i32, BACKGROUND, color, NONE);
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

    /// The band's first and last frame rows.
    pub(super) fn span(&self) -> (i32, i32) {
        let first = self.first as i32;
        (first, first + self.rows as i32 - 1)
    }

    fn glyph(&mut self, origin: [f32; 2], scale: u32, bits: u128, color: Color) {
        let (first, last) = self.span();
        for row in 0..glyphs::HEIGHT {
            for column in 0..glyphs::WIDTH {
                if bits >> (row * glyphs::WIDTH + column) & 1 == 0 {
                    continue;
                }
                for dy in 0..scale {
                    let y = origin[1] as i64 + (row * scale + dy) as i64;
                    if !(i64::from(first)..=i64::from(last)).contains(&y) {
                        continue;
                    }
                    for dx in 0..scale {
                        let x = origin[0] as i64 + (column * scale + dx) as i64;
                        if (0..self.width as i64).contains(&x) {
                            let i = ((y as usize - self.first) * self.width + x as usize) * 3;
                            self.pixels[i..i + 3].copy_from_slice(&color);
                        }
                    }
                }
            }
        }
    }

    /// Paints the band's rows of the sky; every pixel of a frame is painted by it.
    fn paint_sky(&mut self, sky: &Sky, columns: &[f32]) {
        let (w, h) = (self.width, self.height);
        for (row, line) in self.pixels.chunks_exact_mut(w * 3).enumerate() {
            let v = (self.first + row) as f32 / h as f32;
            let base = sky.base(v);
            let profile = sky.profile(v, 1.9);
            if sky.hazy() {
                for (pixel, column) in line.as_chunks_mut::<3>().0.iter_mut().zip(columns) {
                    let density = profile * column;
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

    /// Writes one pixel if it is in the band and `depth` is at least as near as what is there.
    #[inline]
    pub(super) fn pixel(&mut self, x: i32, y: i32, depth: f32, color: Color, pick: u32) {
        let (first, last) = self.span();
        if x < 0 || x >= self.width as i32 || y < first || y > last {
            return;
        }
        let i = (y as usize - self.first) * self.width + x as usize;
        if depth < self.depth[i] {
            return;
        }
        self.depth[i] = depth;
        self.picks[i] = pick;
        self.pixels[i * 3..i * 3 + 3].copy_from_slice(&color);
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

    fn ray(&mut self, a: [f32; 2], b: [f32; 2], color: Color, weight: f32) {
        let steps = (a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .min(8000.0)
            .ceil() as usize;
        let (first, last) = self.span();
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let x = (a[0] + (b[0] - a[0]) * t) as i32;
            let y = (a[1] + (b[1] - a[1]) * t) as i32;
            if x >= 0 && x < self.width as i32 && y >= first && y <= last {
                let i = ((y as usize - self.first) * self.width + x as usize) * 3;
                mix(&mut self.pixels[i..i + 3], color, weight);
            }
        }
    }
}

/// `color` mixed into `pixel` by `weight`, truncated to a level.
#[inline]
pub(super) fn mix(pixel: &mut [u8], color: Color, weight: f32) {
    for (channel, &target) in pixel.iter_mut().zip(&color) {
        *channel = (*channel as f32 + (target as f32 - *channel as f32) * weight) as u8;
    }
}

/// `mix` rounded to the nearest level, as the GPU's blend unit rounds.
#[inline]
pub(super) fn mix_rounded(pixel: &mut [u8], color: Color, weight: f32) {
    for (channel, &target) in pixel.iter_mut().zip(&color) {
        *channel = (*channel as f32 + (target as f32 - *channel as f32) * weight + 0.5) as u8;
    }
}

impl Frame {
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
        mix(&mut self.pixels[i..i + 3], color, weight);
    }

    #[cfg(test)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::frame::{Backdrop, Buffers, sky_of};

    #[test]
    fn the_index_helpers_agree_with_floor_and_ceil() {
        let mut values = vec![
            0.0,
            -0.0,
            0.5,
            -0.5,
            1.0,
            -1.0,
            255.5,
            1919.999,
            1e9,
            -1e9,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            f32::MIN_POSITIVE,
            1.0e-45,
            16_777_216.0,
            4_294_967_296.0,
            2_147_483_648.0,
            -2_147_483_648.0,
            -2_147_483_904.0,
        ];
        // Whole numbers and halves, and the floats on either side of them.
        for k in -4..2200 {
            for half in [0.0, 0.5] {
                let center = (k as f32 + half).to_bits();
                values.extend((center.saturating_sub(3)..=center + 3).map(f32::from_bits));
            }
        }
        // And one pattern in four thousand of all of them.
        values.extend((0..u32::MAX / 4093).map(|i| f32::from_bits(i * 4093)));
        for last in [0.0, 1.0, 479.0, 1079.0, 16_383.0] {
            for &v in &values {
                assert_eq!(
                    first_index(v),
                    v.floor().max(0.0) as i32,
                    "first_index({v})"
                );
                assert_eq!(
                    last_index(v, last),
                    v.ceil().min(last) as i32,
                    "last_index({v}, {last})"
                );
            }
        }
    }

    #[test]
    fn stacked_puffs_round_to_the_nearest_level_where_glows_truncate() {
        // Over black, a centred half-strength disc of 255 blends to 127.5, then 191.5, then 223.5.
        let mut puffs = Frame::new(4, 4, sky_of(Backdrop::Void), Buffers::default());
        let mut glows = Frame::new(4, 4, sky_of(Backdrop::Void), Buffers::default());
        let (mut rounded, mut truncated) = ([0u8; 3], [0u8; 3]);
        for k in 0..3 {
            puffs.items.push(Item::Puff {
                center: [1.5, 1.5],
                radius: 2.0,
                color: [255; 3],
                strength: 0.5,
            });
            glows.items.push(Item::Glow {
                center: [1.5, 1.5],
                radius: 2.0,
                color: [255; 3],
                strength: 0.5,
            });
            puffs.rasterize();
            glows.rasterize();
            rounded[k] = puffs.pixels[(4 + 1) * 3];
            truncated[k] = glows.pixels[(4 + 1) * 3];
        }
        // The three discs are stacked in one list, so the k-th frame holds k + 1 of them.
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
