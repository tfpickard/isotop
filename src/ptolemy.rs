//! Ptolemy: a geocentric cosmos. isotop itself is Earth at the centre. The seven processes whose
//! CPU and memory varied most over the last minute are planets: each rides a deferent circle and
//! carries a chain of epicycles, the largest terms of a direct DFT of its last 60 samples, that
//! replays that minute once a minute. Every other process is a fixed star on a ring that turns
//! once a minute, as bright as its memory. Trails show each planet's apparent path, red where its
//! longitude seen from Earth runs backwards.

use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, TAU};

use crate::history::{History, Reading, SERIES, Sample};
use crate::model::{Identity, Process, Snapshot};
use crate::palette::{kind_color, tint};
use crate::render::shared::SPACE_BASE;
use crate::render::{
    Camera, Color, Frame, Item, NONE, Point, SCALE, Stage, TourTarget, label_name, ring_bounds,
};
use crate::simulation::{Complex, Rng, dft, smoothstep};
use crate::stellar::{Flash, flash, magnitude, spread};
use crate::views::ViewModule;

/// The sky radius in world units; every radius below is a fraction of it.
const SKY: f32 = 40.0;
/// Deferent radii of the spheres as fractions of the sky, innermost first, and their names.
const RADII: [f32; 7] = [0.245, 0.33, 0.415, 0.50, 0.585, 0.67, 0.755];
const SPHERES: [&str; 7] = [
    "Moon", "Mercury", "Venus", "Sun", "Mars", "Jupiter", "Saturn",
];
/// An incumbent keeps its sphere while it ranks this high by variance.
const KEEP_RANK: usize = 10;
/// Samples a process needs in the window before it may wander.
const FORMING: usize = 30;
/// Epicycles kept per planet, largest first, the mean (DC) term left out.
const TERMS: usize = 12;
/// World units per core or per doubling of memory, and the soft limit on a chain's length.
const GAIN: f32 = 0.085 * SKY;
const REACH: f32 = 0.19 * SKY;
/// Deferent period of the outermost sphere, in seconds; inner spheres follow Kepler's T ~ r^1.5.
const OUTER_PERIOD: f32 = 1200.0;
/// The chain replays the window once in this many seconds, whatever the sample rate.
const REPLAY: f64 = 60.0;
/// Seconds over which a re-sorted planet moves to its new sphere.
const RESORT: f32 = 1.5;
/// Coefficients ease to a new sample over the measured sample interval, within these seconds.
const EASE: (f32, f32) = (0.1, 10.0);
/// A trail covers at most this many seconds and this share of its deferent period, sampled on a
/// grid of TRAIL_STEP seconds; its oldest TRAIL_FADE fades out.
const TRAIL_SECONDS: f32 = 240.0;
const TRAIL_SHARE: f32 = 0.8;
const TRAIL_STEP: f32 = 0.25;
const TRAIL_FADE: f32 = 0.45;
/// Fixed stars sit at (0.935 + 0.06 u) of the sky radius; the ring turns once in STAR_TURN s.
const STAR_RING: (f32, f32) = (0.935, 0.06);
const STAR_TURN: f32 = 60.0;
/// How far back, in seconds, births and exits are looked for: longer than a nova lasts.
const FLASH_WINDOW: f64 = 8.0;
/// Epicycles smaller than this many pixels across are not drawn.
const SMALLEST_EPICYCLE: f32 = 1.5;
const RETROGRADE: Color = [255, 70, 66];
const EARTH: Color = [60, 170, 160];
/// Separate this view's per-process draws from every other use of the identity hash.
const PHASE_SALT: u64 = 0x7074_6f6c_656d_7901;
const STAR_SALT: u64 = 0x7074_6f6c_656d_7902;
const MEBIBYTE: f32 = 1_048_576.0;
/// Depth offsets that stack the flat strokes, which all lie on the ground plane, so that neither
/// rasterizer breaks a tie: the decorative rings at the back, then the sight lines, the trails
/// (each sphere in a band of its own, newer above older) and the chains in front. Bodies stand
/// in front of them all.
const BEHIND_RINGS: f32 = -0.6;
const BEHIND_SIGHT: f32 = -0.52;
const BEHIND_TRAILS: f32 = -0.45;
const TRAIL_BAND: f32 = 0.035;
const BEHIND_CHAINS: f32 = -0.1;

/// One epicycle: its frequency in turns per REPLAY seconds, and its coefficient.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Term {
    frequency: i32,
    coefficient: Complex,
}

/// A term easing from one sample's coefficient to the next one's.
#[derive(Clone, Copy, Debug)]
struct Eased {
    frequency: i32,
    from: Complex,
    to: Complex,
}

/// Where a planet's deferent carries it: a circle about Earth whose radius and speed ease to a
/// new sphere's over RESORT seconds from `epoch`, keeping the angle continuous.
#[derive(Clone, Copy, Debug)]
struct Deferent {
    epoch: f32,
    /// The angle at `epoch`, in radians.
    angle: f32,
    /// Radius and turns per second before and after `epoch`.
    radius: (f32, f32),
    rate: (f32, f32),
}

impl Deferent {
    /// A deferent that has always turned at `period`: angle `phase + 2 pi t / period`.
    fn steady(phase: f32, radius: f32, period: f32, time: f32) -> Deferent {
        Deferent {
            epoch: time,
            angle: phase + TAU * (time / period).rem_euclid(1.0),
            radius: (radius, radius),
            rate: (period.recip(), period.recip()),
        }
    }

    /// This deferent moving to `radius` and `period` from `time` on.
    fn moved(&self, time: f32, radius: f32, period: f32) -> Deferent {
        Deferent {
            epoch: time,
            angle: self.angle(time),
            radius: (self.radius(time), radius),
            rate: (self.rate(time), period.recip()),
        }
    }

    fn progress(&self, time: f32) -> f32 {
        smoothstep((time - self.epoch) / RESORT)
    }

    fn radius(&self, time: f32) -> f32 {
        let (from, to) = self.radius;
        from + (to - from) * self.progress(time)
    }

    /// Turns per second at `time`.
    fn rate(&self, time: f32) -> f32 {
        let (from, to) = self.rate;
        from + (to - from) * self.progress(time)
    }

    fn period(&self, time: f32) -> f32 {
        self.rate(time).recip()
    }

    /// The angle at `time`: the rate integrated from `epoch`. The smoothstep ease integrates to
    /// `u^3 - u^4 / 2` over `u = t / RESORT`, and to a half once it is over.
    fn angle(&self, time: f32) -> f32 {
        let since = time - self.epoch;
        let (from, to) = self.rate;
        if since <= 0.0 {
            return self.angle + TAU * from * since;
        }
        let u = (since / RESORT).min(1.0);
        let eased = RESORT * (u.powi(3) - u.powi(4) * 0.5) + (since - RESORT).max(0.0);
        self.angle + TAU * (from * since + (to - from) * eased)
    }

    fn point(&self, time: f32) -> Complex {
        Complex::from_angle(self.angle(time)).scale(self.radius(time))
    }

