//! The hr view: processes as stars on a planetarium dome, and the same stars on a
//! Hertzsprung-Russell diagram of the machine.
//!
//! Every process that is not a kernel thread is a star. Its colour is a blackbody at the
//! temperature its CPU gives it (`temperature`, drawn through `blackbody::color`), its size and
//! brightness follow its memory on the astronomical magnitude scale (`stellar::magnitude` and
//! `stellar::spread`), and it twinkles by the variation of its CPU over the last ten seconds.
//! Cgroups are constellations: one disc each from `layout::Discs`, stars seated inside it by their
//! identity hash, stepped boundaries between them and stick figures along the socket links.
//! Kernel threads hold no memory and so no luminosity; they are the dust of a Milky Way band whose
//! brightness is their count.
//!
//! `m` sends every star to its place on a Hertzsprung-Russell diagram: temperature on x with hot
//! on the left, magnitude on y, the machine's own main sequence fitted through the stars
//! (`theil_sen`), and every star classified against it (`classification`, such as `K3 III`).
//!
//! A sky place is a point of the dome in *sky units*: pixels of a 1600 by 900 window at zoom 1.
//! The dome always faces the viewer (`Plot::world` maps a sky point to the ground point that
//! appears at that screen offset), so pan and zoom apply, and rotate and tilt leave it upright.
//! A diagram place is a pure function of the star's own temperature and magnitude, in the same
//! plane, and a flight between the two is driven by `Stage::time` alone.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::f32::consts::TAU;
use std::sync::OnceLock;

use crate::blackbody;
use crate::history::History;
use crate::layout::Discs;
use crate::model::{Identity, Kind, Process, Snapshot};
use crate::palette::mix;
use crate::platform::Source;
use crate::render::shared::LINE_DEPTH_BIAS;
use crate::render::{
    Camera, Color, Field, Frame, Item, Links, NONE, Placement, Point, Relief, Side, Stage, Tag,
    TourTarget, label_name,
};
use crate::simulation::Rng;
use crate::status::missing_note;
use crate::stellar::{Flash, MEMORY_FLOOR, exit_flash, flash, magnitude, spread};
use crate::views::{ViewKey, ViewModule, ViewOptions};

/// The sky window in sky units: the part of the dome the camera frames.
const HALF: [f32; 2] = [800.0, 450.0];
/// The dome's colour field covers this many windows, so it fills the frame at any zoom the fit
/// chooses and at any aspect ratio the terminal has.
const FIELD_SPAN: f32 = 1.5;
/// How far above and below the dome's edge the dark rim fades in, in sky units.
const RIM_FADE: (f32, f32) = (26.0, 6.0);
/// Nodes of the dome field: 192 by 108 per window.
const FIELD_NODES: [usize; 2] = [288, 162];
/// The key of the dome field among the fields a frame holds.
const FIELD_KEY: u64 = 0x6872_646f_6d65_0001;
/// The reference frame height of the prototype's pixel sizes.
const REFERENCE_HEIGHT: f32 = 720.0;

/// Sky units per unit of a disc's need.
const UNIT: f32 = 24.0;
/// Constellation centres are spread this much wider than tall, to fill a wide window.
const STRETCH: f32 = 1.6;
/// How much of the window a layout may fill before it is shrunk to fit.
const FILL: f32 = 0.92;
/// A star sits within this fraction of its constellation's disc.
const SEAT_RADIUS: f32 = 0.85;
/// The least distance between two stars of a constellation at the moment the second is seated, in
/// units of need.
const SEPARATION: f32 = 1.7;
/// The fewest members a constellation's disc is sized for, and how much more it is sized for each
/// time it is outgrown (by half), so a disc is re-placed rarely and a tenth of a population coming
/// and going leaves it where it is.
const MIN_CAPACITY: usize = 12;
const CAPACITY_GROWTH: f32 = 1.5;
/// A layout that has been shrunk to fit is made bigger again only when it would fit this much
/// bigger, so the places of the stars do not follow every constellation that comes and goes.
const REGROW: f32 = 1.5;
/// Cells of the boundary grid across the window.
const CELLS: [usize; 2] = [64, 36];

/// Seconds a flight between the sky and the diagram takes.
const TRANSITION_SECONDS: f32 = 1.5;
/// The longest a star waits before it takes off, as a fraction of the flight.
const MAX_LAG: f32 = 0.28;
/// How far a flight's path bends away from the straight line, as a fraction of its length.
const MAX_BEND: f32 = 0.25;

/// The window of the diagram in sky units: left, right, top and bottom of the axes.
const DIAGRAM: [f32; 4] = [-650.0, 670.0, -358.0, 358.0];
/// log10 of the temperatures at the diagram's left (hot) and right (cold) edges.
const LOG_HOT: f32 = 4.653_212_5;
const LOG_COLD: f32 = 3.352_182_5;
/// Magnitudes at the top and the bottom of the diagram, which are also the limits of a star's.
const MAGNITUDE_TOP: f32 = -6.0;
const MAGNITUDE_BOTTOM: f32 = 9.0;

/// Seconds of CPU history the twinkle reads, and the samples it needs.
const TWINKLE_SECONDS: f64 = 10.0;
const TWINKLE_SAMPLES: usize = 3;
/// Seconds a vanished star is remembered for, to draw its exit: a little over the longest flash,
/// a supernova's eight.
const RECORD_KEEP: f64 = 10.0;
/// Above this many stars the main-sequence fit uses every k-th one, by identity.
const FIT_STARS: usize = 400;
/// Pairs of stars closer than this in log10 T give no slope.
const FIT_MINIMUM_SPREAD: f32 = 0.01;

/// Residual thresholds of the luminosity classes, in magnitudes below (negative) or above the
/// fitted main sequence: a star is the first class whose bound it is under.
const LUMINOSITY_BOUNDS: [(f32, Luminosity); 7] = [
    (-6.0, Luminosity::Ia),
    (-4.5, Luminosity::Ib),
    (-3.25, Luminosity::II),
    (-1.75, Luminosity::III),
    (-0.9, Luminosity::IV),
    (1.5, Luminosity::V),
    (2.75, Luminosity::VI),
];
/// From this residual on a star is a white dwarf if it is hot enough, a subdwarf if not.
const DWARF_RESIDUAL: f32 = 2.75;
const DWARF_KELVIN: f32 = 7500.0;

/// The spectral classes with the temperatures they run between, hottest first.
const CLASSES: [(char, f32, f32); 7] = [
    ('O', 30_000.0, 50_000.0),
    ('B', 10_000.0, 30_000.0),
    ('A', 7_500.0, 10_000.0),
    ('F', 6_000.0, 7_500.0),
    ('G', 5_200.0, 6_000.0),
    ('K', 3_700.0, 5_200.0),
    ('M', 2_400.0, 3_700.0),
];

/// A constellation's name is searched for on rings of its disc at these fractions of its radius,
/// at `NAME_ANGLES` angles each, with horizontal distances counted this much shorter, since the
/// text is wide. It stays where it was while that spot is `NAME_KEEP` as clear as the best.
const NAME_RINGS: [f32; 4] = [0.35, 0.6, 0.85, 1.0];
const NAME_ANGLES: usize = 12;
const NAME_SQUASH: f32 = 0.45;
const NAME_KEEP: f32 = 0.8;

/// The stars labelled with their class, the brightest.
const LABELLED: usize = 6;
/// The stars the tour and the Space visits go to, the brightest.
const TOURED: usize = 12;

/// The magnitude at which a star's flux is 1, the planetarium reference of `stellar::spread`.
const FLUX_REFERENCE: f32 = 7.0;
/// Stars brighter than this flux get a wide soft bloom as well.
const BLOOM_FLUX: f32 = 60.0;
/// A bloom's strength is `BLOOM_GAIN * flux^0.3`, at most `BLOOM_LIMIT`.
const BLOOM_GAIN: f32 = 0.026;
const BLOOM_LIMIT: f32 = 0.22;
/// The widest a bloom grows, in pixels at a frame 1080 pixels high.
const BLOOM_RADIUS: f32 = 48.0;
/// The least radius, in pixels, at which a star can be picked, whatever its size.
const PICK_RADIUS: f32 = 2.5;

const SEAT_SALT: u64 = 0x6872_5345_4154;
const FLIGHT_SALT: u64 = 0x6872_464c_4947;
const TWINKLE_SALT: u64 = 0x6872_5457_494e;
const DUST_SALT: u64 = 0x6872_4455_5354;

/// The sky's colours: the prototype's dark base and its plum glow.
const SKY_BASE: [f32; 3] = [10.0, 9.0, 22.0];
const SKY_GLOW: [f32; 3] = [81.0, 33.0, 81.0];
/// The colours of the Milky Way's core and edge.
const BAND_CORE: [f32; 3] = [235.0, 220.0, 205.0];
const BAND_EDGE: [f32; 3] = [120.0, 130.0, 190.0];
/// How bright the band is at full count.
const BAND_GAIN: f32 = 0.42;
/// Kernel threads that make the band 63% of its brightness.
const BAND_COUNT: f32 = 40.0;
/// The colour of constellation boundaries, stick figures and the diagram's lines.
const BOUNDARY: Color = [196, 120, 96];
const FIGURE: Color = [96, 150, 225];
const FIGURE_HOT: Color = [200, 225, 255];
const AXIS: Color = [180, 188, 215];
const GRID: Color = [150, 160, 200];
const SEQUENCE: Color = [150, 185, 255];
const SEQUENCE_GLOW: Color = [120, 150, 230];

/// How much of its twinkle a star keeps on the diagram, where its place is the message.
const TWINKLE_FADE: f32 = 0.7;
/// A newborn's brightness starts at this fraction and reaches its own over `NOVA_RAMP_SECONDS`.
const NOVA_RAMP_FLOOR: f32 = 0.1;
const NOVA_RAMP_SECONDS: f32 = 0.5;
/// Seconds the burst of light round a newborn lasts, and its colour.
const NOVA_BURST_SECONDS: f32 = 1.5;
const NOVA_COLOR: Color = [200, 225, 255];
/// A flight is "at rest" within this much progress of an end, which is when labels are shown.
const LABEL_FADE: f32 = 0.02;

/// How strongly the dashed boundaries show over the sky.
const BOUNDARY_ALPHA: f32 = 0.3;
/// The glow along the fitted main sequence: this many puffs of this radius (at 720p) and strength.
const SEQUENCE_PUFFS: usize = 36;
const SEQUENCE_RADIUS: f32 = 30.0;
const SEQUENCE_STRENGTH: f32 = 0.045;

/// The gap a stick figure leaves at a star, in pixels at 720p: `4.5 + 1.6 * max(0, 3.5 - m)`.
const GAP_BASE: f32 = 4.5;
const GAP_PER_MAGNITUDE: f32 = 1.6;
const GAP_FROM: f32 = 3.5;
/// The brightness of a stick figure with no traffic reading, how strongly a figure and a link
/// between constellations show, and the longest of the latter in pixels at 720p.
const FIGURE_FLAT: f32 = 0.65;
const FIGURE_ALPHA: f32 = 0.9;
const CROSS_ALPHA: f32 = 0.13;
const CROSS_REACH: f32 = 360.0;

/// A supernova's flash dies away with this time constant, in seconds, and its shell is drawn
/// from this many segments in these colours, blue-white turning red.
const SUPERNOVA_GLOW_SECONDS: f32 = 0.45;
const SHELL_SEGMENTS: usize = 48;
const SHELL_BLUE: Color = [150, 195, 255];
const SHELL_RED: Color = [255, 105, 70];

/// A kernel thread's dust: radius (pixels at 720p, plus a fixed pad), colour and strength.
const DUST_RADIUS: f32 = 1.3;
const DUST_PAD: f32 = 0.6;
const DUST: Color = [225, 220, 240];
const DUST_STRENGTH: f32 = 0.5;

/// Surface temperature in kelvin of a process using `cpu` percent of one core:
/// `2400 K x (1 + cpu / 5)^0.47`, at most 40,000 K. Idle is 2,400 K (class M), one core about
/// 10,000 K and twenty cores 40,000 K.
pub fn temperature(cpu: f32) -> f32 {
    (2400.0 * (1.0 + cpu.max(0.0) / 5.0).powf(0.47)).min(40_000.0)
}

/// The magnitude of a star holding `memory` bytes: `stellar::magnitude`, clamped to the diagram's
/// range, and +9 for a process with none, which is the bottom of the diagram.
pub fn star_magnitude(memory: u64) -> f32 {
    if memory == 0 {
        MAGNITUDE_BOTTOM
    } else {
        magnitude(memory).clamp(MAGNITUDE_TOP, MAGNITUDE_BOTTOM)
    }
}

/// Whether a star's magnitude says something about it: it holds at least the memory
/// `stellar::magnitude` floors at (below it every process reads alike) and is not at a limit of
/// the range, so it may inform the main-sequence fit.
fn is_measured(memory: u64) -> bool {
    memory >= MEMORY_FLOOR && {
        let raw = magnitude(memory);
        raw > MAGNITUDE_TOP && raw < MAGNITUDE_BOTTOM
    }
}

/// The spectral class letter and subclass digit of a star at `kelvin`: the standard boundaries,
/// and a subclass 0 to 9 that interpolates the logarithm of the temperature within the class,
/// hottest first, so a star at the cool edge of K is K9 and one at its hot edge K0.
pub fn spectral_class(kelvin: f32) -> (char, u8) {
    for (letter, low, high) in CLASSES {
        if kelvin >= low || letter == 'M' {
            let fraction = (high.log10() - kelvin.log10()) / (high.log10() - low.log10());
            return (letter, (10.0 * fraction).floor().clamp(0.0, 9.0) as u8);
        }
    }
    unreachable!("the last class takes every temperature")
}

/// A luminosity class, from the brightest giants to the dwarfs. `D` is a white dwarf: far below the
/// main sequence and hot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::upper_case_acronyms,
    reason = "the classes are Roman numerals, written as astronomers write them"
)]
pub enum Luminosity {
    Ia,
    Ib,
    II,
    III,
    IV,
    V,
    VI,
    D,
}

impl Luminosity {
    fn name(self) -> &'static str {
        match self {
            Self::Ia => "Ia",
            Self::Ib => "Ib",
            Self::II => "II",
            Self::III => "III",
            Self::IV => "IV",
            Self::V => "V",
            Self::VI => "VI",
            Self::D => "D",
        }
    }
}

/// The luminosity class of a star whose magnitude is `residual` above the fitted main sequence
/// (negative is brighter than it) and whose temperature is `kelvin`. Residuals under -6, -4.5,
/// -3.25, -1.75, -0.9, 1.5 and 2.75 are Ia, Ib, II, III, IV, V and VI; from 2.75 on a star is a
/// white dwarf (D) if it is at least 7,500 K and a subdwarf (VI) if not. A convention, not a
/// measurement.
pub fn luminosity_class(residual: f32, kelvin: f32) -> Luminosity {
    if residual >= DWARF_RESIDUAL {
        return if kelvin >= DWARF_KELVIN {
            Luminosity::D
        } else {
            Luminosity::VI
        };
    }
    LUMINOSITY_BOUNDS
        .iter()
        .find(|(bound, _)| residual < *bound)
        .map_or(Luminosity::VI, |&(_, class)| class)
}

/// The main sequence of the machine: magnitude as a straight line of log10 temperature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub intercept: f32,
    pub slope: f32,
}

impl Fit {
    /// The magnitude the main sequence has at `log_temperature`.
    pub fn at(self, log_temperature: f32) -> f32 {
        self.intercept + self.slope * log_temperature
    }

    /// How far above (positive: fainter than) the main sequence a star of `magnitude` at
    /// `kelvin` is.
    pub fn residual(self, kelvin: f32, magnitude: f32) -> f32 {
        magnitude - self.at(kelvin.log10())
    }
}

