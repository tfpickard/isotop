//! The triangle of the CPU rasterizer, which draws the most pixels of any item.
//!
//! A pixel is inside when its three barycentric weights are all at least 0, each taken from the
//! edge functions evaluated at the pixel's centre and divided by the triangle's area, the third as
//! `1 - w0 - w1`. That formula is the contract, bit for bit: the GPU, the golden hashes and
//! `differential` all hold to what it rounds to. So nothing here changes an arithmetic
//! operation. What is done is to
//!
//! - evaluate it a row at a time, with the terms that depend on the row only computed once, and
//!   four pixels at a time, so that the divisions run as vectors (IEEE division, multiplication
//!   and subtraction round alike in one lane or four). A pixel is inside when the least of its
//!   weights is at least 0, which is the same test where no weight is NaN;
//! - write a chunk of four pixels without a branch on which of them are painted (a lane that is
//!   not keeps its depth, pick and colour, as selects and a byte mask), because where a
//!   triangle's edge crosses the chunk the branch cannot be predicted;
//! - for a wide, thin triangle, find the columns of each row where a pixel can be inside without
//!   evaluating the ones outside: see [`Strip`], which proves it exact;
//! - do it only where every value stays finite (`REACH`), and otherwise run the scalar loop the
//!   formula was first written as.

use super::{Band, first_index, last_index};
use crate::render::{Color, Point};

/// Triangles with every vertex coordinate within this of the origin have finite edge functions
/// and weights at every pixel of a frame: the edge functions are at most about `4 * REACH^2` and
/// the area, which is at least 0.001, divides them. Beyond it the arithmetic may overflow, and
/// monotonicity along a row (see [`Strip`]) is no longer something to lean on.
const REACH: f32 = 1_048_576.0;

/// A box narrower than this is cheaper to evaluate whole than to find the span of each row of.
const NARROWING_WIDTH: i32 = 32;

/// Pixels evaluated together: what an SSE register holds of f32, which every x86-64 has.
const LANES: usize = 4;

/// For each set of lanes of a chunk, the bytes of the chunk's pixels that are theirs.
const KEEP: [[u8; LANES * 3]; 1 << LANES] = {
    let mut table = [[0u8; LANES * 3]; 1 << LANES];
    let mut bits = 0;
    while bits < 1 << LANES {
        let mut i = 0;
        while i < LANES * 3 {
            if bits >> (i / 3) & 1 == 1 {
                table[bits][i] = 0xFF;
            }
            i += 1;
        }
        bits += 1;
    }
    table
};

/// How a triangle paints the pixels it covers: the colour and the pick it writes.
#[derive(Clone, Copy)]
struct Paint {
    color: Color,
    pick: u32,
    /// The colour of `LANES` pixels in a row, for the chunks that are covered whole.
    tile: [u8; LANES * 3],
}