    /// The velocity along the circle; the brief ease of the radius is left out.
    fn velocity(&self, time: f32) -> Complex {
        let point = self.point(time);
        let speed = TAU * self.rate(time);
        Complex::new(-point.im * speed, point.re * speed)
    }
}

/// One wanderer: its sphere, its deferent and its epicycles.
#[derive(Clone, Debug)]
struct Planet {
    id: Identity,
    sphere: usize,
    deferent: Deferent,
    terms: Vec<Eased>,
    /// When the terms began easing towards their targets, and over how many seconds.
    eased_at: f32,
    ease_seconds: f32,
    mean_cpu: f32,
    score: f32,
}

impl Planet {
    /// The eased terms at `time`, largest first.
    fn terms_at(&self, time: f32) -> Vec<Term> {
        let progress = smoothstep((time - self.eased_at) / self.ease_seconds);
        let mut terms: Vec<Term> = self
            .terms
            .iter()
            .map(|term| Term {
                frequency: term.frequency,
                coefficient: term.from + (term.to - term.from).scale(progress),
            })
            .collect();
        terms.sort_by(|a, b| {
            b.coefficient
                .norm()
                .total_cmp(&a.coefficient.norm())
                .then(a.frequency.cmp(&b.frequency))
        });
        terms
    }

    /// Eases from the terms as they stand at `time` to `targets` over `seconds`; a term that
    /// left the twelve eases to nothing, and a new one from nothing.
    fn retarget(&mut self, targets: &[Term], time: f32, seconds: f32) {
        let mut terms: Vec<Eased> = self
            .terms_at(time)
            .into_iter()
            .map(|term| Eased {
                frequency: term.frequency,
                from: term.coefficient,
                to: Complex::default(),
            })
            .collect();
        for target in targets {
            match terms
                .iter_mut()
                .find(|term| term.frequency == target.frequency)
            {
                Some(term) => term.to = target.coefficient,
                None => terms.push(Eased {
                    frequency: target.frequency,
                    from: Complex::default(),
                    to: target.coefficient,
                }),
            }
        }
        terms.retain(|term| term.from.norm() > 1e-6 || term.to.norm() > 1e-6);
        self.terms = terms;
        self.eased_at = time;
        self.ease_seconds = seconds;
    }

    fn position(&self, time: f32) -> Complex {
        self.deferent.point(time) + chain(&self.terms_at(time), f64::from(time))
    }

    fn retrograde(&self, time: f32) -> bool {
        let terms = self.terms_at(time);
        let position = self.deferent.point(time) + chain(&terms, f64::from(time));
        let velocity = self.deferent.velocity(time) + chain_velocity(&terms, f64::from(time));
        retrograde(position, velocity)
    }
}

/// Whether a body at `position` moving at `velocity` runs backwards as seen from Earth at the
/// origin: its apparent longitude falls, `d lambda / dt = Im(conj(p) p') / |p|^2 < 0`.
fn retrograde(position: Complex, velocity: Complex) -> bool {
    (position.conj() * velocity).im < 0.0
}

/// The rotation of a term of `frequency` turns per REPLAY seconds at `time` seconds.
fn rotor(frequency: i32, time: f64) -> Complex {
    Complex::from_angle(TAU * (f64::from(frequency) * time / REPLAY).rem_euclid(1.0) as f32)
}

/// The tip of a chain of `terms` relative to its first joint at `time` seconds.
fn chain(terms: &[Term], time: f64) -> Complex {
    terms.iter().fold(Complex::default(), |sum, term| {
        sum + term.coefficient * rotor(term.frequency, time)
    })
}

/// How fast the tip of a chain moves at `time` seconds.
fn chain_velocity(terms: &[Term], time: f64) -> Complex {
    terms.iter().fold(Complex::default(), |sum, term| {
        let turned = term.coefficient * rotor(term.frequency, time);
        let omega = TAU * term.frequency as f32 / REPLAY as f32;
        sum + Complex::new(-turned.im * omega, turned.re * omega)
    })
}

/// The epicycles of a window `z` whose first sample has the absolute number `oldest`: a direct
/// DFT, the `keep` largest terms without the mean, each turned back by its phase at `oldest`. So
/// `chain(terms, oldest + n)` is `z[n]` minus the mean when every term is kept, and the
/// coefficients barely change when the window slides on by a sample.
fn epicycles(z: &[Complex], oldest: i64, keep: usize) -> Vec<Term> {
    let length = z.len();
    if length == 0 {
        return Vec::new();
    }
    let spectrum = dft(z);
    let mut order: Vec<usize> = (1..length).collect();
    order.sort_by(|&a, &b| {
        spectrum[b]
            .norm()
            .total_cmp(&spectrum[a].norm())
            .then(a.cmp(&b))
    });
    order.truncate(keep);
    let span = length as i64;
    order
        .into_iter()
        .map(|k| {
            let frequency = if k < length / 2 {
                k as i64
            } else {
                k as i64 - span
            };
            let turns = (frequency * oldest).rem_euclid(span) as f32 / span as f32;
            Term {
                frequency: frequency as i32,
                coefficient: spectrum[k].scale(1.0 / length as f32)
                    * Complex::from_angle(-TAU * turns),
            }
        })
        .collect()
}

/// Terms in cores and doublings scaled into world units by the one global gain, soft-clamped so
/// a chain is never longer than REACH: `gain = K L tanh(T / L) / T` with `T = K sum |c|`.
fn scaled(terms: &[Term]) -> Vec<Term> {
    let total = GAIN
        * terms
            .iter()
            .map(|term| term.coefficient.norm())
            .sum::<f32>();
    let gain = if total > 0.0 {
        GAIN * REACH * (total / REACH).tanh() / total
    } else {
        GAIN
    };
    terms
        .iter()
        .map(|term| Term {
            frequency: term.frequency,
            coefficient: term.coefficient.scale(gain),
        })
        .collect()
}

/// The deferent period of a sphere of `radius`: Kepler's `T ~ r^1.5` from the outermost one.
fn period(radius: f32) -> f32 {
    OUTER_PERIOD * (radius / (RADII[6] * SKY)).powf(1.5)
}

/// Memory in doublings: `log2` of MiB, floored at 1 MiB.
fn doublings(memory: u64) -> f32 {
    (memory as f32 / MEBIBYTE).max(1.0).log2()
}

/// Where a fixed star sits at `time`: an angle and a radius fixed by its identity, on a ring
/// that turns once every STAR_TURN seconds.
fn star_seat(id: Identity, time: f32) -> Point {
    let mut rng = Rng::for_identity(id, STAR_SALT);
    let angle = TAU * rng.unit() + TAU * (time / STAR_TURN).rem_euclid(1.0);
    let radius = (STAR_RING.0 + STAR_RING.1 * rng.unit()) * SKY;
    [radius * angle.cos(), radius * angle.sin(), 0.0]
}

fn ground(point: Complex) -> Point {
    [point.re, point.im, 0.0]
}

