//! The moon as an observer sees it: its phase, where it stands in the sky and how its bright
//! limb is tilted, from the clock and (when known) a place on the earth. Pure functions of the
//! time, so a paused or rewound view puts the moon where it was.
//!
//! The positions come from the Astronomical Almanac's low-precision series for the sun and the
//! moon (a handful of periodic terms). Against PyEphem (VSOP87 and ELP) at 300 random times
//! from 2020 to 2030 and six places, the altitude is within 0.26 degrees, the azimuth within 0.6
//! and the lit fraction within 0.003, enough for a disc of 30 pixels. Time is taken as UT; the
//! minute of the difference to dynamical time moves the moon by less than the series' error.

use std::f64::consts::TAU;

/// Julian date of 1970-01-01T00:00:00Z.
const UNIX_EPOCH_JULIAN_DATE: f64 = 2_440_587.5;
/// Julian date of J2000.0.
const J2000: f64 = 2_451_545.0;

/// Where an observer sees the moon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    /// Degrees above the horizon, topocentric (the parallax of up to a degree is taken out;
    /// the refraction of half a degree at the horizon is not put back).
    pub altitude: f64,
    /// Degrees clockwise from north.
    pub azimuth: f64,
    /// Where the bright limb points, in radians counterclockwise from the direction of the
    /// zenith, as the observer looks up at the sky: 0 is lit at the top, a quarter turn lit on
    /// the left and 3/4 of a turn lit on the right. A waxing moon high in the south of a
    /// northern sky is lit on the right (towards the sun, which has set in the west); the same
    /// moon high in the north of a southern sky is lit on the left, tilted up.
    pub limb: f64,
}

/// The moon at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moon {
    /// The fraction of the disc that is lit, 0 (new) to 1 (full).
    pub lit: f64,
    /// Whether the moon is between new and full, east of the sun in the evening sky.
    pub waxing: bool,
    /// Where it stands for the observer, when the observer's place is known.
    pub sky: Option<Sky>,
}

/// A direction on the celestial sphere, in radians.
#[derive(Clone, Copy)]
struct Equatorial {
    right_ascension: f64,
    declination: f64,
}

/// Degrees of a sum of `terms` of `amplitude * sin(phase + rate * T)`, `T` in Julian centuries.
fn series(terms: &[(f64, f64, f64)], t: f64, wave: fn(f64) -> f64) -> f64 {
    terms
        .iter()
        .map(|&(amplitude, phase, rate)| amplitude * wave((phase + rate * t).to_radians()))
        .sum()
}

/// The ecliptic longitude and latitude (radians) and horizontal parallax (radians) of the moon
/// `t` Julian centuries after J2000.
fn moon_ecliptic(t: f64) -> (f64, f64, f64) {
    let longitude = 218.32
        + 481_267.881 * t
        + series(
            &[
                (6.29, 135.0, 477_198.87),
                (-1.27, 259.3, -413_335.36),
                (0.66, 235.7, 890_534.22),
                (0.21, 269.9, 954_397.74),
                (-0.19, 357.5, 35_999.05),
                (-0.11, 186.5, 966_404.03),
            ],
            t,
            f64::sin,
        );
    let latitude = series(
        &[
            (5.13, 93.3, 483_202.02),
            (0.28, 228.2, 960_400.89),
            (-0.28, 318.3, 6_003.15),
            (-0.17, 217.6, -407_332.21),
        ],
        t,
        f64::sin,
    );
    let parallax = 0.9508
        + series(
            &[
                (0.0518, 135.0, 477_198.87),
                (0.0095, 259.3, -413_335.36),
                (0.0078, 235.7, 890_534.22),
                (0.0028, 269.9, 954_397.74),
            ],
            t,
            f64::cos,
        );
    (
        longitude.to_radians(),
        latitude.to_radians(),
        parallax.to_radians(),
    )
}

/// The sun's ecliptic longitude in radians, `days` after J2000.
fn sun_longitude(days: f64) -> f64 {
    let mean = 280.460 + 0.985_647_4 * days;
    let anomaly = (357.528 + 0.985_600_3 * days).to_radians();
    (mean + 1.915 * anomaly.sin() + 0.020 * (2.0 * anomaly).sin()).to_radians()
}

/// The obliquity of the ecliptic in radians, `days` after J2000.
fn obliquity(days: f64) -> f64 {
    (23.439 - 0.000_000_4 * days).to_radians()
}

/// An ecliptic direction as an equatorial one.
fn equatorial(longitude: f64, latitude: f64, obliquity: f64) -> Equatorial {
    let x = latitude.cos() * longitude.cos();
    let y = latitude.cos() * longitude.sin() * obliquity.cos() - latitude.sin() * obliquity.sin();
    let z = latitude.cos() * longitude.sin() * obliquity.sin() + latitude.sin() * obliquity.cos();
    Equatorial {
        right_ascension: y.atan2(x),
        declination: z.asin(),
    }
}

