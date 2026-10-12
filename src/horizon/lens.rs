//! The physics of the horizon view, with no drawing: a Schwarzschild black hole seen by an
//! observer at infinity, in units where G = c = M = 1, so one world unit is one M.
//!
//! The horizon is at 2, the photon sphere at 3, the innermost stable circular orbit (ISCO) at 6,
//! and a photon that reaches infinity with impact parameter below `SHADOW_RADIUS` = 3 sqrt(3)
//! (about 5.196) has fallen in, which is the radius of the black shadow.
//!
//! Lensing is done on points, not pixels: [`LensTable`] answers where on the screen the image of
//! a point at a radius and an angle from the line of sight lies, and the view draws ordinary
//! triangles between the images of the vertices of a mesh. The functions here port the physics
//! of `docs/mockups/code/horizon.py`: `LensTable` (the prototype's `LensTable`),
//! `beloborodov_impact_parameter` (`beloborodov_b`), `flux` (`page_thorne`), `one_plus_z`
//! (`one_plus_z`), `disk_emission` (the colour temperature and intensity of `Scene._build_layer`),
//! and `PlungeTrack` (`plunge_track`).

use std::f64::consts::{PI, TAU};
use std::sync::OnceLock;

/// The radius of the event horizon.
pub const HORIZON: f32 = 2.0;
/// The innermost stable circular orbit: closer in, no circular orbit is stable.
pub const ISCO: f32 = 6.0;
/// The critical impact parameter 3 sqrt(3) that separates photons that escape from photons that
/// fall in: the radius of the shadow on the screen.
pub const SHADOW_RADIUS: f32 = 5.196_152;
/// The inner edge of the accretion disk, where the Page-Thorne flux starts from zero.
pub const DISK_INNER: f32 = ISCO;
/// The outer edge of the drawn disk.
pub const DISK_OUTER: f32 = 30.0;
/// The radius of the shell the background stars sit on, behind the hole.
pub const STAR_SHELL: f32 = 40.0;
/// The innermost radius a plunging image is drawn at: its image freezes just outside the horizon.
pub const PLUNGE_FLOOR: f32 = 2.0006;

/// 3 sqrt(3), `SHADOW_RADIUS` at full precision, which the grid of rays is cut at.
const CRITICAL_IMPACT: f64 = 5.196_152_422_706_632;

/// Radii the table covers, log-spaced from `NEAREST_RADIUS` to `FARTHEST_RADIUS`.
const RADII: usize = 360;
/// Sweep angles the table covers, evenly from 0 to pi.
const SWEEPS: usize = 721;
const NEAREST_RADIUS: f64 = 2.0005;
const FARTHEST_RADIUS: f64 = 150.0;
/// The Runge-Kutta step in the photon's sweep angle.
const PHOTON_STEP: f64 = 0.002;
/// A photon whose u = 1/r passes this has crossed the horizon (u = 1/2) for good, and one whose
/// u falls below `ESCAPED` has left to infinity.
const ABSORBED: f64 = 0.6;
const ESCAPED: f64 = -0.02;
/// Impact parameters traced: 1e-4, then 0 to the critical value in steps that crowd towards it,
/// then the critical value to 80 above it in steps that crowd towards it.
const BELOW_IMPACTS: usize = 700;
const ABOVE_IMPACTS: usize = 1100;

/// The colour temperature, in kelvin, of the brightest disk annulus before shifts, and the
/// exponent that widens the ramp from red to blue-white (stylized).
const DISK_KELVIN: f32 = 7200.0;
const KELVIN_EXPONENT: f32 = 1.55;

/// The impact parameter at which a photon seen from infinity at the angle `sweep` from the line
/// of sight to a point at `radius` reaches it, by Beloborodov (2002): the oracle for primary
/// images, accurate to a few percent at radii of 6 and more. `1 - cos(alpha) = (1 - cos(sweep))
/// (1 - 2/r)` gives the emission angle `alpha`, and `b = r sin(alpha) / sqrt(1 - 2/r)`.
#[cfg(test)]
pub fn beloborodov_impact_parameter(radius: f32, sweep: f32) -> f32 {
    let darkening = 1.0 - 2.0 / radius;
    let cosine = 1.0 - (1.0 - sweep.cos()) * darkening;
    radius * (1.0 - cosine * cosine).max(0.0).sqrt() / darkening.sqrt()
}

/// The impact parameters of the photons traced, ascending.
fn impact_grid() -> Vec<f64> {
    let critical = CRITICAL_IMPACT;
    let geometric = |from: f64, to: f64, count: usize| -> Vec<f64> {
        (0..count)
            .map(|k| from * (to / from).powf(k as f64 / (count - 1) as f64))
            .collect()
    };
    let mut impacts = vec![1e-4];
    impacts.extend(
        geometric(1e-7, 1.0, BELOW_IMPACTS)
            .iter()
            .rev()
            .map(|fraction| critical * (1.0 - fraction))
            .filter(|&impact| impact > 1e-4),
    );
    impacts.extend(
        geometric(1e-7, 80.0, ABOVE_IMPACTS)
            .iter()
            .map(|excess| critical + excess),
    );
    impacts
}