/// The lesser of two floats that are not NaN, as a select, which is one `minps`.
#[inline(always)]
fn lesser(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// What `evaluate` takes of a triangle, each value repeated across the lanes so that it stays
/// in a vector register for the length of a row.
struct Lanes {
    area: [f32; LANES],
    origin: [[f32; LANES]; 2],
    slope: [[f32; LANES]; 2],
    depth: [[f32; LANES]; 3],
}

impl Lanes {
    fn new(weights: &Weights, depths: [f32; 3]) -> Self {
        Self {
            area: [weights.area; LANES],
            origin: weights.origin.map(|value| [value; LANES]),
            slope: weights.slope.map(|value| [value; LANES]),
            depth: depths.map(|value| [value; LANES]),
        }
    }
}

/// The least of the three weights of each of the `LANES` pixels whose centres are `center`, and
/// the depth each would take. A pixel is inside when the least is at least 0 (no weight is NaN
/// in a bounded triangle).
#[inline(always)]
fn evaluate(
    center: [f32; LANES],
    lanes: &Lanes,
    rows: [[f32; LANES]; 2],
) -> ([f32; LANES], [f32; LANES]) {
    let w0: [f32; LANES] = std::array::from_fn(|lane| {
        ((center[lane] - lanes.origin[0][lane]) * lanes.slope[0][lane] - rows[0][lane])
            / lanes.area[lane]
    });
    let w1: [f32; LANES] = std::array::from_fn(|lane| {
        ((center[lane] - lanes.origin[1][lane]) * lanes.slope[1][lane] - rows[1][lane])
            / lanes.area[lane]
    });
    let w2: [f32; LANES] = std::array::from_fn(|lane| 1.0 - w0[lane] - w1[lane]);
    let least = std::array::from_fn(|lane| lesser(lesser(w0[lane], w1[lane]), w2[lane]));
    let depth = std::array::from_fn(|lane| {
        w0[lane] * lanes.depth[0][lane]
            + w1[lane] * lanes.depth[1][lane]
            + w2[lane] * lanes.depth[2][lane]
    });
    (least, depth)
}

/// The edge function of the edge from `a` to `b` at (`x`, `y`).
#[inline]
fn edge(a: Point, b: Point, x: f32, y: f32) -> f32 {
    (x - a[0]) * (b[1] - a[1]) - (y - a[1]) * (b[0] - a[0])
}

/// What a triangle's rows share: the weights `w0` and `w1` of a pixel at column `x` of a row are
/// `((x + 0.5 - origin) * slope - row) / area`, `row` being `(y + 0.5 - origin_y) * run`, which
/// is the same for the whole row.
struct Weights {
    area: f32,
    /// The x of the vertex each weight's edge starts from.
    origin: [f32; 2],
    /// The change of each weight's edge function per unit of x.
    slope: [f32; 2],
    /// The y of that vertex and the change per unit of y, which is `row`'s factor.
    origin_y: [f32; 2],
    run: [f32; 2],
}

impl Weights {
    fn new(a: Point, b: Point, c: Point, area: f32) -> Self {
        Self {
            area,
            // `edge(b, c)` and `edge(c, a)`, with the pixel's offsets taken from b and c.
            origin: [b[0], c[0]],
            slope: [c[1] - b[1], a[1] - c[1]],
            origin_y: [b[1], c[1]],
            run: [c[0] - b[0], a[0] - c[0]],
        }
    }

    /// The part of each weight's edge function that depends on the row alone.
    fn rows(&self, y: i32) -> [f32; 2] {
        let center = y as f32 + 0.5;
        [
            (center - self.origin_y[0]) * self.run[0],
            (center - self.origin_y[1]) * self.run[1],
        ]
    }

    #[inline]
    fn weight(&self, which: usize, x: i32, row: f32) -> f32 {
        (((x as f32 + 0.5) - self.origin[which]) * self.slope[which] - row) / self.area
    }
}

/// The least `x` in `lo..end` for which `holds(x)`, or `end` if there is none, given that
/// `holds` is false up to some point and true from it on. `guess` is where it probably starts: a
/// good one takes two calls of `holds`, a bad one only a few more.
fn first_true(lo: i32, end: i32, guess: i32, holds: impl Fn(i32) -> bool) -> i32 {
    let (mut a, mut b) = (lo, end);
    let guess = guess.clamp(lo, end);
    let mut step = 1;
    if guess == end || holds(guess) {
        // The answer is at most `guess`: look below it, further each time.
        b = guess;
        while a < b {
            let probe = (b - step).max(a);
            if holds(probe) {
                b = probe;
                step = step.saturating_mul(2);
            } else {
                a = probe + 1;
                break;
            }
        }
    } else {
        // The answer is above `guess`.
        a = guess + 1;
        while a < b {
            let probe = (a + step - 1).min(b - 1);
            if holds(probe) {
                b = probe;
                break;
            }
            a = probe + 1;
            step = step.saturating_mul(2);
        }
    }
    while a < b {
        let middle = a + (b - a) / 2;
        if holds(middle) {
            b = middle;
        } else {
            a = middle + 1;
        }
    }
    a
}

/// The columns of a row where a weight can be in `0..=1`: [`Strip::columns`].
struct Strip<'a> {
    weights: &'a Weights,
    which: usize,
    row: f32,
}

