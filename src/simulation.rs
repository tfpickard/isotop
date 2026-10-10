//! Shared, pure simulation helpers used by views: neighbour queries over 2D points and a small
//! deterministic random generator. Nothing here draws or reads the system.

use std::collections::HashMap;

use crate::model::Identity;

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

/// A small deterministic generator (splitmix64). The same seed always yields the same stream.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

/// The splitmix64 output finaliser, a bijection that scrambles every input bit.
fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
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
        Self::new(mix(mix(mix(u64::from(id.pid)) ^ id.start) ^ salt))
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.state)
    }

    /// Uniform in [0, 1). Uses 24 bits, the width of an `f32` mantissa, so 1.0 cannot occur.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    /// Uniform in [-1, 1).
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