/// A process's variance score and mean CPU over the window: the variance of its CPU in cores
/// plus that of its memory in doublings, over the samples it is in. None with fewer than
/// FORMING samples.
fn measure(window: &[&Sample], id: Identity) -> Option<(f32, f32)> {
    let readings: Vec<Reading> = window
        .iter()
        .filter_map(|sample| sample.reading(id))
        .collect();
    if readings.len() < FORMING {
        return None;
    }
    let variance = |values: &[f64]| {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let spread = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
        (mean, spread)
    };
    let cores: Vec<f64> = readings.iter().map(|r| f64::from(r.cpu) / 100.0).collect();
    let memory: Vec<f64> = readings
        .iter()
        .map(|r| f64::from(doublings(r.memory)))
        .collect();
    let ((mean_cores, cpu), (_, memory)) = (variance(&cores), variance(&memory));
    Some(((cpu + memory) as f32, (mean_cores * 100.0) as f32))
}

/// A process's window as `cpu in cores + i log2 memory`, minus its mean, padded to SERIES
/// samples by holding its first reading before it and its last across a gap.
fn signal(window: &[&Sample], id: Identity) -> Vec<Complex> {
    let readings: Vec<Option<Reading>> = window.iter().map(|sample| sample.reading(id)).collect();
    let first = readings
        .iter()
        .flatten()
        .next()
        .copied()
        .unwrap_or_default();
    let mut held = first;
    let mut values: Vec<Complex> = vec![first; SERIES - readings.len().min(SERIES)]
        .into_iter()
        .chain(readings.into_iter().map(|reading| {
            held = reading.unwrap_or(held);
            held
        }))
        .map(|reading| Complex::new(reading.cpu / 100.0, doublings(reading.memory)))
        .collect();
    let mean = values
        .iter()
        .fold(Complex::default(), |sum, value| sum + *value)
        .scale(1.0 / values.len() as f32);
    for value in &mut values {
        *value = *value - mean;
    }
    values
}

/// A flat stroke from `a` to `b`, `width` pixels wide, as two triangles that pick nothing. Both
/// rasterizers fill triangles alike, which they do not do for hardware lines. `bias` lifts it in
/// depth off the ground it lies on.
fn stroke(
    frame: &mut Frame,
    camera: &Camera,
    (a, b): (Point, Point),
    width: f32,
    color: Color,
    bias: f32,
) {
    let (a, b) = (frame.project(camera, a), frame.project(camera, b));
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    if !(0.01..=12000.0).contains(&length) {
        return;
    }
    let (nx, ny) = (-dy / length * width * 0.5, dx / length * width * 0.5);
    let corner = |p: Point, side: f32| [p[0] + side * nx, p[1] + side * ny, p[2] + bias];
    let quad = [
        corner(a, 1.0),
        corner(b, 1.0),
        corner(b, -1.0),
        corner(a, -1.0),
    ];
    frame
        .items
        .push(Item::Triangle([quad[0], quad[1], quad[2]], color, NONE));
    frame
        .items
        .push(Item::Triangle([quad[0], quad[2], quad[3]], color, NONE));
}

/// A closed loop of `segments` strokes through `point(angle)`.
fn hoop(
    frame: &mut Frame,
    camera: &Camera,
    segments: usize,
    point: impl Fn(f32) -> Point,
    width: f32,
    color: Color,
    bias: f32,
) {
    let mut last = point(0.0);
    for segment in 1..=segments {
        let next = point(segment as f32 / segments as f32 * TAU);
        stroke(frame, camera, (last, next), width, color, bias);
        last = next;
    }
}

/// A circle on the ground about `center`.
fn circle(center: Complex, radius: f32) -> impl Fn(f32) -> Point {
    move |angle| ground(center + Complex::from_angle(angle).scale(radius))
}

/// `color` drawn at `alpha` over the plain space sky, as the trail strokes would blend if
/// triangles were translucent.
fn over_sky(color: Color, alpha: f32) -> Color {
    std::array::from_fn(|k| (SPACE_BASE[k] + color[k] as f32 * alpha).min(255.0) as u8)
}

/// What the legend leads with: how far the window has filled, or who is in retrograde.
fn status(samples: usize, wanderers: usize, retrograde: &[String]) -> String {
    if samples < FORMING {
        return format!("the heavens are still forming: {samples} of {FORMING} samples");
    }
    match retrograde {
        [] if wanderers == 0 => "no process varies enough to wander".into(),
        [] => format!("{wanderers} wanderers, none in retrograde"),
        [one] => format!("{one} is in retrograde"),
        [first, second] => format!("{first} and {second} are in retrograde"),
        [first, second, rest @ ..] => {
            format!(
                "{first}, {second} and {} more are in retrograde",
                rest.len()
            )
        }
    }
}

#[derive(Default)]
pub struct Ptolemy {
    /// The absolute number of the newest sample the state was brought up to.
    observed: Option<u64>,
    planets: Vec<Planet>,
    /// Samples in the window at the last update.
    samples: usize,
    /// Births and exits of the last FLASH_WINDOW seconds: when each was seen, and an exit's last
    /// reading.
    births: HashMap<Identity, f64>,
    exits: Vec<(Identity, f64, Reading)>,
    /// Processes that stopped wandering lately, and when: their exits do not flash on the ring.
    former: HashMap<Identity, f64>,
    /// The colour each star was last drawn in, for its fade after it exits.
    colors: HashMap<Identity, Color>,
    /// Names of the planets in retrograde in the last frame drawn, for the legend.
    retrograde: Vec<String>,
    /// When the last frame was drawn, what it drew and which of it was Earth: the harness's
    /// anchors.
    #[cfg(test)]
    seated: (f32, Vec<Identity>, Option<Identity>),
}