impl Strip<'_> {
    #[inline]
    fn at(&self, x: i32) -> f32 {
        self.weights.weight(self.which, x, self.row)
    }

    /// The columns in `start..end` where this weight is between 0 and 1, as a half-open range
    /// (empty when `start` is not below `end`), exactly.
    ///
    /// A pixel is inside only if both its first weights are in `0..=1`: both at least 0 is
    /// the test; and `w2 = (1 - w0) - w1 >= 0` with `w1 >= 0` needs `1 - w0 >= 0`, since a
    /// subtraction of floats is zero only for equal operands, and so `w0 <= 1`, and then
    /// `w1 <= 1 - w0 <= 1` in the same way. (The set of such columns holds the triangle's
    /// pixels and as many again, never more.)
    ///
    /// Along a row the weight is nondecreasing or nonincreasing in the column, whatever rounding
    /// does, because it is built of operations that each are (a sum or difference with a
    /// constant, a product with a constant, a quotient by a constant, all rounded to nearest, and
    /// rounding never reverses an order). So where it is at least 0 and where it is at most 1
    /// are each half-lines of the row, found by looking at the weight itself, at the column that
    /// double precision predicts first. Nothing here depends on the prediction being right, only
    /// its speed does. Everything is finite (see `REACH`), so no comparison meets a NaN.
    fn columns(&self, start: i32, end: i32) -> (i32, i32) {
        let Weights {
            area,
            origin,
            slope,
            ..
        } = *self.weights;
        let (origin, slope) = (origin[self.which], slope[self.which]);
        if slope == 0.0 {
            // The edge function is the same along the row.
            return if (0.0..=1.0).contains(&self.at(start)) {
                (start, end)
            } else {
                (start, start)
            };
        }
        let rising = (slope > 0.0) == (area > 0.0);
        // The column at which the weight reaches `level`, if it were exact.
        let reach = |level: f64| {
            f64::from(origin) + (f64::from(self.row) + level * f64::from(area)) / f64::from(slope)
                - 0.5
        };
        let column = |x: f64| x.clamp(f64::from(start), f64::from(end)) as i32;
        if rising {
            let low = first_true(start, end, column(reach(0.0).ceil()), |x| self.at(x) >= 0.0);
            let high = first_true(start, end, column(reach(1.0).floor() + 1.0), |x| {
                self.at(x) > 1.0
            });
            (low, high)
        } else {
            let low = first_true(start, end, column(reach(1.0).ceil()), |x| self.at(x) <= 1.0);
            let high = first_true(start, end, column(reach(0.0).floor() + 1.0), |x| {
                self.at(x) < 0.0
            });
            (low, high)
        }
    }
}

