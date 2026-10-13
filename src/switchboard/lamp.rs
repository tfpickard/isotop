//! The lamps of the switchboard: how bright an interrupt rate makes one, and when it flickers.
//! Pure functions of the rate and the time, so a frame at a given time is the same every run.

use crate::simulation::hash::{self, unit_from};

/// The rate at which a jack lamp reaches full brightness, and a softirq lamp: brightness is
/// `log10(rate + 1) / log10(scale)`, clamped to 1.
pub const JACK_FULL: f32 = 8000.0;
pub const SOFTIRQ_FULL: f32 = 5000.0;

/// How long a flash takes to fall to 1/e of its height, in seconds.
const DECAY: f64 = 0.05;
/// A flash older than this is gone, in seconds.
const MEMORY: f64 = 0.5;
/// How many frames' worth of the target rate the flicker may use: the cap is below the frame rate
/// so that a lamp never flickers faster than a frame can show.
const CAP: f32 = 0.9;
/// How much of its brightness a lamp shows at a flash when the line is nearly silent.
const RARE: f32 = 0.8;

/// The brightness a rate gives a lamp, 0 to 1: the logarithm of the rate, so that a line at 10
/// events a second is visibly lit next to one at 5,000.
pub fn brightness(rate: f32, full: f32) -> f32 {
    ((rate.max(0.0) + 1.0).log10() / full.log10()).clamp(0.0, 1.0)
}

/// The most flashes a second a lamp shows for `rate` events a second at `fps` frames a second.
/// The true rate belongs in the inspector.
pub fn flicker_hz(rate: f32, fps: u32) -> f32 {
    rate.clamp(0.0, CAP * fps as f32)
}

/// Where in its slot event `k` of the line `seed` falls, from 0 up to 1 (exclusive).
fn jitter(seed: u64, k: i64) -> f64 {
    let bytes = seed.to_le_bytes().into_iter().chain(k.to_le_bytes());
    f64::from(unit_from((hash::fnv1a_64_bytes(bytes) >> 16) as u32))
}

/// The times of the flashes of line `seed` in `[from, to)`: event `k` falls at
/// `(k + jitter(seed, k)) / f`, with `f` the capped rate, so there is exactly one in each slot of
/// `1/f` seconds and never more than `f` a second. Depends only on its arguments.
pub fn events(seed: u64, rate: f32, fps: u32, from: f64, to: f64) -> Vec<f64> {
    let f = f64::from(flicker_hz(rate, fps));
    if f <= 0.0 || to <= from {
        return Vec::new();
    }
    let first = (from * f).floor() as i64 - 1;
    let last = (to * f).ceil() as i64;
    (first..=last)
        .map(|k| (k as f64 + jitter(seed, k)) / f)
        .filter(|&at| (from..to).contains(&at))
        .collect()
}

/// How lit the flash of the latest event within the last half second still is, 1 at the event and
/// falling to 1/e in 50 ms; 0 when there was none.
pub fn flash(seed: u64, rate: f32, fps: u32, time: f64) -> f32 {
    events(seed, rate, fps, time - MEMORY, time + 1e-9)
        .into_iter()
        .map(|at| (-(time - at) / DECAY).exp())
        .fold(0.0, f64::max) as f32
}

/// The brightness of a jack lamp: the steady glow of the rate with the flashes of its events on
/// top; a line at under half an event a second has no steady glow and shows only its flashes.
pub fn jack(rate: f32, seed: u64, fps: u32, time: f64) -> f32 {
    let base = brightness(rate, JACK_FULL);
    let flash = flash(seed, rate, fps, time);
    if rate > 0.5 {
        base * (0.42 + 0.58 * flash)
    } else {
        base * flash * RARE
    }
}

/// The brightness of a jack lamp while its cord is out of the jack's operator: it flutters.
pub fn fluttering(rate: f32, time: f64) -> f32 {
    let flutter = 0.5 + 0.5 * (time * 30.0).cos() as f32;
    brightness(rate, JACK_FULL) * (0.42 + 0.58 * flutter)
}

