//! Shared, pure simulation helpers used by views: neighbour queries over 2D and 3D points
//! ([`SpatialHash`], [`SpatialHash3`] with exact k-nearest), small deterministic random generators
//! (uniform and standard normal) and hashes, complex transforms ([`dft`], [`fft`],
//! [`inverse_fft`]), and the fixed-step clock [`FixedStep`]. Nothing here draws or reads the
//! system.
//!
//! New views seed from a process with [`Rng::for_identity`] or [`hash_identity`], which mix the
//! pid and the start time so a reused pid does not look like its predecessor. Everything else in
//! [`hash`] and [`XorShift32`] exists so that existing views keep their pixels: each function
//! keeps the exact arithmetic of the hand-rolled copy it replaced, and the golden render hashes
//! would change if one of them did.

use std::collections::HashMap;
use std::hash::BuildHasherDefault;

use crate::model::Identity;

/// Seconds a process takes to finish growing in after it appears.
pub const GROW_SECONDS: f32 = 0.9;

/// Spawn animation progress, 0 to 1, a cubic ease-out over [`GROW_SECONDS`] after a process
/// appears. A process with no recorded birth, or one born in the future, counts as fully grown.
pub fn growth(births: &HashMap<Identity, f32>, id: Identity, time: f32) -> f32 {
    let born = births.get(&id).copied().unwrap_or(f32::NEG_INFINITY);
    if time < born {
        return 1.0;
    }
    let t = ((time - born) / GROW_SECONDS).min(1.0);
    1.0 - (1.0 - t).powi(3)
}

/// A size scale that starts at 0.3 and reaches 1 as `growth` does, so a newborn is small but
/// never a speck.
pub fn sprout(growth: f32) -> f32 {
    0.3 + 0.7 * growth
}

/// The weight of one exponential-smoothing step over `dt` seconds with time constant `tau`:
/// `value += smoothing(dt, tau) * (target - value)`.
pub fn smoothing(dt: f32, tau: f32) -> f32 {
    1.0 - (-dt / tau).exp()
}

/// The smoothstep ease, `3x^2 - 2x^3`, of `x` clamped to 0..1: 0 and 1 at the ends, with no
/// speed at either.
pub fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// A complex number with `f32` parts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    /// The magnitude, `sqrt(re^2 + im^2)`.
    pub fn norm(self) -> f32 {
        self.re.hypot(self.im)
    }

    /// The complex conjugate, `re - i*im`.
    pub fn conj(self) -> Self {
        Self::new(self.re, -self.im)
    }

    /// The unit number at `angle` radians, `exp(i*angle)`.
    pub fn from_angle(angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self::new(cos, sin)
    }

    /// Both parts multiplied by `factor`.
    pub fn scale(self, factor: f32) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }
}

impl std::ops::Add for Complex {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.re + other.re, self.im + other.im)
    }
}

impl std::ops::Sub for Complex {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::new(self.re - other.re, self.im - other.im)
    }
}

impl std::ops::Mul for Complex {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        Self::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }
}