impl Moon {
    /// The moon at `wall` seconds since the Unix epoch, seen from `observer` as (latitude,
    /// longitude) in degrees, north and east positive.
    pub fn at(wall: f64, observer: Option<(f64, f64)>) -> Self {
        let days = wall / 86400.0 + UNIX_EPOCH_JULIAN_DATE - J2000;
        let (longitude, latitude, parallax) = moon_ecliptic(days / 36525.0);
        let sun = sun_longitude(days);
        let elongation = latitude.cos() * (longitude - sun).cos();
        let obliquity = obliquity(days);
        Self {
            lit: (1.0 - elongation) / 2.0,
            waxing: (longitude - sun).sin() > 0.0,
            sky: observer.map(|(north, east)| {
                let moon = equatorial(longitude, latitude, obliquity);
                let sun = equatorial(sun, 0.0, obliquity);
                sky_of(
                    days,
                    north.to_radians(),
                    east.to_radians(),
                    parallax,
                    moon,
                    sun,
                )
            }),
        }
    }

    /// The traditional name of the phase.
    pub fn phase(&self) -> &'static str {
        match (self.lit, self.waxing) {
            (lit, _) if lit < 0.01 => "new moon",
            (lit, _) if lit > 0.99 => "full moon",
            (lit, true) if (lit - 0.5).abs() < 0.02 => "first quarter",
            (lit, false) if (lit - 0.5).abs() < 0.02 => "last quarter",
            (lit, true) if lit < 0.5 => "waxing crescent",
            (_, true) => "waxing gibbous",
            (lit, false) if lit < 0.5 => "waning crescent",
            (_, false) => "waning gibbous",
        }
    }
}

/// Where `moon` stands for an observer at `north` and `east` radians, with the sun at `sun`,
/// `days` after J2000.
fn sky_of(
    days: f64,
    north: f64,
    east: f64,
    parallax: f64,
    moon: Equatorial,
    sun: Equatorial,
) -> Sky {
    // Greenwich mean sidereal time plus the east longitude is the right ascension on the
    // observer's meridian; the hour angle is how far west of it the moon is.
    let sidereal = (280.460_618_37 + 360.985_647_366_29 * days).to_radians() + east;
    let hour = sidereal - moon.right_ascension;
    let (sin_dec, cos_dec) = moon.declination.sin_cos();
    let sin_altitude = north.sin() * sin_dec + north.cos() * cos_dec * hour.cos();
    let altitude = sin_altitude.asin();
    let azimuth_east = -cos_dec * hour.sin();
    let azimuth_north = sin_dec * north.cos() - cos_dec * north.sin() * hour.cos();
    let azimuth = azimuth_east.atan2(azimuth_north);

    // The moon is 1 / sin(parallax) earth radii away and the observer one up from the centre,
    // so the vector from the observer lies that much lower than the geocentric one.
    let topocentric = (sin_altitude - parallax.sin()).atan2(altitude.cos());

    // Meeus, Astronomical Algorithms, chapters 14 and 48: the bright limb's angle from the
    // north point of the disc through east, less the parallactic angle, is its angle from the
    // direction of the zenith.
    let separation = sun.right_ascension - moon.right_ascension;
    let (sin_sun, cos_sun) = sun.declination.sin_cos();
    let bright = (cos_sun * separation.sin())
        .atan2(sin_sun * cos_dec - cos_sun * sin_dec * separation.cos());
    let parallactic = hour
        .sin()
        .atan2(north.tan() * cos_dec - sin_dec * hour.cos());

    Sky {
        altitude: topocentric.to_degrees(),
        azimuth: azimuth.to_degrees().rem_euclid(360.0),
        limb: (bright - parallactic).rem_euclid(TAU),
    }
}