/// The path of the photon with impact parameter `impact` seen backwards from the observer:
/// `path[k]` is u = 1/r after a sweep of k steps, from the equation d2u/dphi2 = 3u^2 - u by
/// fourth-order Runge-Kutta. It freezes once the photon is absorbed or has escaped.
fn trace_photon(impact: f64, path: &mut [f64]) {
    let acceleration = |u: f64| 3.0 * u * u - u;
    let (mut u, mut v) = (0.0, 1.0 / impact);
    path[0] = 0.0;
    let mut frozen = false;
    for slot in path.iter_mut().skip(1) {
        if !frozen {
            let h = PHOTON_STEP;
            let (k1u, k1v) = (v, acceleration(u));
            let (k2u, k2v) = (v + 0.5 * h * k1v, acceleration(u + 0.5 * h * k1u));
            let (k3u, k3v) = (v + 0.5 * h * k2v, acceleration(u + 0.5 * h * k2u));
            let (k4u, k4v) = (v + h * k3v, acceleration(u + h * k3u));
            let next_u = u + h / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u);
            let next_v = v + h / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v);
            if next_u >= ABSORBED || next_u <= ESCAPED {
                frozen = true;
                v = 0.0;
            } else {
                v = next_v;
            }
            u = next_u.clamp(ESCAPED, ABSORBED);
        }
        *slot = u;
    }
}

/// `numpy.interp` for ascending `xs`: the value of the piecewise-linear function through
/// `(xs, ys)` at `x`, held at the end values beyond them.
fn interpolate(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let above = xs.partition_point(|&value| value <= x);
    if above == 0 {
        ys[0]
    } else if above == xs.len() {
        ys[above - 1]
    } else {
        let (x0, x1) = (xs[above - 1], xs[above]);
        ys[above - 1] + (ys[above] - ys[above - 1]) * (x - x0) / (x1 - x0)
    }
}

/// Where the traced photons cross each radius of the table, per impact parameter: the sweep
/// inbound and, for photons that turn round before the horizon, outbound; NaN where a photon
/// never crosses the radius.
struct Crossings {
    inbound: Vec<f64>,
    outbound: Vec<f64>,
}

/// The sweeps at which one photon, with `impact`, crosses each `inverse_radii` (u = 1/r values).
fn crossings(
    impact: f64,
    inverse_radii: &[f64],
    path: &mut [f64],
    inbound: &mut [f64],
    outbound: &mut [f64],
) {
    trace_photon(impact, path);
    let steps = path.len() - 1;
    let sweeps: Vec<f64> = (0..=steps).map(|k| k as f64 * PHOTON_STEP).collect();
    // The first step at which u peaks: before it the photon falls in, after it climbs out.
    let peak_step = path
        .iter()
        .enumerate()
        .fold(0, |best, (k, &u)| if u > path[best] { k } else { best });
    let peak = path[peak_step];
    let rising = &path[..=peak_step];
    for (slot, &u) in inbound.iter_mut().zip(inverse_radii) {
        *slot = if u > peak {
            f64::NAN
        } else {
            interpolate(u, rising, &sweeps[..=peak_step])
        };
    }
    outbound.fill(f64::NAN);
    if impact > CRITICAL_IMPACT && peak < 0.5 && peak_step < steps {
        let falling: Vec<f64> = path[peak_step..].iter().rev().copied().collect();
        let returning: Vec<f64> = sweeps[peak_step..].iter().rev().copied().collect();
        for (slot, &u) in outbound.iter_mut().zip(inverse_radii) {
            if u >= falling[0] && u <= peak {
                *slot = interpolate(u, &falling, &returning);
            }
        }
    }
}

/// How far from the line of sight to the hole a ray must have swept to carry light from a point
/// at each radius to the observer: the deflection of light, tabulated as the impact parameter `b`
/// of the ray for a point at radius `r` seen at the sweep `psi` from the line of sight. The
/// primary image of a point is the ray that sweeps `psi`; the secondary image is the ray that
/// sweeps `2 pi - psi`, round the other side of the hole, and appears on the opposite side of
/// the shadow.
///
/// Photons are traced back from the observer for a grid of impact parameters. For each radius
/// the sweep at which a photon crosses it inbound and, after its closest approach, outbound
/// rises monotonically with the path's impact parameter from 0 to its maximum for that radius
/// and back down to the critical value, from 0 to infinity, so it inverts by interpolation.
pub struct LensTable {
    log_radius_from: f64,
    log_radius_step: f64,
    /// Impact parameter by radius then sweep, row-major with `SWEEPS` per row.
    primary: Vec<f32>,
    secondary: Vec<f32>,
}