impl Ptolemy {
    /// Brings the wanderers, their spheres and epicycles, and the births and exits up to the
    /// newest sample at or before the shown one, once per sample. `time` is the scene time.
    fn observe(
        &mut self,
        history: &History,
        processes: &[&Process],
        snapshot: &Snapshot,
        time: f32,
    ) {
        let until = snapshot.elapsed;
        let window: Vec<&Sample> = history.recent(until, SERIES).collect();
        let Some(newest) = window.last() else {
            self.samples = 0;
            self.planets.clear();
            return;
        };
        if self.observed == Some(newest.sequence) {
            return;
        }
        self.observed = Some(newest.sequence);
        self.samples = window.len();
        let interval = match window.as_slice() {
            [.., a, b] => (b.elapsed - a.elapsed) as f32,
            _ => 1.0,
        }
        .clamp(EASE.0, EASE.1);
        // Easing starts when the sample arrived, so it does not depend on the frame rate.
        let start = if history.newest() == Some(until) {
            history.arrived()
        } else {
            time
        };
        self.births.clear();
        self.exits.clear();
        for change in history.changes(until, FLASH_WINDOW) {
            if change.born {
                self.births.insert(change.id, change.elapsed);
            } else {
                self.exits.push((change.id, change.elapsed, change.reading));
            }
        }
        let observer = snapshot.observer.as_ref().map(|process| process.id);
        let mut candidates: Vec<(f32, f32, Identity)> = processes
            .iter()
            .filter(|process| Some(process.id) != observer)
            .filter_map(|process| {
                let (score, mean_cpu) = measure(&window, process.id)?;
                (score > 0.0).then_some((score, mean_cpu, process.id))
            })
            .collect();
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.cmp(&b.2)));
        let rank: HashMap<Identity, usize> = candidates
            .iter()
            .enumerate()
            .map(|(rank, &(_, _, id))| (id, rank))
            .collect();
        let before: HashSet<Identity> = self.planets.iter().map(|planet| planet.id).collect();
        let (kept, left): (Vec<Planet>, Vec<Planet>) = std::mem::take(&mut self.planets)
            .into_iter()
            .partition(|planet| rank.get(&planet.id).is_some_and(|&rank| rank < KEEP_RANK));
        for planet in left {
            self.former.insert(planet.id, until);
        }
        let mut members: Vec<(Identity, f32, f32, Option<Planet>)> = kept
            .into_iter()
            .map(|planet| (planet.id, 0.0, 0.0, Some(planet)))
            .collect();
        for &(_, _, id) in &candidates {
            if members.len() >= SPHERES.len() {
                break;
            }
            if !before.contains(&id) {
                members.push((id, 0.0, 0.0, None));
            }
        }
        for member in &mut members {
            let &(score, mean_cpu, _) = &candidates[rank[&member.0]];
            (member.1, member.2) = (score, mean_cpu);
        }
        let after: HashSet<Identity> = members.iter().map(|member| member.0).collect();
        if after == before {
            members.sort_by_key(|member| member.3.as_ref().map_or(0, |planet| planet.sphere));
        } else {
            members.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
        }
        let oldest = newest.sequence as i64 - (SERIES - 1) as i64;
        self.planets = members
            .into_iter()
            .enumerate()
            .map(|(sphere, (id, score, mean_cpu, existing))| {
                let targets = scaled(&epicycles(&signal(&window, id), oldest, TERMS));
                let radius = RADII[sphere] * SKY;
                let mut planet = match existing {
                    Some(mut planet) => {
                        if planet.sphere != sphere {
                            planet.deferent = planet.deferent.moved(start, radius, period(radius));
                            planet.sphere = sphere;
                        }
                        planet.retarget(&targets, start, interval);
                        planet
                    }
                    None => {
                        let phase = TAU * Rng::for_identity(id, PHASE_SALT).unit();
                        Planet {
                            id,
                            sphere,
                            deferent: Deferent::steady(phase, radius, period(radius), start),
                            terms: targets
                                .iter()
                                .map(|term| Eased {
                                    frequency: term.frequency,
                                    from: term.coefficient,
                                    to: term.coefficient,
                                })
                                .collect(),
                            eased_at: start,
                            ease_seconds: interval,
                            mean_cpu,
                            score,
                        }
                    }
                };
                (planet.mean_cpu, planet.score) = (mean_cpu, score);
                planet
            })
            .collect();
        self.former
            .retain(|_, &mut left| left > until - FLASH_WINDOW);
        let alive: HashSet<Identity> = processes.iter().map(|process| process.id).collect();
        let exited: HashSet<Identity> = self.exits.iter().map(|exit| exit.0).collect();
        self.colors
            .retain(|id, _| alive.contains(id) || exited.contains(id));
    }

    /// The wanderers and their spheres, innermost first.
    #[cfg(test)]
    fn spheres(&self) -> Vec<(Identity, usize)> {
        self.planets
            .iter()
            .map(|planet| (planet.id, planet.sphere))
            .collect()
    }

    /// The decorative armillary rings and ecliptic scale, and the deferents in use.
    fn draw_armillary(&self, frame: &mut Frame, camera: &Camera, time: f32, pixel: f32) {
        let thin = pixel.max(1.0);
        let center = Complex::default();
        hoop(
            frame,
            camera,
            128,
            circle(center, 1.045 * SKY),
            (2.0 * pixel).max(1.0),
            [70, 66, 100],
            BEHIND_RINGS,
        );
        for radius in [0.905, 0.87] {
            hoop(
                frame,
                camera,
                128,
                circle(center, radius * SKY),
                thin,
                [52, 50, 80],
                BEHIND_RINGS,
            );
        }
        for (turn, tilt, color) in [
            (0.4, 66.0_f32, [46, 40, 70]),
            (0.4 + FRAC_PI_2, 78.0, [40, 36, 62]),
        ] {
            let squash = tilt.to_radians().cos();
            let turned = Complex::from_angle(turn);
            hoop(
                frame,
                camera,
                128,
                |angle| {
                    let point = Complex::new(angle.cos(), angle.sin() * squash);
                    ground((point * turned).scale(0.88 * SKY))
                },
                thin,
                color,
                BEHIND_RINGS,
            );
        }
        for degree in (0..360).step_by(5) {
            let direction = Complex::from_angle((degree as f32).to_radians());
            let outer = match degree {
                d if d % 30 == 0 => 0.905,
                d if d % 10 == 0 => 0.89,
                _ => 0.88,
            };
            let color = if degree % 30 == 0 {
                [92, 88, 128]
            } else {
                [60, 58, 88]
            };
            stroke(
                frame,
                camera,
                (
                    ground(direction.scale(0.87 * SKY)),
                    ground(direction.scale(outer * SKY)),
                ),
                thin,
                color,
                BEHIND_RINGS,
            );
        }
        for planet in &self.planets {
            hoop(
                frame,
                camera,
                96,
                circle(center, planet.deferent.radius(time)),
                thin,
                [58, 60, 92],
                BEHIND_RINGS,
            );
        }
    }

    /// A fixed star, flashing as a nova for a while after its birth.
    fn draw_star(&mut self, stage: &mut Stage, process: &Process, pick: u32, age: Option<f32>) {
        let seat = star_seat(process.id, stage.time);
        let state = age.and_then(|age| flash(Flash::Nova, age));
        let offset = state.map_or(0.0, |state| state.magnitude_offset);
        let color = kind_color(process);
        self.colors.insert(process.id, color);
        let spread = spread(magnitude(process.memory) + offset, stage.frame.height);
        stage.frame.star(
            stage.camera,
            seat,
            &spread,
            color,
            pick,
            stage.selected == Some(process.id),
        );
        stage.positions.insert(process.id, seat);
    }

    /// The trail of `planet` over the last `span` seconds, with the chain and its sight line.
    fn draw_planet_paths(
        frame: &mut Frame,
        camera: &Camera,
        planet: &Planet,
        color: Color,
        time: f32,
        pixel: f32,
    ) {
        let terms = planet.terms_at(time);
        let span = TRAIL_SECONDS.min(TRAIL_SHARE * planet.deferent.period(time));
        let head = (time / TRAIL_STEP).floor() * TRAIL_STEP;
        let count = (span / TRAIL_STEP) as usize;
        // Rotors stepped back a grid step at a time, so a trail costs one multiply per term
        // per point rather than a sine and cosine.
        let steps: Vec<Complex> = terms
            .iter()
            .map(|term| rotor(-term.frequency, f64::from(TRAIL_STEP)))
            .collect();
        let mut rotors: Vec<Complex> = terms
            .iter()
            .map(|term| rotor(term.frequency, f64::from(head)))
            .collect();
        let exact = {
            let position = planet.deferent.point(time) + chain(&terms, f64::from(time));
            let velocity = planet.deferent.velocity(time) + chain_velocity(&terms, f64::from(time));
            (position, retrograde(position, velocity), 0.0)
        };
        let mut points = Vec::with_capacity(count + 2);
        points.push(exact);
        for step in 0..=count {
            let at = head - step as f32 * TRAIL_STEP;
            let (mut tip, mut speed) = (Complex::default(), Complex::default());
            for ((term, rotor), turn) in terms.iter().zip(&mut rotors).zip(&steps) {
                let turned = term.coefficient * *rotor;
                let omega = TAU * term.frequency as f32 / REPLAY as f32;
                tip = tip + turned;
                speed = speed + Complex::new(-turned.im * omega, turned.re * omega);
                *rotor = *rotor * *turn;
            }
            let position = planet.deferent.point(at) + tip;
            let velocity = planet.deferent.velocity(at) + speed;
            points.push((position, retrograde(position, velocity), (time - at) / span));
        }
        let width = if frame.width > 1280 { 2.2 } else { 1.3 };
        let band = BEHIND_TRAILS + TRAIL_BAND * planet.sphere as f32;
        for pair in points.windows(2) {
            let ((newer, backwards, age), (older, _, _)) = (pair[0], pair[1]);
            let fade = ((1.0 - age) / TRAIL_FADE).clamp(0.0, 1.0).powf(1.3);
            let alpha = fade * if backwards { 0.95 } else { 0.75 };
            if alpha < 0.02 {
                continue;
            }
            let base = if backwards { RETROGRADE } else { color };
            stroke(
                frame,
                camera,
                (ground(older), ground(newer)),
                width,
                over_sky(base, alpha),
                band + 0.03 * (1.0 - age),
            );
        }
        // The chain: epicycles largest first from the point on the deferent.
        let thin = (1.2 * pixel).max(1.0);
        let mut joint = planet.deferent.point(time);
        let mut joints = vec![joint];
        for (index, term) in terms.iter().enumerate() {
            let radius = term.coefficient.norm();
            if radius * camera.zoom * SCALE * 2.0 >= SMALLEST_EPICYCLE {
                let shade = if index == 0 {
                    [110, 118, 156]
                } else {
                    [84, 98, 132]
                };
                hoop(
                    frame,
                    camera,
                    32,
                    circle(joint, radius),
                    thin,
                    shade,
                    BEHIND_CHAINS,
                );
            }
            joint = joint + term.coefficient * rotor(term.frequency, f64::from(time));
            joints.push(joint);
        }
        for pair in joints.windows(2) {
            stroke(
                frame,
                camera,
                (ground(pair[0]), ground(pair[1])),
                (1.4 * pixel).max(1.0),
                [170, 174, 191],
                BEHIND_CHAINS + 0.01,
            );
        }
        let anchor = frame.project(camera, ground(joints[0]));
        frame.items.push(Item::Sphere {
            center: anchor,
            radius: 2.2 * pixel,
            depth: 2.2 * pixel / camera.zoom,
            color: [150, 156, 190],
            pick: NONE,
            selected: false,
        });
        stroke(
            frame,
            camera,
            ([0.0; 3], ground(joints[0])),
            1.0,
            [40, 42, 64],
            BEHIND_RINGS + 0.02,
        );
        // A planet in retrograde: a dashed sight line from Earth and a marker pointing back
        // on the ecliptic scale.
        if exact.1 && exact.0.norm() > 0.0 {
            let direction = exact.0.scale(exact.0.norm().recip());
            let (dash, gap) = (0.5, 0.4);
            let mut from = 0.0;
            while from < 0.905 * SKY {
                let to = (from + dash).min(0.905 * SKY);
                stroke(
                    frame,
                    camera,
                    (ground(direction.scale(from)), ground(direction.scale(to))),
                    (1.4 * pixel).max(1.0),
                    [150, 60, 60],
                    BEHIND_SIGHT,
                );
                from = to + gap;
            }
            let angle = direction.im.atan2(direction.re);
            let corner = |radius: f32, turn: f32| {
                let mut point = frame.project(
                    camera,
                    ground(Complex::from_angle(angle + turn).scale(radius * SKY)),
                );
                point[2] += BEHIND_SIGHT;
                point
            };
            frame.items.push(Item::Triangle(
                [
                    corner(0.918, -0.035),
                    corner(0.905, 0.02),
                    corner(0.932, 0.02),
                ],
                RETROGRADE,
                NONE,
            ));
        }
    }
}