/// An angle in degrees as the equivalent in (-180, 180].
pub fn wrap_degrees(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(360.0);
    if wrapped > 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phase_matches_known_new_quarter_and_full_moons() {
        // Moon phases from the USNO: new 2024-01-11 11:57 UTC, first quarter 2024-01-18 03:53,
        // full 2024-01-25 17:54, last quarter 2024-02-02 23:18, new 2026-10-10 15:50.
        let new = Moon::at(1_704_974_220.0, None);
        // The moon is new when its longitude meets the sun's, not its latitude, so up to 5
        // degrees of the disc (0.002) may still be lit.
        assert!(new.lit < 0.005, "{}", new.lit);
        assert_eq!(new.phase(), "new moon");
        let first = Moon::at(1_705_549_980.0, None);
        assert!((first.lit - 0.5).abs() < 0.01, "{}", first.lit);
        assert!(first.waxing);
        assert_eq!(first.phase(), "first quarter");
        let full = Moon::at(1_706_205_240.0, None);
        assert!(full.lit > 0.995, "{}", full.lit);
        assert_eq!(full.phase(), "full moon");
        let last = Moon::at(1_706_915_880.0, None);
        assert!((last.lit - 0.5).abs() < 0.01, "{}", last.lit);
        assert!(!last.waxing);
        assert_eq!(last.phase(), "last quarter");
        let october = Moon::at(1_791_647_400.0, None);
        assert!(october.lit < 0.005, "{}", october.lit);
    }

    #[test]
    fn the_phase_names_follow_the_lit_fraction_and_the_direction() {
        let at = |lit, waxing| {
            Moon {
                lit,
                waxing,
                sky: None,
            }
            .phase()
        };
        assert_eq!(at(0.2, true), "waxing crescent");
        assert_eq!(at(0.8, true), "waxing gibbous");
        assert_eq!(at(0.8, false), "waning gibbous");
        assert_eq!(at(0.2, false), "waning crescent");
    }

    #[test]
    fn wrapping_keeps_angles_in_a_half_turn_either_side() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-190.0), 170.0);
        assert_eq!(wrap_degrees(180.0), 180.0);
        assert_eq!(wrap_degrees(720.0), 0.0);
    }

    /// The moon's sky at the instant of the day it is nearest the meridian, from `start`.
    fn on_the_meridian(start: f64, north: f64, east: f64, facing: f64) -> Sky {
        (0..24 * 60)
            .map(|minute| start + f64::from(minute) * 60.0)
            .map(|wall| Moon::at(wall, Some((north, east))).sky.unwrap())
            .min_by(|a, b| {
                wrap_degrees(a.azimuth - facing)
                    .abs()
                    .total_cmp(&wrap_degrees(b.azimuth - facing).abs())
            })
            .unwrap()
    }

    #[test]
    fn a_waxing_moon_is_lit_towards_the_sun_in_either_hemisphere() {
        // First quarter 2024-01-18 03:53 UTC: the moon is east of the sun, which has set in the
        // west and below the moon.
        let start = 1_705_549_980.0;
        let north = on_the_meridian(start, 52.0, 0.0, 180.0);
        assert!(north.altitude > 10.0, "{}", north.altitude);
        // Lit on the right (west, facing south), a little below the horizontal.
        assert!(north.limb.sin() < -0.8, "limb {} rad", north.limb);
        let south = on_the_meridian(start, -34.0, 151.0, 0.0);
        assert!(south.altitude > 10.0, "{}", south.altitude);
        // The same moon, high in the north, is lit on the left and tilted up.
        assert!(south.limb.sin() > 0.5, "limb {} rad", south.limb);
        assert!(south.limb.cos() > 0.2, "limb {} rad", south.limb);
    }

    #[test]
    fn the_moon_rises_in_the_east_and_sets_in_the_west() {
        let start = 1_706_205_240.0; // full moon
        let track: Vec<Sky> = (0..24 * 12)
            .map(|step| start + f64::from(step) * 300.0)
            .map(|wall| Moon::at(wall, Some((40.0, -75.0))).sky.unwrap())
            .collect();
        let up: Vec<&Sky> = track.iter().filter(|sky| sky.altitude > 0.0).collect();
        assert!(!up.is_empty() && up.len() < track.len());
        assert!(
            wrap_degrees(up[0].azimuth - 90.0).abs() < 40.0,
            "{}",
            up[0].azimuth
        );
        let last = up[up.len() - 1];
        assert!(
            wrap_degrees(last.azimuth - 270.0).abs() < 40.0,
            "{}",
            last.azimuth
        );
    }

    #[test]
    fn the_position_agrees_with_an_independent_ephemeris() {
        // PyEphem 4.2.1 (VSOP87 and ELP, no refraction): seconds since the Unix epoch, latitude,
        // longitude, altitude and azimuth in degrees and the lit fraction.
        const REFERENCE: [(f64, f64, f64, f64, f64, f64); 6] = [
            (1628239012.0, 0.00, 100.00, 13.23, 296.26, 0.0519),
            (1730093705.0, 52.37, 4.90, 30.19, 125.53, 0.1569),
            (1637140588.0, 40.00, -75.00, 6.76, 277.08, 0.9642),
            (1728277691.0, -34.00, 151.00, 80.04, 334.91, 0.1697),
            (1739680559.0, 64.10, -21.90, 20.65, 186.63, 0.8798),
            (1726509506.0, 35.70, 139.70, 9.45, 246.63, 0.9718),
        ];
        for (wall, latitude, longitude, altitude, azimuth, lit) in REFERENCE {
            let moon = Moon::at(wall, Some((latitude, longitude)));
            let sky = moon.sky.unwrap();
            assert!(
                (sky.altitude - altitude).abs() < 0.5,
                "{wall}: {}",
                sky.altitude
            );
            assert!(
                wrap_degrees(sky.azimuth - azimuth).abs() < 1.0,
                "{wall}: {}",
                sky.azimuth
            );
            assert!((moon.lit - lit).abs() < 0.005, "{wall}: {}", moon.lit);
        }
    }
}
