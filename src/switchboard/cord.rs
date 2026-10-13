//! The cords of the switchboard, pure: the catenary a cord of fixed length hangs in between a jack
//! and a plug seat, the colour of the operator it runs to, how wide it is, and the path of a plug
//! that is being moved from one operator to another. Positions are board coordinates: `x` along
//! the board, `d` towards the viewer and `z` up.

use crate::model::CoreKind;
use crate::render::Color;
use crate::simulation::{hash, smoothstep};

/// A point in board coordinates, `[x, d, z]`.
pub type Spot = [f32; 3];

/// Seconds the plug takes to lift out of its seat, to swing over, and to settle into the new one.
pub const LIFT: f32 = 0.35;
pub const SWING: f32 = 1.05;
pub const SEAT: f32 = 0.25;
/// Seconds from pulling a plug to its being seated again.
pub const MOVE: f32 = LIFT + SWING + SEAT;

/// How high the plug rises when it is lifted, and the extra arc of the swing, in board units.
const LIFTED: f32 = 70.0;
const ARC: f32 = 60.0;
/// The plug is also drawn forward a little while it is in the air.
const FORWARD: f32 = 8.0;

/// The true catenary of a cord of arc length `length` hanging between `a` and `b`, as
/// `segments` plus 1 points from `a` to `b`. Let `h` be the horizontal span and `v` the rise. A
/// catenary `z = c + k cosh((u - u0) / k)` of that length satisfies
/// `2 k sinh(h / 2k) = sqrt(length^2 - v^2)`, whose left side falls as `k` grows, so `k` is found
/// by bisection (on its logarithm, since it ranges over many orders of magnitude). A cord no
/// longer than the chord is straight.
pub fn catenary(a: Spot, b: Spot, length: f32, segments: usize) -> Vec<Spot> {
    let (a64, b64) = (a.map(f64::from), b.map(f64::from));
    let (dx, dd) = (b64[0] - a64[0], b64[1] - a64[1]);
    let span = dx.hypot(dd);
    let rise = b64[2] - a64[2];
    let chord = span.hypot(rise);
    let length = f64::from(length).max(chord);
    let along = |fraction: f64, height: f64| -> Spot {
        [
            (a64[0] + dx * fraction) as f32,
            (a64[1] + dd * fraction) as f32,
            height as f32,
        ]
    };
    let straight = || -> Vec<Spot> {
        (0..=segments)
            .map(|k| {
                let f = k as f64 / segments as f64;
                along(f, a64[2] + rise * f)
            })
            .collect()
    };
    let target = (length * length - rise * rise).max(0.0).sqrt();
    if span < 1e-6 || target <= span * (1.0 + 1e-9) {
        return straight();
    }
    let reach = |k: f64| 2.0 * k * (span / (2.0 * k)).sinh();
    let (mut low, mut high) = ((span * 1e-3).ln(), (span * 1e7).ln());
    for _ in 0..80 {
        let middle = (low + high) / 2.0;
        if reach(middle.exp()) > target {
            low = middle;
        } else {
            high = middle;
        }
    }
    let k = ((low + high) / 2.0).exp();
    let start = span / 2.0 - k * (rise / length).clamp(-0.999_999, 0.999_999).atanh();
    let base = a64[2] - k * (-start / k).cosh();
    (0..=segments)
        .map(|n| {
            let fraction = n as f64 / segments as f64;
            let u = span * fraction;
            along(fraction, base + k * ((u - start) / k).cosh())
        })
        .collect()
}

/// The length of a polyline.
pub fn polyline_length(points: &[Spot]) -> f32 {
    points
        .windows(2)
        .map(|pair| {
            let d = [
                pair[1][0] - pair[0][0],
                pair[1][1] - pair[0][1],
                pair[1][2] - pair[0][2],
            ];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        })
        .sum()
}

/// How much longer than its chord a line's cord is, 1.02 to 1.07, from the line's number.
pub fn slack(line: u64) -> f32 {
    let unit = hash::unit_from(hash::fnv1a_64_bytes(line.to_le_bytes()) as u32 ^ 0x5bd1_e995);
    1.02 + 0.05 * unit
}