impl ViewModule for Ptolemy {
    #[expect(clippy::too_many_lines, reason = "the drawing reads in frame order")]
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let time = stage.time;
        self.observe(stage.history, processes, snapshot, time);
        let until = snapshot.elapsed;
        let since = if stage.history.newest() == Some(until) {
            (time - stage.history.arrived()).clamp(0.0, EASE.1)
        } else {
            0.0
        };
        let index: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(index, process)| (process.id, index))
            .collect();
        let observer = snapshot.observer.as_ref();
        // Sizes in the mockup's pixels, which assume a 900-pixel-high frame.
        let pixel = stage.frame.height as f32 / 900.0;
        stage
            .bounds
            .extend(ring_bounds([0.0, 0.0], 1.05 * SKY, 0.0, 0.0));
        self.draw_armillary(stage.frame, stage.camera, time, pixel);

        let wanderers: HashSet<Identity> = self.planets.iter().map(|planet| planet.id).collect();
        let mut shown = 0;
        for (pick, process) in processes.iter().enumerate() {
            if wanderers.contains(&process.id) || Some(process.id) == observer.map(|o| o.id) {
                continue;
            }
            let age = self
                .births
                .get(&process.id)
                .map(|&born| (until - born) as f32 + since);
            self.draw_star(stage, process, pick as u32, age);
            shown += 1;
        }
        for &(id, gone, reading) in &self.exits {
            if index.contains_key(&id) || self.former.contains_key(&id) {
                continue;
            }
            let age = (until - gone) as f32 + since;
            let Some(state) = flash(Flash::Fade, age).filter(|state| state.opacity > 0.0) else {
                continue;
            };
            let mut faded = spread(
                magnitude(reading.memory) + state.magnitude_offset,
                stage.frame.height,
            );
            faded.core *= state.opacity;
            faded.halo *= state.opacity;
            let color = self.colors.get(&id).copied().unwrap_or([200, 205, 225]);
            stage.frame.star(
                stage.camera,
                star_seat(id, time),
                &faded,
                tint(color, state.opacity),
                NONE,
                false,
            );
        }

        let drawn: Vec<(&Planet, &Process)> = self
            .planets
            .iter()
            .filter_map(|planet| Some((planet, processes[*index.get(&planet.id)?])))
            .collect();
        for &(planet, process) in &drawn {
            Self::draw_planet_paths(
                stage.frame,
                stage.camera,
                planet,
                kind_color(process),
                time,
                pixel,
            );
        }
        let mut retrograde = Vec::new();
        let mut tour = Vec::new();
        for &(planet, process) in &drawn {
            let position = ground(planet.position(time));
            let backwards = planet.retrograde(time);
            let color = kind_color(process);
            let radius = (0.34
                + 0.105 * (process.memory as f32 / (64.0 * MEBIBYTE)).max(1.0).log2())
            .min(1.2);
            let glow = radius * stage.camera.zoom;
            stage
                .frame
                .glow(stage.camera, position, glow * 2.2, color, 0.55);
            if backwards {
                stage
                    .frame
                    .glow(stage.camera, position, glow * 3.2, RETROGRADE, 0.35);
                retrograde.push(label_name(&process.name).into_owned());
            }
            stage.frame.sphere(
                stage.camera,
                position,
                radius,
                color,
                index[&planet.id] as u32,
                stage.selected == Some(planet.id),
            );
            stage.positions.insert(planet.id, position);
            let sphere = SPHERES[planet.sphere];
            stage.places.push((
                position,
                format!("Sphere of {sphere}: {}", label_name(&process.name)),
            ));
            stage.popup.push((
                planet.id,
                format!(
                    "sphere of {sphere} | {} epicycles{}",
                    planet.terms.len().min(TERMS),
                    if backwards { " | in retrograde" } else { "" }
                ),
            ));
            tour.push(TourTarget {
                id: planet.id,
                position,
                weight: planet.score,
            });
            shown += 1;
        }
        tour.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
        stage.tour.extend(tour);

        // Earth: isotop itself, the observer.
        let origin = [0.0; 3];
        let zoom = stage.camera.zoom;
        stage
            .frame
            .glow(stage.camera, origin, 1.3 * zoom, EARTH, 0.45);
        let earth =
            observer.and_then(|observer| index.get(&observer.id).map(|&i| (observer.id, i)));
        stage.frame.sphere(
            stage.camera,
            origin,
            0.77,
            EARTH,
            earth.map_or(NONE, |(_, pick)| pick as u32),
            earth.is_some_and(|(id, _)| stage.selected == Some(id)),
        );
        let center = stage.frame.project(stage.camera, origin);
        for (dx, dy, radius) in [(-2.5, -1.5, 3.2), (2.8, 2.2, 2.4)] {
            stage.frame.items.push(Item::Glow {
                center: [center[0] + dx * pixel, center[1] + dy * pixel],
                radius: radius * pixel,
                color: [30, 90, 40],
                strength: 0.35,
            });
        }
        if let Some((id, _)) = earth {
            stage.positions.insert(id, origin);
            shown += 1;
        }
        let below = |distance: f32| stage.camera.front(origin, distance);
        match observer {
            Some(observer) => {
                stage
                    .places
                    .push((below(1.8), format!("isotop (pid {})", observer.id.pid)));
                stage.places.push((
                    below(3.6),
                    format!(
                        "{:.1} cores | {:.0} MiB",
                        observer.cpu / 100.0,
                        observer.memory as f32 / MEBIBYTE
                    ),
                ));
            }
            None => stage.places.push((below(1.8), "isotop".into())),
        }
        for degree in (0..360).step_by(30) {
            let direction = Complex::from_angle((degree as f32).to_radians());
            stage
                .places
                .push((ground(direction.scale(0.845 * SKY)), degree.to_string()));
        }
        self.retrograde = retrograde;
        #[cfg(test)]
        {
            self.seated = (
                time,
                stage.positions.keys().copied().collect(),
                earth.map(|(id, _)| id),
            );
        }
        shown
    }

    fn legend(&self, _snapshot: &Snapshot) -> String {
        format!(
            " {} | planet = most varied process, busiest innermost | epicycles = {TERMS} largest DFT terms of CPU + i log2 memory, last {SERIES} samples | red trail = retrograde | stars = steady processes, brightness = memory | Earth = isotop",
            status(self.samples, self.planets.len(), &self.retrograde)
        )
    }

    /// Each drawn process's seat on the ring of fixed stars, where a star shines and where a
    /// wanderer shines again once it stops wandering; Earth's is the centre. Which processes
    /// wander is a measurement, their variance, so the harness checks the seats, which come
    /// from identities alone; the spheres' hysteresis is tested here.
    #[cfg(test)]
    fn anchors(&self) -> Option<HashMap<Identity, Point>> {
        let (time, drawn, earth) = &self.seated;
        Some(
            drawn
                .iter()
                .map(|&id| {
                    let seat = if Some(id) == *earth {
                        [0.0; 3]
                    } else {
                        star_seat(id, *time)
                    };
                    (id, seat)
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{churned, rescaled};
    use crate::model::demo_with;
    use crate::model::testing::process;
    use crate::render::{Camera, RenderInput, Scene};
    use crate::views::view;

    const MIB: u64 = 1 << 20;

    /// A process `pid` in sample `n` whose CPU swings by `swing` percent about `mean`.
    fn swinging(pid: u32, n: usize, mean: f32, swing: f32) -> Process {
        Process {
            cpu: mean + swing * (n as f32 * 0.9 + pid as f32).sin(),
            memory: 64 * MIB,
            ..process(pid, 1)
        }
    }

    /// Records `count` one-second samples after the history's newest, each holding a process per
    /// `(pid, mean, swing)`, and returns the last.
    fn feed(history: &mut History, count: usize, figures: &[(u32, f32, f32)]) -> Snapshot {
        let first = history.newest().map_or(0, |newest| newest as usize + 1);
        let mut last = Snapshot::default();
        for n in first..first + count {
            last = Snapshot {
                elapsed: n as f64,
                processes: figures
                    .iter()
                    .map(|&(pid, mean, swing)| swinging(pid, n, mean, swing))
                    .collect(),
                ..Snapshot::default()
            };
            history.record(&last, n as f32);
        }
        last
    }

    fn observe(ptolemy: &mut Ptolemy, history: &History, snapshot: &Snapshot) {
        let processes: Vec<&Process> = snapshot.processes.iter().collect();
        ptolemy.observe(history, &processes, snapshot, snapshot.elapsed as f32);
    }

    fn pids(ptolemy: &Ptolemy) -> Vec<u32> {
        ptolemy.spheres().iter().map(|(id, _)| id.pid).collect()
    }

    /// Twelve processes whose swings fall with their pid and whose means rise with it.
    fn twelve(swing: impl Fn(u32) -> f32, mean: impl Fn(u32) -> f32) -> Vec<(u32, f32, f32)> {
        (1..=12).map(|pid| (pid, mean(pid), swing(pid))).collect()
    }

    #[test]
    fn dft_reconstruction_equals_the_samples_at_integer_times() {
        let mut rng = Rng::new(7);
        let mut z: Vec<Complex> = (0..SERIES)
            .map(|n| {
                let phase = n as f32 * TAU / SERIES as f32;
                Complex::new(
                    (3.0 * phase).sin() + 0.4 * rng.signed(),
                    (7.0 * phase).cos() * 0.5 + 0.2 * rng.signed(),
                )
            })
            .collect();
        let mean = z
            .iter()
            .fold(Complex::default(), |sum, value| sum + *value)
            .scale(1.0 / SERIES as f32);
        for value in &mut z {
            *value = *value - mean;
        }
        let oldest = 1234;
        let terms = epicycles(&z, oldest, SERIES - 1);
        for (n, sample) in z.iter().enumerate() {
            let tip = chain(&terms, (oldest + n as i64) as f64);
            assert!(
                (tip - *sample).norm() < 1e-4,
                "sample {n}: {tip:?} {sample:?}"
            );
        }
        // Sliding a periodic window by a sample leaves the turned-back coefficients alone.
        let mut slid = z.clone();
        slid.rotate_left(1);
        let next = epicycles(&slid, oldest + 1, SERIES - 1);
        for (a, b) in terms.iter().zip(&next) {
            assert_eq!(a.frequency, b.frequency);
            assert!((a.coefficient - b.coefficient).norm() < 1e-5);
        }
    }

    #[test]
    fn a_planet_whose_epicycle_outruns_its_deferent_is_in_retrograde_and_a_slow_one_is_not() {
        // The deferent moves at 20 * 2 pi / 1200 = 0.105 units a second, an epicycle of radius
        // r turning once a minute at r * 2 pi / 60.
        let planet = |radius: f32| Planet {
            id: process(1, 0).id,
            sphere: 0,
            deferent: Deferent::steady(0.0, 20.0, 1200.0, 0.0),
            terms: vec![Eased {
                frequency: 1,
                from: Complex::new(radius, 0.0),
                to: Complex::new(radius, 0.0),
            }],
            eased_at: 0.0,
            ease_seconds: 1.0,
            mean_cpu: 0.0,
            score: 0.0,
        };
        let backwards = |planet: &Planet| {
            (0..600)
                .filter(|&step| planet.retrograde(step as f32 * 0.1))
                .count()
        };
        assert!(
            backwards(&planet(3.0)) > 0,
            "0.31 units a second outruns 0.105"
        );
        assert_eq!(backwards(&planet(0.5)), 0, "0.05 units a second does not");
    }

    #[test]
    fn wanderers_keep_their_spheres_while_ranks_shuffle_inside_the_top_ten() {
        let mut history = History::default();
        let mut ptolemy = Ptolemy::default();
        let first = feed(
            &mut history,
            SERIES,
            &twelve(
                |pid| 130.0 - 10.0 * pid as f32,
                |pid| 250.0 + 10.0 * pid as f32,
            ),
        );
        observe(&mut ptolemy, &history, &first);
        let before = ptolemy.spheres();
        assert_eq!(pids(&ptolemy).len(), 7);
        assert!(pids(&ptolemy).iter().all(|&pid| pid <= 7));
        // 8, 9 and 10 now swing hardest; the seven wanderers rank fourth to tenth.
        let shuffled = feed(
            &mut history,
            SERIES,
            &twelve(
                |pid| match pid {
                    8..=10 => 230.0 - 10.0 * pid as f32,
                    11 | 12 => 70.0 - 10.0 * (pid - 10) as f32,
                    _ => 130.0 - 10.0 * pid as f32,
                },
                |pid| 250.0 + 10.0 * pid as f32,
            ),
        );
        observe(&mut ptolemy, &history, &shuffled);
        assert_eq!(ptolemy.spheres(), before);
    }

    #[test]
    fn a_wanderer_leaves_its_sphere_when_it_drops_out_of_the_top_ten_or_exits() {
        let mut history = History::default();
        let mut ptolemy = Ptolemy::default();
        let first = feed(
            &mut history,
            SERIES,
            &twelve(
                |pid| 130.0 - 10.0 * pid as f32,
                |pid| 250.0 + 10.0 * pid as f32,
            ),
        );
        observe(&mut ptolemy, &history, &first);
        // 1 swings least of all, 2 exits, and 8 and 9 are the best of the rest.
        let figures: Vec<(u32, f32, f32)> = twelve(
            |pid| match pid {
                1 => 5.0,
                3..=7 => 130.0 - 10.0 * pid as f32,
                _ => 63.0 - 2.0 * pid as f32,
            },
            |pid| 250.0 + 10.0 * pid as f32,
        )
        .into_iter()
        .filter(|&(pid, _, _)| pid != 2)
        .collect();
        let after = feed(&mut history, SERIES, &figures);
        observe(&mut ptolemy, &history, &after);
        let mut wanderers = pids(&ptolemy);
        wanderers.sort_unstable();
        assert_eq!(wanderers, [3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn spheres_are_named_busiest_innermost_and_renamed_only_when_membership_changes() {
        let mut history = History::default();
        let mut ptolemy = Ptolemy::default();
        let swing = |pid: u32| 130.0 - 10.0 * pid as f32;
        let first = feed(
            &mut history,
            SERIES,
            &twelve(swing, |pid| 250.0 + 10.0 * pid as f32),
        );
        observe(&mut ptolemy, &history, &first);
        assert_eq!(pids(&ptolemy), [7, 6, 5, 4, 3, 2, 1], "Moon is the busiest");
        // The order by mean CPU turns round, but the same seven wander: nobody moves.
        let reversed = feed(
            &mut history,
            SERIES,
            &twelve(swing, |pid| 400.0 - 10.0 * pid as f32),
        );
        observe(&mut ptolemy, &history, &reversed);
        assert_eq!(pids(&ptolemy), [7, 6, 5, 4, 3, 2, 1]);
        // 1 stops swinging and 8 takes its place: all seven are named again.
        let changed = feed(
            &mut history,
            SERIES,
            &twelve(
                |pid| if pid == 1 { 1.0 } else { swing(pid) },
                |pid| 400.0 - 10.0 * pid as f32,
            ),
        );
        observe(&mut ptolemy, &history, &changed);
        assert_eq!(pids(&ptolemy), [2, 3, 4, 5, 6, 7, 8]);
        let spheres: Vec<usize> = ptolemy
            .spheres()
            .iter()
            .map(|&(_, sphere)| sphere)
            .collect();
        assert_eq!(spheres, [0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn the_heavens_are_still_forming_until_thirty_samples_exist() {
        let mut history = History::default();
        let mut ptolemy = Ptolemy::default();
        let figures = twelve(
            |pid| 130.0 - 10.0 * pid as f32,
            |pid| 250.0 + 10.0 * pid as f32,
        );
        let early = feed(&mut history, FORMING - 1, &figures);
        observe(&mut ptolemy, &history, &early);
        assert!(ptolemy.spheres().is_empty());
        assert!(
            ptolemy
                .legend(&early)
                .starts_with(" the heavens are still forming: 29 of 30 samples | "),
            "{}",
            ptolemy.legend(&early)
        );
        let formed = feed(&mut history, 1, &figures);
        observe(&mut ptolemy, &history, &formed);
        assert_eq!(ptolemy.spheres().len(), 7);
        assert!(!ptolemy.legend(&formed).contains("forming"));
    }

    #[test]
    fn a_wild_process_cannot_swallow_the_sky() {
        let mut history = History::default();
        for n in 0..SERIES {
            let wild = Process {
                cpu: if n % 2 == 0 { 0.0 } else { 100_000.0 },
                memory: if n % 3 == 0 { MIB } else { 1 << 40 },
                ..process(1, 0)
            };
            let snapshot = Snapshot {
                elapsed: n as f64,
                processes: vec![wild],
                ..Snapshot::default()
            };
            history.record(&snapshot, n as f32);
        }
        let snapshot = Snapshot {
            elapsed: (SERIES - 1) as f64,
            processes: vec![process(1, 0)],
            ..Snapshot::default()
        };
        let mut ptolemy = Ptolemy::default();
        observe(&mut ptolemy, &history, &snapshot);
        let planet = &ptolemy.planets[0];
        let length: f32 = planet
            .terms_at(0.0)
            .iter()
            .map(|t| t.coefficient.norm())
            .sum();
        assert!(length <= REACH * 1.0001, "{length} > {REACH}");
        for step in 0..600 {
            let distance = planet.position(step as f32 * 0.1).norm();
            assert!(distance <= RADII[0] * SKY + REACH + 0.01, "{distance}");
            assert!(distance < STAR_RING.0 * SKY);
        }
    }

    #[test]
    fn the_chain_does_not_jump_when_a_sample_arrives() {
        let mut history = History::default();
        let mut ptolemy = Ptolemy::default();
        let figures = twelve(
            |pid| 130.0 - 10.0 * pid as f32,
            |pid| 250.0 + 10.0 * pid as f32,
        );
        let snapshot = feed(&mut history, SERIES, &figures);
        observe(&mut ptolemy, &history, &snapshot);
        let arrival = snapshot.elapsed as f32 + 1.0;
        let before: Vec<Complex> = ptolemy
            .planets
            .iter()
            .map(|planet| planet.position(arrival))
            .collect();
        // A sample with every swing doubled, which changes every chain.
        let louder: Vec<(u32, f32, f32)> = figures
            .iter()
            .map(|&(pid, mean, swing)| (pid, mean, swing * 2.0))
            .collect();
        let next = feed(&mut history, 1, &louder);
        assert_eq!(history.arrived(), arrival);
        observe(&mut ptolemy, &history, &next);
        let mut moved = 0.0_f32;
        for (planet, before) in ptolemy.planets.iter().zip(&before) {
            assert!((planet.position(arrival) - *before).norm() < 1e-3);
            let old = planet.terms.iter().map(|term| (term.to - term.from).norm());
            moved = moved.max(old.fold(0.0, f32::max));
        }
        assert!(moved > 0.01, "the new sample changed the chains");
    }

    /// A scene that has taken in the demo samples of the minute before `time` and then
    /// `last(demo at time)`, which it returns.
    fn scene_at(time: f64, last: impl Fn(Snapshot) -> Snapshot) -> (Scene, Snapshot) {
        let row = view("ptolemy").info();
        let mut scene = Scene::new();
        for back in (1..=SERIES).rev() {
            let at = time - back as f64;
            scene.record(&demo_with(at, 192, row.demo), at as f32);
        }
        let snapshot = last(demo_with(time, 192, row.demo));
        scene.record(&snapshot, time as f32);
        (scene, snapshot)
    }

    fn render(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> crate::render::Frame {
        let camera = Camera::default();
        let mut frame = scene.render(&RenderInput::small(
            snapshot,
            view("ptolemy"),
            &camera,
            time,
        ));
        frame.rasterize();
        frame
    }

    /// The drawn processes on the ring of fixed stars, and where.
    fn stars(scene: &Scene) -> HashMap<Identity, Point> {
        scene
            .positions
            .iter()
            .filter(|(_, p)| p[0].hypot(p[1]) >= STAR_RING.0 * SKY - 1e-3)
            .map(|(&id, &p)| (id, p))
            .collect()
    }

    #[test]
    fn fixed_stars_keep_their_places_when_cpu_memory_or_population_changes() {
        let (mut steady, snapshot) = scene_at(300.0, |snapshot| snapshot);
        render(&mut steady, &snapshot, 300.0);
        let before = stars(&steady);
        assert!(before.len() > 150);
        for change in [rescaled as fn(&Snapshot) -> Snapshot, |s| churned(s).0] {
            let (mut changed, snapshot) = scene_at(300.0, |snapshot| change(&snapshot));
            render(&mut changed, &snapshot, 300.0);
            let after = stars(&changed);
            let mut compared = 0;
            for (id, place) in &after {
                if let Some(old) = before.get(id) {
                    assert_eq!(old, place, "pid {}", id.pid);
                    compared += 1;
                }
            }
            assert!(compared > 150, "{compared}");
        }
    }

    #[test]
    fn isotop_is_earth_and_never_a_star_or_planet() {
        let mut history = History::default();
        let mut figures = twelve(
            |pid| 130.0 - 10.0 * pid as f32,
            |pid| 250.0 + 10.0 * pid as f32,
        );
        // isotop swings hardest of all.
        figures.push((4242, 500.0, 400.0));
        let snapshot = feed(&mut history, SERIES, &figures);
        let isotop = snapshot
            .processes
            .last()
            .cloned()
            .expect("isotop is sampled");
        let snapshot = Snapshot {
            observer: Some(isotop.clone()),
            ..snapshot
        };
        let mut ptolemy = Ptolemy::default();
        observe(&mut ptolemy, &history, &snapshot);
        assert!(!pids(&ptolemy).contains(&4242));
        assert_eq!(pids(&ptolemy).len(), 7);
        let mut scene = Scene::new();
        for n in 0..SERIES {
            let mut sample = snapshot.clone();
            sample.elapsed = n as f64;
            sample.processes = figures
                .iter()
                .map(|&(pid, mean, swing)| swinging(pid, n, mean, swing))
                .collect();
            scene.record(&sample, n as f32);
        }
        let frame = render(&mut scene, &snapshot, snapshot.elapsed as f32);
        assert_eq!(scene.positions.get(&isotop.id), Some(&[0.0; 3]));
        assert!(!stars(&scene).contains_key(&isotop.id));
        let earth = frame
            .identities
            .iter()
            .position(|&id| id == isotop.id)
            .expect("isotop is drawn");
        assert!(
            frame.picks.contains(&(earth as u32)),
            "Earth can be clicked"
        );
    }

    #[test]
    fn the_demo_observer_pid_matches_no_demo_process_including_churn() {
        let options = view("ptolemy").info().demo;
        assert!(options.churn && options.observer);
        for count in [16, 192, 1000] {
            for step in 0..60 {
                let time = step as f64 * 7.3;
                let snapshot = demo_with(time, count, options);
                let observer = snapshot.observer.expect("the demo synthesizes isotop");
                assert_eq!((observer.id.pid, observer.name.as_str()), (999, "isotop"));
                assert!(
                    snapshot
                        .processes
                        .iter()
                        .all(|p| p.id.pid != observer.id.pid),
                    "pid 999 is a process at {time} for {count}"
                );
            }
        }
        assert!(crate::model::demo(12.0, 192).observer.is_none());
    }

    #[test]
    fn ptolemy_renders_the_same_frame_twice_from_the_same_inputs() {
        let frames: Vec<_> = (0..2)
            .map(|_| {
                let (mut scene, snapshot) = scene_at(300.0, |snapshot| snapshot);
                render(&mut scene, &snapshot, 300.5)
            })
            .collect();
        assert!(frames[0].picks.iter().any(|&pick| pick != NONE));
        assert!(frames[0].pixels == frames[1].pixels, "pixels differ");
        assert!(frames[0].picks == frames[1].picks, "picks differ");
    }
}