/// The direct discrete Fourier transform, O(n^2), for any length (60 samples need no padding).
///
/// Forward sign: `X[k] = sum over n of x[n] * exp(-2*pi*i*k*n/N)`, with no scaling, the same as
/// [`fft`]. Twiddles and sums are `f64`; only the result is rounded to `f32`, so this is the
/// reference the fast transform is tested against.
pub fn dft(input: &[Complex]) -> Vec<Complex> {
    let length = input.len();
    (0..length)
        .map(|frequency| {
            let (mut re, mut im) = (0.0_f64, 0.0_f64);
            for (index, sample) in input.iter().enumerate() {
                // Reducing k*n modulo N first keeps the angle small and accurate.
                let turns = ((frequency * index) % length) as f64 / length as f64;
                let (sin, cos) = (-std::f64::consts::TAU * turns).sin_cos();
                let (sample_re, sample_im) = (f64::from(sample.re), f64::from(sample.im));
                re += sample_re * cos - sample_im * sin;
                im += sample_re * sin + sample_im * cos;
            }
            Complex::new(re as f32, im as f32)
        })
        .collect()
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
fn require_power_of_two(name: &str, length: usize) {
    assert!(
        length == 0 || length.is_power_of_two(),
        "{name} needs a power-of-two length, got {length}"
    );
}

/// In-place iterative radix-2 Cooley-Tukey FFT with a bit-reversal permutation.
///
/// Forward sign: `X[k] = sum over n of x[n] * exp(-2*pi*i*k*n/N)`, with no scaling, the same as
/// [`dft`]. Data is `f32`; the twiddle factors are computed in `f64` and rounded once, so error
/// grows with `log2(N)` rounding steps rather than with the angle error of an `f32` recurrence.
/// Length 0 does nothing and length 1 is the identity.
///
/// # Panics
///
/// Panics, naming the length, unless the length is a power of two. Use [`dft`] for other lengths.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
pub fn fft(buffer: &mut [Complex]) {
    require_power_of_two("fft", buffer.len());
    transform(buffer);
}

/// The inverse of [`fft`]: the same transform with the opposite sign,
/// `x[n] = (1/N) * sum over k of X[k] * exp(+2*pi*i*k*n/N)`, scaled by `1/N`.
///
/// # Panics
///
/// Panics, naming the length, unless the length is a power of two.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
pub fn inverse_fft(buffer: &mut [Complex]) {
    require_power_of_two("inverse_fft", buffer.len());
    // conj(fft(conj(X))) is the unscaled inverse.
    for value in buffer.iter_mut() {
        *value = value.conj();
    }
    transform(buffer);
    let scale = 1.0 / buffer.len().max(1) as f32;
    for value in buffer.iter_mut() {
        let conjugate = value.conj();
        *value = Complex::new(conjugate.re * scale, conjugate.im * scale);
    }
}

/// The forward transform of a power-of-two length.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
fn transform(buffer: &mut [Complex]) {
    let length = buffer.len();
    if length < 2 {
        return;
    }
    let shift = usize::BITS - length.trailing_zeros();
    for index in 0..length {
        let partner = index.reverse_bits() >> shift;
        if partner > index {
            buffer.swap(index, partner);
        }
    }
    let twiddles: Vec<Complex> = (0..length / 2)
        .map(|index| {
            let (sin, cos) = (-std::f64::consts::TAU * index as f64 / length as f64).sin_cos();
            Complex::new(cos as f32, sin as f32)
        })
        .collect();
    let mut half = 1;
    while half < length {
        // A stage of butterflies spans 2 * half; its twiddles are every `stride`-th table entry.
        let stride = length / (2 * half);
        for start in (0..length).step_by(2 * half) {
            for offset in 0..half {
                let upper = buffer[start + offset];
                let lower = buffer[start + offset + half] * twiddles[offset * stride];
                buffer[start + offset] = upper + lower;
                buffer[start + offset + half] = upper - lower;
            }
        }
        half *= 2;
    }
}

/// A uniform grid for neighbour queries over 2D points.
///
/// Cost model: `rebuild` is O(n log n) (one sort of point indices by cell key) and, once the
/// buffers have grown to the point count, allocates nothing. Points live in one flat array
/// ordered by cell, and a map from cell key to `(start, end)` range replaces a `Vec` per cell.
/// `neighbours` visits the cells overlapping the query square, about `(2 * radius / cell + 1)^2`
/// lookups plus the points in them, so choose a cell near the typical query radius. When the
/// square covers more cells than there are points, it scans every point instead.
#[derive(Debug, Clone)]
pub struct SpatialHash {
    inverse_cell: f32,
    points: Vec<[f32; 2]>,
    keys: Vec<u64>,
    order: Vec<usize>,
    ranges: HashMap<u64, (usize, usize)>,
}

impl SpatialHash {
    /// Creates an empty grid whose cells are `cell` wide. Non-positive sizes fall back to 1.
    pub fn new(cell: f32) -> Self {
        let cell = if cell > 0.0 && cell.is_finite() {
            cell
        } else {
            1.0
        };
        Self {
            inverse_cell: 1.0 / cell,
            points: Vec::new(),
            keys: Vec::new(),
            order: Vec::new(),
            ranges: HashMap::new(),
        }
    }

    fn coordinate(&self, value: f32) -> i32 {
        // Float-to-int casts saturate and map NaN to zero, so any input yields a valid cell.
        (value * self.inverse_cell).floor() as i32
    }

    fn key(cell_x: i32, cell_y: i32) -> u64 {
        (u64::from(cell_x as u32) << 32) | u64::from(cell_y as u32)
    }

    /// Replaces the stored points, reusing the existing allocations.
    pub fn rebuild(&mut self, points: &[[f32; 2]]) {
        self.points.clear();
        self.points.extend_from_slice(points);
        self.keys.clear();
        for point in points {
            let key = Self::key(self.coordinate(point[0]), self.coordinate(point[1]));
            self.keys.push(key);
        }
        self.order.clear();
        self.order.extend(0..points.len());
        let keys = &self.keys;
        // Ties on the cell key break by index, so each cell lists its points in ascending order.
        self.order
            .sort_unstable_by_key(|&index| (keys[index], index));
        self.ranges.clear();
        let mut start = 0;
        while start < self.order.len() {
            let key = self.keys[self.order[start]];
            let mut end = start + 1;
            while end < self.order.len() && self.keys[self.order[end]] == key {
                end += 1;
            }
            self.ranges.insert(key, (start, end));
            start = end;
        }
    }

    /// Clears `out`, then fills it with the indices of every stored point whose distance to
    /// `point` is strictly less than `radius`, in ascending index order. A query point that is
    /// itself stored is included at distance zero.
    pub fn neighbours(&self, point: [f32; 2], radius: f32, out: &mut Vec<usize>) {
        out.clear();
        if self.points.is_empty() || radius.is_nan() || radius <= 0.0 {
            return;
        }
        let limit = radius * radius;
        let within = |index: usize| {
            let other = self.points[index];
            let dx = other[0] - point[0];
            let dy = other[1] - point[1];
            dx * dx + dy * dy < limit
        };
        let low_x = self.coordinate(point[0] - radius);
        let high_x = self.coordinate(point[0] + radius);
        let low_y = self.coordinate(point[1] - radius);
        let high_y = self.coordinate(point[1] + radius);
        // Each span fits in an i64, but their product overflows for a radius near the edge of
        // the i32 cell range; saturating keeps such a query on the point scan.
        let cells = (i64::from(high_x) - i64::from(low_x) + 1)
            .saturating_mul(i64::from(high_y) - i64::from(low_y) + 1);
        if cells > self.points.len() as i64 {
            out.extend((0..self.points.len()).filter(|&index| within(index)));
            return;
        }
        for cell_x in low_x..=high_x {
            for cell_y in low_y..=high_y {
                if let Some(&(start, end)) = self.ranges.get(&Self::key(cell_x, cell_y)) {
                    out.extend(
                        self.order[start..end]
                            .iter()
                            .copied()
                            .filter(|&index| within(index)),
                    );
                }
            }
        }
        out.sort_unstable();
    }
}

/// The sort key of an `(index, distance)` candidate: distance, then index, as one integer. The
/// bits of a non-negative `f32` order like its value, and a NaN distance ranks after every number.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
fn rank(candidate: &(usize, f32)) -> u64 {
    u64::from(candidate.1.to_bits()) << 32 | candidate.0 as u64
}

/// Orders candidates by distance, then index.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
fn closer(a: &(usize, f32), b: &(usize, f32)) -> std::cmp::Ordering {
    rank(a).cmp(&rank(b))
}

/// A uniform grid over 3D points with radius queries and exact k-nearest queries.
///
/// Built like [`SpatialHash`]: points in one flat array ordered by cell, a map from cell to
/// `(start, end)`, and buffers that a rebuild at a steady point count reuses without allocating.
/// Cells are keyed by their three `i32` coordinates, so distant cells never share a key.
///
/// [`neighbours`](Self::neighbours) visits the cells overlapping the query cube, about
/// `(2 * radius / cell + 1)^3` lookups. [`nearest`](Self::nearest) searches cubic shells of cells
/// outward from the query cell; choose a cell a little wider than the typical distance to the
/// k-th neighbour (6.5 for the 22nd neighbour in the flock of the timing test), so one or two
/// shells suffice. Coordinates must be finite; a grid holding non-finite points, or points
/// whose cell coordinates leave `+-2^30`, answers every query by scanning all points.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
pub struct SpatialHash3 {
    inverse_cell: f32,
    points: Vec<[f32; 3]>,
    /// `points` in `order`, each with its original index, so a cell's points sit next to each
    /// other in memory.
    ordered: Vec<([f32; 3], u32)>,
    keys: Vec<Cell>,
    order: Vec<usize>,
    ranges: HashMap<Cell, (usize, usize), BuildHasherDefault<CellHasher>>,
    /// False when a point is non-finite or lies beyond the cell coordinates the shell search can
    /// reason about, which sends every query to the full scan.
    gridded: bool,
}

/// The integer coordinates of one grid cell. Its own hash (below) is cheaper than the default
/// SipHash, which cost a k-nearest query about as much as the distances it computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
struct Cell([i32; 3]);

impl std::hash::Hash for Cell {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let [x, y, z] = self.0.map(|axis| u64::from(axis as u32));
        // Distinct odd multipliers keep (x, y, z) permutations apart; `CellHasher::finish` mixes.
        state.write_u64(
            (x | y << 32).wrapping_mul(SPLITMIX_GAMMA) ^ z.wrapping_mul(0xbf58_476d_1ce4_e5b9),
        );
    }
}

/// Hashes the one `u64` a [`Cell`] writes: one multiply, then the high half folded onto the low.
#[derive(Default)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
struct CellHasher(u64);

impl std::hash::Hasher for CellHasher {
    fn finish(&self) -> u64 {
        // The multiply moves the entropy to the high bits; folding them down gives hashbrown good
        // low bits for the bucket and good high bits for its tag.
        let mixed = self.0.wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ (mixed >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

/// Cell coordinates beyond this magnitude may be saturated casts, so their cell geometry is
/// unreliable.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
const CELL_LIMIT: i32 = 1 << 30;

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the first view that uses it")
)]
impl SpatialHash3 {
    /// Creates an empty grid whose cells are `cell` wide. Non-positive sizes fall back to 1.
    pub fn new(cell: f32) -> Self {
        let cell = if cell > 0.0 && cell.is_finite() {
            cell
        } else {
            1.0
        };
        Self {
            inverse_cell: 1.0 / cell,
            points: Vec::new(),
            ordered: Vec::new(),
            keys: Vec::new(),
            order: Vec::new(),
            ranges: HashMap::default(),
            gridded: true,
        }
    }

    fn coordinate(&self, value: f32) -> i32 {
        // Float-to-int casts saturate and map NaN to zero, so any input yields a valid cell.
        (value * self.inverse_cell).floor() as i32
    }

    fn cell_of(&self, point: [f32; 3]) -> Cell {
        Cell(point.map(|value| self.coordinate(value)))
    }

    /// Replaces the stored points, reusing the existing allocations.
    pub fn rebuild(&mut self, points: &[[f32; 3]]) {
        self.points.clear();
        self.points.extend_from_slice(points);
        self.keys.clear();
        self.gridded = true;
        for point in points {
            let key = self.cell_of(*point);
            self.gridded &= point.iter().all(|value| value.is_finite())
                && key
                    .0
                    .iter()
                    .all(|cell| (-CELL_LIMIT..CELL_LIMIT).contains(cell));
            self.keys.push(key);
        }
        self.order.clear();
        self.order.extend(0..points.len());
        let keys = &self.keys;
        // Ties on the cell break by index, so each cell lists its points in ascending order.
        self.order
            .sort_unstable_by_key(|&index| (keys[index], index));
        self.ordered.clear();
        self.ordered.extend(
            self.order
                .iter()
                .map(|&index| (points[index], index as u32)),
        );
        self.gridded &= u32::try_from(points.len()).is_ok();
        self.ranges.clear();
        let mut start = 0;
        while start < self.order.len() {
            let key = self.keys[self.order[start]];
            let mut end = start + 1;
            while end < self.order.len() && self.keys[self.order[end]] == key {
                end += 1;
            }
            self.ranges.insert(key, (start, end));
            start = end;
        }
    }