/// A cord's width in board units: wider for a busier line, `0.6 + 0.5 log10(rate + 1)` clamped.
pub fn width(rate: f32) -> f32 {
    (0.6 + 0.5 * (rate.max(0.0) + 1.0).log10()).clamp(0.6, 3.0)
}

/// HLS to sRGB bytes, hue in turns.
fn hls(hue: f32, lightness: f32, saturation: f32) -> Color {
    let m2 = if lightness <= 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let m1 = 2.0 * lightness - m2;
    let channel = |turn: f32| -> u8 {
        let turn = turn.rem_euclid(1.0);
        let value = if turn < 1.0 / 6.0 {
            m1 + (m2 - m1) * turn * 6.0
        } else if turn < 0.5 {
            m2
        } else if turn < 2.0 / 3.0 {
            m1 + (m2 - m1) * (2.0 / 3.0 - turn) * 6.0
        } else {
            m1
        };
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [
        channel(hue + 1.0 / 3.0),
        channel(hue),
        channel(hue - 1.0 / 3.0),
    ]
}

/// The colour of the cords that run to an operator: warm cloth dyes for performance cores, cool
/// ones for efficiency cores and cores of unknown kind. `ordinal` counts the operator among its
/// kind, from 0.
pub fn operator_color(kind: CoreKind, ordinal: usize) -> Color {
    const WARM_HUE: [f32; 8] = [0.0, 0.035, 0.07, 0.105, 0.135, 0.955, 0.02, 0.09];
    const WARM_LIGHT: [f32; 8] = [0.52, 0.60, 0.50, 0.62, 0.55, 0.50, 0.40, 0.42];
    const COOL_HUE: [f32; 8] = [0.47, 0.42, 0.53, 0.38, 0.57, 0.50, 0.33, 0.62];
    const COOL_LIGHT: [f32; 8] = [0.50, 0.45, 0.58, 0.52, 0.55, 0.40, 0.46, 0.60];
    let slot = ordinal % 8;
    match kind {
        CoreKind::Performance => hls(WARM_HUE[slot], WARM_LIGHT[slot], 0.72),
        CoreKind::Efficiency | CoreKind::Unknown => hls(COOL_HUE[slot], COOL_LIGHT[slot], 0.55),
    }
}

/// Where a cord's plug is part of the way through a move, and how far along it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plug {
    /// Board position of the plug's tip.
    pub at: Spot,
    /// How far above its seat it is, for the cord's extra slack and the plug's stem.
    pub lifted: f32,
    /// Progress of the colour crossfade, 0 at the old operator and 1 at the new.
    pub blend: f32,
}