/// The class a star carries: `K3 III`, or `DA` for a white dwarf of class A, or just `K3` where
/// there is no fit to measure a residual against.
pub fn classification(kelvin: f32, magnitude: f32, fit: Option<Fit>) -> String {
    let (letter, subclass) = spectral_class(kelvin);
    match fit.map(|fit| luminosity_class(fit.residual(kelvin, magnitude), kelvin)) {
        Some(Luminosity::D) => format!("D{letter}"),
        Some(class) => format!("{letter}{subclass} {}", class.name()),
        None => format!("{letter}{subclass}"),
    }
}

/// The median of `values`, which are reordered; the mean of the middle two when there is an even
/// number. `values` must not be empty.
fn median(values: &mut [f32]) -> f32 {
    values.sort_by(f32::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

/// The Theil-Sen line through `points` of (x, y): the median slope over every pair at least
/// `FIT_MINIMUM_SPREAD` apart in x, and the median intercept it leaves. A robust fit: a fifth of
/// the points may be anywhere. None for fewer than three points or when no pair is far enough
/// apart to give a slope.
pub fn theil_sen(points: &[(f32, f32)]) -> Option<Fit> {
    if points.len() < 3 {
        return None;
    }
    let mut slopes = Vec::with_capacity(points.len() * (points.len() - 1) / 2);
    for (index, &(x, y)) in points.iter().enumerate() {
        for &(other_x, other_y) in &points[index + 1..] {
            let spread = other_x - x;
            if spread.abs() > FIT_MINIMUM_SPREAD {
                slopes.push((other_y - y) / spread);
            }
        }
    }
    if slopes.is_empty() {
        return None;
    }
    let slope = median(&mut slopes);
    let mut intercepts: Vec<f32> = points.iter().map(|&(x, y)| y - slope * x).collect();
    Some(Fit {
        intercept: median(&mut intercepts),
        slope,
    })
}

/// The main sequence of `snapshot`: the Theil-Sen fit of magnitude on log10 temperature over its
/// stars, leaving out kernel threads, processes with no memory and stars at the limits of the
/// magnitude range, and using every k-th star by identity above `FIT_STARS`.
pub fn main_sequence(snapshot: &Snapshot) -> Option<Fit> {
    let mut stars: Vec<&Process> = snapshot
        .processes
        .iter()
        .filter(|process| process.kind != Kind::Kernel && is_measured(process.memory))
        .collect();
    stars.sort_by_key(|process| process.id);
    let step = stars.len().div_ceil(FIT_STARS).max(1);
    let points: Vec<(f32, f32)> = stars
        .iter()
        .step_by(step)
        .map(|process| {
            (
                temperature(process.cpu).log10(),
                star_magnitude(process.memory),
            )
        })
        .collect();
    theil_sen(&points)
}

/// The coefficient of variation of `readings`, a CPU history: the standard deviation over the
/// mean. 0 for fewer than three readings and for a mean under a thousandth of a percent.
pub fn variation(readings: &[f32]) -> f32 {
    if readings.len() < TWINKLE_SAMPLES {
        return 0.0;
    }
    let count = readings.len() as f32;
    let mean = readings.iter().sum::<f32>() / count;
    if mean <= 1e-3 {
        return 0.0;
    }
    let variance = readings
        .iter()
        .map(|reading| (reading - mean).powi(2))
        .sum::<f32>()
        / count;
    variance.sqrt() / mean
}

/// How much a star's brightness swings, as a fraction: `0.45` of its variation, at most `0.8`.
pub fn twinkle_amplitude(variation: f32) -> f32 {
    (0.45 * variation).min(0.8)
}

/// The place of a star on the diagram, in sky units, from its temperature and magnitude alone.
pub fn diagram_position(kelvin: f32, magnitude: f32) -> [f32; 2] {
    [
        diagram_x(kelvin.log10()),
        diagram_y(magnitude.clamp(MAGNITUDE_TOP, MAGNITUDE_BOTTOM)),
    ]
}

/// The diagram's x for `log_temperature`: hot on the left.
fn diagram_x(log_temperature: f32) -> f32 {
    DIAGRAM[0] + (LOG_HOT - log_temperature) / (LOG_HOT - LOG_COLD) * (DIAGRAM[1] - DIAGRAM[0])
}

/// The diagram's y for `magnitude`: bright at the top.
fn diagram_y(magnitude: f32) -> f32 {
    DIAGRAM[2]
        + (magnitude - MAGNITUDE_TOP) / (MAGNITUDE_BOTTOM - MAGNITUDE_TOP)
            * (DIAGRAM[3] - DIAGRAM[2])
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Cubic ease in and out.
fn ease(x: f32) -> f32 {
    if x < 0.5 {
        4.0 * x * x * x
    } else {
        1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
    }
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// Where a star is at flight progress `progress` (0 sky, 1 diagram): along a quadratic Bezier from
/// its sky place to its diagram place, bent sideways by `bend`, after waiting `lag` of the flight.
/// At 0 and 1 it is exactly at the ends.
fn flight(sky: [f32; 2], diagram: [f32; 2], lag: f32, bend: f32, progress: f32) -> [f32; 2] {
    if progress <= 0.0 {
        return sky;
    }
    if progress >= 1.0 {
        return diagram;
    }
    let k = ease(((progress - lag * 0.5) / (1.0 - MAX_LAG * 0.5)).clamp(0.0, 1.0));
    let (dx, dy) = (diagram[0] - sky[0], diagram[1] - sky[1]);
    let control = [
        (sky[0] + diagram[0]) / 2.0 - dy * bend,
        (sky[1] + diagram[1]) / 2.0 + dx * bend,
    ];
    let (a, b, c) = ((1.0 - k).powi(2), 2.0 * (1.0 - k) * k, k * k);
    [
        a * sky[0] + b * control[0] + c * diagram[0],
        a * sky[1] + b * control[1] + c * diagram[1],
    ]
}

/// The constellation a process belongs to: its cgroup, or its app group where the platform has
/// no cgroups.
fn constellation(process: &Process) -> &str {
    if process.cgroup.is_empty() {
        &process.group
    } else {
        &process.cgroup
    }
}

/// Where a star is seated in its constellation's disc, in units of need from its centre: a point
/// within `SEAT_RADIUS` of the disc from the identity hash, pushed once out of the way of every
/// star `seated` already, and then kept for good.
fn seat(id: Identity, radius: f32, seated: &[[f32; 2]]) -> [f32; 2] {
    let mut rng = Rng::for_identity(id, SEAT_SALT);
    let reach = SEAT_RADIUS * radius;
    let angle = TAU * rng.unit();
    let distance = reach * rng.unit().sqrt();
    let mut at = [distance * angle.cos(), distance * angle.sin()];
    for other in seated {
        let (dx, dy) = (at[0] - other[0], at[1] - other[1]);
        let apart = dx.hypot(dy);
        if apart < SEPARATION {
            let (ux, uy) = if apart > 1e-4 {
                (dx / apart, dy / apart)
            } else {
                (angle.cos(), angle.sin())
            };
            at = [
                at[0] + ux * (SEPARATION - apart),
                at[1] + uy * (SEPARATION - apart),
            ];
        }
    }
    let out = at[0].hypot(at[1]);
    if out > reach {
        at = [at[0] * reach / out, at[1] * reach / out];
    }
    at
}

/// One constellation as the sky draws it.
#[derive(Clone, Debug)]
struct Group {
    key: String,
    /// The name written across it: the process's `group`.
    label: String,
    /// Its centre and disc radius in sky units.
    centre: [f32; 2],
    radius: f32,
    /// Where its name is written, in sky units: the clearest spot of its disc.
    name_at: [f32; 2],
}

/// A stretch of boundary between two constellations, in sky units, and the sky colour under it.
#[derive(Clone, Copy, Debug)]
struct Run {
    from: [f32; 2],
    to: [f32; 2],
    ground: Color,
}

/// The number of members a constellation of `members` stars has a disc sized for.
fn capacity(members: usize) -> usize {
    let mut capacity = MIN_CAPACITY;
    while capacity < members {
        capacity = (capacity as f32 * CAPACITY_GROWTH).ceil() as usize;
    }
    capacity
}

/// The shrinking that makes a layout of discs (centre, radius, in units of need) fit the window: 1
/// while it already does.
fn layout_scale(discs: &[([f32; 2], f32)]) -> f32 {
    let (mut across, mut down) = (0.0_f32, 0.0_f32);
    for &(centre, radius) in discs {
        across = across.max(centre[0].abs() * STRETCH + radius);
        down = down.max(centre[1].abs() + radius);
    }
    if across <= 0.0 || down <= 0.0 {
        return 1.0;
    }
    (FILL * HALF[0] / (across * UNIT))
        .min(FILL * HALF[1] / (down * UNIT))
        .min(1.0)
}

/// The edges between cells of a fixed grid over the window that belong to different
/// constellations, each cell going to the constellation whose disc it is nearest by power
/// distance (the squared distance from its centre less the squared radius), merged into straight
/// runs.
fn boundaries(groups: &[Group], dome: &Dome) -> Vec<Run> {
    if groups.len() < 2 {
        return Vec::new();
    }
    let cell = [
        2.0 * HALF[0] / CELLS[0] as f32,
        2.0 * HALF[1] / CELLS[1] as f32,
    ];
    let owner: Vec<usize> = (0..CELLS[0] * CELLS[1])
        .map(|index| {
            let u = -HALF[0] + ((index % CELLS[0]) as f32 + 0.5) * cell[0];
            let v = -HALF[1] + ((index / CELLS[0]) as f32 + 0.5) * cell[1];
            let power = |group: &Group| {
                (u - group.centre[0]).powi(2) + (v - group.centre[1]).powi(2) - group.radius.powi(2)
            };
            groups
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| power(a).total_cmp(&power(b)))
                .map_or(0, |(index, _)| index)
        })
        .collect();
    let at = |i: usize, j: usize| owner[j * CELLS[0] + i];
    let mut runs = Vec::new();
    let mut push = |from: [f32; 2], to: [f32; 2]| {
        let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
        runs.push(Run {
            from,
            to,
            ground: dome.ground(middle),
        });
    };
    for i in 1..CELLS[0] {
        let x = -HALF[0] + i as f32 * cell[0];
        let mut start = None;
        for j in 0..=CELLS[1] {
            let edge = j < CELLS[1] && at(i - 1, j) != at(i, j);
            match (edge, start) {
                (true, None) => start = Some(j),
                (false, Some(first)) => {
                    push(
                        [x, -HALF[1] + first as f32 * cell[1]],
                        [x, -HALF[1] + j as f32 * cell[1]],
                    );
                    start = None;
                }
                _ => {}
            }
        }
    }
    for j in 1..CELLS[1] {
        let y = -HALF[1] + j as f32 * cell[1];
        let mut start = None;
        for i in 0..=CELLS[0] {
            let edge = i < CELLS[0] && at(i, j - 1) != at(i, j);
            match (edge, start) {
                (true, None) => start = Some(i),
                (false, Some(first)) => {
                    push(
                        [-HALF[0] + first as f32 * cell[0], y],
                        [-HALF[0] + i as f32 * cell[0], y],
                    );
                    start = None;
                }
                _ => {}
            }
        }
    }
    runs
}

/// A hash of a lattice point to [-1, 1).
fn lattice(x: i32, y: i32, seed: u64) -> f32 {
    let mut z = seed ^ (u64::from(x as u32) << 32) ^ u64::from(y as u32);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 40) as f32 / 8_388_608.0 - 1.0
}

/// Smooth value noise in [-1, 1).
fn value_noise(x: f32, y: f32, seed: u64) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (tx, ty) = (x - fx, y - fy);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (ix, iy) = (fx as i32, fy as i32);
    let row = |j: i32| lerp(lattice(ix, j, seed), lattice(ix + 1, j, seed), sx);
    lerp(row(iy), row(iy + 1), sy)
}

/// Layered value noise over `(spacing, amplitude)` octaves, scaled to about [-1, 1].
fn fbm(u: f32, v: f32, seed: u64, octaves: &[(f32, f32)]) -> f32 {
    let total: f32 = octaves.iter().map(|&(_, amplitude)| amplitude).sum();
    octaves
        .iter()
        .enumerate()
        .map(|(layer, &(spacing, amplitude))| {
            amplitude * value_noise(u / spacing, v / spacing, seed + layer as u64 * 0x9e37)
        })
        .sum::<f32>()
        / total
        * 1.7
}

/// The line the Milky Way follows across the window: its v at a given u.
fn band_line(u: f32) -> f32 {
    0.62 * u - 190.0 + 0.000_28 * u * u
}

/// Where along the band, and how far across it, the dust of a kernel thread sits, in sky units.
fn dust_place(id: Identity) -> [f32; 2] {
    let mut rng = Rng::for_identity(id, DUST_SALT);
    let u = (rng.unit() * 2.0 - 1.0) * 0.95 * HALF[0];
    let slope = 0.62 + 0.000_56 * u;
    let across = rng.normal() * 30.0;
    let along = 1.0 / (1.0 + slope * slope).sqrt();
    [u - across * slope * along, band_line(u) + across * along]
}

/// The sky behind everything: a plum glow in the window, and the Milky Way's shape, both on the
/// nodes of the dome's field. Built once: it is a function of constants alone.
struct Dome {
    /// The colour of each node without the band.
    base: Vec<[f32; 3]>,
    /// What the band adds to each node at full strength.
    band: Vec<[f32; 3]>,
}

impl Dome {
    /// The sky unit position of node (i, j).
    fn node(i: usize, j: usize) -> [f32; 2] {
        let span = [2.0 * HALF[0] * FIELD_SPAN, 2.0 * HALF[1] * FIELD_SPAN];
        [
            -span[0] / 2.0 + i as f32 / (FIELD_NODES[0] - 1) as f32 * span[0],
            -span[1] / 2.0 + j as f32 / (FIELD_NODES[1] - 1) as f32 * span[1],
        ]
    }

    fn build() -> Self {
        let nodes = FIELD_NODES[0] * FIELD_NODES[1];
        let (mut base, mut band) = (Vec::with_capacity(nodes), Vec::with_capacity(nodes));
        for j in 0..FIELD_NODES[1] {
            for i in 0..FIELD_NODES[0] {
                let [u, v] = Self::node(i, j);
                base.push(Self::base_at(u, v));
                band.push(Self::band_at(u, v));
            }
        }
        Self { base, band }
    }

    /// The glow, the cove of light along the horizon and the vignette, then the dark rim below the
    /// dome's edge, which arches across the bottom of the window.
    fn base_at(u: f32, v: f32) -> [f32; 3] {
        let (across, down) = (
            (u + HALF[0]) / (2.0 * HALF[0]),
            (v + HALF[1]) / (2.0 * HALF[1]),
        );
        let glow = (-(((across - 0.18) / 0.38).powi(2) + ((down - 0.30) / 0.5).powi(2))).exp();
        let mut color = [0.0; 3];
        for k in 0..3 {
            color[k] = SKY_BASE[k] + glow * SKY_GLOW[k] * 0.6;
        }
        let centred = 2.0 * across - 1.0;
        let horizon = 2.0 * HALF[1] - 26.0 * (1.0 - centred * centred);
        let y = v + HALF[1];
        let cove = (-(horizon - y).max(0.0) / 70.0).exp();
        for (k, channel) in color.iter_mut().enumerate() {
            *channel += cove * [70.0, 42.0, 52.0][k] * 0.55;
        }
        let vignette = 1.0 - 0.32 * ((centred.hypot((down - 0.45) / 0.6)) - 0.55).max(0.0);
        for channel in &mut color {
            *channel *= vignette;
        }
        // The rim fades in over about five node rows: a shallow arc sampled on the field's grid
        // would otherwise read as a staircase.
        let rim = smoothstep(horizon - RIM_FADE.0, horizon + RIM_FADE.1, y);
        for (k, channel) in color.iter_mut().enumerate() {
            *channel = lerp(*channel, *channel * 0.15 + [6.0, 5.0, 12.0][k], rim);
        }
        color
    }

    /// The Milky Way at one point: a bright core with a dust lane, a wide faint halo, brighter
    /// toward the lower right, its width and brightness varied by noise.
    fn band_at(u: f32, v: f32) -> [f32; 3] {
        let slope = 0.62 + 0.000_56 * u;
        let across = (v - band_line(u)) / (1.0 + slope * slope).sqrt();
        let noise = fbm(
            u,
            v,
            11,
            &[(180.0, 1.0), (72.0, 0.7), (26.0, 0.4), (11.0, 0.18)],
        );
        let dust = fbm(u, v, 29, &[(80.0, 1.0), (32.0, 0.55), (12.0, 0.25)]);
        let width = 62.0 + 17.0 * noise;
        let core = (-(across / width).powi(2)).exp();
        let wide = (-(across / 160.0).powi(2)).exp() * 0.35;
        let bulge = 0.55 + 0.45 * (-((u - 250.0) / 300.0).powi(2)).exp();
        let lane = 1.0
            - 0.75
                * (-((across - 8.0 + 12.0 * dust) / 18.0).powi(2)).exp()
                * (0.6 + dust).clamp(0.0, 1.0);
        let strength = ((core * (0.75 + 0.5 * noise) * lane + wide) * bulge).max(0.0);
        let edge = core.clamp(0.0, 1.0);
        std::array::from_fn(|k| strength * (BAND_CORE[k] * edge + BAND_EDGE[k] * (1.0 - edge)))
    }

    /// The shared dome.
    fn shared() -> &'static Dome {
        static DOME: OnceLock<Dome> = OnceLock::new();
        DOME.get_or_init(Dome::build)
    }

    /// The sky colour at a sky unit position, without the band: what a line drawn there blends
    /// toward.
    fn ground(&self, at: [f32; 2]) -> Color {
        let span = [2.0 * HALF[0] * FIELD_SPAN, 2.0 * HALF[1] * FIELD_SPAN];
        let i = (((at[0] + span[0] / 2.0) / span[0]) * (FIELD_NODES[0] - 1) as f32).round();
        let j = (((at[1] + span[1] / 2.0) / span[1]) * (FIELD_NODES[1] - 1) as f32).round();
        let i = (i.max(0.0) as usize).min(FIELD_NODES[0] - 1);
        let j = (j.max(0.0) as usize).min(FIELD_NODES[1] - 1);
        self.base[j * FIELD_NODES[0] + i].map(|channel| channel.clamp(0.0, 255.0) as u8)
    }

    /// The colours of the dome's nodes with the band at `strength`, 0 to 1.
    fn colors(&self, strength: f32) -> Vec<Color> {
        self.base
            .iter()
            .zip(&self.band)
            .map(|(base, band)| {
                std::array::from_fn(|k| (base[k] + band[k] * strength).clamp(0.0, 255.0) as u8)
            })
            .collect()
    }
}