    fn distance(&self, index: usize, point: [f32; 3]) -> f32 {
        Self::distance_between(self.points[index], point)
    }

    fn squared_distance(other: [f32; 3], point: [f32; 3]) -> f32 {
        let dx = other[0] - point[0];
        let dy = other[1] - point[1];
        let dz = other[2] - point[2];
        dx * dx + dy * dy + dz * dz
    }

    fn distance_between(other: [f32; 3], point: [f32; 3]) -> f32 {
        Self::squared_distance(other, point).sqrt()
    }

    /// Clears `out`, then fills it with the indices of every stored point whose distance to
    /// `point` is strictly less than `radius`, in ascending index order. A query point that is
    /// itself stored is included at distance zero.
    pub fn neighbours(&self, point: [f32; 3], radius: f32, out: &mut Vec<usize>) {
        out.clear();
        if self.points.is_empty() || radius.is_nan() || radius <= 0.0 {
            return;
        }
        let limit = radius * radius;
        let within = |index: usize| {
            let other = self.points[index];
            let dx = other[0] - point[0];
            let dy = other[1] - point[1];
            let dz = other[2] - point[2];
            dx * dx + dy * dy + dz * dz < limit
        };
        let low = point.map(|value| self.coordinate(value - radius));
        let high = point.map(|value| self.coordinate(value + radius));
        // Each span fits in an i64, but their product overflows for a radius near the edge of
        // the i32 cell range; saturating keeps such a query on the point scan.
        let cells = (0..3).fold(1_i64, |cells, axis| {
            cells.saturating_mul(i64::from(high[axis]) - i64::from(low[axis]) + 1)
        });
        if cells > self.points.len() as i64 {
            out.extend((0..self.points.len()).filter(|&index| within(index)));
            return;
        }
        for cell_x in low[0]..=high[0] {
            for cell_y in low[1]..=high[1] {
                for cell_z in low[2]..=high[2] {
                    if let Some(&(start, end)) = self.ranges.get(&Cell([cell_x, cell_y, cell_z])) {
                        out.extend(
                            self.order[start..end]
                                .iter()
                                .copied()
                                .filter(|&index| within(index)),
                        );
                    }
                }
            }
        }
        out.sort_unstable();
    }

    /// Clears `out`, then fills it with the `k` stored points nearest to `point` as
    /// `(index, distance)`, sorted by distance and then by index. If `k` exceeds the point count
    /// it returns every point. A query point that is itself stored is included at distance 0;
    /// the caller filters it.
    ///
    /// The result is exact. The search visits cubic shells of cells at Chebyshev distance
    /// `0, 1, 2, ...` from the query cell. After each shell it keeps the best `k` candidates
    /// and stops once it holds `k` and the k-th distance is no greater than the distance from
    /// `point` to the nearest face of the cube searched so far (every unvisited point lies
    /// beyond that face, so none can be closer). If the cube through the next shell would
    /// cover more cells than there are points, it scans every point instead. Within a shell it
    /// skips cells that the sphere through the current k-th candidate cannot reach, which
    /// changes the cost but not the result.
    pub fn nearest(&self, point: [f32; 3], k: usize, out: &mut Vec<(usize, f32)>) {
        out.clear();
        let count = self.points.len();
        let k = k.min(count);
        if k == 0 {
            return;
        }
        let query = self.cell_of(point).0;
        let queryable = self.gridded
            && point.iter().all(|value| value.is_finite())
            && query
                .iter()
                .all(|cell| (-CELL_LIMIT..CELL_LIMIT).contains(cell));
        if queryable {
            let width = self.width();
            let mut kth = f32::INFINITY;
            for radius in 0_i32.. {
                let side = 2 * i64::from(radius) + 1;
                if side.saturating_mul(side).saturating_mul(side) > count as i64 {
                    break;
                }
                self.visit_shell(query, radius, point, kth, out);
                if out.len() >= k {
                    // Partition the k best to the front; only the k-th decides whether to stop.
                    out.select_nth_unstable_by(k - 1, closer);
                    out.truncate(k);
                    kth = out[k - 1].1;
                    let face = Self::face_distance(point, query, radius, width);
                    if f64::from(kth) * (1.0 + 1.0e-6) <= face {
                        out.sort_unstable_by(closer);
                        return;
                    }
                }
            }
        }
        out.clear();
        out.extend((0..count).map(|index| (index, self.distance(index, point))));
        if k < count {
            out.select_nth_unstable_by(k - 1, closer);
            out.truncate(k);
        }
        out.sort_unstable_by(closer);
    }

    /// Appends the points in the cells at Chebyshev distance exactly `radius` from `query` whose
    /// distance is at most `kth`, skipping any cell that the `kth` sphere around `point` cannot
    /// touch.
    fn visit_shell(
        &self,
        query: [i32; 3],
        radius: i32,
        point: [f32; 3],
        kth: f32,
        out: &mut Vec<(usize, f32)>,
    ) {
        let width = self.width();
        // Where the point sits inside its own cell, and the slack for rounding a coordinate to a
        // cell at the far edge of this shell.
        let inside: [f64; 3] =
            std::array::from_fn(|axis| f64::from(point[axis]) - f64::from(query[axis]) * width);
        let slack: [f64; 3] = std::array::from_fn(|axis| {
            (f64::from(query[axis].abs()) + f64::from(radius) + 1.0) * width * 4.0e-7
        });
        // The squared distance along one axis from the point to the cell `offset` cells away.
        let squared_gap = |axis: usize, offset: i32| {
            let gap = match offset {
                0 => 0.0,
                1.. => f64::from(offset) * width - inside[axis],
                _ => inside[axis] + f64::from(-offset - 1) * width,
            };
            let gap = (gap - slack[axis]).max(0.0);
            gap * gap
        };
        // The k-th distance squared, widened by the f32 rounding of a distance so a skipped cell
        // holds no point that the exact comparison would have kept.
        let limit = (f64::from(kth) * (1.0 + 1.0e-6)).powi(2);
        // Candidates are filtered on the squared distance with the same margin. A few beyond the
        // k-th may pass; the selection after the shell discards them.
        let kth_squared = (limit * (1.0 + 1.0e-6)) as f32;
        for dx in -radius..=radius {
            let gap_x = squared_gap(0, dx);
            if gap_x > limit {
                continue;
            }
            for dy in -radius..=radius {
                let gap_xy = gap_x + squared_gap(1, dy);
                if gap_xy > limit {
                    continue;
                }
                let mut visit = |dz: i32| {
                    if gap_xy + squared_gap(2, dz) > limit {
                        return;
                    }
                    let cell = Cell([query[0] + dx, query[1] + dy, query[2] + dz]);
                    if let Some(&(start, end)) = self.ranges.get(&cell) {
                        for &(other, index) in &self.ordered[start..end] {
                            let squared = Self::squared_distance(other, point);
                            if squared <= kth_squared {
                                out.push((index as usize, squared.sqrt()));
                            }
                        }
                    }
                };
                if dx.abs() == radius || dy.abs() == radius {
                    for dz in -radius..=radius {
                        visit(dz);
                    }
                } else {
                    // Interior columns touch the shell only at its two z faces.
                    visit(-radius);
                    visit(radius);
                }
            }
        }
    }

    /// The width a cell actually has: the reciprocal of the stored `f32` reciprocal.
    fn width(&self) -> f64 {
        1.0 / f64::from(self.inverse_cell)
    }