/// The plug `age` seconds after a move began, from seat `from` to seat `to`: it lifts, swings
/// over on an arc under a smoothstep, then settles.
pub fn plug_at(from: Spot, to: Spot, age: f32) -> Plug {
    let lerp =
        |f: f32| -> Spot { std::array::from_fn(|axis| from[axis] * (1.0 - f) + to[axis] * f) };
    if age <= 0.0 {
        return Plug {
            at: from,
            lifted: 0.0,
            blend: 0.0,
        };
    }
    if age < LIFT {
        let f = age / LIFT;
        let lifted = LIFTED * (1.0 - (1.0 - f) * (1.0 - f));
        return Plug {
            at: [from[0], from[1] + FORWARD * f, from[2] + lifted],
            lifted,
            blend: 0.0,
        };
    }
    if age < LIFT + SWING {
        let f = smoothstep((age - LIFT) / SWING);
        let lifted = LIFTED + ARC * (std::f32::consts::PI * f).sin();
        let mut at = lerp(f);
        at[1] += FORWARD;
        at[2] += lifted;
        return Plug {
            at,
            lifted,
            blend: f,
        };
    }
    if age < MOVE {
        let f = (age - LIFT - SWING) / SEAT;
        let lifted = LIFTED * (1.0 - f).powf(1.5);
        return Plug {
            at: [to[0], to[1] + FORWARD * (1.0 - f), to[2] + lifted],
            lifted,
            blend: 1.0,
        };
    }
    Plug {
        at: to,
        lifted: 0.0,
        blend: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catenary_has_the_requested_length_and_meets_both_ends() {
        let cases = [
            ([100.0, 4.0, 300.0], [160.0, 34.0, 20.0], 1.05),
            ([0.0, 0.0, 100.0], [200.0, 0.0, 100.0], 1.2),
            ([50.0, 4.0, 50.0], [52.0, 30.0, 400.0], 1.03),
            ([10.0, 0.0, 0.0], [10.0, 40.0, 0.0], 1.5),
        ];
        for (a, b, slack) in cases {
            let chord = polyline_length(&[a, b]);
            let length = chord * slack;
            let points = catenary(a, b, length, 400);
            assert_eq!(points.len(), 401);
            for axis in 0..3 {
                assert!((points[0][axis] - a[axis]).abs() < 1e-3, "{a:?} start");
                assert!((points[400][axis] - b[axis]).abs() < 1e-2, "{b:?} end");
            }
            let measured = polyline_length(&points);
            assert!(
                (measured - length).abs() < length * 2e-3,
                "{a:?} to {b:?}: {measured} against {length}"
            );
            if slack > 1.1 {
                let lowest = points.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
                assert!(lowest < a[2].min(b[2]), "a slack cord sags below its ends");
            }
        }
    }

    #[test]
    fn a_cord_no_longer_than_its_chord_is_straight_and_coincident_ends_do_not_break() {
        let a = [0.0, 0.0, 0.0];
        let b = [30.0, 40.0, 0.0];
        let straight = catenary(a, b, 10.0, 24);
        assert!((polyline_length(&straight) - 50.0).abs() < 1e-3);
        assert!(straight.iter().all(|p| p[2] == 0.0));
        let still = catenary(a, a, 5.0, 24);
        assert_eq!(still.len(), 25);
        assert!(still.iter().all(|p| p.iter().all(|c| c.is_finite())));
    }

    #[test]
    fn slack_stays_in_the_cords_range_and_width_grows_with_the_rate() {
        for line in 0..500 {
            let s = slack(line);
            assert!((1.02..1.07).contains(&s), "{line}: {s}");
            assert_eq!(s, slack(line));
        }
        assert!(width(10_000.0) > width(10.0) && width(10.0) > width(0.0));
        assert_eq!(width(0.0), 0.6);
        assert_eq!(width(1e30), 3.0);
        assert_eq!(width(-4.0), 0.6);
    }

    #[test]
    fn a_plug_lifts_swings_and_seats_in_the_time_the_prototype_took() {
        let (from, to) = ([100.0, 34.0, 10.0], [300.0, 34.0, 10.0]);
        assert_eq!(plug_at(from, to, 0.0).at, from);
        let up = plug_at(from, to, LIFT);
        assert!((up.lifted - 70.0).abs() < 1e-3 && up.blend == 0.0);
        let half = plug_at(from, to, LIFT + SWING / 2.0);
        assert!((half.at[0] - 200.0).abs() < 1e-2 && (half.blend - 0.5).abs() < 1e-4);
        assert!(half.lifted > 120.0, "the arc peaks mid-swing");
        assert_eq!(plug_at(from, to, MOVE), plug_at(from, to, MOVE + 5.0));
        assert_eq!(plug_at(from, to, MOVE).at, to);
        let mut last = from[0];
        for step in 0..=60 {
            let x = plug_at(from, to, MOVE * step as f32 / 60.0).at[0];
            assert!(x >= last - 1e-3, "the plug never goes back");
            last = x;
        }
    }

    #[test]
    fn warm_cords_for_performance_cores_and_cool_ones_for_the_rest() {
        for ordinal in 0..8 {
            let [r, _, b] = operator_color(CoreKind::Performance, ordinal);
            assert!(r >= b, "performance {ordinal} is warm: r {r} b {b}");
        }
        assert_eq!(
            operator_color(CoreKind::Unknown, 3),
            operator_color(CoreKind::Efficiency, 3)
        );
        assert_eq!(
            operator_color(CoreKind::Efficiency, 3),
            operator_color(CoreKind::Efficiency, 11)
        );
        assert_ne!(
            operator_color(CoreKind::Efficiency, 0),
            operator_color(CoreKind::Efficiency, 1)
        );
    }
}