/// How bright the Milky Way is for `count` kernel threads, 0 to 1: `1 - exp(-count / 40)`.
pub fn band_strength(count: usize) -> f32 {
    1.0 - (-(count as f32) / BAND_COUNT).exp()
}

/// What the view keeps about each star between samples.
#[derive(Clone, Debug)]
struct Record {
    /// The constellation it was seated in, and its index among the layout's groups.
    key: String,
    group: usize,
    /// Its seat within the constellation, in units of need from the centre.
    offset: [f32; 2],
    /// Its place on the dome, in sky units.
    sky: [f32; 2],
    /// Its last reading while it was alive: a zombie holds nothing.
    cpu: f32,
    memory: u64,
    /// Whether the newest sample has it, alive.
    present: bool,
    /// The `elapsed` of the first sample that had it, when the view had an earlier sample to
    /// compare with: a birth, which is a nova.
    born: Option<f64>,
    /// The `elapsed` of the first sample that lacked it or had it a zombie: an exit, which is a
    /// supernova above one GiB and a fade below.
    left: Option<f64>,
    /// How long it waits to take off for the diagram, and how its path bends, from its identity.
    lag: f32,
    bend: f32,
    /// The three sines of its twinkle: frequency in hertz and phase.
    waves: [(f32, f32); 3],
}

impl Record {
    fn new(id: Identity, born: Option<f64>) -> Self {
        let mut rng = Rng::for_identity(id, FLIGHT_SALT);
        let (lag, bend) = (rng.unit() * MAX_LAG, (rng.unit() - 0.5) * 2.0 * MAX_BEND);
        let mut rng = Rng::for_identity(id, TWINKLE_SALT);
        let waves = std::array::from_fn(|_| (0.9 + 2.6 * rng.unit(), TAU * rng.unit()));
        Self {
            key: String::new(),
            group: 0,
            offset: [0.0; 2],
            sky: [0.0; 2],
            cpu: 0.0,
            memory: 0,
            present: false,
            born,
            left: None,
            lag,
            bend,
            waves,
        }
    }

    /// The factor a star's brightness is multiplied by at `time`: three sines at 0.9 to 3.5 Hz,
    /// swinging by `amplitude`.
    fn twinkle(&self, amplitude: f32, time: f32) -> f32 {
        let wave: f32 = self
            .waves
            .iter()
            .map(|&(frequency, phase)| (TAU * frequency * time + phase).sin())
            .sum();
        (1.0 + amplitude * wave / 1.7).max(0.12)
    }
}

/// The state of the flight between the sky and the diagram.
#[derive(Clone, Copy, Debug, Default)]
struct Mode {
    /// 0 for the sky, 1 for the diagram: where the flight is going.
    goal: f32,
    /// Where it started from, and when on the scene's clock; None while it has never flown.
    from: f32,
    since: Option<f32>,
}

impl Mode {
    /// The flight's progress at `time`: 0 sky, 1 diagram, linear in time over 1.5 s for a whole
    /// crossing.
    fn progress(&self, time: f32) -> f32 {
        let Some(since) = self.since else {
            return self.goal;
        };
        let length = (self.goal - self.from).abs() * TRANSITION_SECONDS;
        if length <= 0.0 {
            return self.goal;
        }
        lerp(
            self.from,
            self.goal,
            ((time - since) / length).clamp(0.0, 1.0),
        )
    }

    /// Turns around to the other mode at `time`, from wherever the flight is.
    fn toggle(&mut self, time: f32) {
        self.from = self.progress(time);
        self.goal = 1.0 - self.goal;
        self.since = Some(time);
    }
}

/// Everything the view works out once per sample from the shared history.
#[derive(Default)]
struct Sampled {
    /// The absolute number of the newest sample it was worked out for.
    sequence: Option<u64>,
    /// Each star's coefficient of variation of CPU over the last ten seconds.
    variation: HashMap<Identity, f32>,
}

/// The view: a Hertzsprung-Russell sky of the machine.
#[derive(Default)]
pub struct Hr {
    mode: Mode,
    /// `m` was pressed since the last frame.
    toggle: bool,
    /// The `elapsed` of the sample the layout and the fit were last worked out for.
    observed: Option<f64>,
    records: BTreeMap<Identity, Record>,
    /// The constellations, in key order, and the layout they came from.
    groups: Vec<Group>,
    discs: Discs,
    signature: Vec<(String, [f32; 2], f32)>,
    /// The sky units a unit of need is drawn as, over `UNIT`: 1, or less where the layout was
    /// shrunk to fit the window. 0 until the first layout.
    scale: f32,
    runs: Vec<Run>,
    fit: Option<Fit>,
    kernel_threads: usize,
    sampled: Sampled,
    /// The axis, tick and region labels of the diagram, in sky units, worked out with the fit.
    diagram_labels: Vec<([f32; 2], String)>,
    /// Where each constellation's name stands in its disc, as a fraction of its radius from its
    /// centre, kept while it stays clear.
    names: HashMap<String, [f32; 2]>,
    /// The dome's colour field and what it was last built for: the kernel thread count and the
    /// sky's opacity in 255ths.
    field: Option<Field>,
    built: Option<(usize, u32)>,
    /// How far the camera has panned the sky, in sky units: the sky point at the middle of the
    /// screen. It is the sum of the camera centre's moves, each taken as the screen offset it made
    /// at the rotation and tilt of that frame, so turning or tilting the camera leaves the dome
    /// on its pixels whatever the centre.
    pan: [f32; 2],
    /// The camera centre and the scene's frame count at the last draw.
    followed: Option<([f32; 2], u64)>,
}

/// The sky-plane to world and screen mapping of one frame.
struct Plot<'a> {
    camera: &'a Camera,
    /// The sky point at the middle of the screen.
    pan: [f32; 2],
    /// Pixel sizes are scaled by the frame's height over the prototype's 720.
    scale: f32,
    /// How far a flat thing is lifted toward the viewer so it is drawn over the dome, in the
    /// view's depth.
    lift: f32,
}

impl Plot<'_> {
    /// The ground point whose screen offset from the middle of the screen is the sky point `at`
    /// less the pan, so the dome faces the viewer whatever the rotation and tilt.
    fn world(&self, at: [f32; 2]) -> Point {
        let zoom = self.camera.zoom;
        let [x, y] = self
            .camera
            .ground((at[0] - self.pan[0]) * zoom, (at[1] - self.pan[1]) * zoom);
        [self.camera.center[0] + x, self.camera.center[1] + y, 0.0]
    }

    /// The frame position of a sky point, lifted over the dome.
    fn screen(&self, frame: &Frame, at: [f32; 2]) -> Point {
        let mut point = frame.project(self.camera, self.world(at));
        point[2] += self.lift;
        point
    }

    /// A stroke between two screen points, at least a pixel wide.
    fn stroke(&self, frame: &mut Frame, from: Point, to: Point, color: Color, width: f32) {
        frame.hairline_between(from, to, color, width.max(1.0));
    }

    /// A dashed stroke between two screen points: `on` pixels drawn, `off` skipped.
    fn dashed(
        &self,
        frame: &mut Frame,
        from: Point,
        to: Point,
        dash: (f32, f32),
        color: Color,
        width: f32,
    ) {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let length = dx.hypot(dy);
        if length < 1e-3 {
            return;
        }
        let (on, off) = (dash.0.max(2.0), dash.1.max(2.0));
        let at = |along: f32| {
            let t = along / length;
            [
                from[0] + dx * t,
                from[1] + dy * t,
                from[2] + (to[2] - from[2]) * t,
            ]
        };
        let mut start = 0.0;
        while start < length {
            let end = (start + on).min(length);
            self.stroke(frame, at(start), at(end), color, width);
            start = end + off;
        }
    }
}

/// One star of a frame.
struct Star<'a> {
    process: &'a Process,
    /// Its index in the process slice: its pick.
    pick: u32,
    /// The magnitude it has before the flashes and the twinkle.
    magnitude: f32,
    /// The variation of its CPU over the last ten seconds.
    variation: f32,
    /// Its temperature and magnitude as the newest sample read them, which the main-sequence fit
    /// was made from: the class and residual are worked out from these, so they change with the
    /// samples and not with the easing between them.
    sampled: (f32, f32),
    /// Its place in sky units, in the world and in the frame.
    at: [f32; 2],
    world: Point,
    screen: Point,
    color: Color,
    /// The magnitude it is drawn at: its own, less the twinkle's and a nova's brightening.
    shown: f32,
    /// The age of its nova, while it has one.
    nova: Option<f32>,
}

impl Hr {
    /// Adds what the camera's centre moved since the last frame to the pan. After a gap (another
    /// view was shown) the sky starts again at the middle of the screen.
    fn follow(&mut self, camera: &Camera, frames: u64) {
        let continued = self
            .followed
            .is_some_and(|(_, before)| frames == before + 1 || frames == before);
        if let (Some((centre, _)), true) = (self.followed, continued) {
            let moved =
                camera.ground_offset([camera.center[0] - centre[0], camera.center[1] - centre[1]]);
            self.pan[0] += moved[0];
            self.pan[1] += moved[1];
        } else {
            self.pan = [0.0; 2];
        }
        self.followed = Some((camera.center, frames));
    }

    /// Brings the layout, the records of the stars and the main sequence up to `snapshot`, once per
    /// sample.
    fn observe(&mut self, snapshot: &Snapshot) {
        let until = snapshot.elapsed;
        if self.observed == Some(until) {
            return;
        }
        // Births and exits are what changed since the sample before, which only a step forward in
        // time has; going back (a rewind) forgets the events.
        let watching = self.observed.is_some_and(|before| until > before);
        self.observed = Some(until);
        self.kernel_threads = snapshot
            .processes
            .iter()
            .filter(|process| process.kind == Kind::Kernel)
            .count();
        self.fit = main_sequence(snapshot);
        // A zombie has died: it holds nothing, and its star is gone.
        let mut stars: Vec<&Process> = snapshot
            .processes
            .iter()
            .filter(|process| process.kind != Kind::Kernel && process.state != 'Z')
            .collect();
        stars.sort_by_key(|process| process.id);

        let mut members: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
        for process in &stars {
            members
                .entry(constellation(process))
                .or_insert((0, process.group.as_str()))
                .0 += 1;
        }
        let needs: Vec<(String, f32)> = members
            .iter()
            .map(|(key, &(count, _))| ((*key).to_owned(), ((capacity(count) + 3) as f32).sqrt()))
            .collect();
        let centres = self.discs.arrange(&needs);
        let placed: Vec<(String, String, [f32; 2], f32)> = members
            .iter()
            .map(|(key, &(_, label))| {
                (
                    (*key).to_owned(),
                    label.to_owned(),
                    centres[*key],
                    self.discs.reserved(key),
                )
            })
            .collect();
        self.place(&placed);

        // Seat every star that has no seat in its constellation yet, newcomers in identity order,
        // each against the stars already seated there.
        let mut seated: HashMap<&str, Vec<[f32; 2]>> = HashMap::new();
        for process in &stars {
            if let Some(record) = self.records.get(&process.id)
                && record.key == constellation(process)
            {
                seated
                    .entry(constellation(process))
                    .or_default()
                    .push(record.offset);
            }
        }
        let scale = self.scale;
        for process in &stars {
            let key = constellation(process);
            let record = self
                .records
                .entry(process.id)
                .or_insert_with(|| Record::new(process.id, watching.then_some(until)));
            if record.key != key {
                let group = &self.groups[group_index(&self.groups, key)];
                let neighbours = seated.entry(key).or_default();
                record.offset = seat(process.id, group.radius / (UNIT * scale), neighbours);
                neighbours.push(record.offset);
                record.key = key.to_owned();
            }
        }
        for process in &stars {
            let key = constellation(process);
            let index = group_index(&self.groups, key);
            let centre = self.groups[index].centre;
            let record = self
                .records
                .get_mut(&process.id)
                .expect("every star has a record");
            record.group = index;
            record.sky = [
                centre[0] + record.offset[0] * UNIT * scale,
                centre[1] + record.offset[1] * UNIT * scale,
            ];
            record.cpu = process.cpu;
            record.memory = process.memory;
            record.present = true;
        }
        let now: HashSet<Identity> = stars.iter().map(|process| process.id).collect();
        for (id, record) in &mut self.records {
            if record.present && !now.contains(id) {
                record.present = false;
                record.left = watching.then_some(until);
            }
        }
        self.records.retain(|_, record| {
            record.present || record.left.is_some_and(|left| until - left <= RECORD_KEEP)
        });
        self.place_names();
        self.diagram_labels = diagram_labels(self.fit);
    }