    /// The distance from `point` to the nearest face of the cube of cells within `radius` of the
    /// `query` cell, shrunk by the rounding slack of mapping a coordinate to its cell, so a point
    /// a few ulps over a cell boundary can never be missed.
    fn face_distance(point: [f32; 3], query: [i32; 3], radius: i32, width: f64) -> f64 {
        (0..3)
            .map(|axis| {
                let low = f64::from(query[axis] - radius) * width;
                let high = f64::from(query[axis] + radius + 1) * width;
                let slack = low.abs().max(high.abs()) * 4.0e-7;
                let value = f64::from(point[axis]);
                (value - low).min(high - value) - slack
            })
            .fold(f64::INFINITY, f64::min)
    }
}

/// A small deterministic generator (splitmix64). The same seed always yields the same stream.
#[derive(Debug, Clone, Default)]
pub struct Rng {
    state: u64,
}

/// The splitmix64 increment, the 64-bit golden ratio.
const SPLITMIX_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

/// The 32-bit golden ratio, 2^32 / phi. Seeds Flow's xorshift stream and multiplies in
/// [`hash::murmur_mix`].
pub const GOLDEN_RATIO_32: u32 = 0x9e37_79b9;

/// The splitmix64 output finaliser, a bijection that scrambles every input bit.
fn splitmix_finalizer(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// A 64-bit seed for one process: its pid and start time, then `salt`, each passed through the
/// splitmix64 finaliser. Consecutive pids are unrelated, and a reused pid differs from its
/// predecessor because the start time differs. This is the entry point for new views.
pub fn hash_identity(id: Identity, salt: u64) -> u64 {
    splitmix_finalizer(splitmix_finalizer(splitmix_finalizer(u64::from(id.pid)) ^ id.start) ^ salt)
}

impl Rng {
    /// Starts a stream from `seed`.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Starts a stream tied to a process, so it behaves the same across samples and runs.
    /// `salt` separates independent uses (for example drift versus colour) of one process.
    /// Each field passes through the splitmix64 finaliser so consecutive pids are unrelated.
    pub fn for_identity(id: Identity, salt: u64) -> Self {
        Self::new(hash_identity(id, salt))
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(SPLITMIX_GAMMA);
        splitmix_finalizer(self.state)
    }

    /// Uniform in [0, 1). Uses 24 bits, the width of an `f32` mantissa, so 1.0 cannot occur.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    /// Uniform in [-1, 1).
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// A standard normal sample (mean 0, variance 1) by the Box-Muller transform of two
    /// [`unit`](Self::unit) draws: `sqrt(-2 ln u1) * cos(2 pi u2)`. `u1` is `1 - unit()`, which
    /// lies in (0, 1], so the logarithm never sees zero. The second value of the pair is
    /// discarded, which keeps the generator a single `state` word.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no caller until the first view that uses it")
    )]
    pub fn normal(&mut self) -> f32 {
        let radius = (-2.0 * (1.0 - self.unit()).ln()).sqrt();
        radius * (std::f32::consts::TAU * self.unit()).cos()
    }
}

/// A fixed-step clock: turns wall-clock time into whole simulation steps, so every frame rate
/// follows the same trajectory. Steps are taken in whole `step`s from an accumulator, and the part
/// of a step left over is the interpolation fraction for drawing.
#[derive(Debug, Clone)]
pub struct FixedStep {
    step: f32,
    max_steps: usize,
    max_gap: f32,
    clock: Option<f32>,
    accumulator: f32,
}

impl FixedStep {
    /// A clock that steps by `step` seconds. A single frame takes at most `max_steps` steps and
    /// counts at most `max_gap` seconds of wall-clock time, so a stall or a pause is dropped
    /// rather than caught up.
    pub fn new(step: f32, max_steps: usize, max_gap: f32) -> Self {
        Self {
            step,
            max_steps,
            max_gap,
            clock: None,
            accumulator: 0.0,
        }
    }

    /// Advances to wall-clock `time`, calling `tick(step)` once per whole step taken; returns how
    /// far the frame lies into the next step, in [0, 1]. The first call, an unchanged `time` and
    /// a `time` that went back (a rewind) take no steps; the clock resumes from the new time.
    pub fn advance(&mut self, time: f32, mut tick: impl FnMut(f32)) -> f32 {
        let dt = match self.clock {
            Some(last) if time >= last => (time - last).min(self.max_gap),
            _ => 0.0,
        };
        self.clock = Some(time);
        self.accumulator += dt;
        let mut steps = 0;
        while self.accumulator >= self.step && steps < self.max_steps {
            tick(self.step);
            self.accumulator -= self.step;
            steps += 1;
        }
        self.accumulator = self.accumulator.min(self.step);
        (self.accumulator / self.step).clamp(0.0, 1.0)
    }
}

/// A 32-bit xorshift generator (13, 17, 5). A zero state stays zero, so seed it non-zero.
#[derive(Debug, Clone)]
pub struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    /// Starts a stream from `seed`.
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// Uniform in [0, 1). Uses the top 24 bits of the state.
    pub fn unit(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state >> 8) as f32 / (1 << 24) as f32
    }
}

/// Integer and string hashes, named by algorithm. Each keeps the exact arithmetic of the copy it
/// replaced, so existing views draw the same pixels. They hash a pid or a name alone, so a reused
/// pid repeats its predecessor's look; new code uses [`hash_identity`] instead.
pub mod hash {
    /// Knuth's multiplicative hash: the 32-bit golden ratio prime times `value`. The high bits
    /// are the well mixed ones, so callers shift right.
    pub fn knuth(value: u32) -> u32 {
        value.wrapping_mul(2_654_435_761)
    }

    /// `value` times 2_246_822_519, the second xxHash32 prime. Not the same multiplier as
    /// [`knuth`], so it gives unrelated bits from the same pid.
    pub fn xxhash_prime2(value: u32) -> u32 {
        value.wrapping_mul(2_246_822_519)
    }

    /// The reef's mix: [`knuth`] xor the rotated [`xxhash_prime2`] product.
    pub fn knuth_xor_rotated_prime2(value: u32) -> u32 {
        knuth(value) ^ xxhash_prime2(value).rotate_left(13)
    }

    /// The coop's murmur-style 32-bit mix (golden ratio multiply, then two xor-shift and multiply
    /// rounds, the second left at its xor-shift).
    pub fn murmur_mix(value: u32) -> u32 {
        let mut x = value.wrapping_mul(super::GOLDEN_RATIO_32) ^ 0x85eb_ca6b;
        x ^= x >> 15;
        x = x.wrapping_mul(0x2c1b_3c6d);
        x ^= x >> 12;
        x
    }

    /// [`murmur_mix`] as a uniform value in [0, 1), from its top 24 bits.
    pub fn unit_from(value: u32) -> f32 {
        (murmur_mix(value) >> 8) as f32 / 16_777_216.0
    }

    /// The matrix rain's glyph hash for a column, row and time tick; use the top bits.
    pub fn lattice_flicker(column: u64, row: u64, tick: u64) -> u64 {
        ((column * 0x9e37_79b9) ^ (row * 0x85eb_ca6b) ^ tick).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
    }

    /// The splitmix64 increment times `counter`, shifted down 32 bits. Serves the demo journal,
    /// which hashes a line counter rather than a process.
    pub fn golden_gamma_high(counter: u64) -> u64 {
        counter.wrapping_mul(super::SPLITMIX_GAMMA) >> 32
    }