impl LensTable {
    /// The table every view shares, built on the first call (about a fifth of a second in a
    /// release build, on every core).
    pub fn shared() -> &'static LensTable {
        static TABLE: OnceLock<LensTable> = OnceLock::new();
        TABLE.get_or_init(LensTable::build)
    }

    fn build() -> LensTable {
        let impacts = impact_grid();
        let log_from = NEAREST_RADIUS.ln();
        let log_step = (FARTHEST_RADIUS.ln() - log_from) / (RADII - 1) as f64;
        let inverse_radii: Vec<f64> = (0..RADII)
            .map(|i| 1.0 / (log_from + log_step * i as f64).exp())
            .collect();
        let steps = ((TAU + 0.8) / PHOTON_STEP) as usize;
        let mut inbound = vec![f64::NAN; impacts.len() * RADII];
        let mut outbound = vec![f64::NAN; impacts.len() * RADII];
        let workers = std::thread::available_parallelism().map_or(1, usize::from);
        let chunk = impacts.len().div_ceil(workers);
        std::thread::scope(|scope| {
            let rows = inbound
                .chunks_mut(chunk * RADII)
                .zip(outbound.chunks_mut(chunk * RADII))
                .zip(impacts.chunks(chunk));
            for ((inbound, outbound), impacts) in rows {
                let inverse_radii = &inverse_radii;
                scope.spawn(move || {
                    let mut path = vec![0.0; steps + 1];
                    let rows = inbound
                        .chunks_mut(RADII)
                        .zip(outbound.chunks_mut(RADII))
                        .zip(impacts);
                    for ((inbound, outbound), &impact) in rows {
                        crossings(impact, inverse_radii, &mut path, inbound, outbound);
                    }
                });
            }
        });
        let mut table = LensTable {
            log_radius_from: log_from,
            log_radius_step: log_step,
            primary: vec![0.0; RADII * SWEEPS],
            secondary: vec![0.0; RADII * SWEEPS],
        };
        let crossings = Crossings { inbound, outbound };
        for i in 0..RADII {
            table.fill_radius(i, &impacts, &crossings);
        }
        table
    }

    /// Fills the row of radius `i` from the crossings of every traced photon.
    fn fill_radius(&mut self, i: usize, impacts: &[f64], crossings: &Crossings) {
        let mut sweeps = vec![0.0];
        let mut along = vec![0.0];
        // Rising impact parameter, photons that have not yet turned round at this radius...
        for (j, &impact) in impacts.iter().enumerate() {
            let sweep = crossings.inbound[j * RADII + i];
            if !sweep.is_nan() {
                sweeps.push(sweep);
                along.push(impact);
            }
        }
        // ...then falling impact parameter, photons coming back out.
        for (j, &impact) in impacts.iter().enumerate().rev() {
            let sweep = crossings.outbound[j * RADII + i];
            if !sweep.is_nan() {
                sweeps.push(sweep);
                along.push(impact);
            }
        }
        for k in 1..sweeps.len() {
            sweeps[k] = sweeps[k].max(sweeps[k - 1]);
        }
        let row = i * SWEEPS..(i + 1) * SWEEPS;
        for (s, (first, second)) in self.primary[row.clone()]
            .iter_mut()
            .zip(&mut self.secondary[row])
            .enumerate()
        {
            let psi = PI * s as f64 / (SWEEPS - 1) as f64;
            *first = interpolate(psi, &sweeps, &along) as f32;
            *second = interpolate(TAU - psi, &sweeps, &along) as f32;
        }
    }

    /// The impact parameter of the primary image of a point at `radius` whose direction makes the
    /// angle `sweep` (0 to pi) with the line of sight to the observer: the screen distance of the
    /// image from the centre of the shadow, in M.
    pub fn primary(&self, radius: f32, sweep: f32) -> f32 {
        self.lookup(&self.primary, radius, sweep)
    }

    /// The impact parameter of the secondary image of the same point: the ray that sweeps
    /// `2 pi - sweep` the other way round. It is never inside the shadow.
    pub fn secondary(&self, radius: f32, sweep: f32) -> f32 {
        self.lookup(&self.secondary, radius, sweep)
    }

    /// Bilinear interpolation in the log of the radius and in the sweep, held at the table's edges.
    fn lookup(&self, table: &[f32], radius: f32, sweep: f32) -> f32 {
        let row = ((f64::from(radius).ln() - self.log_radius_from) / self.log_radius_step)
            .clamp(0.0, RADII as f64 - 1.001);
        let column =
            (f64::from(sweep) / (PI / (SWEEPS - 1) as f64)).clamp(0.0, SWEEPS as f64 - 1.001);
        let (i, j) = (row as usize, column as usize);
        let (fraction_row, fraction_column) = ((row - i as f64) as f32, (column - j as f64) as f32);
        let at = |i: usize, j: usize| table[i * SWEEPS + j];
        let near = at(i, j) * (1.0 - fraction_column) + at(i, j + 1) * fraction_column;
        let far = at(i + 1, j) * (1.0 - fraction_column) + at(i + 1, j + 1) * fraction_column;
        near * (1.0 - fraction_row) + far * fraction_row
    }
}

/// The screen positions of the two images of one point, in M from the centre of the shadow, x to
/// the right and y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Images {
    pub primary: [f32; 2],
    pub secondary: [f32; 2],
}

/// An observer at infinity, `inclination` radians from the normal of the disk plane (0 is face-on).
/// The disk is the plane z = 0 with the observer at (0, -sin i, cos i), so the part of the disk
/// at positive y is the far side and appears at the top of the screen. The screen's right is +x
/// and its up is (0, cos i, sin i).
#[derive(Clone, Copy, Debug)]
pub struct Observer {
    sin: f32,
    cos: f32,
}

impl Observer {
    pub fn new(inclination: f32) -> Observer {
        let (sin, cos) = inclination.sin_cos();
        Observer { sin, cos }
    }

    /// The inclination, in radians.
    pub fn inclination(self) -> f32 {
        self.sin.atan2(self.cos)
    }

    /// The images of a point `point` of the hole's frame (the disk plane is z = 0).
    pub fn images(self, table: &LensTable, point: [f32; 3]) -> Images {
        let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
        let direction = point.map(|component| component / radius);
        let toward = -self.sin * direction[1] + self.cos * direction[2];
        let sweep = toward.clamp(-1.0, 1.0).acos();
        let across = direction[0];
        let up = self.cos * direction[1] + self.sin * direction[2];
        self.place(table, radius, sweep, across, up)
    }

    /// The images of the point of the disk plane at `radius` and `azimuth` from the screen's
    /// right, counted towards the far side: `images` of `(r cos a, r sin a, 0)`, without the
    /// division and the trigonometry.
    pub fn disk_images(self, table: &LensTable, radius: f32, azimuth: f32) -> Images {
        let (sin, cos) = azimuth.sin_cos();
        let sweep = (-self.sin * sin).clamp(-1.0, 1.0).acos();
        self.place(table, radius, sweep, cos, self.cos * sin)
    }

    /// Places the images along the screen direction `(across, up)`, which need not be normalized.
    fn place(self, table: &LensTable, radius: f32, sweep: f32, across: f32, up: f32) -> Images {
        let length = across.hypot(up).max(1e-9);
        let (across, up) = (across / length, up / length);
        let (first, second) = (table.primary(radius, sweep), table.secondary(radius, sweep));
        Images {
            primary: [first * across, first * up],
            secondary: [-second * across, -second * up],
        }
    }
}