    /// Finds each constellation's name its spot: the point of its disc farthest from its stars,
    /// the text being wider than tall. A name keeps its spot, as a fraction of the disc's
    /// radius, while that spot is nearly as clear as the best one, so a name does not hop
    /// between samples.
    fn place_names(&mut self) {
        let mut members: Vec<Vec<[f32; 2]>> = vec![Vec::new(); self.groups.len()];
        for record in self.records.values().filter(|record| record.present) {
            if let Some(stars) = members.get_mut(record.group) {
                stars.push(record.sky);
            }
        }
        for (group, stars) in self.groups.iter_mut().zip(&members) {
            let clearance = |at: [f32; 2]| {
                stars
                    .iter()
                    .map(|star| ((at[0] - star[0]) * NAME_SQUASH).hypot(at[1] - star[1]))
                    .fold(f32::INFINITY, f32::min)
            };
            let at = |ring: f32, angle: f32| {
                [
                    group.centre[0] + group.radius * ring * angle.cos(),
                    group.centre[1] + group.radius * ring * angle.sin(),
                ]
            };
            let mut best = (clearance(group.centre), group.centre);
            for ring in NAME_RINGS {
                for step in 0..NAME_ANGLES {
                    let point = at(ring, TAU * (step as f32 + 0.5 * ring) / NAME_ANGLES as f32);
                    let score = clearance(point);
                    if score > best.0 {
                        best = (score, point);
                    }
                }
            }
            if let Some(&offset) = self.names.get(&group.key) {
                let old = [
                    group.centre[0] + offset[0] * group.radius,
                    group.centre[1] + offset[1] * group.radius,
                ];
                if clearance(old) >= NAME_KEEP * best.0 {
                    best.1 = old;
                }
            }
            group.name_at = best.1;
            self.names.insert(
                group.key.clone(),
                [
                    (best.1[0] - group.centre[0]) / group.radius.max(1e-3),
                    (best.1[1] - group.centre[1]) / group.radius.max(1e-3),
                ],
            );
        }
        let groups = &self.groups;
        self.names
            .retain(|key, _| groups.iter().any(|group| group.key == *key));
    }

    /// Takes in where the discs now are: the groups in sky units and, when any disc moved, grew
    /// or arrived, or the layout's scale changed, the boundaries between them.
    fn place(&mut self, placed: &[(String, String, [f32; 2], f32)]) {
        let signature: Vec<(String, [f32; 2], f32)> = placed
            .iter()
            .map(|(key, _, centre, radius)| (key.clone(), *centre, *radius))
            .collect();
        let discs: Vec<([f32; 2], f32)> = signature
            .iter()
            .map(|(_, centre, radius)| (*centre, *radius))
            .collect();
        let ideal = layout_scale(&discs);
        let before = self.scale;
        self.scale = if before <= 0.0 || ideal < before || ideal > before * REGROW {
            ideal
        } else {
            before
        };
        let changed = signature != self.signature || self.scale != before;
        self.signature = signature;
        let scale = self.scale;
        self.groups = placed
            .iter()
            .map(|(key, label, centre, radius)| {
                let centre = [centre[0] * UNIT * scale * STRETCH, centre[1] * UNIT * scale];
                Group {
                    key: key.clone(),
                    label: label.clone(),
                    centre,
                    radius: radius * UNIT * scale,
                    name_at: centre,
                }
            })
            .collect();
        if changed {
            self.runs = boundaries(&self.groups, Dome::shared());
        }
    }

    /// Brings what the history gives, once per sample: each star's CPU variation over the last
    /// ten seconds.
    fn observe_history(&mut self, history: &History, snapshot: &Snapshot) {
        let until = snapshot.elapsed;
        let Some(newest) = history
            .recent(until, 1)
            .last()
            .map(|sample| sample.sequence)
        else {
            self.sampled = Sampled::default();
            return;
        };
        if self.sampled.sequence == Some(newest) {
            return;
        }
        let window: Vec<_> = history.window(until, TWINKLE_SECONDS).collect();
        let mut variations = HashMap::new();
        for process in &snapshot.processes {
            if process.kind == Kind::Kernel {
                continue;
            }
            let readings: Vec<f32> = window
                .iter()
                .filter_map(|sample| sample.cpu(process.id))
                .collect();
            variations.insert(process.id, variation(&readings));
        }
        self.sampled = Sampled {
            sequence: Some(newest),
            variation: variations,
        };
    }

    /// The dome's colour field, rebuilt when the band's strength or the sky's opacity changed.
    fn dome_field(&mut self, sky_opacity: f32) -> &Field {
        let key = (self.kernel_threads, (sky_opacity * 255.0).round() as u32);
        let field = self
            .field
            .get_or_insert_with(|| Field::new(FIELD_KEY, FIELD_NODES[0], FIELD_NODES[1]));
        if self.built != Some(key) {
            let strength = band_strength(self.kernel_threads) * BAND_GAIN * sky_opacity;
            field.colors = Dome::shared().colors(strength);
            field.revision += 1;
            self.built = Some(key);
        }
        field
    }
}

/// The index of the group called `key` among `groups`, which are in key order and hold it.
fn group_index(groups: &[Group], key: &str) -> usize {
    groups
        .binary_search_by(|group| group.key.as_str().cmp(key))
        .expect("every star's constellation is placed")
}

/// The age of a star's nova: seconds since its birth, if it was born within the flash's window and
/// not after `until`. `since` is how long ago the shown sample arrived.
fn nova_age(record: &Record, until: f64, since: f32) -> Option<f32> {
    record
        .born
        .filter(|&born| born <= until)
        .map(|born| (until - born) as f32 + since)
}

/// A size of memory for an axis: `40 GiB`, `162 MiB` or `4 KiB`, whole numbers.
fn memory_text(bytes: f64) -> String {
    const KIB: f64 = 1024.0;
    if bytes >= KIB.powi(3) {
        format!("{:.0} GiB", bytes / KIB.powi(3))
    } else if bytes >= KIB.powi(2) {
        format!("{:.0} MiB", bytes / KIB.powi(2))
    } else {
        format!("{:.0} KiB", bytes / KIB)
    }
}

/// The CPU, in percent of one core, that makes a process `kelvin` hot: `temperature` inverted.
fn cpu_for(kelvin: f32) -> f32 {
    5.0 * ((kelvin / 2400.0).powf(1.0 / 0.47) - 1.0)
}

/// The text of the diagram, each label at a point in sky units: the title, the spectral classes
/// along the top, the temperature ticks with the CPU that makes them, the magnitude ticks with the
/// memory they stand for, the axes' names and the regions of the diagram, which follow the fit.
fn diagram_labels(fit: Option<Fit>) -> Vec<([f32; 2], String)> {
    let [left, right, top, bottom] = DIAGRAM;
    let mut labels = vec![
        (
            [left + 200.0, top - 52.0],
            "Hertzsprung-Russell diagram of this machine".to_owned(),
        ),
        ([left - 16.0, top - 26.0], "m".to_owned()),
        ([right + 44.0, top - 26.0], "memory".to_owned()),
        (
            [(left + right) / 2.0, bottom + 46.0],
            "surface temperature (hot on the left)".to_owned(),
        ),
    ];
    for (letter, low, high) in CLASSES {
        let (low, high) = (
            low.max(10.0_f32.powf(LOG_COLD)),
            high.min(10.0_f32.powf(LOG_HOT)),
        );
        let x = (diagram_x(low.log10()) + diagram_x(high.log10())) / 2.0;
        labels.push(([x, top - 26.0], letter.to_string()));
    }
    for kelvin in [
        40_000.0_f32,
        20_000.0,
        10_000.0,
        7_000.0,
        5_000.0,
        3_500.0,
        2_400.0,
    ] {
        let x = diagram_x(kelvin.log10());
        let cpu = cpu_for(kelvin);
        labels.push(([x, bottom + 16.0], format!("{kelvin:.0} K")));
        labels.push((
            [x, bottom + 32.0],
            if cpu >= 10.0 {
                format!("{cpu:.0}% cpu")
            } else {
                format!("{cpu:.1}% cpu")
            },
        ));
    }
    let mut magnitude = MAGNITUDE_TOP;
    while magnitude <= MAGNITUDE_BOTTOM - 1.0 {
        let y = diagram_y(magnitude);
        labels.push((
            [left - 16.0, y],
            if magnitude == 0.0 {
                "0".to_owned()
            } else {
                format!("{magnitude:+.0}")
            },
        ));
        let memory = 10.0_f64.powf(-f64::from(magnitude) / 2.5) * (1u64 << 30) as f64;
        labels.push(([right + 44.0, y], memory_text(memory)));
        magnitude += 2.0;
    }
    if let Some(fit) = fit {
        for (text, log_temperature, offset) in REGIONS {
            let y = fit.at(log_temperature) + offset;
            if (MAGNITUDE_TOP + 0.4..MAGNITUDE_BOTTOM - 0.4).contains(&y) {
                labels.push(([diagram_x(log_temperature), diagram_y(y)], text.to_owned()));
            }
        }
    }
    labels
}

/// The regions of the diagram named on it: where, as log10 of a temperature and a magnitude
/// from the fitted main sequence at that temperature.
const REGIONS: [(&str, f32, f32); 4] = [
    ("main sequence", 4.10, 1.25),
    ("giants", 3.66, -3.2),
    ("supergiants", 3.50, -6.6),
    ("white dwarfs", 4.20, 5.8),
];

/// The sky colour under a segment from `a` to `b`, which a stroke drawn there blends toward.
fn ground_between(a: [f32; 2], b: [f32; 2]) -> Color {
    Dome::shared().ground([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])
}