/// The brightness of a softirq lamp: none under half an event a second.
pub fn softirq(rate: f32, seed: u64, fps: u32, time: f64) -> f32 {
    if rate < 0.5 {
        return 0.0;
    }
    brightness(rate, SOFTIRQ_FULL) * (0.4 + 0.6 * flash(seed, rate, fps, time))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamp_flicker_stays_under_the_cap_and_repeats_for_the_same_time() {
        for (rate, fps) in [
            (5.0, 10),
            (3688.0, 10),
            (41_203.0, 10),
            (3688.0, 60),
            (0.3, 10),
        ] {
            let cap = 0.9 * fps as f32;
            let hz = flicker_hz(rate, fps);
            assert!(hz <= cap && hz <= rate.max(0.0), "{rate} at {fps}: {hz}");
            for start in [0.0, 12.3, 1000.0] {
                let found = events(7, rate, fps, start, start + 1.0);
                // One event in each slot of 1/f: a window that is not aligned to the slots can
                // hold the tail of one and the head of the next, one more than a whole second's.
                assert!(
                    found.len() as f32 <= cap.ceil() + 1.0,
                    "{} flashes in a second at a cap of {cap}",
                    found.len()
                );
                let ten = events(7, rate, fps, start, start + 10.0).len() as f32;
                assert!(
                    ten <= cap * 10.0 + 1.0,
                    "{ten} flashes in ten seconds at a cap of {cap}"
                );
                assert_eq!(found, events(7, rate, fps, start, start + 1.0));
            }
        }
        assert_eq!(flash(7, 800.0, 10, 12.34), flash(7, 800.0, 10, 12.34));
        assert_ne!(
            events(7, 800.0, 10, 0.0, 5.0),
            events(8, 800.0, 10, 0.0, 5.0)
        );
        assert!(
            events(7, 0.0, 10, 0.0, 5.0).is_empty(),
            "a line with no events never flashes"
        );
    }

    #[test]
    fn each_slot_of_the_capped_rate_holds_exactly_one_event() {
        let (rate, fps) = (50.0, 10);
        let hz = f64::from(flicker_hz(rate, fps));
        let found = events(3, rate, fps, 0.0, 10.0);
        assert_eq!(found.len(), (hz * 10.0) as usize);
        for (slot, at) in found.iter().enumerate() {
            assert_eq!(((at * hz).floor()) as usize, slot);
        }
    }

    #[test]
    fn a_flash_is_brightest_at_its_event_and_gone_after_half_a_second() {
        let at = events(5, 6.0, 10, 20.0, 21.0)[0];
        assert!((flash(5, 6.0, 10, at) - 1.0).abs() < 1e-6);
        assert!(flash(5, 6.0, 10, at + 0.05) < 0.4 && flash(5, 6.0, 10, at + 0.05) > 0.3);
        let next = events(5, 6.0, 10, at + 1e-3, at + 3.0)[0];
        let quiet = (at + 0.5..next).contains(&(at + 0.6));
        assert!(!quiet || flash(5, 6.0, 10, at + 0.6) == 0.0);
    }

    #[test]
    fn brightness_is_logarithmic_and_clamped() {
        assert_eq!(brightness(0.0, JACK_FULL), 0.0);
        assert!((brightness(JACK_FULL - 1.0, JACK_FULL) - 1.0).abs() < 1e-6);
        assert_eq!(brightness(1e9, JACK_FULL), 1.0);
        let (ten, hundred) = (brightness(10.0, JACK_FULL), brightness(100.0, JACK_FULL));
        assert!(
            hundred > ten && (hundred - ten) < ten,
            "each decade adds about as much"
        );
        assert_eq!(brightness(-5.0, JACK_FULL), 0.0);
    }

    #[test]
    fn a_quiet_softirq_is_dark_and_a_busy_one_never_exceeds_one() {
        assert_eq!(softirq(0.2, 1, 10, 3.0), 0.0);
        for time in [0.0, 1.1, 7.7] {
            assert!(softirq(1e6, 1, 10, time) <= 1.0);
            assert!(jack(1e6, 1, 10, time) <= 1.0);
        }
    }
}