/// The Page-Thorne (Novikov-Thorne) flux of a thin disk around a Schwarzschild hole, in arbitrary
/// units, after Luminet (1979): zero at and inside the ISCO, peaking near 9.5 M and falling as
/// 1/r^3 far out. It is `I / ((r - 3) r^(5/2))`, where `I` is the integral of
/// `(E - Omega L) dL/dr` from the ISCO to r that Page and Thorne's flux formula holds. With
/// `x = sqrt r` the circular orbits of Schwarzschild have `E - Omega L = sqrt(x^2 - 3) / x` and
/// `dL/dr = (x^2 - 6) / (2 (x^2 - 3)^(3/2))`, so the integrand is `(x^2 - 6) / (x^2 - 3)` in `x`
/// and `I = sqrt r - sqrt 6 + (sqrt 3 / 2) ln((sqrt r + sqrt 3)(sqrt 6 - sqrt 3) / ((sqrt r -
/// sqrt 3)(sqrt 6 + sqrt 3)))`. (`horizon.py` writes the coefficient `sqrt 3 / 3`, which makes
/// the flux rise linearly from the ISCO instead of quadratically; the test against the
/// integral itself pins the coefficient.)
pub fn flux(radius: f32) -> f32 {
    let radius = f64::from(radius);
    if radius <= f64::from(ISCO) {
        return 0.0;
    }
    let (root3, root6, root) = (3.0_f64.sqrt(), 6.0_f64.sqrt(), radius.sqrt());
    let argument = (root + root3) * (root6 - root3) / ((root - root3) * (root6 + root3));
    ((root - root6 + root3 / 2.0 * argument.ln()) / ((radius - 3.0) * radius.powf(2.5))) as f32
}

/// The flux scaled so its maximum over the disk is 1, and tapered to nothing over the last 10 M
/// of the disk so the outer edge fades into the sky instead of ending.
pub fn disk_flux(radius: f32) -> f32 {
    static PEAK: OnceLock<f32> = OnceLock::new();
    let peak = *PEAK.get_or_init(|| {
        (0..=2000)
            .map(|k| flux(6.01 + (DISK_OUTER - 6.01) * k as f32 / 2000.0))
            .fold(0.0, f32::max)
    });
    let taper = ((DISK_OUTER - radius) / 10.0).clamp(0.0, 1.0);
    flux(radius) / peak * taper * taper * (3.0 - 2.0 * taper)
}

/// The angular velocity of a circular orbit at `radius`, in radians per M of coordinate time:
/// sqrt(M / r^3), identical to Kepler's.
pub fn angular_velocity(radius: f32) -> f32 {
    radius.powf(-1.5)
}

/// The period of a circular orbit at `radius`, in M of coordinate time: 2 pi sqrt(r^3 / M).
pub fn period(radius: f32) -> f32 {
    std::f32::consts::TAU / angular_velocity(radius)
}

/// One plus the redshift of light from the disk at `radius` as the observer sees it, at the
/// screen position `screen_x` (M, positive to the right) of its image, for an observer
/// `inclination` radians from the disk normal (Luminet 1979):
/// `(1 - 3/r)^(-1/2) (1 + sqrt(1/r^3) sin(i) x)`, where `x` is b sin(alpha) of the image. The
/// first factor is the gravitational redshift together with the transverse Doppler shift; the
/// second is the line-of-sight Doppler shift of matter that goes round counterclockwise as seen
/// from above, so it approaches on the left of the screen (`x < 0`) and recedes on the right.
pub fn one_plus_z(radius: f32, screen_x: f32, inclination: f32) -> f32 {
    (1.0 - 3.0 / radius).powf(-0.5)
        * (1.0 + angular_velocity(radius) * inclination.sin() * screen_x)
}

/// What the observer sees of a patch of the disk: its intensity and its colour temperature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emission {
    /// The observed intensity, `F g^4` with `g = 1 / (1 + z)`, in units of the peak flux.
    pub intensity: f32,
    /// The colour temperature of the light as received: `T = 7200 K (F^(1/4) g)^1.55`.
    pub kelvin: f32,
}

/// The emission of the disk at `radius` as seen at the screen position `screen_x` of its image.
pub fn disk_emission(radius: f32, screen_x: f32, inclination: f32) -> Emission {
    let flux = disk_flux(radius);
    let shift = 1.0 / one_plus_z(radius, screen_x, inclination).max(0.05);
    Emission {
        intensity: flux * shift.powi(4),
        kelvin: DISK_KELVIN * (flux.powf(0.25) * shift).powf(KELVIN_EXPONENT),
    }
}

/// The path of a body that leaves the ISCO with 0.95 of its angular momentum and a small inward
/// kick, in coordinate time: the radius and the azimuth it has gone round, which tend to 2 M but
/// never reach it, as a distant observer sees a body fall in. Integrated in proper time with
/// `dr/dtau = -sqrt(E^2 - (1 - 2/r)(1 + L^2/r^2))`, `dt/dtau = E / (1 - 2/r)` and
/// `dphi/dtau = L / r^2`.
pub struct PlungeTrack {
    time: Vec<f32>,
    radius: Vec<f32>,
    azimuth: Vec<f32>,
}

/// The longest coordinate time the track covers, in M.
const TRACK_SPAN: f64 = 400.0;