impl Hr {
    /// Every star of the frame, in the order of `processes`: each process that is not a kernel
    /// thread, at its place for flight progress `progress`, with its temperature, magnitude and
    /// colour, and the magnitude it is drawn at once the twinkle and a nova's brightening are
    /// in. `since` is how long ago the shown sample arrived, in seconds of the animation clock.
    fn stars_of<'a>(
        &self,
        stage: &Stage,
        plot: &Plot,
        processes: &[&'a Process],
        progress: f32,
        since: f32,
        until: f64,
    ) -> Vec<Star<'a>> {
        let gain = 1.0 - TWINKLE_FADE * progress;
        let mut stars = Vec::with_capacity(processes.len());
        for (pick, &process) in processes.iter().enumerate() {
            if process.kind == Kind::Kernel {
                continue;
            }
            let Some(record) = self
                .records
                .get(&process.id)
                .filter(|record| record.present)
            else {
                continue;
            };
            let kelvin = temperature(process.cpu);
            let magnitude = star_magnitude(process.memory);
            let at = flight(
                record.sky,
                diagram_position(kelvin, magnitude),
                record.lag,
                record.bend,
                progress,
            );
            let variation = self
                .sampled
                .variation
                .get(&process.id)
                .copied()
                .unwrap_or(0.0);
            let mut brightness = record.twinkle(twinkle_amplitude(variation) * gain, stage.time)
                * stage.growth(process.id);
            let mut shown = magnitude;
            let nova =
                nova_age(record, until, since).filter(|&age| flash(Flash::Nova, age).is_some());
            if let Some(age) = nova
                && let Some(state) = flash(Flash::Nova, age)
            {
                shown += state.magnitude_offset;
                brightness *= (NOVA_RAMP_FLOOR + age / NOVA_RAMP_SECONDS).min(1.0);
            }
            shown -= 2.5 * brightness.max(1e-3).log10();
            stars.push(Star {
                process,
                pick: pick as u32,
                magnitude,
                variation,
                sampled: (temperature(record.cpu), star_magnitude(record.memory)),
                at,
                world: plot.world(at),
                screen: plot.screen(stage.frame, at),
                color: blackbody::color(kelvin, 1.0),
                shown,
                nova,
            });
        }
        stars
    }

    /// The dashed boundaries between constellations, fading with the sky.
    fn draw_boundaries(&self, frame: &mut Frame, plot: &Plot, sky_opacity: f32) {
        let dash = (5.0 * plot.scale, 4.0 * plot.scale);
        for run in &self.runs {
            let color = mix(run.ground, BOUNDARY, BOUNDARY_ALPHA * sky_opacity);
            let (from, to) = (plot.screen(frame, run.from), plot.screen(frame, run.to));
            plot.dashed(frame, from, to, dash, color, plot.scale);
        }
    }

    /// The axes, the class dividers and grid, the glow of the main sequence and its fitted line,
    /// dashed, all fading with `opacity`.
    fn draw_diagram(&self, frame: &mut Frame, plot: &Plot, opacity: f32) {
        let [left, right, top, bottom] = DIAGRAM;
        let line =
            |frame: &mut Frame, a: [f32; 2], b: [f32; 2], color: Color, alpha: f32, width: f32| {
                let color = mix(ground_between(a, b), color, alpha * opacity);
                let (a, b) = (plot.screen(frame, a), plot.screen(frame, b));
                plot.stroke(frame, a, b, color, width);
            };
        for &(_, low, _) in &CLASSES[..CLASSES.len() - 1] {
            let x = diagram_x(low.log10());
            line(frame, [x, top - 18.0], [x, bottom], GRID, 0.18, plot.scale);
        }
        let mut magnitude = MAGNITUDE_TOP;
        while magnitude <= MAGNITUDE_BOTTOM - 1.0 {
            let y = diagram_y(magnitude);
            line(frame, [left, y], [right, y], GRID, 0.07, plot.scale);
            magnitude += 2.0;
        }
        line(
            frame,
            [left, top - 18.0],
            [left, bottom],
            AXIS,
            0.55,
            1.2 * plot.scale,
        );
        line(
            frame,
            [left, bottom],
            [right, bottom],
            AXIS,
            0.55,
            1.2 * plot.scale,
        );
        for kelvin in [
            40_000.0_f32,
            20_000.0,
            10_000.0,
            7_000.0,
            5_000.0,
            3_500.0,
            2_400.0,
        ] {
            let x = diagram_x(kelvin.log10());
            line(frame, [x, bottom], [x, bottom + 5.0], AXIS, 0.8, plot.scale);
        }
        let Some((hot, cold)) = self.fit.and_then(sequence_span) else {
            return;
        };
        let fit = self.fit.expect("a span comes from a fit");
        let end = |log_temperature: f32| {
            [
                diagram_x(log_temperature),
                diagram_y(fit.at(log_temperature)),
            ]
        };
        let (from, to) = (end(hot), end(cold));
        for step in 0..SEQUENCE_PUFFS {
            let t = (step as f32 + 0.5) / SEQUENCE_PUFFS as f32;
            let centre = plot.screen(frame, [lerp(from[0], to[0], t), lerp(from[1], to[1], t)]);
            frame.items.push(Item::Puff {
                center: [centre[0], centre[1]],
                radius: SEQUENCE_RADIUS * plot.scale,
                color: SEQUENCE_GLOW,
                strength: SEQUENCE_STRENGTH * opacity,
            });
        }
        let color = mix(ground_between(from, to), SEQUENCE, 0.6 * opacity);
        let (a, b) = (plot.screen(frame, from), plot.screen(frame, to));
        plot.dashed(
            frame,
            a,
            b,
            (9.0 * plot.scale, 5.0 * plot.scale),
            color,
            1.3 * plot.scale,
        );
    }

    /// The stick figures: each socket link between two stars of one constellation as a hairline
    /// with a gap at each star, as bright as the traffic it carries, and the links between
    /// constellations as faint dashes. `Stage::links` says which are drawn.
    fn draw_figures(
        &self,
        stage: &mut Stage,
        plot: &Plot,
        stars: &[Star],
        snapshot: &Snapshot,
        sky_opacity: f32,
    ) {
        if stage.links == Links::Off {
            return;
        }
        let index: HashMap<Identity, usize> = stars
            .iter()
            .enumerate()
            .map(|(index, star)| (star.process.id, index))
            .collect();
        let measured = !snapshot.missing.contains(Source::SocketTraffic);
        for &(a, b, _) in &snapshot.links {
            let (Some(&i), Some(&j)) = (index.get(&a), index.get(&b)) else {
                continue;
            };
            let (first, second) = (&stars[i], &stars[j]);
            let hot = stage.highlight.contains(&a) || stage.highlight.contains(&b);
            if i == j || (stage.links == Links::Focused && !hot) {
                continue;
            }
            let (from, to) = (first.screen, second.screen);
            let length = (to[0] - from[0]).hypot(to[1] - from[1]);
            let gap = |star: &Star| {
                plot.scale * (GAP_BASE + GAP_PER_MAGNITUDE * (GAP_FROM - star.magnitude).max(0.0))
            };
            let (gap_from, gap_to) = (gap(first), gap(second));
            if length < gap_from + gap_to + 2.0 * plot.scale {
                continue;
            }
            let along = [(to[0] - from[0]) / length, (to[1] - from[1]) / length];
            let start = [
                from[0] + along[0] * gap_from,
                from[1] + along[1] * gap_from,
                from[2],
            ];
            let end = [to[0] - along[0] * gap_to, to[1] - along[1] * gap_to, to[2]];
            let traffic = measured
                .then(|| {
                    snapshot
                        .link_traffic
                        .get(&(a, b))
                        .or_else(|| snapshot.link_traffic.get(&(b, a)))
                })
                .flatten();
            let brightness = traffic.map_or(FIGURE_FLAT, |&rate| link_brightness(rate));
            let ground = ground_between(first.at, second.at);
            if constellation(first.process) == constellation(second.process) {
                let (color, width) = if hot {
                    (FIGURE_HOT, 1.8)
                } else {
                    (FIGURE, 1.15)
                };
                let color = mix(ground, color, FIGURE_ALPHA * brightness * sky_opacity);
                plot.stroke(stage.frame, start, end, color, width * plot.scale);
            } else if length < CROSS_REACH * plot.scale {
                let color = mix(ground, FIGURE, CROSS_ALPHA * brightness * sky_opacity);
                plot.dashed(
                    stage.frame,
                    start,
                    end,
                    (3.0 * plot.scale, 5.0 * plot.scale),
                    color,
                    plot.scale,
                );
            }
        }
    }

    /// The stars that went out in the last seconds: a fading star, or a supernova's flash and
    /// expanding shell, where the star stood in the mode and flight the frame is in.
    fn draw_exits(&self, stage: &mut Stage, plot: &Plot, progress: f32, since: f32, until: f64) {
        let height = stage.frame.height;
        for record in self.records.values().filter(|record| !record.present) {
            let Some(left) = record.left.filter(|&left| left <= until) else {
                continue;
            };
            let kind = exit_flash(record.memory);
            let age = (until - left) as f32 + since;
            let Some(state) = flash(kind, age) else {
                continue;
            };
            let kelvin = temperature(record.cpu);
            let magnitude = star_magnitude(record.memory);
            let at = flight(
                record.sky,
                diagram_position(kelvin, magnitude),
                record.lag,
                record.bend,
                progress,
            );
            let centre = plot.screen(stage.frame, at);
            if state.opacity > 0.0 {
                let shown = magnitude + state.magnitude_offset - 2.5 * state.opacity.log10();
                stage.frame.star(
                    plot.camera,
                    plot.world(at),
                    &spread(shown, height).puffed(),
                    blackbody::color(kelvin, 1.0),
                    NONE,
                    false,
                );
            }
            if kind == Flash::Supernova && age < SUPERNOVA_GLOW_SECONDS * 4.0 {
                let strength = (-age / SUPERNOVA_GLOW_SECONDS).exp();
                let grow = 1.0 - (-age / 0.15).exp();
                for (radius, color, gain) in [
                    ((16.0 + 26.0 * grow) * plot.scale, [215, 225, 255], 1.0),
                    (5.0 * plot.scale, [255; 3], 1.0),
                ] {
                    stage.frame.items.push(Item::Puff {
                        center: [centre[0], centre[1]],
                        radius,
                        color,
                        strength: (gain * 2.2 * strength).min(1.0),
                    });
                }
            }
            if let Some(shell) = state.shell {
                let radius = plot.scale * (8.0 + 95.0 * shell.radius);
                let color = mix(SHELL_BLUE, SHELL_RED, shell.warmth);
                stage.frame.items.push(Item::Puff {
                    center: [centre[0], centre[1]],
                    radius,
                    color,
                    strength: 0.2 * shell.alpha,
                });
                let ring = mix(ground_between(at, at), color, (1.1 * shell.alpha).min(1.0));
                let width = (2.0 + 2.5 * age.min(1.5)) * plot.scale;
                let point = |step: usize| {
                    let angle = TAU * step as f32 / SHELL_SEGMENTS as f32;
                    [
                        centre[0] + radius * angle.cos(),
                        centre[1] + radius * angle.sin(),
                        centre[2],
                    ]
                };
                for step in 0..SHELL_SEGMENTS {
                    plot.stroke(stage.frame, point(step), point(step + 1), ring, width);
                }
            }
        }
    }

    /// The dust of the Milky Way: one faint point for each kernel thread, along the band, as
    /// bright as it is whatever the thread does, which holds no memory. Each is a stand-in
    /// that cannot be picked, whose place is kept for search and the tour. How many were drawn.
    fn draw_dust(
        &self,
        stage: &mut Stage,
        plot: &Plot,
        processes: &[&Process],
        sky_opacity: f32,
    ) -> usize {
        let mut shown = 0;
        for &process in processes {
            if process.kind != Kind::Kernel {
                continue;
            }
            let at = dust_place(process.id);
            stage.positions.insert(process.id, plot.world(at));
            let centre = plot.screen(stage.frame, at);
            stage.frame.items.push(Item::Puff {
                center: [centre[0], centre[1]],
                radius: DUST_RADIUS * plot.scale + DUST_PAD,
                color: DUST,
                strength: DUST_STRENGTH * sky_opacity * stage.growth(process.id),
            });
            shown += 1;
        }
        shown
    }

    /// The stars themselves, faintest first so the brightest stand over their neighbours: a nova's
    /// burst, a wide bloom for the brightest, then the pickable core in its halo.
    fn draw_stars(&self, stage: &mut Stage, plot: &Plot, stars: &[Star]) {
        let height = stage.frame.height;
        let (selected, toured) = (stage.selected, stage.toured);
        let mut order: Vec<usize> = (0..stars.len()).collect();
        order.sort_by(|&a, &b| stars[b].shown.total_cmp(&stars[a].shown).then(a.cmp(&b)));
        for index in order {
            let star = &stars[index];
            let centre = [star.screen[0], star.screen[1]];
            if let Some(age) = star.nova
                && age < NOVA_BURST_SECONDS
            {
                stage.frame.items.push(Item::Puff {
                    center: centre,
                    radius: (9.0 + 20.0 * age.min(1.0)) * plot.scale,
                    color: NOVA_COLOR,
                    strength: (1.6 * (-age / 0.35).exp()).min(1.0),
                });
            }
            let point = spread(star.shown, height).puffed();
            let flux = 10.0_f32.powf(-0.4 * (star.shown - FLUX_REFERENCE));
            if flux > BLOOM_FLUX {
                let radius = (plot.scale * (10.0 + 4.0 * flux.powf(0.2)))
                    .min(BLOOM_RADIUS * height as f32 / 1080.0);
                stage.frame.items.push(Item::Puff {
                    center: centre,
                    radius,
                    color: star.color,
                    strength: (BLOOM_GAIN * flux.powf(0.3)).min(BLOOM_LIMIT),
                });
            }
            let chosen = selected == Some(star.process.id) || toured == Some(star.process.id);
            stage.frame.star(
                plot.camera,
                star.world,
                &point,
                star.color,
                star.pick,
                chosen,
            );
            if point.core_px < PICK_RADIUS {
                stage
                    .frame
                    .pick_disc(plot.camera, star.world, PICK_RADIUS, star.pick);
            }
        }
    }

    /// What the app reads back: where each star is, the labels of the six brightest, the names of
    /// the constellations or the diagram's text, the inspector lines of the stars that are shown
    /// one, and the stops of the tour.
    fn report(&self, stage: &mut Stage, plot: &Plot, stars: &[Star], progress: f32) {
        for star in stars {
            stage.positions.insert(star.process.id, star.world);
        }
        let mut order: Vec<usize> = (0..stars.len()).collect();
        order.sort_by(|&a, &b| {
            stars[a]
                .magnitude
                .total_cmp(&stars[b].magnitude)
                .then(stars[a].process.id.cmp(&stars[b].process.id))
        });
        if !(LABEL_FADE..=1.0 - LABEL_FADE).contains(&progress) {
            for &index in order.iter().take(LABELLED) {
                let star = &stars[index];
                stage.tags.push(Tag {
                    at: star.world,
                    side: Side::Right,
                    text: format!(
                        "{} {}",
                        label_name(&star.process.name),
                        classification(star.sampled.0, star.sampled.1, self.fit)
                    ),
                });
            }
        }
        stage
            .tour
            .extend(order.iter().take(TOURED).map(|&index| TourTarget {
                id: stars[index].process.id,
                position: stars[index].world,
                weight: MAGNITUDE_BOTTOM - stars[index].magnitude,
            }));
        if progress <= LABEL_FADE {
            for group in &self.groups {
                if !group.label.is_empty() {
                    stage.places.push((
                        plot.world(group.name_at),
                        label_name(&group.label).into_owned(),
                    ));
                }
            }
        } else if progress >= 1.0 - LABEL_FADE {
            stage.places.extend(
                self.diagram_labels
                    .iter()
                    .map(|(at, text)| (plot.world(*at), text.clone())),
            );
        }
        for star in stars {
            let id = star.process.id;
            let wanted = stage.selected == Some(id)
                || stage.toured == Some(id)
                || stage.highlight.contains(&id);
            if !wanted {
                continue;
            }
            let lines = self.inspector_lines(star);
            if stage.toured == Some(id) {
                stage.callout.push((id, lines[0].clone()));
            }
            stage.notes.insert(id, lines);
        }
    }

    /// The lines the inspector adds for a star: its class, temperature and magnitude, where it
    /// stands against the main sequence, and the variation its twinkle follows.
    fn inspector_lines(&self, star: &Star) -> Vec<String> {
        let (kelvin, magnitude) = star.sampled;
        let mut lines = vec![format!(
            "class {} | T {kelvin:.0} K | m {magnitude:+.2}",
            classification(kelvin, magnitude, self.fit),
        )];
        if let Some(fit) = self.fit {
            lines.push(format!(
                "main sequence m {:+.2} | residual {:+.2} mag",
                fit.at(kelvin.log10()),
                fit.residual(kelvin, magnitude)
            ));
        }
        lines.push(format!(
            "CPU variation (CV) {:.2} over {:.0} s | twinkle {:.0}%",
            star.variation,
            TWINKLE_SECONDS,
            100.0 * twinkle_amplitude(star.variation)
        ));
        lines
    }
}

/// How bright a stick figure is for a link carrying `rate` bytes a second: `0.35 + 0.65 *
/// min(1, log10(1 + rate) / 6)`, which is full at a megabyte a second.
pub fn link_brightness(rate: f32) -> f32 {
    0.35 + 0.65 * ((1.0 + rate.max(0.0)).log10() / 6.0).min(1.0)
}

/// The temperatures, as log10, between which the fitted main sequence is drawn: the part of the
/// line inside the diagram's window. None when the line never enters it.
fn sequence_span(fit: Fit) -> Option<(f32, f32)> {
    let (mut hot, mut cold) = (LOG_HOT, LOG_COLD);
    let (top, bottom) = (MAGNITUDE_TOP + 0.15, MAGNITUDE_BOTTOM);
    if fit.slope.abs() < 1e-6 {
        return (top..=bottom)
            .contains(&fit.intercept)
            .then_some((hot, cold));
    }
    let (a, b) = (
        (top - fit.intercept) / fit.slope,
        (bottom - fit.intercept) / fit.slope,
    );
    let (low, high) = (a.min(b), a.max(b));
    hot = hot.min(high);
    cold = cold.max(low);
    (hot > cold).then_some((hot, cold))
}

impl ViewModule for Hr {
    fn configure(&mut self, options: &ViewOptions) {
        if options.hr_diagram {
            self.mode = Mode {
                goal: 1.0,
                from: 1.0,
                since: None,
            };
        }
    }

    fn record(&mut self, snapshot: &Snapshot, _time: f32) {
        self.observe(snapshot);
    }

    fn key(&mut self, key: ViewKey) -> bool {
        if key == ViewKey::Local('m') {
            self.toggle = !self.toggle;
        }
        false
    }

    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        self.observe(snapshot);
        self.observe_history(stage.history, snapshot);
        let time = stage.time;
        if std::mem::take(&mut self.toggle) {
            self.mode.toggle(time);
        }
        let progress = self.mode.progress(time);
        let sky_opacity = 1.0 - smoothstep(0.0, 0.62, progress);
        let diagram_opacity = smoothstep(0.38, 1.0, progress);

        let camera = stage.camera;
        self.follow(camera, stage.frames);
        let frame_height = stage.frame.height;
        let plot = Plot {
            camera,
            pan: self.pan,
            scale: frame_height as f32 / REFERENCE_HEIGHT,
            lift: LINE_DEPTH_BIAS,
        };
        stage.bounds.extend(
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .map(|(x, y)| plot.world([x * HALF[0], y * HALF[1]])),
        );
        let span = [2.0 * HALF[0] * FIELD_SPAN, 2.0 * HALF[1] * FIELD_SPAN];
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, y)| plot.world([x * span[0] / 2.0, y * span[1] / 2.0]));
        let field = self.dome_field(sky_opacity);
        stage
            .frame
            .field(camera, Placement::Corners(corners), field, Relief::Flat);

        let until = snapshot.elapsed;
        let since = if stage.history.newest() == Some(until) {
            (time - stage.history.arrived()).clamp(0.0, 10.0)
        } else {
            0.0
        };
        let stars = self.stars_of(stage, &plot, processes, progress, since, until);

        if sky_opacity > 0.0 {
            self.draw_boundaries(stage.frame, &plot, sky_opacity);
        }
        if diagram_opacity > 0.0 {
            self.draw_diagram(stage.frame, &plot, diagram_opacity);
        }
        if sky_opacity > 0.0 {
            self.draw_figures(stage, &plot, &stars, snapshot, sky_opacity);
        }
        self.draw_exits(stage, &plot, progress, since, until);
        let shown_dust = if sky_opacity > 0.0 {
            self.draw_dust(stage, &plot, processes, sky_opacity)
        } else {
            0
        };
        self.draw_stars(stage, &plot, &stars);
        self.report(stage, &plot, &stars, progress);
        stars.len() + shown_dust
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let stars = snapshot
            .processes
            .iter()
            .filter(|process| process.kind != Kind::Kernel)
            .count();
        let diagram = self.mode.goal > 0.5;
        let mut text = format!(" {stars} stars");
        if diagram {
            if let Some(fit) = self.fit {
                text.push_str(&format!(
                    " | main sequence m = {:.1} {} {:.2} log T",
                    fit.intercept,
                    if fit.slope < 0.0 { '-' } else { '+' },
                    fit.slope.abs()
                ));
            } else {
                text.push_str(" | no main sequence yet");
            }
        } else {
            text.push_str(&format!(
                " | {} kernel threads in the Milky Way | {} constellations",
                self.kernel_threads,
                self.groups.len()
            ));
        }
        text.extend(missing_note(
            snapshot,
            Source::Cgroups,
            "constellations = app groups",
        ));
        text.extend(missing_note(
            snapshot,
            Source::SocketTraffic,
            "lines at one brightness",
        ));
        if diagram {
            text.push_str(
                " | x = temperature from CPU, hot on the left | y = magnitude from memory | class from T | luminosity class from the residual m - fit: Ia Ib II III IV V VI from bright to faint, D = a hot star 2.75 mag past the fit | m sky",
            );
        } else {
            text.push_str(
                " | colour = blackbody T = 2400 K x (1 + cpu/5)^0.47 | size, brightness = memory, m = -2.5 log10(mem / 1 GiB) | twinkle = CPU variation over 10 s | lines = socket links | dashed = cgroup boundaries | m diagram",
            );
        }
        text
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
impl Hr {
    /// Whether the view is in the diagram, or flying to it.
    pub fn in_diagram(&self) -> bool {
        self.mode.goal > 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DemoOptions, demo_with, testing::process};
    use crate::palette::tint;
    use crate::platform::Sources;
    use crate::render::{RenderInput, Scene};
    use crate::views::{View, view};

    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    const DEMO: DemoOptions = DemoOptions {
        churn: true,
        early_giant: true,
        ..DemoOptions::NONE
    };