impl Band<'_> {
    pub(super) fn triangle(&mut self, a: Point, b: Point, c: Point, color: Color, pick: u32) {
        let (first, last) = self.span();
        let top = a[1].min(b[1]).min(c[1]);
        let bottom = a[1].max(b[1]).max(c[1]);
        // The box's rows run from `floor(top)` to `ceil(bottom)`: none of them in this band.
        if bottom <= first as f32 - 1.0 || top >= last as f32 + 1.0 {
            return;
        }
        let min_y = first_index(top).max(first);
        let max_y = last_index(bottom, self.height as f32 - 1.0).min(last);
        if min_y > max_y {
            return;
        }
        let area = edge(a, b, c[0], c[1]);
        // A NaN area makes every weight NaN, so no pixel is inside.
        if area.abs() < 0.001 || area.is_nan() {
            return;
        }
        let left = a[0].min(b[0]).min(c[0]);
        let right = a[0].max(b[0]).max(c[0]);
        let min_x = first_index(left);
        let max_x = last_index(right, self.width as f32 - 1.0);
        if min_x > max_x {
            return;
        }
        // Every coordinate within `REACH`, when the least and the greatest are (none is NaN: the
        // area would be).
        let bounded = left >= -REACH && right <= REACH && top >= -REACH && bottom <= REACH;
        if !bounded {
            self.triangle_unbounded([a, b, c], area, [min_x, max_x, min_y, max_y], color, pick);
            return;
        }
        let weights = Weights::new(a, b, c, area);
        let lanes = Lanes::new(&weights, [a[2], b[2], c[2]]);
        let paint = Paint {
            color,
            pick,
            tile: std::array::from_fn(|i| color[i % 3]),
        };
        let wide = max_x - min_x + 1;
        // Narrowing finds half of a row or more not to evaluate, or is not worth it.
        let narrowing = wide >= NARROWING_WIDTH && area.abs() < 0.5 * wide as f32 * (bottom - top);
        for y in min_y..=max_y {
            let rows = weights.rows(y);
            let (mut start, mut end) = (min_x, max_x + 1);
            if narrowing {
                (start, end) = (Strip {
                    weights: &weights,
                    which: 0,
                    row: rows[0],
                })
                .columns(start, end);
                if start < end {
                    (start, end) = (Strip {
                        weights: &weights,
                        which: 1,
                        row: rows[1],
                    })
                    .columns(start, end);
                }
                if start >= end {
                    continue;
                }
            }
            self.cover(y, start..end, &lanes, rows, &paint);
        }
    }

    /// One row of a triangle: the columns in `columns`, `LANES` at a time.
    fn cover(
        &mut self,
        y: i32,
        columns: std::ops::Range<i32>,
        lanes: &Lanes,
        rows: [f32; 2],
        paint: &Paint,
    ) {
        let rows = rows.map(|row| [row; LANES]);
        let base = (y as usize - self.first) * self.width;
        let from = base + columns.start as usize;
        let count = (columns.end - columns.start) as usize;
        let (near_chunks, near_rest) = self.depth[from..from + count].as_chunks_mut::<LANES>();
        let (pick_chunks, pick_rest) = self.picks[from..from + count].as_chunks_mut::<LANES>();
        let (pixel_chunks, pixel_rest) =
            self.pixels[from * 3..(from + count) * 3].as_chunks_mut::<{ LANES * 3 }>();
        // The centres of the next chunk's pixels. Adding `LANES` to a half-integer this small is
        // exact, so they are what `(x + lane) as f32 + 0.5` gives.
        let mut center: [f32; LANES] =
            std::array::from_fn(|lane| (columns.start + lane as i32) as f32 + 0.5);
        for ((near, picks), pixels) in near_chunks
            .iter_mut()
            .zip(pick_chunks.iter_mut())
            .zip(pixel_chunks.iter_mut())
        {
            let (least, depth) = evaluate(center, lanes, rows);
            center = center.map(|center| center + LANES as f32);
            let held = *near;
            let passes: [bool; LANES] = std::array::from_fn(|lane| {
                let below = depth[lane] < held[lane];
                (least[lane] >= 0.0) & !below
            });
            let bits = usize::from(passes[0])
                | usize::from(passes[1]) << 1
                | usize::from(passes[2]) << 2
                | usize::from(passes[3]) << 3;
            if bits == 0 {
                continue;
            }
            // Painted lanes take the new value and the others keep theirs, with no branch to
            // mispredict where the triangle's edge crosses the chunk.
            *near = std::array::from_fn(|lane| {
                if passes[lane] {
                    depth[lane]
                } else {
                    held[lane]
                }
            });
            let old = *picks;
            *picks = std::array::from_fn(|lane| if passes[lane] { paint.pick } else { old[lane] });
            let keep = KEEP[bits];
            let old = *pixels;
            *pixels = std::array::from_fn(|i| (old[i] & !keep[i]) | (paint.tile[i] & keep[i]));
        }
        if !near_rest.is_empty() {
            let (least, depth) = evaluate(center, lanes, rows);
            for lane in 0..near_rest.len() {
                let below = depth[lane] < near_rest[lane];
                if (least[lane] >= 0.0) & !below {
                    near_rest[lane] = depth[lane];
                    pick_rest[lane] = paint.pick;
                    pixel_rest[lane * 3..lane * 3 + 3].copy_from_slice(&paint.color);
                }
            }
        }
    }

    /// The triangle as it was first written: every pixel of the box, one at a time. For the
    /// triangles whose coordinates are too large for the weights to be trusted to stay finite.
    fn triangle_unbounded(
        &mut self,
        [a, b, c]: [Point; 3],
        area: f32,
        [min_x, max_x, min_y, max_y]: [i32; 4],
        color: Color,
        pick: u32,
    ) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_true_finds_the_start_wherever_the_guess_is() {
        for end in 0..40 {
            for start_at in 0..=end {
                for guess in -3..end + 3 {
                    let found = first_true(0, end, guess, |x| x >= start_at);
                    assert_eq!(
                        found, start_at,
                        "end {end}, start {start_at}, guess {guess}"
                    );
                }
            }
        }
        // And on a range that does not begin at 0.
        assert_eq!(first_true(-7, 5, 100, |x| x >= -2), -2);
        assert_eq!(first_true(5, 5, 5, |_| true), 5);
    }
}
