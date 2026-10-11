//! Shared, pure simulation helpers used by views: neighbour queries over 2D points, small
//! deterministic random generators and hashes. Nothing here draws or reads the system.
//!
//! New views seed from a process with [`Rng::for_identity`] or [`hash_identity`], which mix the
//! pid and the start time so a reused pid does not look like its predecessor. Everything else in
//! [`hash`] and [`XorShift32`] exists so that existing views keep their pixels: each function
//! keeps the exact arithmetic of the hand-rolled copy it replaced, and the golden render hashes
//! would change if one of them did.

use std::collections::HashMap;

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
}