    fn hr_view() -> View {
        view("hr")
    }

    fn snapshot(elapsed: f64, processes: Vec<Process>) -> Snapshot {
        Snapshot {
            elapsed,
            processes,
            ..Snapshot::default()
        }
    }

    /// A process at `cpu` percent holding `memory` bytes, in the constellation `cgroup`.
    fn star(pid: u32, cpu: f32, memory: u64, cgroup: &str) -> Process {
        let mut star = process(pid, 1);
        star.cpu = cpu;
        star.memory = memory;
        star.name = format!("star-{pid}");
        star.group = cgroup.to_owned();
        star.cgroup = format!("/{cgroup}");
        star
    }

    /// The memory of a star of magnitude `m`.
    fn memory_of(magnitude: f32) -> u64 {
        (GIB as f64 * 10.0_f64.powf(-0.4 * f64::from(magnitude))) as u64
    }

    /// The CPU, in percent, of a process at `kelvin`.
    fn cpu_at(kelvin: f32) -> f32 {
        cpu_for(kelvin)
    }

    /// A machine whose stars lie on a main sequence `m = 7.2 - 4.6667 (log10 T - 3.4)` from 2,500
    /// K to 20,000 K, with a giant, a supergiant and a white dwarf off it.
    fn population() -> Vec<Process> {
        let line = |kelvin: f32| 7.2 - 4.6667 * (kelvin.log10() - 3.4);
        let mut stars: Vec<Process> = (0..60)
            .map(|index| {
                let kelvin = 2500.0 * 8.0_f32.powf(index as f32 / 59.0);
                star(
                    100 + index,
                    cpu_at(kelvin),
                    memory_of(line(kelvin)),
                    "sequence",
                )
            })
            .collect();
        for (pid, kelvin, offset) in [
            (900, 4600.0, -2.5),
            (901, 3300.0, -5.5),
            (902, 12_000.0, 2.9),
        ] {
            stars.push(star(
                pid,
                cpu_at(kelvin),
                memory_of(line(kelvin) + offset),
                "off",
            ));
        }
        stars
    }