impl PlungeTrack {
    pub fn shared() -> &'static PlungeTrack {
        static TRACK: OnceLock<PlungeTrack> = OnceLock::new();
        TRACK.get_or_init(|| PlungeTrack::new(0.95, 0.1))
    }

    /// The track of a body with `fraction` of the ISCO's angular momentum, 2 sqrt(3), and an
    /// inward speed of `kick` (in proper time) at r = 6.
    pub fn new(fraction: f64, kick: f64) -> PlungeTrack {
        let angular_momentum = fraction * 12.0_f64.sqrt();
        let energy = ((1.0 - 2.0 / 6.0) * (1.0 + angular_momentum * angular_momentum / 36.0)
            + kick * kick)
            .sqrt();
        let inward_speed = |r: f64| {
            let potential = (1.0 - 2.0 / r) * (1.0 + angular_momentum * angular_momentum / (r * r));
            -(energy * energy - potential).max(1e-12).sqrt()
        };
        let (mut time, mut azimuth, mut radius) = (0.0, 0.0, 6.0);
        let mut step = 0.01;
        let mut track = PlungeTrack {
            time: vec![0.0],
            radius: vec![6.0],
            azimuth: vec![0.0],
        };
        while time < TRACK_SPAN {
            let k1 = inward_speed(radius);
            let k2 = inward_speed(radius + 0.5 * step * k1);
            let k3 = inward_speed(radius + 0.5 * step * k2);
            let k4 = inward_speed(radius + step * k3);
            let next = radius + step / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
            let middle = 0.5 * (radius + next);
            time += step * energy / (1.0 - 2.0 / middle);
            azimuth += step * angular_momentum / (middle * middle);
            radius = next;
            if radius < 2.0 + 1e-6 {
                break;
            }
            track.time.push(time as f32);
            track.radius.push(radius as f32);
            track.azimuth.push(azimuth as f32);
            // The proper-time step shrinks towards the horizon so coordinate time keeps advancing.
            step = f64::min(0.01, 0.02 * (radius - 2.0));
        }
        track
    }

    /// The radius and the azimuth gone round at coordinate time `time` (M) after leaving the ISCO,
    /// held at the last point of the track once the image has frozen. The radius is never below
    /// `PLUNGE_FLOOR`.
    pub fn at(&self, time: f32) -> (f32, f32) {
        let time = time.max(0.0);
        let above = self.time.partition_point(|&t| t <= time);
        let (radius, azimuth) = if above >= self.time.len() {
            (self.radius[above - 1], self.azimuth[above - 1])
        } else {
            let fraction =
                (time - self.time[above - 1]) / (self.time[above] - self.time[above - 1]);
            let lerp =
                |values: &[f32]| values[above - 1] + (values[above] - values[above - 1]) * fraction;
            (lerp(&self.radius), lerp(&self.azimuth))
        };
        (radius.max(PLUNGE_FLOOR), azimuth)
    }

    /// The coordinate time at which the track ends, in M.
    #[cfg(test)]
    pub fn duration(&self) -> f32 {
        *self.time.last().expect("the track has its first point")
    }
}

