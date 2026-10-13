//! What one ant stands for: K, the smallest of 1, 2, 5, 10, 20, 50, ... that keeps about 3,000 ants
//! on the ground, and the rule that K only changes once three samples in a row ask for the same
//! change, so a burst of I/O does not make the colony's whole population flicker.

/// How many ants the scale aims to have walking at once.
pub const TARGET_ANTS: f32 = 3000.0;

/// How many consecutive samples must agree on a new K before it takes effect.
pub const AGREEING: u8 = 3;

/// The smallest K in the 1-2-5 sequence for which `load / K` is at most `TARGET_ANTS`, where
/// `load` is the sum over streams of units per second times seconds per round trip: the ants that
/// would be walking at K = 1.
pub fn smallest(load: f32) -> u64 {
    if !load.is_finite() || load <= TARGET_ANTS {
        return 1;
    }
    let mut decade = 1_u64;
    loop {
        for step in [1, 2, 5] {
            let k = decade * step;
            if load / k as f32 <= TARGET_ANTS {
                return k;
            }
        }
        decade = decade.saturating_mul(10);
        if decade > 1_000_000_000_000 {
            return decade;
        }
    }
}

/// K as the colony holds it between samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    k: u64,
    /// K has been set from a sample that had something to carry. Until then it is 1 and the first
    /// such sample sets it at once, so that a view that has just opened does not send thousands of
    /// ants at K = 1 while three samples agree.
    settled: bool,
    /// A K the last samples asked for, and how many in a row.
    asked: Option<(u64, u8)>,
}

impl Default for Scale {
    fn default() -> Self {
        Scale {
            k: 1,
            settled: false,
            asked: None,
        }
    }
}

impl Scale {
    /// The first sample with anything to carry sets K at once; every later one that asks for
    /// another K must be followed by `AGREEING - 1` that ask for the same, with no sample between
    /// asking for the current one or a third. Returns K.
    pub fn observe(&mut self, load: f32) -> u64 {
        let wanted = smallest(load);
        if !self.settled {
            if load > 0.0 {
                self.k = wanted;
                self.settled = true;
            }
        } else if wanted == self.k {
            self.asked = None;
        } else {
            let run = match self.asked {
                Some((asked, run)) if asked == wanted => run + 1,
                _ => 1,
            };
            if run >= AGREEING {
                self.k = wanted;
                self.asked = None;
            } else {
                self.asked = Some((wanted, run));
            }
        }
        self.k
    }

    pub fn k(&self) -> u64 {
        self.k
    }
}

/// K as the legend writes it: `50`, `1,000`.
pub fn text(k: u64) -> String {
    let digits = k.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_is_the_smallest_of_one_two_five_that_keeps_the_ants_near_three_thousand() {
        assert_eq!(smallest(0.0), 1);
        assert_eq!(smallest(3000.0), 1);
        assert_eq!(smallest(3000.1), 2);
        assert_eq!(smallest(6000.0), 2);
        assert_eq!(smallest(6001.0), 5);
        assert_eq!(smallest(15_000.0), 5);
        assert_eq!(smallest(15_001.0), 10);
        assert_eq!(smallest(150_000.0), 50);
        assert_eq!(smallest(150_001.0), 100);
        assert_eq!(smallest(f32::NAN), 1);
        for k in 0..60 {
            let load = 1000.0 * 1.37_f32.powi(k);
            let chosen = smallest(load);
            assert!(load / chosen as f32 <= TARGET_ANTS + 0.5, "{load}");
            // The next smaller step would overshoot.
            let smaller = match chosen {
                1 => continue,
                k if k.to_string().starts_with('1') => k / 2,
                k if k.to_string().starts_with('2') => k / 2,
                k => k * 2 / 5,
            };
            assert!(load / smaller as f32 > TARGET_ANTS, "{load} at {chosen}");
        }
    }

    #[test]
    fn k_steps_through_one_two_five_without_flicker() {
        let mut scale = Scale::default();
        assert_eq!(scale.observe(0.0), 1, "nothing to carry yet");
        assert_eq!(
            scale.observe(10_000.0),
            5,
            "the first sample with something to carry sets it at once"
        );
        // A burst that asks for 20 once or twice, between samples that ask for 5, changes nothing.
        for load in [60_000.0, 60_000.0, 10_000.0, 60_000.0, 10_000.0] {
            assert_eq!(scale.observe(load), 5);
        }
        // Three in a row do.
        for _ in 0..2 {
            assert_eq!(scale.observe(60_000.0), 5);
        }
        assert_eq!(scale.observe(60_000.0), 20);
        // Asking for different new values in turn never settles on either.
        for load in [200_000.0, 100_000.0, 200_000.0, 100_000.0] {
            assert_eq!(scale.observe(load), 20);
        }
        assert_eq!(scale.k(), 20);
    }

    #[test]
    fn k_is_written_with_a_comma_for_thousands() {
        assert_eq!(text(1), "1");
        assert_eq!(text(50), "50");
        assert_eq!(text(1000), "1,000");
        assert_eq!(text(20_000), "20,000");
        assert_eq!(text(500_000), "500,000");
    }
}