    /// Records `snapshot` and draws `time` of `view` of it, rasterized.
    fn draw(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> Frame {
        scene.record(snapshot, time);
        let mut frame = scene.render(&RenderInput::small(
            snapshot,
            hr_view(),
            &Camera::default(),
            time,
        ));
        frame.rasterize();
        frame
    }

    /// A scene that has seen `snapshot` at `elapsed` seconds and one before it, showing the sky.
    fn sky(processes: Vec<Process>) -> (Scene, Snapshot) {
        let mut scene = Scene::new();
        let before = snapshot(9.0, processes.clone());
        draw(&mut scene, &before, 9.0);
        let now = snapshot(10.0, processes);
        draw(&mut scene, &now, 10.0);
        (scene, now)
    }

    fn hr_state(scene: &Scene) -> &Hr {
        scene.module::<Hr>(hr_view()).expect("the hr module")
    }

    #[test]
    fn temperature_magnitude_and_spectral_class_follow_the_brief() {
        assert_eq!(temperature(0.0), 2400.0);
        assert!((temperature(100.0) - 10_038.16).abs() < 0.1, "one core");
        assert!((temperature(20.0) - 5113.6).abs() < 0.1);
        assert_eq!(
            temperature(2000.0),
            40_000.0,
            "clamped at twenty cores and over"
        );
        assert_eq!(temperature(-5.0), 2400.0, "no negative CPU");
        assert!(star_magnitude(GIB).abs() < 1e-6);
        assert!((star_magnitude(10 * MIB) - 5.0).abs() < 0.05);
        assert_eq!(
            star_magnitude(0),
            9.0,
            "no memory is the bottom of the diagram"
        );
        assert_eq!(star_magnitude(1 << 50), -6.0, "clamped at the top");
        assert_eq!(
            star_magnitude(MIB / 2),
            magnitude(MIB),
            "the floor of the scale"
        );
        // The classes the brief names for those temperatures.
        assert_eq!(spectral_class(temperature(0.0)), ('M', 9));
        assert_eq!(spectral_class(temperature(100.0)), ('B', 9));
        assert_eq!(spectral_class(temperature(2000.0)).0, 'O');
        // The subclass interpolates log T within the class, 0 at the hot edge.
        assert_eq!(spectral_class(4600.0), ('K', 3));
        assert_eq!(spectral_class(3699.0), ('M', 0));
        assert_eq!(spectral_class(3300.0), ('M', 2));
        // A steady star at 4,600 K and 41 MiB that sits two and a half magnitudes above a fitted
        // line through it is a giant: the label the inspector and the star's tag carry.
        let fit = Fit {
            intercept: 7.2 + 4.6667 * 3.4,
            slope: -4.6667,
        };
        let giant = fit.at(4600.0_f32.log10()) - 2.5;
        assert_eq!(classification(4600.0, giant, Some(fit)), "K3 III");
        assert_eq!(classification(4600.0, giant, None), "K3");
        assert_eq!(
            classification(12_000.0, fit.at(12_000.0_f32.log10()) + 3.0, Some(fit)),
            "DB"
        );
    }

    #[test]
    fn class_boundaries_fall_at_the_standard_temperatures() {
        for (above, below, edge) in [
            ('O', 'B', 30_000.0),
            ('B', 'A', 10_000.0),
            ('A', 'F', 7_500.0),
            ('F', 'G', 6_000.0),
            ('G', 'K', 5_200.0),
            ('K', 'M', 3_700.0),
        ] {
            assert_eq!(
                spectral_class(edge).0,
                above,
                "{edge} K is the first of {above}"
            );
            assert_eq!(
                spectral_class(edge - 1.0).0,
                below,
                "{} K is {below}",
                edge - 1.0
            );
            // The cool end of a class is subclass 9, the hot end of the next one 0.
            assert_eq!(spectral_class(edge), (above, 9));
            assert_eq!(spectral_class(edge - 1.0), (below, 0));
        }
        assert_eq!(spectral_class(2400.0).0, 'M');
        assert_eq!(spectral_class(100.0).0, 'M', "nothing is colder than M");
        for kelvin in (2400..40_000).step_by(37) {
            let (_, subclass) = spectral_class(kelvin as f32);
            assert!(subclass <= 9);
        }
    }

    #[test]
    fn luminosity_classes_follow_the_residual_from_a_robust_main_sequence() {
        // The thresholds the README documents, at a temperature below the dwarf cut.
        for (residual, class) in [
            (-7.0, Luminosity::Ia),
            (-6.01, Luminosity::Ia),
            (-6.0, Luminosity::Ib),
            (-4.51, Luminosity::Ib),
            (-4.5, Luminosity::II),
            (-3.26, Luminosity::II),
            (-3.25, Luminosity::III),
            (-1.76, Luminosity::III),
            (-1.75, Luminosity::IV),
            (-0.91, Luminosity::IV),
            (-0.9, Luminosity::V),
            (0.0, Luminosity::V),
            (1.49, Luminosity::V),
            (1.5, Luminosity::VI),
            (2.74, Luminosity::VI),
            (2.75, Luminosity::VI),
            (6.0, Luminosity::VI),
        ] {
            assert_eq!(
                luminosity_class(residual, 5000.0),
                class,
                "residual {residual}"
            );
        }
        // From 2.75 on a hot star is a white dwarf.
        assert_eq!(luminosity_class(2.75, 7500.0), Luminosity::D);
        assert_eq!(luminosity_class(2.74, 20_000.0), Luminosity::VI);
        assert_eq!(luminosity_class(2.75, 7499.0), Luminosity::VI);
        assert_eq!(luminosity_class(5.0, 30_000.0), Luminosity::D);

        // A synthetic machine with a known sequence, and three stars off it.
        let machine = snapshot(10.0, population());
        let fit = main_sequence(&machine).expect("sixty-three stars make a fit");
        assert!((fit.slope + 4.6667).abs() < 0.1, "slope {}", fit.slope);
        assert!(
            (fit.at(3.8) - (7.2 - 4.6667 * 0.4)).abs() < 0.1,
            "the line through the stars: {}",
            fit.at(3.8)
        );
        let label = |pid: u32| {
            let process = machine
                .processes
                .iter()
                .find(|process| process.id.pid == pid)
                .expect("the star");
            classification(
                temperature(process.cpu),
                star_magnitude(process.memory),
                Some(fit),
            )
        };
        assert_eq!(label(900), "K3 III", "the giant");
        assert!(label(901).starts_with("M2 Ib"), "{}", label(901));
        assert_eq!(label(902), "DB", "the white dwarf");
        for pid in [100, 130, 150] {
            assert!(label(pid).ends_with(" V"), "{pid}: {}", label(pid));
        }
        // Fewer than three points give no fit and so no luminosity class.
        let lonely = snapshot(10.0, vec![star(1, 5.0, GIB, "a"), star(2, 50.0, GIB, "a")]);
        assert_eq!(main_sequence(&lonely), None);
        assert_eq!(classification(5000.0, 1.0, None), "K1");
        // Processes below the 1 MiB floor all read the same faint magnitude whatever they hold,
        // so they stay out of the fit, as the processes with none do.
        assert!(!is_measured(1024) && !is_measured(MEMORY_FLOOR - 1) && is_measured(MEMORY_FLOOR));
        let mut crowded = population();
        crowded.extend(
            (0..40).map(|index| star(2000 + index, 2.0 * index as f32, 100 * 1024, "tiny")),
        );
        assert_eq!(
            main_sequence(&snapshot(10.0, crowded)),
            Some(fit),
            "forty processes under a mebibyte leave the line alone"
        );
    }

    #[test]
    fn theil_sen_ignores_outliers_and_tied_temperatures() {
        let line: Vec<(f32, f32)> = (0..30)
            .map(|index| {
                let x = 3.4 + index as f32 * 0.03;
                (x, 7.0 - 4.0 * (x - 3.4))
            })
            .collect();
        let fit = theil_sen(&line).expect("a line");
        assert!(
            (fit.slope + 4.0).abs() < 1e-3 && (fit.at(3.4) - 7.0).abs() < 1e-3,
            "{fit:?}"
        );
        // A fifth of the points anywhere do not move it much.
        let mut spoiled = line.clone();
        for (index, point) in spoiled.iter_mut().enumerate().step_by(5) {
            point.1 = if index % 10 == 0 { -50.0 } else { 80.0 };
        }
        let robust = theil_sen(&spoiled).expect("still a line");
        assert!((robust.slope + 4.0).abs() < 0.5, "{robust:?}");
        // A least-squares line through the same points is far off.
        let mean_x = spoiled.iter().map(|p| p.0).sum::<f32>() / 30.0;
        let mean_y = spoiled.iter().map(|p| p.1).sum::<f32>() / 30.0;
        let least_squares = spoiled
            .iter()
            .map(|p| (p.0 - mean_x) * (p.1 - mean_y))
            .sum::<f32>()
            / spoiled.iter().map(|p| (p.0 - mean_x).powi(2)).sum::<f32>();
        assert!((least_squares + 4.0).abs() > (robust.slope + 4.0).abs());
        // Stars at one temperature, the idle ones, give no slope and no division by zero.
        let tied: Vec<(f32, f32)> = (0..20).map(|index| (3.38, index as f32)).collect();
        assert_eq!(theil_sen(&tied), None);
        // Pairs closer than 0.01 in log T are ignored, the rest still give the line.
        let mut mixed = line.clone();
        mixed.extend((0..40).map(|index| (3.38 + index as f32 * 0.0001, index as f32 * 3.0)));
        let fit = theil_sen(&mixed).expect("the spread-out pairs");
        assert!(fit.slope.is_finite() && fit.intercept.is_finite());
        assert_eq!(theil_sen(&line[..2]), None, "two points are not enough");
        assert_eq!(theil_sen(&[]), None);
        // The fit takes every k-th star by identity above 400 stars, and all of them below.
        let big: Vec<Process> = (0..1000)
            .map(|index| {
                star(
                    100 + index,
                    (index % 90) as f32,
                    memory_of(2.0 + (index % 7) as f32),
                    "a",
                )
            })
            .collect();
        let fit = main_sequence(&snapshot(1.0, big)).expect("a fit");
        assert!(fit.slope.is_finite());
    }

    #[test]
    fn twinkle_is_zero_for_steady_cpu_and_grows_with_variation() {
        assert_eq!(variation(&[5.0; 10]), 0.0);
        assert_eq!(
            variation(&[0.0; 10]),
            0.0,
            "an idle process has no mean to vary around"
        );
        assert_eq!(variation(&[1.0, 9.0]), 0.0, "fewer than three samples");
        assert_eq!(variation(&[]), 0.0);
        let mild = variation(&[4.0, 6.0, 4.0, 6.0]);
        let wild = variation(&[0.0, 20.0, 0.0, 20.0]);
        assert!((mild - 0.2).abs() < 1e-5, "{mild}");
        assert!(wild > mild && (wild - 1.0).abs() < 1e-5, "{wild}");
        assert_eq!(twinkle_amplitude(0.0), 0.0);
        assert!((twinkle_amplitude(mild) - 0.09).abs() < 1e-5);
        assert_eq!(twinkle_amplitude(5.0), 0.8, "at most 0.8");
        // The sum of three sines never goes dark, and holds still when the amplitude is nothing.
        let record = Record::new(Identity { pid: 5, start: 5 }, None);
        for step in 0..200 {
            let time = step as f32 * 0.05;
            assert_eq!(record.twinkle(0.0, time), 1.0);
            assert!(record.twinkle(0.8, time) >= 0.12);
        }
        let swing = (0..200)
            .map(|step| record.twinkle(0.8, step as f32 * 0.05))
            .fold((f32::MAX, f32::MIN), |(low, high), value| {
                (low.min(value), high.max(value))
            });
        assert!(swing.1 - swing.0 > 0.4, "{swing:?}");
        // Through the scene: ten seconds of samples, one star steady and one erratic.
        let mut scene = Scene::new();
        for second in 0..=10 {
            let erratic = if second % 2 == 0 { 2.0 } else { 40.0 };
            let processes = vec![
                star(1, 10.0, GIB, "a"),
                star(2, erratic, GIB, "a"),
                star(3, 10.0, GIB, "a"),
            ];
            draw(
                &mut scene,
                &snapshot(f64::from(second), processes),
                second as f32,
            );
        }
        let seen = &hr_state(&scene).sampled.variation;
        let id = |pid: u32| Identity {
            pid,
            start: u64::from(pid),
        };
        assert_eq!(seen[&id(1)], 0.0);
        assert!(seen[&id(2)] > 0.5, "{}", seen[&id(2)]);
    }

    #[test]
    fn kernel_threads_join_the_milky_way_and_are_never_plotted() {
        let mut processes: Vec<Process> = (0..10)
            .map(|index| star(100 + index, 20.0, 400 * MIB, "sky"))
            .collect();
        let threads: Vec<Process> = (0..40)
            .map(|index| {
                let mut thread = process(2 + index, 2);
                thread.kind = Kind::Kernel;
                thread.memory = 0;
                thread.name = format!("kworker/{index}");
                thread.cgroup.clear();
                thread.group = "kernel".into();
                thread
            })
            .collect();
        let kernel_ids: Vec<Identity> = threads.iter().map(|thread| thread.id).collect();
        processes.extend(threads);
        let (mut scene, now) = sky(processes.clone());
        let hr = hr_state(&scene);
        assert_eq!(hr.kernel_threads, 40);
        assert_eq!(hr.records.len(), 10, "only the stars are seated");
        assert_eq!(hr.groups.len(), 1, "a constellation holds no kernel thread");
        // Search and the tour still land on a thread: it has a place, in the band.
        for id in &kernel_ids {
            assert!(scene.positions.contains_key(id), "{id:?} has no place");
        }
        assert_eq!(scene.visible, 10 + 40, "the dust counts as shown");
        // Not one of them is pickable, and none reaches the tour.
        let mut camera = Camera::default();
        scene.fit(&mut camera, 960, 540);
        let mut frame = scene.render(&RenderInput {
            width: 960,
            height: 540,
            ..RenderInput::small(&now, hr_view(), &camera, 10.5)
        });
        frame.rasterize();
        for &pick in frame.picks.iter().filter(|&&pick| pick != NONE) {
            assert!(!kernel_ids.contains(&frame.identities[pick as usize]));
        }
        assert!(
            scene
                .tour
                .iter()
                .all(|target| !kernel_ids.contains(&target.id))
        );
        assert!(scene.tags.iter().all(|tag| !tag.text.contains("kworker")));
        // And the dome's colours follow it: more threads, a brighter band.
        let (few, _) = sky(processes[..14].to_vec());
        let sum = |scene: &Scene| -> u64 {
            hr_state(scene)
                .field
                .as_ref()
                .expect("the dome is built")
                .colors
                .iter()
                .map(|color| color.iter().map(|&channel| u64::from(channel)).sum::<u64>())
                .sum()
        };
        assert!(
            sum(&scene) > sum(&few),
            "{} against {}",
            sum(&scene),
            sum(&few)
        );
        // On the diagram they are gone.
        scene.key(hr_view(), ViewKey::Local('m'));
        let mut later = now.clone();
        later.elapsed = 12.0;
        let mut frame = scene.render(&RenderInput::small(&later, hr_view(), &camera, 20.0));
        frame.rasterize();
        scene.render(&RenderInput::small(&later, hr_view(), &camera, 22.0));
        for id in &kernel_ids {
            assert!(
                !scene.positions.contains_key(id),
                "{id:?} is on the diagram"
            );
        }
        // The band's brightness is the count: 1 - exp(-count / 40).
        assert_eq!(band_strength(0), 0.0);
        assert!((band_strength(40) - 0.632_120_6).abs() < 1e-5);
        assert!(band_strength(400) > 0.9999);
        assert!(band_strength(10) < band_strength(20));
    }

    /// Where each star of `scene` is, after drawing `snapshot` at `time`.
    fn places(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> HashMap<Identity, Point> {
        draw(scene, snapshot, time);
        scene.positions.clone()
    }

    #[test]
    fn stars_keep_their_sky_places_when_cpu_memory_or_population_change() {
        let base = demo_with(12.0, 192, DEMO);
        let mut scene = Scene::new();
        for second in 0..=12 {
            scene.record(&demo_with(f64::from(second), 192, DEMO), second as f32);
        }
        let before = places(&mut scene, &base, 12.0);
        assert!(before.len() > 100);
        let mut later = crate::harness::rescaled(&base);
        later.elapsed = 13.0;
        let after = places(&mut scene, &later, 13.0);
        for (id, place) in &before {
            let moved = (after[id][0] - place[0]).hypot(after[id][1] - place[1]);
            assert!(
                moved < 1e-3,
                "{id:?} moved {moved} when CPU and memory changed"
            );
        }
        let (mut churned, gone) = crate::harness::churned(&base);
        churned.elapsed = 14.0;
        let after = places(&mut scene, &churned, 14.0);
        let mut checked = 0;
        for (id, place) in &before {
            if gone.contains(id) {
                continue;
            }
            let moved = (after[id][0] - place[0]).hypot(after[id][1] - place[1]);
            assert!(
                moved < 1e-3,
                "{id:?} moved {moved} when a tenth came and went"
            );
            checked += 1;
        }
        assert!(checked > 100);
    }

    #[test]
    fn diagram_position_depends_only_on_a_stars_own_cpu_and_memory() {
        // The pure function first: the same figures, the same place; hot to the left, bright up.
        assert_eq!(diagram_position(5000.0, 2.0), diagram_position(5000.0, 2.0));
        let [hot, top] = diagram_position(40_000.0, -6.0);
        let [cold, bottom] = diagram_position(2400.0, 9.0);
        assert!(hot < cold && top < bottom);
        assert_eq!(diagram_position(45_000.0, -6.0)[0], DIAGRAM[0]);
        assert!((diagram_position(2250.0, 9.0)[0] - DIAGRAM[1]).abs() < 0.5);
        assert_eq!(diagram_position(2400.0, -6.0)[1], DIAGRAM[2]);
        assert_eq!(diagram_position(2400.0, 9.0)[1], DIAGRAM[3]);
        // Then through the scene: one star among different crowds stands at the same place.
        let options = ViewOptions { hr_diagram: true };
        let stand = |crowd: u32, cgroup: &str| {
            let mut processes = vec![star(7, 30.0, 2 * GIB, cgroup)];
            processes
                .extend((0..crowd).map(|index| star(200 + index, index as f32, 50 * MIB, "crowd")));
            let mut scene = Scene::new();
            scene.configure(&options);
            let now = snapshot(10.0, processes);
            draw(&mut scene, &now, 10.0);
            scene.positions[&Identity { pid: 7, start: 7 }]
        };
        let alone = stand(0, "a");
        assert_eq!(alone, stand(40, "a"), "the others do not move it");
        assert_eq!(alone, stand(40, "b"), "nor does its constellation");
        // And the rest of the machine changing under it does not either.
        let mut scene = Scene::new();
        scene.configure(&options);
        let mut processes = vec![star(7, 30.0, 2 * GIB, "a"), star(8, 5.0, 100 * MIB, "a")];
        draw(&mut scene, &snapshot(10.0, processes.clone()), 10.0);
        let first = scene.positions[&Identity { pid: 7, start: 7 }];
        processes[1].cpu = 60.0;
        processes.push(star(9, 400.0, 8 * GIB, "b"));
        draw(&mut scene, &snapshot(11.0, processes), 12.0);
        assert_eq!(first, scene.positions[&Identity { pid: 7, start: 7 }]);
    }

    #[test]
    fn the_transition_ends_exactly_on_the_diagram_after_one_and_a_half_seconds() {
        let machine = demo_with(12.0, 96, DEMO);
        let camera = Camera::default();
        let draw_at = |scene: &mut Scene, time: f32| {
            let mut later = machine.clone();
            later.elapsed = 12.0;
            scene.render(&RenderInput::small(&later, hr_view(), &camera, time));
            scene.positions.clone()
        };
        // At rest in the diagram, from the command line.
        let mut at_rest = Scene::new();
        at_rest.configure(&ViewOptions { hr_diagram: true });
        for second in 0..=12 {
            at_rest.record(&demo_with(f64::from(second), 96, DEMO), second as f32);
        }
        let diagram = draw_at(&mut at_rest, 20.0);
        // Pressing `m` at 20 s: the sky at the start, the diagram after 1.5 s, exactly.
        let mut scene = Scene::new();
        for second in 0..=12 {
            scene.record(&demo_with(f64::from(second), 96, DEMO), second as f32);
        }
        let sky = draw_at(&mut scene, 19.0);
        scene.key(hr_view(), ViewKey::Local('m'));
        assert_eq!(
            draw_at(&mut scene, 20.0),
            sky,
            "the flight starts from the sky"
        );
        let middle = draw_at(&mut scene, 20.75);
        let moved = sky
            .iter()
            .filter(|(id, place)| middle[id] != **place && middle[id] != diagram[id])
            .count();
        assert!(
            moved > sky.len() / 2,
            "{moved} of {} are in flight halfway",
            sky.len()
        );
        let end = draw_at(&mut scene, 21.5);
        for (id, place) in &diagram {
            assert_eq!(&end[id], place, "{id:?} at 1.5 s");
        }
        let later = draw_at(&mut scene, 30.0);
        assert_eq!(later, end, "and it stays");
        // Back again with `m`, and a flight turned round halfway goes back from where it is.
        scene.key(hr_view(), ViewKey::Local('m'));
        draw_at(&mut scene, 31.0);
        let back = draw_at(&mut scene, 32.5);
        for (id, place) in &sky {
            if !diagram.contains_key(id) {
                continue;
            }
            assert_eq!(&back[id], place, "{id:?} back in the sky");
        }
        // The pure progress: linear in time over one and a half seconds, and turnable.
        let mut mode = Mode::default();
        assert_eq!(mode.progress(5.0), 0.0);
        mode.toggle(10.0);
        assert_eq!(mode.progress(10.0), 0.0);
        assert!((mode.progress(10.75) - 0.5).abs() < 1e-6);
        assert_eq!(mode.progress(11.5), 1.0);
        assert_eq!(mode.progress(99.0), 1.0);
        let mut turned = Mode::default();
        turned.toggle(10.0);
        let halfway = turned.progress(10.6);
        turned.toggle(10.6);
        assert!(
            (turned.progress(10.6) - halfway).abs() < 1e-6,
            "no jump when turned"
        );
        assert!(turned.progress(10.9) < halfway, "it heads back");
        assert_eq!(turned.progress(20.0), 0.0);
        // Every star lags a little and bends its own way, from its identity.
        let lags: HashSet<u32> = (0..50)
            .map(|pid| Record::new(Identity { pid, start: 1 }, None).lag.to_bits())
            .collect();
        assert!(lags.len() > 40);
        for pid in 0..50 {
            let record = Record::new(Identity { pid, start: 1 }, None);
            assert!((0.0..=MAX_LAG).contains(&record.lag));
            assert!(record.bend.abs() <= MAX_BEND);
            assert_eq!(
                flight([1.0, 2.0], [5.0, 6.0], record.lag, record.bend, 0.0),
                [1.0, 2.0]
            );
            assert_eq!(
                flight([1.0, 2.0], [5.0, 6.0], record.lag, record.bend, 1.0),
                [5.0, 6.0]
            );
        }
    }

    #[test]
    fn only_processes_over_one_gib_go_supernova() {
        assert_eq!(
            exit_flash(GIB),
            Flash::Fade,
            "exactly one GiB is not over it"
        );
        assert_eq!(exit_flash(GIB + 1), Flash::Supernova);
        assert_eq!(exit_flash(0), Flash::Fade);
        assert_eq!(exit_flash(40 * GIB), Flash::Supernova);
        // Through the scene: a big and a small process leave, and a big one dies as a zombie,
        // which holds nothing by then and must still go out as what it was.
        let alive = |zombie: bool| {
            let big = star(1, 10.0, 3 * GIB, "a");
            let small = star(2, 10.0, 200 * MIB, "a");
            let mut dying = star(3, 10.0, 4 * GIB, "a");
            let keeper = star(4, 10.0, 2 * GIB, "a");
            if zombie {
                dying.state = 'Z';
                dying.memory = 0;
                dying.cpu = 0.0;
            }
            vec![big, small, dying, keeper]
        };
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot(1.0, alive(false)), 1.0);
        let mut remaining = alive(true);
        remaining.remove(1);
        draw(&mut scene, &snapshot(2.0, remaining), 2.0);
        let hr = hr_state(&scene);
        let id = |pid: u32| Identity {
            pid,
            start: u64::from(pid),
        };
        let went = |pid: u32| {
            let record = &hr.records[&id(pid)];
            (record.left, exit_flash(record.memory))
        };
        assert_eq!(went(2), (Some(2.0), Flash::Fade), "the small one fades");
        assert_eq!(
            went(3),
            (Some(2.0), Flash::Supernova),
            "the zombie was four GiB"
        );
        assert_eq!(hr.records[&id(1)].left, None, "the others are still there");
        // A supernova flashes, leaves a shell and is over after eight seconds, a fade after one
        // and a half.
        assert!(
            flash(Flash::Supernova, 3.0)
                .and_then(|state| state.shell)
                .is_some()
        );
        assert!(flash(Flash::Supernova, 8.5).is_none());
        assert!(flash(Flash::Fade, 2.0).is_none());
        // Drawn: the supernova's rings and flashes, and nothing for the fade after its time.
        let mut camera = Camera::default();
        scene.fit(&mut camera, 320, 180);
        let now = snapshot(
            2.0,
            vec![star(1, 10.0, 3 * GIB, "a"), star(4, 10.0, 2 * GIB, "a")],
        );
        let items = |scene: &mut Scene, time: f32| {
            scene
                .render(&RenderInput::small(&now, hr_view(), &camera, time))
                .items
                .len()
        };
        let fresh = items(&mut scene, 2.2);
        let old = items(&mut scene, 12.0);
        assert!(
            fresh > old,
            "the shell is gone after ten seconds: {fresh} against {old}"
        );
    }

    #[test]
    fn hr_legend_names_missing_cgroups_and_socket_traffic() {
        let mut machine = snapshot(10.0, population());
        let (scene, _) = sky(machine.processes.clone());
        let plain = scene.legend(hr_view(), &machine);
        assert!(plain.contains("stars"), "{plain}");
        assert!(
            plain.contains("blackbody") && plain.contains("memory"),
            "{plain}"
        );
        assert!(
            plain.contains("twinkle") && plain.contains("m diagram"),
            "{plain}"
        );
        assert!(!plain.contains("no cgroups") && !plain.contains("no socket traffic"));
        machine.missing = Sources::from(Source::Cgroups);
        assert!(
            scene
                .legend(hr_view(), &machine)
                .contains(" | no cgroups: constellations = app groups")
        );
        machine.missing = Sources::from(Source::SocketTraffic);
        let text = scene.legend(hr_view(), &machine);
        assert!(
            text.contains(" | no socket traffic: lines at one brightness"),
            "{text}"
        );
        assert!(!text.contains("no cgroups"));
        machine.missing.insert(Source::Cgroups);
        let both = scene.legend(hr_view(), &machine);
        assert!(both.contains("no cgroups") && both.contains("no socket traffic"));
        // The diagram's legend names its axes and the fit.
        let mut diagram = Scene::new();
        diagram.configure(&ViewOptions { hr_diagram: true });
        draw(&mut diagram, &machine, 10.0);
        let text = diagram.legend(hr_view(), &machine);
        assert!(text.contains("main sequence m = "), "{text}");
        assert!(
            text.contains("temperature") && text.contains("luminosity class"),
            "{text}"
        );
        assert!(text.contains("m sky"), "{text}");
    }

    #[test]
    fn hr_demo_supplies_every_consumed_field() {
        let options = hr_view().info().demo;
        assert!(options.churn && options.early_giant);
        let machine = demo_with(12.0, 192, options);
        let stars: Vec<&Process> = machine
            .processes
            .iter()
            .filter(|process| process.kind != Kind::Kernel)
            .collect();
        assert!(stars.len() > 100);
        assert!(
            machine
                .processes
                .iter()
                .filter(|process| process.kind == Kind::Kernel)
                .count()
                >= 10
        );
        let constellations: HashSet<&str> =
            stars.iter().map(|process| constellation(process)).collect();
        assert!(constellations.len() >= 5, "{constellations:?}");
        assert!(stars.iter().all(|process| !process.group.is_empty()));
        assert!(
            stars.iter().any(|process| process.cpu > 50.0)
                && stars.iter().any(|process| process.cpu == 0.0)
        );
        assert!(stars.iter().any(|process| process.memory > 200 * MIB));
        assert!(stars.iter().any(|process| process.memory < 50 * MIB));
        assert!(!machine.links.is_empty() && !machine.link_traffic.is_empty());
        assert!(
            machine.missing.is_empty(),
            "nothing the view consumes is missing"
        );
        // As a headless run replays it: births and a supernova by 12 s.
        let mut scene = Scene::new();
        for second in 0..=12 {
            let sample = demo_with(f64::from(second), 192, options);
            draw(&mut scene, &sample, second as f32);
        }
        let hr = hr_state(&scene);
        let born = hr
            .records
            .values()
            .filter(|record| record.born.is_some())
            .count();
        assert!(born >= 3, "{born} births");
        let supernovae = hr
            .records
            .values()
            .filter(|record| record.left.is_some() && exit_flash(record.memory) == Flash::Supernova)
            .count();
        assert!(supernovae >= 1, "no supernova by 12 s");
        let fades = hr
            .records
            .values()
            .filter(|record| record.left.is_some() && exit_flash(record.memory) == Flash::Fade)
            .count();
        assert!(fades >= 1);
        assert!(hr.fit.is_some() && hr.kernel_threads >= 10);
    }

    #[test]
    fn hr_renders_the_same_frame_twice_from_the_same_inputs() {
        let run = |diagram: bool| {
            let mut scene = Scene::new();
            scene.configure(&ViewOptions {
                hr_diagram: diagram,
            });
            let camera = Camera::default();
            for second in 0..=12 {
                scene.record(&demo_with(f64::from(second), 96, DEMO), second as f32);
            }
            let machine = demo_with(12.0, 96, DEMO);
            let mut frames = Vec::new();
            for step in 0..4 {
                let mut frame = scene.render(&RenderInput::small(
                    &machine,
                    hr_view(),
                    &camera,
                    12.0 + step as f32 * 0.25,
                ));
                frame.rasterize();
                frames.push((frame.pixels, frame.picks));
            }
            frames
        };
        assert!(run(false) == run(false), "the sky");
        assert!(run(true) == run(true), "the diagram");
        assert!(run(false) != run(true), "the two modes differ");
    }

    #[test]
    fn a_panned_camera_that_turns_or_tilts_leaves_every_star_on_its_pixel() {
        let machine = snapshot(10.0, population());
        for diagram in [false, true] {
            let mut scene = Scene::new();
            scene.configure(&ViewOptions {
                hr_diagram: diagram,
            });
            draw(&mut scene, &snapshot(9.0, population()), 9.0);
            let mut camera = Camera::default();
            let screen = |scene: &mut Scene, camera: &Camera| -> Vec<[f32; 2]> {
                let input = RenderInput {
                    width: 960,
                    height: 540,
                    ..RenderInput::small(&machine, hr_view(), camera, 10.5)
                };
                let frame = scene.render(&input);
                machine
                    .processes
                    .iter()
                    .filter(|process| scene.positions.contains_key(&process.id))
                    .map(|process| {
                        let at = frame.project(camera, scene.positions[&process.id]);
                        [at[0], at[1]]
                    })
                    .collect()
            };
            let centred = screen(&mut scene, &camera);
            camera.pan(40.0, 25.0);
            let panned = screen(&mut scene, &camera);
            let moved = centred
                .iter()
                .zip(&panned)
                .map(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]))
                .fold(0.0_f32, f32::max);
            assert!(
                moved > 20.0,
                "diagram {diagram}: the pan moved the sky {moved} px"
            );
            assert!(camera.center != [0.0, 0.0]);
            camera.rotation += 0.6;
            let turned = screen(&mut scene, &camera);
            camera.tilt(-0.25);
            let tilted = screen(&mut scene, &camera);
            for (name, after) in [("turn", &turned), ("tilt", &tilted)] {
                assert_eq!(after.len(), panned.len());
                for (before, after) in panned.iter().zip(after) {
                    let off = (before[0] - after[0]).hypot(before[1] - after[1]);
                    assert!(
                        off < 0.5,
                        "diagram {diagram}: a {name} moved a star {off} px"
                    );
                }
            }
        }
    }

    #[test]
    fn a_stars_class_comes_from_the_sample_the_fit_was_made_from_not_from_the_easing() {
        let mut hot = population();
        hot.iter_mut()
            .find(|process| process.id.pid == 901)
            .expect("the supergiant")
            .cpu = cpu_at(9000.0);
        let now = snapshot(10.0, population());
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot(9.0, hot), 9.0);
        // A tenth of a second on, the supergiant's eased CPU is still mostly the hot one.
        draw(&mut scene, &now, 9.1);
        let giant = now
            .processes
            .iter()
            .find(|process| process.id.pid == 901)
            .expect("the supergiant");
        let expected = format!(
            "{} {}",
            label_name(&giant.name),
            classification(
                temperature(giant.cpu),
                star_magnitude(giant.memory),
                main_sequence(&now)
            )
        );
        let labels: Vec<&str> = scene.tags.iter().map(|tag| tag.text.as_str()).collect();
        assert!(
            labels.contains(&expected.as_str()),
            "{expected} in {labels:?}"
        );
    }

    #[test]
    fn stars_are_pickable_and_the_six_brightest_carry_their_class_in_both_modes() {
        let machine = snapshot(10.0, population());
        for diagram in [false, true] {
            let mut scene = Scene::new();
            scene.configure(&ViewOptions {
                hr_diagram: diagram,
            });
            draw(&mut scene, &snapshot(9.0, population()), 9.0);
            draw(&mut scene, &machine, 10.0);
            let mut camera = Camera::default();
            scene.fit(&mut camera, 960, 540);
            let input = RenderInput {
                width: 960,
                height: 540,
                ..RenderInput::small(&machine, hr_view(), &camera, 10.5)
            };
            let mut frame = scene.render(&input);
            frame.rasterize();
            // Every star can be clicked on, wherever it stands.
            let mut pickable = 0;
            for process in &machine.processes {
                let at = frame.project(&camera, scene.positions[&process.id]);
                let (x, y) = (at[0].round() as i64, at[1].round() as i64);
                let index = frame
                    .identities
                    .iter()
                    .position(|id| *id == process.id)
                    .expect("drawn") as u32;
                let found = (-2..=2).any(|dy| {
                    (-2..=2).any(|dx| {
                        let (px, py) = (x + dx, y + dy);
                        (0..960).contains(&px)
                            && (0..540).contains(&py)
                            && frame.picks[(py * 960 + px) as usize] == index
                    })
                });
                pickable += usize::from(found);
            }
            assert!(
                pickable >= machine.processes.len() - 4,
                "diagram {diagram}: {pickable} of {} stars can be picked",
                machine.processes.len()
            );
            // Six labels, the brightest first, each the star's name and its class.
            assert_eq!(scene.tags.len(), LABELLED, "diagram {diagram}");
            let fit = main_sequence(&machine);
            let brightest = |count: usize| -> Vec<u32> {
                let mut order: Vec<&Process> = machine.processes.iter().collect();
                order.sort_by(|a, b| {
                    star_magnitude(a.memory)
                        .total_cmp(&star_magnitude(b.memory))
                        .then(a.id.cmp(&b.id))
                });
                order
                    .iter()
                    .take(count)
                    .map(|process| process.id.pid)
                    .collect()
            };
            let named: Vec<String> = brightest(LABELLED)
                .iter()
                .map(|&pid| {
                    let process = machine
                        .processes
                        .iter()
                        .find(|p| p.id.pid == pid)
                        .expect("star");
                    format!(
                        "{} {}",
                        label_name(&process.name),
                        classification(
                            temperature(process.cpu),
                            star_magnitude(process.memory),
                            fit
                        )
                    )
                })
                .collect();
            let tagged: Vec<String> = scene.tags.iter().map(|tag| tag.text.clone()).collect();
            assert_eq!(tagged, named, "diagram {diagram}");
            assert!(scene.tags.iter().all(|tag| tag.side == Side::Right));
            // The tour visits the twelve brightest, and the notes say the same class.
            assert_eq!(scene.tour.len(), TOURED, "diagram {diagram}");
            let toured: Vec<u32> = scene.tour.iter().map(|target| target.id.pid).collect();
            assert_eq!(toured, brightest(TOURED));
            assert!(
                scene
                    .tour
                    .windows(2)
                    .all(|pair| pair[0].weight >= pair[1].weight)
            );
            for target in &scene.tour {
                assert_eq!(target.position, scene.positions[&target.id]);
            }
            let chosen = scene.tour[0].id;
            scene.highlight = vec![chosen];
            scene.render(&input);
            let notes = &scene.notes[&chosen];
            let process = machine
                .processes
                .iter()
                .find(|p| p.id == chosen)
                .expect("star");
            let class = classification(
                temperature(process.cpu),
                star_magnitude(process.memory),
                fit,
            );
            assert!(
                notes[0].starts_with(&format!("class {class} | T ")),
                "{notes:?}"
            );
            assert!(
                notes
                    .iter()
                    .any(|line| line.starts_with("main sequence m "))
            );
            assert!(
                notes
                    .iter()
                    .any(|line| line.starts_with("CPU variation (CV) "))
            );
            assert!(scene.tags[0].text.ends_with(&class));
            // The text of the sky: constellation names, or on the diagram its axes.
            let texts: Vec<&str> = scene.places.iter().map(|(_, text)| text.as_str()).collect();
            if diagram {
                assert!(
                    texts.contains(&"surface temperature (hot on the left)"),
                    "{texts:?}"
                );
                assert!(texts.contains(&"O") && texts.contains(&"M"));
                assert!(texts.contains(&"40000 K") && !texts.contains(&"+9"));
            } else {
                assert_eq!(texts.len(), 2, "{texts:?}");
                assert!(texts.contains(&"sequence") && texts.contains(&"off"));
            }
        }
    }

    #[test]
    fn star_colour_comes_from_the_blackbody_table_and_size_from_the_magnitude_scale() {
        let processes = vec![
            star(1, 0.0, GIB, "a"),
            star(2, 100.0, 40 * MIB, "a"),
            star(3, 2000.0, 8 * GIB, "a"),
            star(4, 15.0, 3 * MIB, "a"),
            star(5, 5.0, 0, "a"),
        ];
        let (mut scene, machine) = sky(processes.clone());
        let camera = Camera::default();
        let input = RenderInput::small(&machine, hr_view(), &camera, 10.5);
        let frame = scene.render(&input);
        for (index, process) in processes.iter().enumerate() {
            let kelvin = temperature(process.cpu);
            let shown = star_magnitude(process.memory);
            let expected = spread(shown, input.height);
            let core = frame
                .items
                .iter()
                .find_map(|item| match *item {
                    Item::Sphere {
                        color,
                        radius,
                        pick,
                        ..
                    } if pick == index as u32 => Some((color, radius)),
                    _ => None,
                })
                .expect("every star has a core");
            let color = tint(
                mix(blackbody::color(kelvin, 1.0), [255; 3], 0.6),
                expected.core,
            );
            assert_eq!(
                core,
                (color, expected.core_px),
                "star {index} at {kelvin} K"
            );
        }
        // The table's own ends: idle is a deep orange, a twenty-core process a blue-white.
        let idle = blackbody::color(temperature(0.0), 1.0);
        let hot = blackbody::color(temperature(2000.0), 1.0);
        assert!(idle[0] > idle[1] && idle[1] > idle[2], "{idle:?}");
        assert!(hot[2] >= hot[0], "{hot:?}");
        // One GiB is magnitude 0 and ten MiB about 5, and the bigger star is the brighter.
        assert!(star_magnitude(8 * GIB) < star_magnitude(GIB));
        assert!(
            spread(star_magnitude(8 * GIB), 540).core_px
                > spread(star_magnitude(40 * MIB), 540).core_px
        );
    }

    #[test]
    fn labels_wait_for_the_flight_to_land() {
        let machine = snapshot(10.0, population());
        let mut scene = Scene::new();
        draw(&mut scene, &machine, 10.0);
        assert_eq!(scene.tags.len(), LABELLED);
        scene.key(hr_view(), ViewKey::Local('m'));
        draw(&mut scene, &machine, 11.0);
        scene.render(&RenderInput::small(
            &machine,
            hr_view(),
            &Camera::default(),
            11.7,
        ));
        assert!(scene.tags.is_empty(), "in flight");
        assert!(
            scene.places.is_empty(),
            "no constellation names or axes in flight"
        );
        assert_eq!(scene.tour.len(), TOURED, "the tour goes on");
        scene.render(&RenderInput::small(
            &machine,
            hr_view(),
            &Camera::default(),
            12.6,
        ));
        assert_eq!(scene.tags.len(), LABELLED);
    }

    #[test]
    fn a_star_is_a_nova_when_born_and_its_boundaries_stay_with_the_layout() {
        let mut scene = Scene::new();
        let mut processes = vec![star(1, 10.0, GIB, "a"), star(2, 10.0, GIB, "b")];
        draw(&mut scene, &snapshot(1.0, processes.clone()), 1.0);
        let runs = hr_state(&scene).runs.len();
        assert!(runs > 0, "two constellations have a boundary");
        assert!(
            hr_state(&scene)
                .records
                .values()
                .all(|record| record.born.is_none())
        );
        processes.push(star(3, 10.0, GIB, "a"));
        draw(&mut scene, &snapshot(2.0, processes), 2.0);
        let hr = hr_state(&scene);
        let id = |pid: u32| Identity {
            pid,
            start: u64::from(pid),
        };
        assert_eq!(hr.records[&id(3)].born, Some(2.0));
        assert_eq!(
            hr.records[&id(1)].born,
            None,
            "the first sample's stars are not newborn"
        );
        assert_eq!(
            hr.runs.len(),
            runs,
            "a star inside a disc leaves the boundaries alone"
        );
        assert_eq!(nova_age(&hr.records[&id(3)], 2.0, 0.25), Some(0.25));
        assert_eq!(nova_age(&hr.records[&id(1)], 2.0, 0.25), None);
    }

    #[test]
    fn stick_figures_dim_with_traffic_and_leave_a_gap_at_each_star() {
        assert!((link_brightness(0.0) - 0.35).abs() < 1e-6);
        assert!((link_brightness(1.0e6) - 1.0).abs() < 0.01);
        assert!(link_brightness(1.0e3) > link_brightness(10.0));
        assert!(link_brightness(1.0e12) <= 1.0);
        let mut processes = vec![star(1, 10.0, GIB, "a"), star(2, 10.0, GIB, "a")];
        processes.push(star(3, 10.0, 20 * MIB, "a"));
        let mut machine = snapshot(10.0, processes.clone());
        let (a, b) = (processes[0].id, processes[1].id);
        machine.links = vec![(a, b, 1)];
        machine.link_traffic.insert((a, b), 1.0e6);
        let mut scene = Scene::new();
        draw(&mut scene, &machine, 9.0);
        let strokes = |scene: &mut Scene, machine: &Snapshot| {
            scene
                .render(&RenderInput::small(
                    machine,
                    hr_view(),
                    &Camera::default(),
                    10.0,
                ))
                .items
                .iter()
                .filter(|item| matches!(item, Item::Triangle(_, color, _) if color[2] > color[0]))
                .count()
        };
        let drawn = strokes(&mut scene, &machine);
        scene.links = Links::Off;
        let hidden = strokes(&mut scene, &machine);
        assert!(
            drawn > hidden,
            "the link is a stroke: {drawn} against {hidden}"
        );
    }
}