/// How bright a plunging body's image is at `radius`: `1.5 (1 - 2/r)`, 1 at the ISCO and
/// falling to 0 at the horizon, because every photon is redshifted by sqrt(1 - 2/r) more as the
/// body nears it.
pub fn plunge_brightness(radius: f32) -> f32 {
    (1.5 * (1.0 - HORIZON / radius)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table, as a view draws with it.
    fn table() -> &'static LensTable {
        LensTable::shared()
    }

    #[test]
    fn the_critical_impact_parameter_is_three_root_three() {
        // A circular photon orbit at r = 3 has b^2 = r^3 / (r - 2) = 27.
        let radius = 3.0_f64;
        let critical = (radius.powi(3) / (radius - 2.0)).sqrt();
        assert!((f64::from(SHADOW_RADIUS) - critical).abs() < 1e-6);
        assert!((f64::from(SHADOW_RADIUS) - 3.0 * 3.0_f64.sqrt()).abs() < 1e-6);
        assert_eq!(CRITICAL_IMPACT, 3.0 * 3.0_f64.sqrt());
    }

    #[test]
    fn the_isco_is_where_the_effective_potential_has_a_double_root() {
        // With L^2 = 12 and E^2 = 8/9, the potential V = (1 - 2/r)(1 + L^2/r^2) touches E^2 at
        // r = 6 with no slope and no curvature: circular orbits are stable outside, not inside.
        let potential = |r: f64, l2: f64| (1.0 - 2.0 / r) * (1.0 + l2 / (r * r));
        let h = 1e-3;
        let (l2, energy2) = (12.0, 8.0 / 9.0);
        let slope = (potential(6.0 + h, l2) - potential(6.0 - h, l2)) / (2.0 * h);
        let curvature =
            (potential(6.0 + h, l2) - 2.0 * potential(6.0, l2) + potential(6.0 - h, l2)) / (h * h);
        assert!((potential(6.0, l2) - energy2).abs() < 1e-12);
        assert!(slope.abs() < 1e-9 && curvature.abs() < 1e-5);
        assert_eq!(f64::from(ISCO), 6.0);
        // At any other radius the circular orbit has L^2 = r^2 / (r - 3), and its curvature
        // changes sign at the ISCO.
        let stability = |r: f64| {
            let l2 = r * r / (r - 3.0);
            (potential(r + h, l2) - 2.0 * potential(r, l2) + potential(r - h, l2)) / (h * h)
        };
        assert!(stability(5.5) < 0.0 && stability(6.5) > 0.0);
    }

    #[test]
    fn lens_table_matches_beloborodov_at_ten_m() {
        let worst = (0..20)
            .map(|k| {
                let sweep = 0.1 + 1.9 * k as f32 / 19.0;
                let ours = table().primary(10.0, sweep);
                (ours / beloborodov_impact_parameter(10.0, sweep) - 1.0).abs()
            })
            .fold(0.0, f32::max);
        assert!(worst < 0.03, "worst relative difference {worst}");
    }

    /// The sweep angle of the photon with impact parameter `impact` between the observer and the
    /// radius `radius`, by quadrature of dphi = du / sqrt(1/b^2 - u^2 + 2 u^3) with the
    /// substitution u = u_end sin^2(theta) that removes the endpoint's slope; None where the
    /// photon never gets that far in.
    fn direct_sweep(impact: f64, radius: f64) -> Option<f64> {
        let end = 1.0 / radius;
        let roots = |u: f64| 1.0 / (impact * impact) - u * u + 2.0 * u * u * u;
        let samples = 4000;
        let mut total = 0.0;
        for k in 0..samples {
            // Composite Simpson over theta in [0, pi/2], with u = end * sin^2(theta).
            let integrand = |theta: f64| {
                let (sin, cos) = theta.sin_cos();
                let u = end * sin * sin;
                let f = roots(u);
                if f <= 0.0 {
                    return f64::NAN;
                }
                2.0 * end * sin * cos / f.sqrt()
            };
            let (a, b) = (
                PI / 2.0 * k as f64 / samples as f64,
                PI / 2.0 * (k + 1) as f64 / samples as f64,
            );
            total += (b - a) / 6.0 * (integrand(a) + 4.0 * integrand(0.5 * (a + b)) + integrand(b));
        }
        total.is_finite().then_some(total)
    }

    #[test]
    fn lens_table_matches_a_direct_photon_integration() {
        // Where the ray is still on its way in when it reaches the radius, invert the quadrature
        // for the impact parameter that sweeps psi (the sweep rises with it), by bisection. Past
        // the largest sweep of the inbound rays (their closest approach is the radius itself) the
        // image belongs to a ray that has turned round, which a finer integration of the
        // equation of motion follows instead.
        let (mut worst, mut inbound, mut outbound) = (0.0_f64, 0, 0);
        for radius in [6.0_f64, 10.0, 20.0, 30.0] {
            let touching = radius / (1.0 - 2.0 / radius).sqrt();
            let farthest = direct_sweep(touching * (1.0 - 1e-9), radius).expect("just inside");
            for sweep in [0.15_f64, 0.4, 0.8, 1.2, 1.6, 2.0, 2.6] {
                // Where the rays turn round the impact parameter has a square-root cusp in the
                // sweep, which the grid of traced rays (0.3 M apart near b = 20) cannot follow
                // to 0.5%: it is off by 1% at 0.08 rad from the cusp, a sliver of the sky.
                if (sweep - farthest).abs() < 0.1 {
                    continue;
                }
                let ours = f64::from(table().primary(radius as f32, sweep as f32));
                if sweep < farthest {
                    let (mut low, mut high) = (1e-3, touching);
                    for _ in 0..60 {
                        let middle = 0.5 * (low + high);
                        match direct_sweep(middle, radius) {
                            Some(found) if found < sweep => low = middle,
                            _ => high = middle,
                        }
                    }
                    worst = worst.max((ours / (0.5 * (low + high)) - 1.0).abs());
                    inbound += 1;
                } else {
                    let swept = traced_outbound_sweep(ours, radius);
                    worst = worst.max((swept / sweep - 1.0).abs());
                    outbound += 1;
                }
            }
        }
        assert!(
            inbound >= 12 && outbound >= 6,
            "{inbound} inbound, {outbound} outbound"
        );
        assert!(worst < 0.005, "worst relative difference {worst}");
    }

    /// The sweep of a photon with impact parameter `impact`, traced from the observer at a finer
    /// step than the table's, until it crosses `radius` outbound.
    fn traced_outbound_sweep(impact: f64, radius: f64) -> f64 {
        let acceleration = |u: f64| 3.0 * u * u - u;
        let h = 0.0005;
        let (mut u, mut v, mut phi) = (0.0, 1.0 / impact, 0.0);
        let mut turned = false;
        loop {
            let (k1u, k1v) = (v, acceleration(u));
            let (k2u, k2v) = (v + 0.5 * h * k1v, acceleration(u + 0.5 * h * k1u));
            let (k3u, k3v) = (v + 0.5 * h * k2v, acceleration(u + 0.5 * h * k2u));
            let (k4u, k4v) = (v + h * k3v, acceleration(u + h * k3u));
            let (next_u, next_v) = (
                u + h / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u),
                v + h / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v),
            );
            turned |= next_v < 0.0;
            if turned && next_u <= 1.0 / radius {
                let fraction = (u - 1.0 / radius) / (u - next_u);
                return phi + h * fraction;
            }
            (u, v, phi) = (next_u, next_v, phi + h);
            assert!(phi < 4.0 * PI, "the photon never came back out");
        }
    }

    #[test]
    fn secondary_images_trace_back_to_the_other_side_of_the_hole() {
        // The ray the table gives for the secondary image really sweeps 2 pi - psi.
        for radius in [8.0_f64, 15.0, 30.0] {
            for sweep in [0.3_f64, 1.0, 1.9, 2.8] {
                let impact = f64::from(table().secondary(radius as f32, sweep as f32));
                let swept = traced_outbound_sweep(impact, radius);
                let wanted = 2.0 * PI - sweep;
                assert!(
                    (swept / wanted - 1.0).abs() < 0.005,
                    "r {radius} psi {sweep}: swept {swept} wanted {wanted}"
                );
            }
        }
    }

    #[test]
    fn secondary_images_always_lie_outside_the_shadow() {
        let lowest = (0..=48)
            .flat_map(|i| {
                (0..=180).map(move |j| {
                    (
                        6.0 * 5.0_f32.powf(i as f32 / 48.0),
                        std::f32::consts::PI * j as f32 / 180.0,
                    )
                })
            })
            .map(|(radius, sweep)| table().secondary(radius, sweep))
            .fold(f32::INFINITY, f32::min);
        assert!(lowest > SHADOW_RADIUS, "closest secondary image {lowest}");
    }

    #[test]
    fn primary_images_of_the_near_side_can_fall_inside_the_shadow() {
        // A point at 6 M right in front of the hole (sweep 0) lies at the centre of the shadow.
        assert!(table().primary(6.0, 0.05) < SHADOW_RADIUS);
        assert!(table().primary(6.0, 0.0) < 0.01);
        // And the far side, seen over the top, is outside it.
        assert!(table().primary(6.0, std::f32::consts::PI) > SHADOW_RADIUS);
    }

    #[test]
    fn the_images_of_a_disk_point_follow_the_line_of_sight() {
        let observer = Observer::new(80.0_f32.to_radians());
        // The near side of the disk (negative y) appears below the shadow's centre, the far side
        // above it, the point at +x on the right, and the secondary image opposite its primary.
        let near = observer.disk_images(table(), 10.0, -std::f32::consts::FRAC_PI_2);
        let far = observer.disk_images(table(), 10.0, std::f32::consts::FRAC_PI_2);
        let right = observer.disk_images(table(), 10.0, 0.0);
        assert!(near.primary[1] < 0.0 && far.primary[1] > 0.0);
        assert!(right.primary[0] > 9.0 && right.primary[1].abs() < 1e-3);
        assert!(far.secondary[1] < 0.0 && near.secondary[1] > 0.0);
        // The disk-plane shortcut is the general projection of the same point.
        for azimuth in [0.3_f32, 1.7, 3.0, 4.4, 5.9] {
            let point = [10.0 * azimuth.cos(), 10.0 * azimuth.sin(), 0.0];
            let general = observer.images(table(), point);
            let special = observer.disk_images(table(), 10.0, azimuth);
            for k in 0..2 {
                assert!((general.primary[k] - special.primary[k]).abs() < 1e-3);
                assert!((general.secondary[k] - special.secondary[k]).abs() < 1e-3);
            }
        }
        assert!((observer.inclination() - 80.0_f32.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn orbital_angular_velocity_matches_kepler() {
        // Newtonian circular motion in the field of mass M = 1: v = sqrt(M / r), omega = v / r,
        // and the period 2 pi r / v is Kepler's third law, 2 pi sqrt(r^3 / M).
        for radius in [6.0_f32, 9.5, 17.0, 26.5, 30.0] {
            let speed = (1.0 / radius).sqrt();
            assert!((angular_velocity(radius) - speed / radius).abs() < 1e-6 * speed / radius);
            let kepler = std::f32::consts::TAU * (radius.powi(3)).sqrt();
            assert!((period(radius) - kepler).abs() < 1e-4 * kepler);
        }
        // The prototype's caption: the ISCO's period is 92 M.
        assert_eq!(period(6.0).round(), 92.0);
    }

    #[test]
    fn page_thorne_flux_is_zero_at_the_isco_and_falls_as_one_over_r_cubed() {
        assert_eq!(flux(6.0), 0.0);
        assert_eq!(flux(4.0), 0.0);
        assert!(
            flux(6.05) > 0.0 && flux(6.05) < flux(7.0),
            "it starts from zero"
        );
        // Far out the integral tends to sqrt(r) and the flux to r^(-3).
        let far = f64::from(flux(1e8)) * 1e8_f64.powi(3);
        assert!((far - 1.0).abs() < 0.005, "r^3 F = {far}");
        // It rises quadratically from the ISCO, because dL/dr vanishes there.
        let ratio = flux(6.2) / flux(6.1);
        assert!((3.5..4.5).contains(&ratio), "F(6.2) / F(6.1) = {ratio}");
        // The peak is near 9.5 M.
        let peak = (0..2000)
            .map(|k| 6.01 + k as f32 * 0.01)
            .max_by(|a, b| flux(*a).total_cmp(&flux(*b)))
            .unwrap();
        assert!((9.0..10.0).contains(&peak), "peak at {peak}");
    }

    #[test]
    fn page_thorne_flux_matches_the_novikov_thorne_integral() {
        // F = f / (4 pi r) with f = -Omega' (E - Omega L)^-2 integral of (E - Omega L) dL/dr
        // from the ISCO, for the circular orbits of a Schwarzschild hole (Page and Thorne 1974),
        // against the closed form that `flux` evaluates, which is that times 8 pi / 3.
        let energy = |r: f64| (r - 2.0) / (r * (r - 3.0)).sqrt();
        let momentum = |r: f64| r / (r - 3.0).sqrt();
        let omega = |r: f64| r.powf(-1.5);
        let integrand = |r: f64| {
            let h = 1e-6;
            let slope = (momentum(r + h) - momentum(r - h)) / (2.0 * h);
            (energy(r) - omega(r) * momentum(r)) * slope
        };
        for radius in [7.0_f64, 9.5, 14.0, 25.0] {
            let samples = 20_000;
            let width = (radius - 6.0) / f64::from(samples);
            let mut integral = 0.0;
            for k in 0..samples {
                let a = 6.0 + f64::from(k) * width;
                integral += width / 6.0
                    * (integrand(a) + 4.0 * integrand(a + 0.5 * width) + integrand(a + width));
            }
            let slope_omega = -1.5 * radius.powf(-2.5);
            let drift = energy(radius) - omega(radius) * momentum(radius);
            let integral_flux = -slope_omega / (drift * drift) * integral / (4.0 * PI * radius);
            let closed = f64::from(flux(radius as f32)) * 3.0 / (8.0 * PI);
            assert!(
                (closed / integral_flux - 1.0).abs() < 2e-3,
                "r {radius}: closed form {closed} integral {integral_flux}"
            );
        }
    }

    #[test]
    fn one_plus_z_combines_the_gravitational_and_the_doppler_shift() {
        let inclination = 80.0_f32.to_radians();
        // On the line through the hole's centre there is no line-of-sight motion: only the
        // gravitational redshift with the transverse Doppler shift, (1 - 3/r)^(-1/2).
        assert!((one_plus_z(6.0, 0.0, inclination) - 2.0_f32.sqrt()).abs() < 1e-6);
        assert!((one_plus_z(12.0, 0.0, inclination) - (1.0 - 0.25_f32).powf(-0.5)).abs() < 1e-6);
        // The Doppler term is odd in x: matter on the left of the screen approaches (blueshift)
        // and on the right recedes, symmetrically about the gravitational factor.
        let (left, right) = (
            one_plus_z(8.0, -8.0, inclination),
            one_plus_z(8.0, 8.0, inclination),
        );
        assert!(
            left < one_plus_z(8.0, 0.0, inclination) && right > one_plus_z(8.0, 0.0, inclination)
        );
        assert!((0.5 * (left + right) - one_plus_z(8.0, 0.0, inclination)).abs() < 1e-5);
        // Face-on there is no line-of-sight velocity at all.
        assert_eq!(one_plus_z(8.0, 8.0, 0.0), one_plus_z(8.0, -8.0, 0.0));
        // Orbital speed at 8 M, v = sqrt(1/8) = 0.354, seen edge-on at x = r gives 1 +- v.
        let edge = std::f32::consts::FRAC_PI_2;
        let doppler = one_plus_z(8.0, 8.0, edge) / one_plus_z(8.0, 0.0, edge);
        assert!((doppler - (1.0 + 0.125_f32.sqrt())).abs() < 1e-5);
    }

    #[test]
    fn the_approaching_side_of_the_disk_is_brighter_and_bluer() {
        let observer = Observer::new(80.0_f32.to_radians());
        for radius in [7.0_f32, 10.0, 16.0, 24.0] {
            // The left of the screen is where azimuth is 180 degrees.
            let side = |azimuth: f32| {
                let at = observer.disk_images(table(), radius, azimuth).primary;
                disk_emission(radius, at[0], observer.inclination())
            };
            let (approaching, receding) = (side(PI as f32), side(0.0));
            assert!(
                approaching.intensity > 2.0 * receding.intensity,
                "r {radius}"
            );
            assert!(approaching.kelvin > receding.kelvin, "r {radius}");
        }
    }

    #[test]
    fn disk_flux_is_scaled_to_its_peak_and_fades_at_the_outer_edge() {
        let peak = (0..2000)
            .map(|k| disk_flux(6.01 + k as f32 * 0.012))
            .fold(0.0, f32::max);
        assert!((peak - 1.0).abs() < 1e-3);
        assert_eq!(disk_flux(6.0), 0.0);
        assert!(disk_flux(29.0) < 0.05 && disk_flux(30.0) == 0.0);
        assert!(disk_flux(18.0) > disk_flux(25.0));
    }

    #[test]
    fn the_plunge_leaves_the_isco_and_freezes_short_of_the_horizon() {
        let track = PlungeTrack::shared();
        assert_eq!(track.at(0.0), (6.0, 0.0));
        let mut last = (6.0, 0.0);
        for k in 1..=4000 {
            let (radius, azimuth) = track.at(k as f32 * 0.1);
            assert!(
                radius <= last.0 && azimuth >= last.1,
                "time {}",
                k as f32 * 0.1
            );
            assert!(radius >= PLUNGE_FLOOR);
            last = (radius, azimuth);
        }
        assert!(
            last.0 < 2.01,
            "the image has all but frozen at the horizon: {}",
            last.0
        );
        // Beyond the end of the track it holds still.
        assert_eq!(
            track.at(track.duration() + 50.0),
            track.at(track.duration() + 500.0)
        );
        // It leaves the ISCO still going round: the angular momentum is 0.95 of the ISCO's.
        let (_, early) = track.at(2.0);
        assert!(
            early > 2.0 * angular_velocity(6.0) * 0.9,
            "azimuth after 2 M: {early}"
        );
    }

    #[test]
    fn a_plunging_image_dims_monotonically_and_never_reaches_the_horizon() {
        let track = PlungeTrack::shared();
        let mut previous = f32::INFINITY;
        let mut crossed = false;
        for k in 0..=6000 {
            let (radius, _) = track.at(k as f32 * 0.05);
            let brightness = plunge_brightness(radius);
            assert!(brightness <= previous, "brightness rose at {k}");
            assert!(radius > HORIZON, "the image reached the horizon");
            assert!(brightness > 0.0, "and never goes out entirely");
            crossed |= brightness < 0.02;
            previous = brightness;
        }
        assert!((plunge_brightness(6.0) - 1.0).abs() < 1e-6);
        assert!(
            crossed,
            "it dims below the 0.02 at which a ghost is removed"
        );
    }

    #[test]
    fn the_displayed_brightness_of_a_plunging_image_dims_by_e_about_every_four_m() {
        // The observed frequency of light from a body falling in, sqrt(1 - 2/r), is the square
        // root of `plunge_brightness`: it falls by e every 4 M of coordinate time close to the
        // horizon (the brightness itself every 2 M), which at 10 M per second is 0.4 s.
        let track = PlungeTrack::shared();
        for start in [40.0_f32, 43.0, 46.0] {
            let (before, after) = (track.at(start).0, track.at(start + 4.0).0);
            let shift = |r: f32| plunge_brightness(r).sqrt();
            let folds = (shift(before) / shift(after)).ln();
            assert!(
                (0.7..1.15).contains(&folds),
                "from {start} M: {folds} e-folds in 4 M"
            );
        }
    }
}