    /// FNV-1a 32 of a string's bytes, with the offset basis and prime from the FNV specification.
    pub fn fnv1a_32(text: &str) -> u32 {
        text.bytes().fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        })
    }

    /// FNV-1a 64 of a byte stream, with the offset basis and prime from the FNV specification.
    pub fn fnv1a_64_bytes(bytes: impl IntoIterator<Item = u8>) -> u64 {
        bytes.into_iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// FNV-1a 64 of a string's bytes.
    pub fn fnv1a_64(text: &str) -> u64 {
        fnv1a_64_bytes(text.bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_and_smoothing_keep_the_arithmetic_of_the_copies_they_replaced() {
        let id = Identity { pid: 1, start: 1 };
        let births = HashMap::from([(id, 10.0)]);
        assert_eq!(
            growth(&births, id, 9.0),
            1.0,
            "born in the future counts as grown"
        );
        assert_eq!(growth(&births, id, 10.0), 0.0);
        assert_eq!(growth(&births, id, 10.0 + GROW_SECONDS), 1.0);
        let t = (10.3_f32 - 10.0) / GROW_SECONDS;
        assert_eq!(
            growth(&births, id, 10.3).to_bits(),
            (1.0 - (1.0 - t).powi(3)).to_bits()
        );
        assert_eq!(growth(&HashMap::new(), id, 0.0), 1.0);
        assert_eq!(sprout(0.0), 0.3);
        assert_eq!(sprout(0.5).to_bits(), (0.3_f32 + 0.7 * 0.5).to_bits());
        for dt in [0.0_f32, 0.016, 0.1, 1.0 / 3.0] {
            for tau in [0.2_f32, 0.3, 1.5] {
                assert_eq!(
                    smoothing(dt, tau).to_bits(),
                    (1.0 - (-dt / tau).exp()).to_bits()
                );
            }
        }
    }

    fn brute_force(points: &[[f32; 2]], point: [f32; 2], radius: f32) -> Vec<usize> {
        (0..points.len())
            .filter(|&index| {
                let dx = points[index][0] - point[0];
                let dy = points[index][1] - point[1];
                dx * dx + dy * dy < radius * radius
            })
            .collect()
    }

    #[test]
    fn spatial_hash_finds_exactly_the_points_brute_force_finds() {
        let mut rng = Rng::new(7);
        let mut hash = SpatialHash::new(2.0);
        let mut found = Vec::new();
        for (count, extent) in [(10, 5.0), (200, 20.0), (1500, 50.0), (400, 3.0)] {
            let points: Vec<[f32; 2]> = (0..count)
                .map(|_| [rng.signed() * extent, rng.signed() * extent])
                .collect();
            hash.rebuild(&points);
            for radius in [0.3, 1.0, 2.0, 3.7, 9.0, 200.0] {
                for &query in points.iter().take(40) {
                    hash.neighbours(query, radius, &mut found);
                    assert_eq!(found, brute_force(&points, query, radius));
                }
                let outside = [rng.signed() * extent * 2.0, rng.signed() * extent * 2.0];
                hash.neighbours(outside, radius, &mut found);
                assert_eq!(found, brute_force(&points, outside, radius));
            }
        }
    }

    #[test]
    fn spatial_hash_handles_empty_and_coincident_points() {
        let mut hash = SpatialHash::new(1.0);
        let mut found = vec![99];
        hash.rebuild(&[]);
        hash.neighbours([0.0, 0.0], 5.0, &mut found);
        assert!(found.is_empty());

        hash.rebuild(&[[-1.5, -1.5], [-1.5, -1.5], [-1.5, -1.5], [4.0, 4.0]]);
        hash.neighbours([-1.5, -1.5], 0.5, &mut found);
        assert_eq!(found, vec![0, 1, 2]);
        hash.neighbours([-1.5, -1.5], 100.0, &mut found);
        assert_eq!(found, vec![0, 1, 2, 3]);
        hash.neighbours([4.0, 4.0], 0.0, &mut found);
        assert!(found.is_empty());
    }

    #[test]
    fn a_radius_beyond_the_cell_range_scans_every_point_without_overflowing() {
        let mut hash = SpatialHash::new(1.0e-3);
        let mut found = Vec::new();
        hash.rebuild(&[[0.0, 0.0], [1.0e6, -1.0e6], [-3.0, 7.0]]);
        hash.neighbours([0.0, 0.0], 1.0e7, &mut found);
        assert_eq!(found, vec![0, 1, 2]);
        hash.neighbours([0.0, 0.0], f32::INFINITY, &mut found);
        assert_eq!(found, vec![0, 1, 2]);
    }

    #[test]
    fn rng_repeats_for_the_same_identity_and_differs_across_identities() {
        let identity = |pid| Identity { pid, start: 1234 };
        let draw = |id, salt| {
            let mut rng = Rng::for_identity(id, salt);
            [rng.next_u64(), rng.next_u64(), rng.next_u64()]
        };
        assert_eq!(draw(identity(100), 1), draw(identity(100), 1));
        assert_ne!(draw(identity(100), 1), draw(identity(101), 1));
        assert_ne!(draw(identity(100), 1), draw(identity(100), 2));
        let other_start = Identity {
            pid: 100,
            start: 1235,
        };
        assert_ne!(draw(identity(100), 1), draw(other_start, 1));
    }

    #[test]
    fn rng_values_stay_in_range_and_spread_evenly() {
        let mut rng = Rng::new(42);
        let draws = 100_000;
        let mut sum = 0.0f64;
        for _ in 0..draws {
            let unit = rng.unit();
            assert!((0.0..1.0).contains(&unit));
            assert!((-1.0..1.0).contains(&rng.signed()));
            sum += f64::from(unit);
        }
        let mean = sum / f64::from(draws);
        assert!((mean - 0.5).abs() < 0.01, "mean was {mean}");
    }

    fn random_signal(rng: &mut Rng, length: usize) -> Vec<Complex> {
        (0..length)
            .map(|_| Complex::new(rng.signed(), rng.signed()))
            .collect()
    }

    /// Relative error in the Euclidean norm: `|actual - expected| / |expected|`.
    fn relative_error(actual: &[Complex], expected: &[Complex]) -> f64 {
        assert_eq!(actual.len(), expected.len());
        let (mut difference, mut reference) = (0.0_f64, 0.0_f64);
        for (a, e) in actual.iter().zip(expected) {
            difference += f64::from((*a - *e).norm()).powi(2);
            reference += f64::from(e.norm()).powi(2);
        }
        (difference / reference.max(f64::MIN_POSITIVE)).sqrt()
    }

    #[test]
    fn fft_matches_the_direct_dft_for_every_power_of_two_up_to_1024() {
        let mut rng = Rng::new(11);
        for bits in 0..=10 {
            let length = 1_usize << bits;
            let signal = random_signal(&mut rng, length);
            let expected = dft(&signal);
            let mut buffer = signal.clone();
            fft(&mut buffer);
            // The f32 butterflies round about log2(N) times; 1e-5 of the spectrum's norm is
            // roughly ten times what that costs at N = 1024 (about 1e-6 measured).
            let error = relative_error(&buffer, &expected);
            assert!(error < 1.0e-5, "length {length}: relative error {error}");
        }
    }

    #[test]
    fn fft_follows_the_negative_exponent_convention() {
        // A cosine at bin 1 puts half its amplitude at bins 1 and N-1; a sine at bin 1 is
        // -N/2 i at bin 1 for exp(-i...) and +N/2 i at bin N-1.
        let length = 8;
        let mut sine: Vec<Complex> = (0..length)
            .map(|index| {
                Complex::new(
                    (std::f32::consts::TAU * index as f32 / length as f32).sin(),
                    0.0,
                )
            })
            .collect();
        fft(&mut sine);
        assert!((sine[1].im + 4.0).abs() < 1.0e-5, "bin 1 was {:?}", sine[1]);
        assert!((sine[7].im - 4.0).abs() < 1.0e-5, "bin 7 was {:?}", sine[7]);
    }

    #[test]
    fn inverse_fft_restores_the_input() {
        let mut rng = Rng::new(12);
        for length in [2_usize, 16, 256, 1024] {
            let signal = random_signal(&mut rng, length);
            let mut buffer = signal.clone();
            fft(&mut buffer);
            inverse_fft(&mut buffer);
            let error = relative_error(&buffer, &signal);
            assert!(error < 1.0e-5, "length {length}: relative error {error}");
        }
    }

    #[test]
    fn fft_of_length_one_is_the_identity() {
        let mut buffer = [Complex::new(3.5, -2.0)];
        fft(&mut buffer);
        assert_eq!(buffer, [Complex::new(3.5, -2.0)]);
        inverse_fft(&mut buffer);
        assert_eq!(buffer, [Complex::new(3.5, -2.0)]);
        let mut empty: [Complex; 0] = [];
        fft(&mut empty);
        inverse_fft(&mut empty);
    }

    #[test]
    #[should_panic(expected = "length, got 12")]
    fn fft_panics_on_a_length_that_is_not_a_power_of_two() {
        fft(&mut [Complex::default(); 12]);
    }

    #[test]
    #[should_panic(expected = "length, got 6")]
    fn inverse_fft_panics_on_a_length_that_is_not_a_power_of_two() {
        inverse_fft(&mut [Complex::default(); 6]);
    }

    #[test]
    fn dft_reconstructs_samples_at_integer_times() {
        let mut rng = Rng::new(13);
        let length = 60;
        let samples = random_signal(&mut rng, length);
        let coefficients = dft(&samples);
        for (time, sample) in samples.iter().enumerate() {
            // x[n] = (1/N) sum over k of X[k] exp(+2 pi i k n / N)
            let (mut re, mut im) = (0.0_f64, 0.0_f64);
            for (frequency, coefficient) in coefficients.iter().enumerate() {
                let turns = ((frequency * time) % length) as f64 / length as f64;
                let (sin, cos) = (std::f64::consts::TAU * turns).sin_cos();
                re += f64::from(coefficient.re) * cos - f64::from(coefficient.im) * sin;
                im += f64::from(coefficient.re) * sin + f64::from(coefficient.im) * cos;
            }
            let (re, im) = (re / length as f64, im / length as f64);
            assert!((re - f64::from(sample.re)).abs() < 1.0e-5, "re at {time}");
            assert!((im - f64::from(sample.im)).abs() < 1.0e-5, "im at {time}");
        }
    }

    #[test]
    fn complex_arithmetic_matches_the_definitions() {
        let a = Complex::new(1.0, 2.0);
        let b = Complex::new(3.0, -4.0);
        assert_eq!(a + b, Complex::new(4.0, -2.0));
        assert_eq!(a - b, Complex::new(-2.0, 6.0));
        assert_eq!(a * b, Complex::new(11.0, 2.0));
        assert_eq!(b.norm(), 5.0);
        assert_eq!(a.conj(), Complex::new(1.0, -2.0));
        assert_eq!(Complex::default(), Complex::new(0.0, 0.0));
    }

    fn brute_force_nearest(points: &[[f32; 3]], point: [f32; 3], k: usize) -> Vec<(usize, f32)> {
        let mut all: Vec<(usize, f32)> = points
            .iter()
            .enumerate()
            .map(|(index, other)| {
                let dx = other[0] - point[0];
                let dy = other[1] - point[1];
                let dz = other[2] - point[2];
                (index, (dx * dx + dy * dy + dz * dz).sqrt())
            })
            .collect();
        all.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        all.truncate(k);
        all
    }

    fn random_points(rng: &mut Rng, count: usize, extent: f32) -> Vec<[f32; 3]> {
        (0..count)
            .map(|_| {
                [
                    rng.signed() * extent,
                    rng.signed() * extent,
                    rng.signed() * extent,
                ]
            })
            .collect()
    }

    #[test]
    fn nearest_finds_exactly_what_brute_force_finds() {
        let mut rng = Rng::new(21);
        let mut found = Vec::new();
        for cell in [0.5_f32, 4.0, 40.0] {
            let mut hash = SpatialHash3::new(cell);
            let mut cases = vec![
                random_points(&mut rng, 700, 30.0),
                // A dense ball inside a wide empty box, and an offset one, exercise empty shells.
                random_points(&mut rng, 300, 2.0),
                random_points(&mut rng, 5, 50.0),
            ];
            // Coincident points tie on distance, so the index decides.
            let mut coincident = random_points(&mut rng, 40, 10.0);
            coincident.extend(std::iter::repeat_n([1.25, -3.5, 7.0], 30));
            coincident.extend(std::iter::repeat_n([-5.0, -5.0, -5.0], 3));
            cases.push(coincident);
            for points in cases {
                hash.rebuild(&points);
                let mut queries: Vec<[f32; 3]> = points.iter().copied().take(60).collect();
                queries.extend(random_points(&mut rng, 20, 80.0));
                queries.push([1.25, -3.5, 7.0]);
                queries.push([1.0e6, -1.0e6, 1.0e6]);
                for query in queries {
                    for k in [1, 7, 22, points.len(), points.len() + 10] {
                        hash.nearest(query, k, &mut found);
                        let expected = brute_force_nearest(&points, query, k);
                        assert_eq!(found, expected, "cell {cell}, k {k}, query {query:?}");
                        assert_eq!(found.len(), k.min(points.len()));
                    }
                }
            }
        }
    }

    #[test]
    fn nearest_includes_the_query_point_itself_at_distance_zero() {
        let mut hash = SpatialHash3::new(1.0);
        let points = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 3.0, 0.0]];
        hash.rebuild(&points);
        let mut found = Vec::new();
        hash.nearest(points[2], 2, &mut found);
        assert_eq!(found, vec![(2, 0.0), (0, 3.0)]);
        hash.nearest(points[0], 0, &mut found);
        assert!(found.is_empty());
    }

    #[test]
    fn nearest_on_an_empty_grid_returns_nothing() {
        let mut hash = SpatialHash3::new(2.0);
        let mut found = vec![(9, 9.0)];
        hash.nearest([0.0, 0.0, 0.0], 5, &mut found);
        assert!(found.is_empty());
        hash.rebuild(&[]);
        found.push((9, 9.0));
        hash.nearest([1.0, 2.0, 3.0], 1, &mut found);
        assert!(found.is_empty());
        let mut indices = vec![1];
        hash.neighbours([0.0, 0.0, 0.0], 5.0, &mut indices);
        assert!(indices.is_empty());
    }

    #[test]
    fn nearest_scans_every_point_for_extreme_or_non_finite_coordinates() {
        let mut hash = SpatialHash3::new(1.0e-3);
        let points = [
            [0.0, 0.0, 0.0],
            [1.0e30, 0.0, 0.0],
            [-3.0, 7.0, 2.0],
            [-1.0e30, 0.0, 0.0],
        ];
        hash.rebuild(&points);
        let mut found = Vec::new();
        let queries = [
            [0.0, 0.0, 0.0],
            [1.0e30, 1.0e30, 0.0],
            [-1.0e30, -1.0e30, 0.0],
            [f32::NAN, 0.0, 0.0],
        ];
        for query in queries {
            hash.nearest(query, 3, &mut found);
            let expected = brute_force_nearest(&points, query, 3);
            assert_eq!(found.len(), 3);
            for (got, want) in found.iter().zip(&expected) {
                assert_eq!(got.0, want.0);
                assert!(got.1 == want.1 || got.1.is_nan() && want.1.is_nan());
            }
        }
        hash.rebuild(&[[0.0, 0.0, 0.0], [f32::INFINITY, 0.0, 0.0]]);
        hash.nearest([1.0, 0.0, 0.0], 1, &mut found);
        assert_eq!(found, vec![(0, 1.0)]);
        let mut indices = Vec::new();
        hash.neighbours([0.0, 0.0, 0.0], f32::INFINITY, &mut indices);
        assert_eq!(indices, vec![0]);
    }

    #[test]
    fn rebuild_at_a_steady_point_count_keeps_its_buffers() {
        let mut rng = Rng::new(41);
        let mut hash = SpatialHash3::new(4.0);
        hash.rebuild(&random_points(&mut rng, 500, 30.0));
        let capacities = |hash: &SpatialHash3| {
            [
                hash.points.capacity(),
                hash.ordered.capacity(),
                hash.keys.capacity(),
                hash.order.capacity(),
            ]
        };
        let before = capacities(&hash);
        let ranges = hash.ranges.capacity();
        // Fewer cells are occupied once the points sit closer together.
        hash.rebuild(&random_points(&mut rng, 500, 10.0));
        assert_eq!(capacities(&hash), before);
        assert_eq!(hash.ranges.capacity(), ranges);
    }

    fn brute_force_neighbours(points: &[[f32; 3]], point: [f32; 3], radius: f32) -> Vec<usize> {
        (0..points.len())
            .filter(|&index| {
                let dx = points[index][0] - point[0];
                let dy = points[index][1] - point[1];
                let dz = points[index][2] - point[2];
                dx * dx + dy * dy + dz * dz < radius * radius
            })
            .collect()
    }

    #[test]
    fn neighbours_in_3d_finds_exactly_what_brute_force_finds() {
        let mut rng = Rng::new(31);
        let mut hash = SpatialHash3::new(2.0);
        let mut found = Vec::new();
        for (count, extent) in [(10, 5.0), (200, 20.0), (1500, 50.0), (400, 3.0)] {
            let points = random_points(&mut rng, count, extent);
            hash.rebuild(&points);
            for radius in [0.0, 0.3, 1.0, 2.0, 3.7, 9.0, 200.0, 1.0e7, f32::INFINITY] {
                for &query in points.iter().take(40) {
                    hash.neighbours(query, radius, &mut found);
                    assert_eq!(found, brute_force_neighbours(&points, query, radius));
                }
                let outside = [
                    rng.signed() * extent * 2.0,
                    rng.signed() * extent * 2.0,
                    rng.signed() * extent * 2.0,
                ];
                hash.neighbours(outside, radius, &mut found);
                assert_eq!(found, brute_force_neighbours(&points, outside, radius));
            }
        }
    }

    #[test]
    fn a_3d_radius_beyond_the_cell_range_scans_every_point_without_overflowing() {
        let mut hash = SpatialHash3::new(1.0e-3);
        let mut found = Vec::new();
        hash.rebuild(&[[0.0; 3], [1.0e6, -1.0e6, 5.0], [-3.0, 7.0, 2.0]]);
        hash.neighbours([0.0; 3], 1.0e7, &mut found);
        assert_eq!(found, vec![0, 1, 2]);
        hash.neighbours([0.0; 3], f32::INFINITY, &mut found);
        assert_eq!(found, vec![0, 1, 2]);
    }

    #[test]
    fn rng_normal_has_zero_mean_and_unit_variance() {
        let mut rng = Rng::new(42);
        let draws = 100_000;
        let (mut sum, mut sum_of_squares) = (0.0_f64, 0.0_f64);
        for _ in 0..draws {
            let value = f64::from(rng.normal());
            assert!(value.is_finite());
            sum += value;
            sum_of_squares += value * value;
        }
        // The standard error of the mean is 0.003 and of the variance 0.0045 at this count, so
        // 0.02 is more than four standard errors either way.
        let mean = sum / f64::from(draws);
        let variance = sum_of_squares / f64::from(draws) - mean * mean;
        assert!(mean.abs() < 0.02, "mean was {mean}");
        assert!((variance - 1.0).abs() < 0.02, "variance was {variance}");
    }

    #[test]
    fn rng_normal_is_deterministic_per_seed() {
        let draw = |seed| {
            let mut rng = Rng::new(seed);
            (0..16).map(|_| rng.normal().to_bits()).collect::<Vec<_>>()
        };
        assert_eq!(draw(5), draw(5));
        assert_ne!(draw(5), draw(6));
    }

    #[test]
    fn rng_normal_leaves_the_uniform_stream_alone() {
        // `normal` consumes exactly two draws and keeps no extra state.
        let mut with_normal = Rng::new(9);
        let mut uniform = Rng::new(9);
        with_normal.normal();
        uniform.unit();
        uniform.unit();
        assert_eq!(with_normal.next_u64(), uniform.next_u64());
    }

    /// The flock of `docs/mockups/code/murmuration.py` scaled to `count` birds: its 11 processes
    /// in their thread proportions, clump centres normal with sigma 18, each bird normal with
    /// sigma 14 around its clump, the whole flock 60 up.
    fn flock_points(count: usize) -> Vec<[f32; 3]> {
        const THREADS: [usize; 11] = [1100, 400, 260, 180, 60, 220, 40, 20, 480, 90, 900];
        let total: usize = THREADS.iter().sum();
        let mut rng = Rng::new(51);
        let centres: Vec<[f32; 3]> = THREADS
            .iter()
            .map(|_| {
                [
                    rng.normal() * 18.0,
                    rng.normal() * 18.0,
                    rng.normal() * 18.0,
                ]
            })
            .collect();
        let mut points = Vec::with_capacity(count);
        for (process, threads) in THREADS.iter().enumerate() {
            let birds = if process + 1 == THREADS.len() {
                count - points.len()
            } else {
                count * threads / total
            };
            for _ in 0..birds {
                let centre = centres[process];
                points.push([
                    centre[0] + rng.normal() * 14.0,
                    centre[1] + rng.normal() * 14.0,
                    centre[2] + rng.normal() * 14.0 + 60.0,
                ]);
            }
        }
        points
    }

    /// Run with `cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing; run in release"]
    fn timing_nearest_22_of_8000_flock_points_and_fft_1024() {
        use std::time::Instant;

        let points = flock_points(8000);
        // The 22nd neighbour of a flock bird is about 5 units away.
        let mut hash = SpatialHash3::new(6.5);
        let mut found = Vec::new();
        let mut best_nearest = f64::INFINITY;
        let mut best_rebuild = f64::INFINITY;
        let mut checksum = 0_usize;
        for _ in 0..50 {
            let start = Instant::now();
            hash.rebuild(&points);
            best_rebuild = best_rebuild.min(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            for &point in &points {
                hash.nearest(point, 22, &mut found);
                checksum += found.last().map_or(0, |last| last.0);
            }
            best_nearest = best_nearest.min(start.elapsed().as_secs_f64() * 1000.0);
        }

        let mut rng = Rng::new(52);
        let signal = random_signal(&mut rng, 1024);
        let mut buffer = signal.clone();
        let mut best_fft = f64::INFINITY;
        for _ in 0..200 {
            buffer.copy_from_slice(&signal);
            let start = Instant::now();
            fft(&mut buffer);
            best_fft = best_fft.min(start.elapsed().as_secs_f64() * 1000.0);
        }
        std::hint::black_box((checksum, &buffer));
        println!(
            "nearest k=22 x 8000: {best_nearest:.3} ms (rebuild {best_rebuild:.3} ms); \
             fft 1024: {:.1} us",
            best_fft * 1000.0
        );
        if !cfg!(debug_assertions) {
            assert!(best_nearest < 10.0, "nearest took {best_nearest} ms");
        }
    }

    /// Values captured from the hand-rolled copies before they moved into `hash`.
    #[test]
    fn moved_hashes_return_the_values_they_returned_before() {
        let inputs = [0u32, 1, 17, 4242, 123_456, u32::MAX];
        let murmur = [
            3_376_160_260,
            3_879_208_680,
            384_141_697,
            2_575_393_800,
            1_141_118_084,
            1_760_540_593,
        ];
        let unit_bits = [
            1_061_764_126,
            1_063_729_158,
            1_035_414_600,
            1_058_636_132,
            1_049_102_376,
            1_053_941_614,
        ];
        let knuth = [
            0,
            2_654_435_761,
            2_175_734_977,
            3_007_215_346,
            16_625_216,
            1_640_531_535,
        ];
        let prime2 = [
            0,
            2_246_822_519,
            3_836_244_455,
            488_695_774,
            1_848_028_096,
            2_048_144_777,
        ];
        let reef = [
            0,
            3_883_501_836,
            2_408_835_156,
            2_948_969_814,
            3_582_305_156,
            3_883_510_029,
        ];
        for (index, &value) in inputs.iter().enumerate() {
            assert_eq!(
                hash::murmur_mix(value),
                murmur[index],
                "murmur_mix({value})"
            );
            assert_eq!(
                hash::unit_from(value).to_bits(),
                unit_bits[index],
                "unit_from({value})"
            );
            assert_eq!(hash::knuth(value), knuth[index], "knuth({value})");
            assert_eq!(
                hash::xxhash_prime2(value),
                prime2[index],
                "xxhash_prime2({value})"
            );
            assert_eq!(
                hash::knuth_xor_rotated_prime2(value),
                reef[index],
                "knuth_xor_rotated_prime2({value})"
            );
        }

        for (text, expected) in [
            ("", 14_695_981_039_346_656_037_u64),
            ("a", 12_638_187_200_555_641_996),
            ("dbus.service", 8_807_976_658_787_556_922),
            ("System /system.slice Ünï", 16_568_780_104_258_180_365),
        ] {
            assert_eq!(hash::fnv1a_64(text), expected, "fnv1a_64({text:?})");
        }
        // The published FNV-1a test vectors.
        assert_eq!(hash::fnv1a_32(""), 0x811c_9dc5);
        assert_eq!(hash::fnv1a_32("a"), 0xe40c_292c);

        for (counter, expected) in [
            (0, 0),
            (1, 2_654_435_769),
            (2, 1_013_904_242),
            (1000, 145_980_569),
            (123_456_789, 3_195_129_860),
            (u64::MAX, 1_640_531_526),
        ] {
            assert_eq!(
                hash::golden_gamma_high(counter),
                expected,
                "golden_gamma_high({counter})"
            );
        }

        let mut flow = XorShift32::new(GOLDEN_RATIO_32);
        let draws: Vec<u32> = (0..8).map(|_| flow.unit().to_bits()).collect();
        assert_eq!(
            draws,
            [
                1_050_810_508,
                1_063_267_925,
                1_056_404_254,
                1_002_558_464,
                1_063_662_445,
                1_064_929_746,
                1_060_775_209,
                1_065_155_565
            ]
        );
    }

    #[test]
    fn hash_identity_separates_reused_pids_by_start_time() {
        let first = Identity {
            pid: 4242,
            start: 100,
        };
        let reused = Identity {
            pid: 4242,
            start: 90_000,
        };
        assert_ne!(hash_identity(first, 0), hash_identity(reused, 0));
        assert_ne!(hash_identity(first, 0), hash_identity(first, 1));
        assert_eq!(hash_identity(first, 7), hash_identity(first, 7));
        assert_eq!(
            Rng::for_identity(first, 7).next_u64(),
            Rng::new(hash_identity(first, 7)).next_u64()
        );
        // The pid-only hashes cannot tell the two processes apart; that is why new code avoids them.
        assert_eq!(hash::knuth(first.pid), hash::knuth(reused.pid));
    }

    /// Every `.rs` file under `directory`, depth first.
    fn rust_sources(directory: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(directory).expect("source directory is readable") {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                rust_sources(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }

    #[test]
    fn no_hand_rolled_hashes_outside_simulation() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_sources(&root, &mut files);
        assert!(files.len() > 10, "found only {} source files", files.len());
        // Built piecewise so this file does not match its own needles. Underscores are dropped
        // before matching, so `2_654_435_761` and `2654435761` both count.
        let knuth = concat!("wrapping_mul(", "2654435761");
        let golden = concat!("0x", "9e3779b9");
        let splitmix_tail = concat!("7f4a", "7c15");
        let mut offenders = Vec::new();
        for path in files {
            if path.file_name().is_some_and(|name| name == "simulation.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("source file is readable");
            let squeezed: String = text.chars().filter(|&character| character != '_').collect();
            // The 64-bit splitmix increment starts with the same digits as the 32-bit golden ratio.
            let bare_golden = squeezed
                .match_indices(golden)
                .any(|(at, _)| !squeezed[at + golden.len()..].starts_with(splitmix_tail));
            let splitmix_increment = squeezed.contains(&format!("{golden}{splitmix_tail}"));
            if squeezed.contains(knuth) || bare_golden || splitmix_increment {
                offenders.push(path.strip_prefix(&root).unwrap().display().to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "hand-rolled hash or random constants in {offenders:?}: use simulation::hash, \
             simulation::Rng or simulation::XorShift32 instead of copying them"
        );
    }

    #[test]
    fn fixed_steps_follow_the_same_path_at_ten_and_sixty_frames_per_second() {
        // A damped oscillator, so any difference in the step sequence would show in the state.
        fn run(frames_per_second: u32) -> HashMap<usize, [f32; 2]> {
            let mut clock = FixedStep::new(0.05, 10, 0.5);
            let mut state = [1.0_f32, 0.0];
            let mut steps = 0;
            let mut states = HashMap::new();
            for frame in 0..=(3 * frames_per_second) {
                let time = 100.0 + frame as f32 / frames_per_second as f32;
                clock.advance(time, |step| {
                    let acceleration = -40.0 * state[0] - 0.8 * state[1];
                    state[1] += acceleration * step;
                    state[0] += state[1] * step;
                    steps += 1;
                });
                states.insert(steps, state);
            }
            states
        }
        let slow = run(10);
        let fast = run(60);
        let mut shared = 0;
        for (steps, state) in &slow {
            if let Some(other) = fast.get(steps) {
                assert_eq!(state, other, "state after {steps} steps");
                shared += 1;
            }
        }
        assert!(shared >= 25, "only {shared} shared step counts");
    }

    #[test]
    fn a_clock_that_does_not_advance_takes_no_steps() {
        let mut clock = FixedStep::new(0.05, 10, 0.5);
        let mut ticks = 0;
        assert_eq!(clock.advance(100.0, |_| ticks += 1), 0.0);
        for _ in 0..5 {
            assert_eq!(clock.advance(100.0, |_| ticks += 1), 0.0);
        }
        assert_eq!(ticks, 0);
    }

    #[test]
    fn a_clock_that_goes_back_takes_no_steps_and_resumes_from_the_new_time() {
        let mut clock = FixedStep::new(0.25, 10, 10.0);
        let mut ticks = 0;
        clock.advance(10.0, |_| ticks += 1);
        clock.advance(11.0, |_| ticks += 1);
        assert_eq!(ticks, 4);
        clock.advance(3.0, |_| ticks += 1);
        assert_eq!(ticks, 4, "a rewind takes no steps");
        clock.advance(3.5, |_| ticks += 1);
        assert_eq!(
            ticks, 6,
            "half a second after the new time is two steps, not a jump back"
        );
    }

    #[test]
    fn a_long_stall_runs_at_most_max_steps_and_drops_the_rest() {
        let mut clock = FixedStep::new(0.05, 10, 100.0);
        let mut ticks = 0;
        clock.advance(0.0, |_| ticks += 1);
        clock.advance(10.0, |_| ticks += 1);
        assert_eq!(ticks, 10);
        // The clamped backlog is one step, so a small advance completes it and nothing more.
        clock.advance(10.01, |_| ticks += 1);
        assert_eq!(ticks, 11, "one more step, not 190 steps of catching up");
        clock.advance(10.02, |_| ticks += 1);
        assert_eq!(ticks, 11);
    }

    #[test]
    fn the_returned_fraction_stays_within_zero_and_one() {
        let mut clock = FixedStep::new(0.05, 10, 0.5);
        let mut time = 5.0_f32;
        for gap in [0.0, 0.01, 0.049, 0.05, 0.2, 3.0, 0.001, 0.0] {
            time += gap;
            let fraction = clock.advance(time, |_| {});
            assert!(
                (0.0..=1.0).contains(&fraction),
                "{fraction} after a gap of {gap}"
            );
        }
    }
}
