//! Tesseract: process worldlines in a Schlegel-projected tesseract whose fourth axis is time.
//! Each process is a worldline through (x, y, z, w) over the last minute: x is the CPU-seconds it
//! used since the window opened, y its memory, z its seat and w the time, from -1 a minute ago to
//! +1 now. x and w share a scale, 30 CPU-seconds and 30 seconds per unit, so the slope of a
//! worldline is its CPU use in cores and c is one core. The 4D rotation is camera only; a Lorentz
//! boost in the (w, x) plane changes the frame of reference to a process's rest frame.
//!
//! The pure parts come first: the `Worldlines` recorder with its seat allocator, the 4D rotation,
//! the boost and the display compression. `Tesseract` draws them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::model::{Identity, Kind, Process, Snapshot, bounded};
use crate::palette::{kind_color, tint};
use crate::platform::MEMORY_LABEL;
use crate::render::{Color, Point, Stage, label_name, ring_bounds};
use crate::status::unreadable_note;
use crate::views::{ViewKey, ViewModule};

/// Seconds of worldline drawn: w = -1 is this long ago.
const WINDOW: f64 = 60.0;
/// Seconds the recorder keeps, so a rewound frame still has its minute.
const RETAIN: f64 = 180.0;
/// The shortest gap between two recorded samples, in seconds.
const CADENCE: f64 = 0.25;
/// CPU-seconds per unit of x, and seconds per unit of w: equal, so a slope of 1 is one core.
const UNIT: f64 = 30.0;
/// The drawn worldline keeps at most one sample per bin of this many seconds.
const BIN: f64 = 1.0;
/// The seconds over which the recent slope (the head's glow, Cherenkov cone and notes) is taken.
const RECENT: f64 = 5.0;
/// Seconds over which a forked child curves from its parent's worldline to its own seat.
const FORK_SECONDS: f64 = 2.0;
/// Steps of the drawn fork curve, in seconds.
const FORK_STEP: f64 = 0.25;
/// Memory, in MiB, at which y is halfway up the bounded scale (`model::bounded`).
const MEMORY_KNEE: f32 = 220.0;
/// The Schlegel eye on the w axis, and the nearest the divide gets to it.
const EYE: f32 = 3.5;
const NEAREST: f32 = 0.75;
/// World units per projected 4D unit at perspective factor 1.
const WORLD: f32 = 25.0;
/// The canonical rotation angles in the XW, YW and ZW planes, in radians.
const CANONICAL: [f32; 3] = [0.12, 0.10, -0.08];
/// Seconds per turn of the idle drift in the XW plane.
const DRIFT_PERIOD: f32 = 120.0;
/// Time constants of the rotation and rapidity easing, in seconds.
const TURN_EASE: f32 = 0.2;
const BOOST_EASE: f32 = 0.4;
/// A chord slope at or above this has no rest frame: the boost would need beta = 1 or more.
const NO_REST_FRAME: f32 = 0.999;
/// Segments steeper than this are superluminal: only a multithreaded process gets there.
const SUPERLUMINAL: f32 = 1.05;
/// How close to 1 a slope reads as lightlike, and below what it reads as at rest.
const LIGHTLIKE: f32 = 0.05;
const AT_REST: f32 = 0.02;
/// Consecutive segments whose slopes differ by less than this, with memory within `MERGE_Y`,
/// are drawn as one.
const MERGE_SLOPE: f32 = 0.02;
const MERGE_Y: f32 = 0.002;
/// Display compression: beyond 1, |x| and |w| approach 1 + this.
const COMPRESSION: f32 = 0.6;
/// The most I/O marks a frame draws, newest first; their width in pixels, and the seconds of
/// worldline each covers on either side of its sample.
const IO_MARKS: usize = 2000;
const IO_WIDTH: f32 = 2.4;
const IO_SPAN: f64 = 0.4;
/// The half-length of a Cherenkov cone beam, in 4D units, and how many beams a cone has.
const CONE_LENGTH: f32 = 0.35;
const CONE_BEAMS: usize = 12;
/// The largest Cherenkov glow, in pixels, so its cost stays bounded.
/// The width of worldline and of frame hairlines (`Frame::hairline`), in pixels.
const STROKE: f32 = 1.4;
const FRAME_STROKE: f32 = 1.1;
const CHERENKOV_CAP: f32 = 56.0;
const CHERENKOV: Color = [90, 170, 255];
const LIGHT_CONE: Color = [255, 235, 160];
const IO_COLOR: Color = [110, 230, 255];
const SELECTED: Color = [255, 235, 171];
const NOW_CELL: Color = [190, 200, 235];
const PAST_CELL: Color = [120, 130, 175];
const FORK: Color = [255, 240, 200];
/// The bands of seats, by kind, from z = -0.9 to 0.9.
const BANDS: usize = 4;
/// The fewest seats a band holds; a full band doubles.
const BAND_SEATS: usize = 16;
const PLANES: [&str; 3] = ["XW", "YW", "ZW"];

/// A point in the tesseract: x (CPU-seconds), y (memory), z (seat), w (time).
pub type Point4 = [f32; 4];

/// One recorded sample of a process.
#[derive(Clone, Copy, Debug)]
struct Sample {
    elapsed: f64,
    cpu_time: f32,
    memory: u64,
    io: bool,
}

/// A seat: the band of the process's kind and its place in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    band: usize,
    index: usize,
}

/// The seats of the four bands. A newcomer takes the lowest free seat of its band; a band that
/// is full doubles, and no band ever shrinks, so seats move only when a band outgrows itself.
/// The first sample reserves twice the seats it fills, so the exited processes, which hold their
/// seats for a minute, and their replacements fit without a doubling.
#[derive(Default)]
struct Bands {
    taken: [Vec<bool>; BANDS],
}

impl Bands {
    fn capacity(&self, band: usize) -> usize {
        self.taken[band].len().max(BAND_SEATS)
    }

    fn take(&mut self, band: usize) -> Seat {
        let capacity = self.capacity(band);
        let taken = &mut self.taken[band];
        taken.resize(capacity, false);
        let index = match taken.iter().position(|&seat| !seat) {
            Some(index) => index,
            None => {
                taken.resize(capacity * 2, false);
                capacity
            }
        };
        taken[index] = true;
        Seat { band, index }
    }

    /// Reserves seats for `count` processes in `band`: the next power of two of at least
    /// `BAND_SEATS` and twice `count`.
    fn reserve(&mut self, band: usize, count: usize) {
        let capacity = (2 * count).max(BAND_SEATS).next_power_of_two();
        if self.taken[band].len() < capacity {
            self.taken[band].resize(capacity, false);
        }
    }

    fn release(&mut self, seat: Seat) {
        if let Some(taken) = self.taken[seat.band].get_mut(seat.index) {
            *taken = false;
        }
    }

    /// The seat's z: the bands share [-0.9, 0.9] in proportion to their capacities.
    fn z(&self, seat: Seat) -> f32 {
        let total: usize = (0..BANDS).map(|band| self.capacity(band)).sum();
        let before: usize = (0..seat.band).map(|band| self.capacity(band)).sum();
        -0.9 + 1.8 * (before + seat.index) as f32 / total as f32 + 0.9 / total as f32
    }
}

fn band(kind: Kind) -> usize {
    match kind {
        Kind::Kernel => 0,
        Kind::System => 1,
        Kind::Session => 2,
        Kind::Container => 3,
    }
}

fn kind_name(band: usize) -> &'static str {
    ["kernel", "system", "session", "container"][band]
}

/// One process's recorded history.
struct Line {
    samples: VecDeque<Sample>,
    color: Color,
    name: String,
    seat: Seat,
    /// Whether the seat is still the process's: it is given back once the worldline has left
    /// the window.
    held: bool,
    /// The process it forked from: set when it was first seen after the recorder's first sample
    /// and its parent was a recorded process alive in the same sample.
    parent: Option<Identity>,
}

/// The window a frame draws: samples up to `now` (the drawn sample's `elapsed`), with w = +1 at
/// `display`, which runs on from `now` between samples so the worldlines scroll smoothly.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub now: f64,
    pub display: f64,
}

impl Window {
    fn start(&self) -> f64 {
        self.display - WINDOW
    }

    fn w(&self, time: f64) -> f32 {
        (1.0 + (time - self.display) / UNIT) as f32
    }
}

/// A line's samples within a window, with the CPU-seconds grown since the first of them (a
/// falling `cpu_time` counts as no growth) and y.
struct Curve {
    times: Vec<f64>,
    growth: Vec<f32>,
    heights: Vec<f32>,
}

impl Curve {
    fn growth(&self, time: f64) -> f32 {
        interpolate(&self.times, &self.growth, time)
    }

    fn height(&self, time: f64) -> f32 {
        interpolate(&self.times, &self.heights, time)
    }
}

/// The value at `time` of a piecewise-linear function through (`times`, `values`), held flat
/// beyond its ends.
fn interpolate(times: &[f64], values: &[f32], time: f64) -> f32 {
    let index = times.partition_point(|&sample| sample <= time);
    if index == 0 {
        return values[0];
    }
    if index == times.len() {
        return values[index - 1];
    }
    let (from, to) = (times[index - 1], times[index]);
    let fraction = ((time - from) / (to - from)) as f32;
    values[index - 1] + (values[index] - values[index - 1]) * fraction
}

/// y for `memory` bytes: -1 + 2 sqrt(bounded(MiB, 220)).
fn memory_axis(memory: u64) -> f32 {
    -1.0 + 2.0 * bounded(memory as f32 / 1_048_576.0, MEMORY_KNEE).sqrt()
}

fn smoothstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

/// One worldline as a frame draws it, in lab coordinates before compression.
pub struct Track {
    pub id: Identity,
    /// From the window start (or birth) to the head at w = +1, or to the last sample of an
    /// exited process; collinear runs merged.
    pub points: Vec<Point4>,
    /// The stretch of worldline around each sample with disk I/O, with the sample's time.
    io: Vec<(f64, Point4, Point4)>,
    /// The fork vertex, where a child left its parent's worldline inside the window.
    pub fork: Option<Point4>,
    /// Whether the process is alive in the drawn sample, so the worldline ends in a head.
    pub alive: bool,
    /// The slope over the last `RECENT` seconds and over the visible window, in cores.
    pub recent: f32,
    pub chord: f32,
}

/// Every process's history, recorded on every sample whichever view is shown, so the tesseract
/// is already a minute deep when it opens.
#[derive(Default)]
pub struct Worldlines {
    lines: HashMap<Identity, Line>,
    bands: Bands,
    /// The `elapsed` of the first and the newest recorded sample.
    first: Option<f64>,
    last: Option<f64>,
    /// Seconds between the newest two recorded samples.
    interval: f64,
    /// Scene time at which the newest sample arrived.
    arrived: f32,
}

impl Worldlines {
    /// Takes in a sample at most every `CADENCE` seconds of `elapsed`, and only a newer one, so
    /// rewinding and the frame's own record call add nothing.
    pub fn record(&mut self, snapshot: &Snapshot, time: f32) {
        let elapsed = snapshot.elapsed;
        if self
            .last
            .is_some_and(|last| elapsed < last + CADENCE - 1e-6)
        {
            return;
        }
        if let Some(last) = self.last {
            self.interval = elapsed - last;
        }
        let forks = self.first.is_some();
        self.first.get_or_insert(elapsed);
        self.last = Some(elapsed);
        self.arrived = time;
        let recorded: HashMap<u32, Identity> = snapshot
            .processes
            .iter()
            .filter(|process| self.lines.contains_key(&process.id))
            .map(|process| (process.id.pid, process.id))
            .collect();
        let mut newcomers = Vec::new();
        for process in &snapshot.processes {
            let sample = Sample {
                elapsed,
                cpu_time: process.cpu_time,
                memory: process.memory,
                io: process.io_rate.is_some_and(|rate| rate > 0.0),
            };
            match self.lines.get_mut(&process.id) {
                Some(line) => {
                    line.samples.push_back(sample);
                    // An exited worldline keeps the colour of its kind, not of its last state.
                    if process.state != 'Z' {
                        line.color = kind_color(process);
                    }
                    if line.name != process.name {
                        line.name.clone_from(&process.name);
                    }
                }
                None => newcomers.push((process, sample)),
            }
        }
        newcomers.sort_by(|(a, _), (b, _)| {
            (band(a.kind), &a.cgroup, a.id.start, a.id.pid).cmp(&(
                band(b.kind),
                &b.cgroup,
                b.id.start,
                b.id.pid,
            ))
        });
        if !forks {
            for band_index in 0..BANDS {
                let count = newcomers
                    .iter()
                    .filter(|(process, _)| band(process.kind) == band_index)
                    .count();
                self.bands.reserve(band_index, count);
            }
        }
        for (process, sample) in newcomers {
            let parent = recorded
                .get(&process.parent)
                .copied()
                .filter(|_| forks && process.parent != process.id.pid);
            let seat = self.bands.take(band(process.kind));
            self.lines.insert(
                process.id,
                Line {
                    samples: VecDeque::from([sample]),
                    color: kind_color(process),
                    name: process.name.clone(),
                    seat,
                    held: true,
                    parent,
                },
            );
        }
        let Worldlines { lines, bands, .. } = self;
        for line in lines.values_mut() {
            let end = line.samples.back().map_or(f64::NEG_INFINITY, |s| s.elapsed);
            if line.held && end < elapsed - WINDOW {
                line.held = false;
                bands.release(line.seat);
            }
            while line.samples.len() > 1 && line.samples[1].elapsed < elapsed - RETAIN {
                line.samples.pop_front();
            }
        }
        lines.retain(|_, line| {
            line.held
                || line
                    .samples
                    .back()
                    .is_some_and(|s| s.elapsed >= elapsed - RETAIN)
        });
    }

    /// Seconds of history behind `elapsed`, at most `WINDOW`.
    fn depth(&self, elapsed: f64) -> f64 {
        self.first
            .map_or(0.0, |first| (elapsed - first).clamp(0.0, WINDOW))
    }

    #[cfg(test)]
    fn seat(&self, id: Identity) -> Option<f32> {
        self.lines
            .get(&id)
            .filter(|line| line.held)
            .map(|line| self.bands.z(line.seat))
    }

    /// The samples of `line` from the last at or before the window start to the last at or
    /// before `now`; None when it has none by `now` or they ended before the window.
    fn curve(line: &Line, window: Window) -> Option<Curve> {
        let samples = &line.samples;
        let end = samples.partition_point(|sample| sample.elapsed <= window.now);
        if end == 0 || samples[end - 1].elapsed < window.start() {
            return None;
        }
        let begin = samples
            .partition_point(|sample| sample.elapsed <= window.start())
            .saturating_sub(1);
        let count = end - begin;
        let mut curve = Curve {
            times: Vec::with_capacity(count),
            growth: Vec::with_capacity(count),
            heights: Vec::with_capacity(count),
        };
        let mut grown = 0.0;
        let mut previous = samples[begin].cpu_time;
        for sample in samples.range(begin..end) {
            grown += (sample.cpu_time - previous).max(0.0);
            previous = sample.cpu_time;
            curve.times.push(sample.elapsed);
            curve.growth.push(grown);
            curve.heights.push(memory_axis(sample.memory));
        }
        Some(curve)
    }

    /// x at `time` on the worldline of `id`: -1 plus the CPU-seconds since the window opened
    /// (or since birth) over `UNIT`, moved to start on the parent's worldline for a child forked
    /// inside the window.
    fn x_at(&self, id: Identity, time: f64, window: Window, depth: usize) -> Option<f32> {
        let line = self.lines.get(&id)?;
        let curve = Self::curve(line, window)?;
        let birth = line.samples.front()?.elapsed;
        let from = window.start().max(birth);
        let origin = self.origin(line, window, depth);
        Some(origin + (curve.growth(time) - curve.growth(from)) / UNIT as f32)
    }

    /// Where a worldline's x starts: -1, or the parent's x at the child's birth when it forked
    /// inside the window. As the birth reaches the window start the parent's x there reaches -1,
    /// so the start moves continuously.
    fn origin(&self, line: &Line, window: Window, depth: usize) -> f32 {
        let birth = line.samples.front().map_or(0.0, |sample| sample.elapsed);
        match line.parent {
            Some(parent) if birth >= window.start() && depth < 16 => {
                self.x_at(parent, birth, window, depth + 1).unwrap_or(-1.0)
            }
            _ => -1.0,
        }
    }

    /// The worldline of `id` in `window`, ending in a head at w = +1 when `alive`.
    pub fn track(&self, id: Identity, window: Window, alive: bool) -> Option<Track> {
        let line = self.lines.get(&id)?;
        let curve = Self::curve(line, window)?;
        let birth = line.samples.front()?.elapsed;
        let start = window.start();
        let from = start.max(birth);
        let origin = self.origin(line, window, 0);
        let own_z = self.bands.z(line.seat);
        let parent = line
            .parent
            .filter(|_| birth >= start)
            .and_then(|parent| self.lines.get(&parent))
            .and_then(|parent| {
                let height = Self::curve(parent, window)?.height(birth);
                Some((height, self.bands.z(parent.seat)))
            });
        let point = |time: f64| -> Point4 {
            let x = origin + (curve.growth(time) - curve.growth(from)) / UNIT as f32;
            let (mut y, mut z) = (curve.height(time), own_z);
            if let Some((parent_y, parent_z)) = parent {
                let blend = smoothstep(((time - birth) / FORK_SECONDS) as f32);
                y = parent_y + (y - parent_y) * blend;
                z = parent_z + (z - parent_z) * blend;
            }
            [x, y, z, window.w(time)]
        };
        let last = *curve.times.last()?;
        let mut times = vec![from];
        let mut bin = (from / BIN).floor();
        let mut replaceable = false;
        for &time in curve.times.iter().filter(|&&time| time > from) {
            let this = (time / BIN).floor();
            if replaceable && this == bin {
                times.pop();
            }
            times.push(time);
            bin = this;
            replaceable = true;
        }
        if parent.is_some() {
            let steps = (FORK_SECONDS / FORK_STEP) as usize;
            times.extend(
                (1..steps)
                    .map(|step| birth + step as f64 * FORK_STEP)
                    .filter(|&time| time < last),
            );
            times.sort_by(f64::total_cmp);
            times.dedup();
        }
        let mut points: Vec<Point4> = times.iter().map(|&time| point(time)).collect();
        let slope_over = |seconds: f64| {
            let span = seconds.min(last - curve.times[0]);
            if span <= 0.0 {
                0.0
            } else {
                (curve.growth(last) - curve.growth(last - span)) / span as f32
            }
        };
        let recent = slope_over(RECENT);
        if alive {
            let head = point(window.display.max(last));
            let steps = curve.times.len();
            let slope = if steps >= 2 {
                let before = curve.times[steps - 2];
                (curve.growth(last) - curve.growth(before)) / (last - before) as f32
            } else {
                0.0
            };
            let ahead = (window.display - last).max(0.0) / UNIT;
            // The head replaces a sample drawn at w = +1 itself.
            if points.last().is_some_and(|point| point[3] >= 1.0 - 1e-6) {
                points.pop();
            }
            points.push([head[0] + slope * ahead as f32, head[1], head[2], 1.0]);
        }
        let chord = match (points.first(), points.last()) {
            (Some(first), Some(head)) if head[3] - first[3] > 1e-4 => {
                (head[0] - first[0]) / (head[3] - first[3])
            }
            _ => 0.0,
        };
        let io = line
            .samples
            .iter()
            .filter(|sample| sample.io && (from..=window.now).contains(&sample.elapsed))
            .map(|sample| {
                let time = sample.elapsed;
                let (before, after) = ((time - IO_SPAN).max(from), (time + IO_SPAN).min(last));
                (time, point(before), point(after))
            })
            .collect();
        Some(Track {
            id,
            points: merge(&points),
            io,
            fork: parent.map(|_| point(birth)),
            alive,
            recent,
            chord,
        })
    }

    /// Recorded processes that are not in `alive` and whose worldline is in the window, newest
    /// exit first.
    fn exited(&self, alive: &HashSet<Identity>, window: Window) -> Vec<Identity> {
        let mut exited: Vec<(f64, Identity)> = self
            .lines
            .iter()
            .filter(|(id, _)| !alive.contains(id))
            .filter_map(|(&id, line)| {
                let end = line.samples.partition_point(|s| s.elapsed <= window.now);
                let last = line.samples.get(end.checked_sub(1)?)?.elapsed;
                (last >= window.start()).then_some((last, id))
            })
            .collect();
        exited.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        exited.into_iter().map(|(_, id)| id).collect()
    }
}

/// The slope of a segment, dx/dw.
fn slope(from: Point4, to: Point4) -> f32 {
    let dw = to[3] - from[3];
    if dw.abs() < 1e-6 {
        0.0
    } else {
        (to[0] - from[0]) / dw
    }
}

/// `points` with each run of segments that share a slope (within `MERGE_SLOPE` of the run's
/// first), a side of the superluminal line, memory (within `MERGE_Y`) and seat drawn as one
/// segment.
fn merge(points: &[Point4]) -> Vec<Point4> {
    let Some((&first, rest)) = points.split_first() else {
        return Vec::new();
    };
    let mut merged = vec![first];
    // The slope of the run that ends at the last merged point, which is always the previous
    // point of `points`.
    let mut run: Option<f32> = None;
    for &to in rest {
        let end = merged[merged.len() - 1];
        let this = slope(end, to);
        let joins = run.is_some_and(|run| {
            let start = merged[merged.len() - 2];
            (this - run).abs() < MERGE_SLOPE
                && (this > SUPERLUMINAL) == (run > SUPERLUMINAL)
                && (to[1] - start[1]).abs() < MERGE_Y
                && to[2] == start[2]
        });
        if joins {
            let last = merged.len() - 1;
            merged[last] = to;
        } else {
            merged.push(to);
            run = Some(this);
        }
    }
    merged
}

/// A 4x4 rotation in the plane of axes `a` and `b` by `angle`.
fn plane_rotation(a: usize, b: usize, angle: f32) -> [[f32; 4]; 4] {
    let mut matrix = [[0.0; 4]; 4];
    for (index, row) in matrix.iter_mut().enumerate() {
        row[index] = 1.0;
    }
    let (sin, cos) = angle.sin_cos();
    matrix[a][a] = cos;
    matrix[a][b] = -sin;
    matrix[b][a] = sin;
    matrix[b][b] = cos;
    matrix
}

fn multiply(left: &[[f32; 4]; 4], right: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| (0..4).map(|k| left[row][k] * right[k][column]).sum())
    })
}

/// The rotation for angles in the XW, YW and ZW planes, composed in that order.
pub fn rotation(angles: [f32; 3]) -> [[f32; 4]; 4] {
    let xw = plane_rotation(0, 3, angles[0]);
    let yw = plane_rotation(1, 3, angles[1]);
    let zw = plane_rotation(2, 3, angles[2]);
    multiply(&multiply(&xw, &yw), &zw)
}

pub fn rotate(matrix: &[[f32; 4]; 4], point: Point4) -> Point4 {
    std::array::from_fn(|row| (0..4).map(|k| matrix[row][k] * point[k]).sum())
}

/// A Lorentz boost with `rapidity` in the (w, x) plane about the event `pivot` (x, w): the frame
/// moving at beta = tanh(rapidity) along x. It keeps dw^2 - dx^2.
pub fn boost(point: Point4, pivot: [f32; 2], rapidity: f32) -> Point4 {
    if rapidity == 0.0 {
        return point;
    }
    let (sinh, cosh) = (rapidity.sinh(), rapidity.cosh());
    let dx = point[0] - pivot[0];
    let dw = point[3] - pivot[1];
    [
        pivot[0] + dx * cosh - dw * sinh,
        point[1],
        point[2],
        pivot[1] + dw * cosh - dx * sinh,
    ]
}

/// The rapidity of the rest frame of a worldline with slope `beta`, or None when it has none.
pub fn rest_rapidity(beta: f32) -> Option<f32> {
    (beta.abs() < NO_REST_FRAME).then(|| beta.atanh())
}

/// Display compression of a coordinate: beyond 1 it approaches 1 + `COMPRESSION`.
fn compress(value: f32) -> f32 {
    let magnitude = value.abs();
    if magnitude <= 1.0 {
        value
    } else {
        value.signum() * (1.0 + COMPRESSION * ((magnitude - 1.0) / COMPRESSION).tanh())
    }
}

/// What a slope is, as the inspector says it.
fn describe_slope(beta: f32) -> String {
    if beta < AT_REST {
        "at rest".into()
    } else if (beta - 1.0).abs() <= LIGHTLIKE {
        format!("slope {beta:.2} c (lightlike)")
    } else if beta > SUPERLUMINAL {
        format!(
            "superluminal: beta {beta:.1}, Cherenkov angle {:.0} deg",
            (1.0 / beta).acos().to_degrees()
        )
    } else {
        format!("slope {beta:.2} c")
    }
}

/// The camera-only 4D orientation: quarter turns per plane, the selected plane and the idle
/// drift in XW. Keys change it between frames; the frame applies what needs the time.
#[derive(Default)]
struct Orientation {
    /// Offsets from `CANONICAL` that the angles ease towards, and where they are now.
    goal: [f32; 3],
    current: [f32; 3],
    plane: usize,
    /// Drift is on unless a 4D key stopped it.
    stopped: bool,
    started: bool,
    /// The time of the last frame, whose angles the keys turn from.
    time: f32,
}

impl Orientation {
    fn drift(time: f32) -> f32 {
        TAU * (time / DRIFT_PERIOD).rem_euclid(1.0)
    }

    /// Stops the drift where the last frame showed it.
    fn stop(&mut self) {
        if !self.stopped {
            let drift = Self::drift(self.time);
            self.stopped = true;
            self.goal[0] += drift;
            self.current[0] += drift;
        }
    }

    fn turn(&mut self, direction: f32) {
        self.stop();
        self.goal[self.plane] += direction * FRAC_PI_2;
    }

    fn cycle(&mut self) {
        self.stop();
        self.plane = (self.plane + 1) % PLANES.len();
    }

    /// Back to the canonical orientation with the drift on, turning the short way from the last
    /// frame's angles.
    fn reset(&mut self) {
        let drift = if self.stopped {
            Self::drift(self.time)
        } else {
            0.0
        };
        self.current[0] -= drift;
        self.current = self.current.map(|angle| (angle + PI).rem_euclid(TAU) - PI);
        self.goal = [0.0; 3];
        self.plane = 0;
        self.stopped = false;
    }

    /// The angles of the frame at `time`, `dt` after the last one that moved.
    fn angles(&mut self, time: f32, dt: f32) -> [f32; 3] {
        self.time = time;
        let drift = Self::drift(time);
        if self.started {
            let amount = 1.0 - (-dt / TURN_EASE).exp();
            for plane in 0..3 {
                self.current[plane] += (self.goal[plane] - self.current[plane]) * amount;
            }
        } else {
            self.current = self.goal;
            self.started = true;
        }
        let drift = if self.stopped { 0.0 } else { drift };
        [
            CANONICAL[0] + self.current[0] + drift,
            CANONICAL[1] + self.current[1],
            CANONICAL[2] + self.current[2],
        ]
    }
}

/// What `b` asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Request {
    #[default]
    Off,
    /// `b` was pressed; the next frame boosts into the selected process, if there is one.
    Wanted,
    On(Identity),
}

/// The frame of reference the legend names.
#[derive(Clone, Debug, Default, PartialEq)]
enum Reference {
    #[default]
    Lab,
    Rest(String),
    None(String, f32),
}

/// The boost: what was asked for and the eased rapidity and pivot event (x, w).
#[derive(Default)]
struct Boost {
    request: Request,
    rapidity: f32,
    pivot: [f32; 2],
    reference: Reference,
}

/// Projects lab-frame 4D points: the boost (physics), the display compression, the camera's 4D
/// rotation, then the Schlegel divide along w.
struct Projection {
    rotation: [[f32; 4]; 4],
    rapidity: f32,
    pivot: [f32; 2],
}

impl Projection {
    fn boosted(&self, point: Point4) -> Point4 {
        boost(point, self.pivot, self.rapidity)
    }

    /// The world point of an already boosted 4D point, and its perspective factor.
    fn world(&self, point: Point4) -> (Point, f32) {
        let compressed = [compress(point[0]), point[1], point[2], compress(point[3])];
        let [x, y, z, w] = rotate(&self.rotation, compressed);
        let factor = 1.0 / (EYE - w).max(NEAREST);
        let scale = WORLD * factor;
        // Memory is up.
        ([x * scale, z * scale, y * scale], factor)
    }

    fn place(&self, point: Point4) -> Point {
        self.world(self.boosted(point)).0
    }
}

/// How far back in w a point is, as a brightness: full at w = +1, about a third at -1.
fn shade(w: f32) -> f32 {
    0.35 + 0.65 * ((w + 1.0) * 0.5).clamp(0.0, 1.0)
}

/// The tesseract view.
#[derive(Default)]
pub struct Tesseract {
    lines: Worldlines,
    orientation: Orientation,
    boost: Boost,
    /// The scroll of the last frame, kept while time stands still.
    scroll: f64,
    /// The rotation and the heads of the last frame, for the harness's anchors.
    rotation: Option<[[f32; 4]; 4]>,
    shown: Vec<(Identity, f32)>,
}

impl Tesseract {
    /// The boost for this frame: towards the rest frame of the process `b` boosted into, or else
    /// of the one the tour visits, when it has one.
    fn steer(&mut self, stage: &mut Stage, tracks: &[Track], names: &HashMap<Identity, &str>) {
        if self.boost.request == Request::Wanted {
            self.boost.request = stage.selected.map_or(Request::Off, Request::On);
        }
        let target = match self.boost.request {
            Request::On(id) => Some(id),
            _ => stage.toured,
        };
        let track = target.and_then(|id| tracks.iter().find(|track| track.id == id && track.alive));
        let (rapidity, pivot) = match (target, track) {
            (Some(id), Some(track)) => {
                let name = label_name(names.get(&id).copied().unwrap_or("")).into_owned();
                let head = *track.points.last().expect("a track has a head");
                match rest_rapidity(track.chord) {
                    Some(rapidity) => {
                        self.boost.reference = Reference::Rest(name);
                        (rapidity, [head[0], head[3]])
                    }
                    None => {
                        self.boost.reference = Reference::None(name, track.chord);
                        if stage.toured == Some(id) {
                            stage.callout.push((id, "no rest frame".into()));
                        }
                        (0.0, self.boost.pivot)
                    }
                }
            }
            (Some(_), None) => {
                // The process left: ease back to the lab frame.
                self.boost.request = Request::Off;
                self.boost.reference = Reference::Lab;
                (0.0, self.boost.pivot)
            }
            (None, _) => {
                self.boost.reference = Reference::Lab;
                (0.0, self.boost.pivot)
            }
        };
        if self.boost.rapidity.abs() < 1e-4 {
            self.boost.pivot = pivot;
        }
        let amount = 1.0 - (-stage.dt / BOOST_EASE).exp();
        self.boost.rapidity += (rapidity - self.boost.rapidity) * amount;
        for (current, goal) in self.boost.pivot.iter_mut().zip(pivot) {
            *current += (goal - *current) * amount;
        }
    }

    /// The tesseract's 32 edges in the lab frame, the dashed light cone along an x-w face and
    /// their labels.
    fn draw_frame(stage: &mut Stage, projection: &Projection) {
        let corner = |index: usize| -> Point4 {
            std::array::from_fn(|axis| if index >> axis & 1 == 1 { 1.0 } else { -1.0 })
        };
        for a in 0..16_usize {
            for axis in 0..4 {
                let b = a | 1 << axis;
                if b == a {
                    continue;
                }
                let (from, to) = (corner(a), corner(b));
                let (color, weight) = match (axis, from[3] > 0.0) {
                    (3, _) => (PAST_CELL, 0.35),
                    (_, true) => (NOW_CELL, 0.55),
                    (_, false) => (PAST_CELL, 0.45),
                };
                let (from, to) = (projection.world(from).0, projection.world(to).0);
                stage
                    .frame
                    .hairline(stage.camera, from, to, tint(color, weight), FRAME_STROKE);
            }
        }
        // A process at exactly one core runs along this diagonal: c.
        let cone = |k: usize| -> Point4 {
            let v = -1.0 + 2.0 * k as f32 / 40.0;
            [v, 1.0, 1.0, v]
        };
        for k in (0..40).step_by(2) {
            let (from, to) = (projection.world(cone(k)).0, projection.world(cone(k + 1)).0);
            stage
                .frame
                .hairline(stage.camera, from, to, tint(LIGHT_CONE, 0.6), FRAME_STROKE);
        }
        stage
            .places
            .push((projection.world(cone(14)).0, "c = one core".into()));
        stage
            .places
            .push((projection.world([-1.0, -1.0, -1.0, 1.0]).0, "now".into()));
        stage.places.push((
            projection.world([-1.0, -1.0, -1.0, -1.0]).0,
            "60 s ago".into(),
        ));
    }

    /// The lab's "now" cell as the boosted frame sees it: no longer simultaneous.
    fn draw_lab_now(stage: &mut Stage, projection: &Projection) {
        let corner = |index: usize| -> Point4 {
            let axis = |bit: usize| if index >> bit & 1 == 1 { 1.0 } else { -1.0 };
            [axis(0), axis(1), axis(2), 1.0]
        };
        for a in 0..8_usize {
            for bit in 0..3 {
                let b = a | 1 << bit;
                if b != a {
                    let from = projection.place(corner(a));
                    let to = projection.place(corner(b));
                    stage
                        .frame
                        .hairline(stage.camera, from, to, tint(NOW_CELL, 0.3), FRAME_STROKE);
                }
            }
        }
    }
}

/// A track projected for drawing: its boosted 4D points and their world points with their
/// perspective factors.
struct Drawn<'a> {
    track: &'a Track,
    boosted: Vec<Point4>,
    world: Vec<(Point, f32)>,
}

impl ViewModule for Tesseract {
    #[expect(
        clippy::too_many_lines,
        reason = "the frame's drawing phases in their order"
    )]
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        self.lines.record(snapshot, stage.time);
        let now = snapshot.elapsed;
        let live = self.lines.last == Some(now);
        self.scroll = if !live {
            0.0
        } else if stage.dt > 0.0 {
            f64::from(stage.time - self.lines.arrived).clamp(0.0, self.lines.interval)
        } else {
            self.scroll.min(self.lines.interval)
        };
        let window = Window {
            now,
            display: now + self.scroll,
        };
        let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        let names: HashMap<Identity, &str> = processes
            .iter()
            .map(|process| (process.id, process.name.as_str()))
            .collect();
        let mut tracks: Vec<Track> = processes
            .iter()
            .filter_map(|process| self.lines.track(process.id, window, true))
            .collect();
        if stage.focus.is_none() {
            let room = stage.limit.saturating_sub(processes.len());
            let exited = self.lines.exited(&alive, window);
            tracks.extend(
                exited
                    .into_iter()
                    .take(room)
                    .filter_map(|id| self.lines.track(id, window, false)),
            );
        }
        self.steer(stage, &tracks, &names);
        let angles = self.orientation.angles(stage.time, stage.dt);
        let projection = Projection {
            rotation: rotation(angles),
            rapidity: self.boost.rapidity,
            pivot: self.boost.pivot,
        };
        let drawn: Vec<Drawn> = tracks
            .iter()
            .map(|track| {
                let boosted: Vec<Point4> = track
                    .points
                    .iter()
                    .map(|&point| projection.boosted(point))
                    .collect();
                let world = boosted
                    .iter()
                    .map(|&point| projection.world(point))
                    .collect();
                Drawn {
                    track,
                    boosted,
                    world,
                }
            })
            .collect();
        // The selected process's worldline lights up, with the fork vertices of its ancestors.
        let mut ancestors = HashSet::new();
        let mut ancestor = stage.selected;
        while let Some(id) = ancestor {
            if !ancestors.insert(id) {
                break;
            }
            ancestor = self.lines.lines.get(&id).and_then(|line| line.parent);
        }
        // Drawn in phases: hairlines, spheres, then glows, which blend over both.
        Self::draw_frame(stage, &projection);
        if self.boost.rapidity.abs() > 0.01 {
            Self::draw_lab_now(stage, &projection);
        }
        for drawn in &drawn {
            let lit = stage.selected == Some(drawn.track.id);
            let color = self.lines.lines[&drawn.track.id].color;
            for (index, pair) in drawn.track.points.windows(2).enumerate() {
                let tone = if lit {
                    SELECTED
                } else if slope(pair[0], pair[1]) > SUPERLUMINAL {
                    CHERENKOV
                } else {
                    let middle = (drawn.boosted[index][3] + drawn.boosted[index + 1][3]) * 0.5;
                    tint(color, shade(middle))
                };
                stage.frame.hairline(
                    stage.camera,
                    drawn.world[index].0,
                    drawn.world[index + 1].0,
                    tone,
                    STROKE,
                );
            }
        }
        // Disk I/O marks the worldline where it happened, newest first.
        let mut marks: Vec<(f64, Point4, Point4)> = tracks
            .iter()
            .flat_map(|track| track.io.iter().copied())
            .collect();
        if marks.len() > IO_MARKS {
            marks.select_nth_unstable_by(IO_MARKS, |a, b| b.0.total_cmp(&a.0));
            marks.truncate(IO_MARKS);
        }
        for (_, from, to) in marks {
            let (from, to) = (projection.place(from), projection.place(to));
            stage
                .frame
                .hairline(stage.camera, from, to, IO_COLOR, IO_WIDTH);
        }
        let index: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(index, process)| (process.id, index))
            .collect();
        let heads: Vec<(&Drawn, usize)> = drawn
            .iter()
            .filter(|drawn| drawn.track.alive)
            .filter_map(|drawn| Some((drawn, *index.get(&drawn.track.id)?)))
            .collect();
        // The Cherenkov cone trails a superluminal head at cos(theta) = 1 / beta.
        for &(drawn, _) in &heads {
            let beta = drawn.track.recent;
            if beta <= SUPERLUMINAL {
                continue;
            }
            let (Some(&head), Some(&(end, _))) = (drawn.track.points.last(), drawn.world.last())
            else {
                continue;
            };
            let angle = (1.0 / beta).acos();
            for beam in 0..CONE_BEAMS {
                let around = beam as f32 / CONE_BEAMS as f32 * TAU;
                let tip = [
                    head[0] - CONE_LENGTH * angle.cos(),
                    head[1] + CONE_LENGTH * angle.sin() * around.cos(),
                    head[2] + CONE_LENGTH * angle.sin() * around.sin(),
                    head[3],
                ];
                let tip = projection.place(tip);
                stage
                    .frame
                    .hairline(stage.camera, end, tip, tint(CHERENKOV, 0.6), FRAME_STROKE);
            }
        }
        self.shown.clear();
        for &(drawn, slot) in &heads {
            let process = processes[slot];
            let id = process.id;
            let (Some(&head), Some(&(end, factor)), Some(boosted)) = (
                drawn.track.points.last(),
                drawn.world.last(),
                drawn.boosted.last(),
            ) else {
                continue;
            };
            let radius = factor * (0.6 + 0.3 * (head[1] + 1.0)) * stage.growth(id);
            let color = tint(kind_color(process), shade(boosted[3]));
            let selected = stage.selected == Some(id);
            stage
                .frame
                .sphere(stage.camera, end, radius, color, slot as u32, selected);
            stage.positions.insert(id, end);
            let seat = self.lines.lines[&id].seat;
            self.shown.push((id, self.lines.bands.z(seat)));
            if selected {
                stage.notes.insert(
                    id,
                    vec![
                        describe_slope(drawn.track.recent),
                        format!(
                            "seat {} of {} in the {} band",
                            seat.index + 1,
                            self.lines.bands.capacity(seat.band),
                            kind_name(seat.band)
                        ),
                    ],
                );
            }
        }
        for drawn in &drawn {
            let track = drawn.track;
            if let Some(fork) = track.fork {
                let point = projection.place(fork);
                let (radius, strength) = if ancestors.contains(&track.id) {
                    (6.0, 1.0)
                } else {
                    (4.0, 0.6)
                };
                stage
                    .frame
                    .glow(stage.camera, point, radius, FORK, strength);
            }
            let Some(&(end, factor)) = drawn.world.last() else {
                continue;
            };
            if !track.alive {
                stage
                    .frame
                    .glow(stage.camera, end, 5.0, [255, 255, 255], 0.25);
            } else if track.recent > SUPERLUMINAL {
                let pixels = factor * stage.camera.zoom * 3.5;
                stage
                    .frame
                    .glow(stage.camera, end, pixels.min(CHERENKOV_CAP), CHERENKOV, 0.6);
            }
        }
        self.rotation = Some(projection.rotation);
        stage
            .bounds
            .extend(ring_bounds([0.0, 0.0], 12.0, -10.0, 10.0));
        heads.len()
    }

    fn record(&mut self, snapshot: &Snapshot, time: f32) {
        self.lines.record(snapshot, time);
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let reference = match &self.boost.reference {
            Reference::Lab => "lab frame".to_owned(),
            Reference::Rest(name) => format!("rest frame of {name}"),
            Reference::None(name, beta) => format!("no rest frame: {name} beta {beta:.1}"),
        };
        let mut legend = format!(
            " x = CPU-s (30/unit, slope = CPU) | y = {MEMORY_LABEL} | z = seat | w = time | blue = superluminal | plane {} | {reference}",
            PLANES[self.orientation.plane]
        );
        let depth = self.lines.depth(snapshot.elapsed);
        if depth < WINDOW - 0.5 {
            legend += &format!(" | window {depth:.0}s");
        }
        let unmeasured = snapshot
            .processes
            .iter()
            .filter(|process| process.io_rate.is_none())
            .count();
        legend += &unreadable_note("I/O", unmeasured).unwrap_or_default();
        legend
    }

    fn key(&mut self, key: ViewKey) -> bool {
        match key {
            ViewKey::Local(',') => self.orientation.turn(-1.0),
            ViewKey::Local('.') => self.orientation.turn(1.0),
            ViewKey::Local(';') => self.orientation.cycle(),
            ViewKey::Local('b') => {
                self.boost.request = match self.boost.request {
                    Request::Off => Request::Wanted,
                    Request::Wanted | Request::On(_) => Request::Off,
                };
            }
            ViewKey::Local(_) => {}
            ViewKey::Reset => {
                self.orientation.reset();
                self.boost.request = Request::Off;
            }
            ViewKey::Escape => {
                return std::mem::take(&mut self.boost.request) != Request::Off;
            }
        }
        false
    }

    /// Each head's seat on the lab frame's "now" cell: x and y are measurements (CPU-seconds
    /// and memory), so the seat is what must stay put.
    #[cfg(test)]
    fn anchors(&self) -> Option<HashMap<Identity, Point>> {
        let rotation = self.rotation?;
        let projection = Projection {
            rotation,
            rapidity: 0.0,
            pivot: [0.0; 2],
        };
        Some(
            self.shown
                .iter()
                .map(|&(id, z)| (id, projection.world([-1.0, -1.0, z, 1.0]).0))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DemoOptions, demo_with, testing::process};
    use crate::render::{Camera, Item, NONE, RenderInput, Scene};
    use crate::views::view;

    const EXACT: DemoOptions = DemoOptions {
        churn: true,
        exact_cpu_time: true,
        observer: false,
    };

    fn snapshot(elapsed: f64, processes: Vec<Process>) -> Snapshot {
        Snapshot {
            elapsed,
            processes,
            ..Snapshot::default()
        }
    }

    /// A process with `cpu_time` CPU-seconds and `memory` bytes.
    fn busy(pid: u32, parent: u32, cpu_time: f32, memory: u64) -> Process {
        let mut process = process(pid, parent);
        process.cpu_time = cpu_time;
        process.memory = memory;
        process
    }

    /// Records one sample per second from `from` to `to` of `processes(t)`.
    fn replay(lines: &mut Worldlines, from: u32, to: u32, processes: impl Fn(f64) -> Vec<Process>) {
        for second in from..=to {
            let t = f64::from(second);
            lines.record(&snapshot(t, processes(t)), t as f32);
        }
    }

    fn window(now: f64) -> Window {
        Window { now, display: now }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_process_using_one_core_follows_the_light_cone() {
        let mut lines = Worldlines::default();
        replay(&mut lines, 0, 60, |t| vec![busy(10, 1, t as f32, 1 << 20)]);
        let track = lines
            .track(process(10, 1).id, window(60.0), true)
            .expect("recorded");
        for point in &track.points {
            assert!(close(point[0], point[3]), "x = w along the cone: {point:?}");
        }
        let (first, head) = (track.points[0], *track.points.last().unwrap());
        assert!(close(first[0], -1.0) && close(first[3], -1.0), "{first:?}");
        assert!(close(head[0], 1.0) && close(head[3], 1.0), "{head:?}");
        assert_eq!(track.points.len(), 2, "one straight run is one segment");
        assert!(close(track.chord, 1.0) && close(track.recent, 1.0));
    }

    #[test]
    fn single_threaded_processes_never_leave_the_light_cone() {
        let mut lines = Worldlines::default();
        for second in 0..=90 {
            let t = f64::from(second);
            lines.record(&demo_with(t, 192, EXACT), t as f32);
        }
        let last = demo_with(90.0, 192, EXACT);
        let single: Vec<&Process> = last.processes.iter().filter(|p| p.threads == 1).collect();
        assert!(single.len() > 20, "the demo has single-threaded processes");
        for process in single {
            let track = lines
                .track(process.id, window(90.0), true)
                .expect("recorded");
            for pair in track.points.windows(2) {
                assert!(
                    slope(pair[0], pair[1]) <= 1.0 + 1e-3,
                    "{} is faster than light: {pair:?}",
                    process.name
                );
            }
        }
    }

    #[test]
    fn rotations_preserve_lengths() {
        let matrix = rotation([0.7, -1.3, 2.1]);
        for point in [[1.0, 0.0, 0.0, 0.0], [0.3, -0.8, 1.7, -2.0], [1.0; 4]] {
            let length = |p: Point4| p.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!(close(length(rotate(&matrix, point)), length(point)));
        }
    }

    #[test]
    fn a_quarter_turn_in_xw_swaps_time_and_cpu() {
        let matrix = rotation([FRAC_PI_2, 0.0, 0.0]);
        let cpu = rotate(&matrix, [1.0, 0.0, 0.0, 0.0]);
        let time = rotate(&matrix, [0.0, 0.0, 0.0, 1.0]);
        assert!(
            close(cpu[3], 1.0) && close(cpu[0], 0.0),
            "CPU becomes w: {cpu:?}"
        );
        assert!(
            close(time[0], -1.0) && close(time[3], 0.0),
            "time becomes x: {time:?}"
        );
        let other = rotate(&matrix, [0.0, 0.4, -0.2, 0.0]);
        assert!(close(other[1], 0.4) && close(other[2], -0.2));
    }

    #[test]
    fn boosts_preserve_the_interval() {
        let pivot = [0.3, 1.0];
        let interval = |p: Point4| (p[3] - pivot[1]).powi(2) - (p[0] - pivot[0]).powi(2);
        for rapidity in [-1.2, 0.4, 2.0] {
            for point in [
                [0.0, 0.5, 0.1, -1.0],
                [1.7, 0.0, 0.0, 0.2],
                [-0.4, 0.0, 0.0, 0.9],
            ] {
                let boosted = boost(point, pivot, rapidity);
                assert!(
                    (interval(boosted) - interval(point)).abs() < 1e-3,
                    "{point:?} by {rapidity}"
                );
                assert_eq!([boosted[1], boosted[2]], [point[1], point[2]]);
            }
        }
    }

    #[test]
    fn boosting_into_a_process_leaves_it_at_rest() {
        let mut lines = Worldlines::default();
        replay(&mut lines, 0, 60, |t| {
            vec![busy(10, 1, 0.6 * t as f32, 1 << 20)]
        });
        let track = lines.track(process(10, 1).id, window(60.0), true).unwrap();
        let rapidity = rest_rapidity(track.chord).expect("slower than light");
        let head = *track.points.last().unwrap();
        for point in &track.points {
            let boosted = boost(*point, [head[0], head[3]], rapidity);
            assert!(
                close(boosted[0], head[0]),
                "the worldline stands still: {boosted:?}"
            );
        }
    }

    #[test]
    fn no_rest_frame_exists_for_a_superluminal_process() {
        let mut lines = Worldlines::default();
        replay(&mut lines, 0, 60, |t| {
            vec![busy(10, 1, 2.3 * t as f32, 1 << 20)]
        });
        let track = lines.track(process(10, 1).id, window(60.0), true).unwrap();
        assert!(close(track.chord, 2.3));
        assert_eq!(rest_rapidity(track.chord), None);
        assert_eq!(rest_rapidity(0.999), None, "lightlike has none either");
        assert!(rest_rapidity(0.5).is_some());
        assert!(describe_slope(2.3).starts_with("superluminal: beta 2.3, Cherenkov angle 64 deg"));
        assert_eq!(describe_slope(0.98), "slope 0.98 c (lightlike)");
        assert_eq!(describe_slope(0.0), "at rest");
    }

    #[test]
    fn worldlines_keep_their_seat_when_resources_change() {
        let mut lines = Worldlines::default();
        let population = |t: f64, scale: f32| -> Vec<Process> {
            (0..40)
                .map(|pid| {
                    busy(
                        100 + pid,
                        1,
                        scale * t as f32 * pid as f32,
                        (pid as u64 + 1) << 22,
                    )
                })
                .collect()
        };
        replay(&mut lines, 0, 10, |t| population(t, 0.01));
        let before: Vec<Option<f32>> = (0..40)
            .map(|pid| lines.seat(process(100 + pid, 1).id))
            .collect();
        replay(&mut lines, 11, 20, |t| {
            let mut processes = population(t, 0.5);
            for process in &mut processes {
                process.memory *= 7;
            }
            processes
        });
        let after: Vec<Option<f32>> = (0..40)
            .map(|pid| lines.seat(process(100 + pid, 1).id))
            .collect();
        assert!(before.iter().all(Option::is_some));
        assert_eq!(before, after);
    }

    #[test]
    fn seats_survive_population_changes_until_the_worldline_leaves_the_window() {
        let mut lines = Worldlines::default();
        let id = |pid: u32| process(pid, 1).id;
        replay(&mut lines, 0, 5, |t| {
            (0..10).map(|pid| busy(100 + pid, 1, t as f32, 1)).collect()
        });
        let seats: Vec<Option<f32>> = (0..10).map(|pid| lines.seat(id(100 + pid))).collect();
        // The first process exits at 6 s and a newcomer arrives.
        let after = |t: f64| -> Vec<Process> {
            (1..10)
                .chain([20])
                .map(|pid| busy(100 + pid, 1, t as f32, 1))
                .collect()
        };
        replay(&mut lines, 6, 30, after);
        for pid in 1..10 {
            assert_eq!(
                lines.seat(id(100 + pid)),
                seats[pid as usize],
                "survivors keep theirs"
            );
        }
        assert_eq!(
            lines.seat(id(100)),
            seats[0],
            "the exited one holds its seat"
        );
        assert_ne!(
            lines.seat(id(120)),
            seats[0],
            "the newcomer takes a free one"
        );
        replay(&mut lines, 31, 66, after);
        assert_eq!(
            lines.seat(id(100)),
            None,
            "a minute after its last sample it is free"
        );
        lines.record(
            &snapshot(67.0, [after(67.0), vec![busy(300, 1, 0.0, 1)]].concat()),
            67.0,
        );
        assert_eq!(
            lines.seat(id(300)),
            seats[0],
            "the lowest free seat is taken again"
        );
    }

    #[test]
    fn children_branch_from_their_parent_worldline() {
        let mut lines = Worldlines::default();
        let parent = busy(50, 1, 0.0, 400 << 20);
        let processes = |t: f64| -> Vec<Process> {
            let mut parent = parent.clone();
            parent.cpu_time = 0.5 * t as f32;
            let mut all = vec![parent];
            if t >= 30.0 {
                let mut child = busy(51, 50, 100.0 + 0.9 * (t - 30.0) as f32, 1 << 20);
                child.id.start = 30;
                all.push(child);
            }
            all
        };
        replay(&mut lines, 0, 50, processes);
        let mut child_id = process(51, 50).id;
        child_id.start = 30;
        let child = lines
            .track(child_id, window(50.0), true)
            .expect("the child");
        let parent = lines
            .track(parent.id, window(50.0), true)
            .expect("the parent");
        let fork = child.fork.expect("a fork vertex");
        assert!(close(fork[3], 1.0 - 20.0 / 30.0), "at the birth: {fork:?}");
        // The parent's worldline at the birth: 15 CPU-seconds since it started at 0 s, inside
        // the window, so x = -1 + 15 / 30; its memory and its seat.
        let on_parent = [-0.5, parent.points[0][1], parent.points[0][2]];
        for axis in 0..3 {
            assert!(
                close(fork[axis], on_parent[axis]),
                "{fork:?} vs {on_parent:?}"
            );
            assert!(close(child.points[0][axis], on_parent[axis]));
        }
        let head = *child.points.last().unwrap();
        assert!(
            close(head[2], lines.seat(child_id).unwrap()),
            "it reached its own seat"
        );
        assert!(
            close(head[0], fork[0] + 0.9 * 20.0 / 30.0),
            "its own CPU from there"
        );
        let mut orphan = busy(52, 999, 0.0, 1);
        orphan.id.start = 40;
        lines.record(
            &snapshot(51.0, [processes(51.0), vec![orphan.clone()]].concat()),
            51.0,
        );
        lines.record(
            &snapshot(52.0, [processes(52.0), vec![orphan.clone()]].concat()),
            52.0,
        );
        let orphan = lines.track(orphan.id, window(52.0), true).unwrap();
        assert!(orphan.fork.is_none(), "an unknown parent is no fork");
    }

    #[test]
    fn exited_processes_keep_their_worldline_until_it_leaves_the_window() {
        let mut lines = Worldlines::default();
        let id = process(10, 1).id;
        replay(&mut lines, 0, 20, |t| {
            vec![busy(10, 1, t as f32, 1), busy(11, 1, 0.0, 1)]
        });
        replay(&mut lines, 21, 50, |_| vec![busy(11, 1, 0.0, 1)]);
        let alive = HashSet::from([process(11, 1).id]);
        assert_eq!(lines.exited(&alive, window(50.0)), vec![id]);
        let track = lines
            .track(id, window(50.0), false)
            .expect("still in the window");
        assert!(!track.alive);
        assert!(
            close(track.points.last().unwrap()[3], 1.0 - 30.0 / 30.0),
            "it ends at its exit"
        );
        replay(&mut lines, 51, 81, |_| vec![busy(11, 1, 0.0, 1)]);
        assert!(lines.exited(&alive, window(81.0)).is_empty());
        assert!(
            lines.track(id, window(81.0), false).is_none(),
            "it slid out the kata side"
        );
    }

    #[test]
    fn rewinding_draws_the_minute_before_the_paused_snapshot() {
        let mut lines = Worldlines::default();
        replay(&mut lines, 0, 120, |t| {
            vec![busy(10, 1, if t < 60.0 { 0.0 } else { t as f32 - 60.0 }, 1)]
        });
        let paused = lines.track(process(10, 1).id, window(60.0), true).unwrap();
        assert!(
            paused.points.iter().all(|point| close(point[0], -1.0)),
            "the minute before 60 s was idle: {:?}",
            paused.points
        );
        let newest = lines.track(process(10, 1).id, window(120.0), true).unwrap();
        assert!(close(newest.points.last().unwrap()[0], 1.0));
        let before = lines.lines[&process(10, 1).id].samples.len();
        lines.record(&snapshot(60.0, vec![busy(10, 1, 0.0, 1)]), 200.0);
        assert_eq!(
            lines.lines[&process(10, 1).id].samples.len(),
            before,
            "a rewind records nothing"
        );
    }

    #[test]
    fn heads_are_pickable_and_worldlines_are_not() {
        let tesseract = view("tesseract");
        let mut scene = Scene::new();
        let camera = Camera::default();
        for second in 0..=20 {
            scene.record(&demo_with(f64::from(second), 64, EXACT), second as f32);
        }
        let snapshot = demo_with(20.0, 64, EXACT);
        let mut frame = scene.render(&RenderInput {
            width: 640,
            height: 360,
            ..RenderInput::small(&snapshot, tesseract, &camera, 20.0)
        });
        let mut fitted = camera.clone();
        scene.fit(&mut fitted, 640, 360);
        let mut frame_fitted = scene.render(&RenderInput {
            width: 640,
            height: 360,
            ..RenderInput::small(&snapshot, tesseract, &fitted, 20.1)
        });
        frame.rasterize();
        frame_fitted.rasterize();
        let spheres = frame_fitted
            .items
            .iter()
            .filter(|item| matches!(item, Item::Sphere { pick, .. } if *pick != NONE))
            .count();
        assert_eq!(
            spheres,
            scene.positions.len(),
            "one pickable head per process"
        );
        let strokes: Vec<[Point; 3]> = frame_fitted
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(points, _, pick) => {
                    assert_eq!(*pick, NONE, "a worldline is never pickable");
                    Some(*points)
                }
                _ => None,
            })
            .collect();
        assert!(!strokes.is_empty(), "worldlines are drawn");
        let picked: HashSet<Identity> = frame_fitted
            .picks
            .iter()
            .filter(|&&pick| pick != NONE)
            .map(|&pick| frame_fitted.identities[pick as usize])
            .collect();
        assert!(
            picked.len() > snapshot.processes.len() / 4,
            "{} heads pick",
            picked.len()
        );
        // Where a worldline is drawn far from any head, nothing picks.
        let heads: Vec<[f32; 2]> = scene
            .positions
            .values()
            .filter_map(|&p| frame_fitted.locate(&fitted, p))
            .collect();
        let lonely = strokes.iter().find_map(|corners| {
            let x = (corners[0][0] + corners[1][0] + corners[2][0]) / 3.0;
            let y = (corners[0][1] + corners[1][1] + corners[2][1]) / 3.0;
            let far = heads.iter().all(|h| (h[0] - x).hypot(h[1] - y) > 30.0);
            let inside = (0.0..640.0).contains(&x) && (0.0..360.0).contains(&y);
            (far && inside).then_some([x, y])
        });
        let middle = lonely.expect("a worldline away from the heads");
        let pixel = middle[1] as usize * 640 + middle[0] as usize;
        assert_eq!(
            frame_fitted.picks[pixel], NONE,
            "a worldline is not pickable"
        );
    }

    #[test]
    fn the_tesseract_renders_the_same_frame_twice_from_the_same_inputs() {
        let draw = || {
            let mut scene = Scene::new();
            let camera = Camera::default();
            for second in 0..=30 {
                scene.record(&demo_with(f64::from(second), 96, EXACT), second as f32);
            }
            let snapshot = demo_with(30.0, 96, EXACT);
            let mut frames = Vec::new();
            for step in 0..4 {
                let mut frame = scene.render(&RenderInput::small(
                    &snapshot,
                    view("tesseract"),
                    &camera,
                    30.0 + step as f32 * 0.25,
                ));
                frame.rasterize();
                frames.push((frame.pixels, frame.picks));
            }
            frames
        };
        assert!(draw() == draw());
    }

    #[test]
    fn the_orientation_eases_alike_at_ten_and_sixty_frames_per_second() {
        let run = |fps: u32| {
            let mut orientation = Orientation::default();
            orientation.angles(0.0, 0.0);
            orientation.turn(1.0);
            let mut angles = [0.0; 3];
            for frame in 1..=fps * 2 {
                angles = orientation.angles(frame as f32 / fps as f32, 1.0 / fps as f32);
            }
            angles
        };
        let (slow, fast) = (run(10), run(60));
        for plane in 0..3 {
            assert!(
                (slow[plane] - fast[plane]).abs() < 1e-3,
                "{slow:?} vs {fast:?}"
            );
        }
    }

    #[test]
    fn the_legend_names_the_mappings_the_frame_the_window_and_unread_io() {
        let mut tesseract = Tesseract::default();
        let mut quiet = busy(10, 1, 0.0, 1);
        quiet.io_rate = None;
        let mut reading = busy(11, 1, 0.0, 1);
        reading.io_rate = Some(0.0);
        tesseract.record(&snapshot(0.0, vec![quiet.clone(), reading.clone()]), 0.0);
        let at = |elapsed: f64| snapshot(elapsed, vec![quiet.clone(), reading.clone()]);
        let mappings = format!(
            " x = CPU-s (30/unit, slope = CPU) | y = {MEMORY_LABEL} | z = seat | w = time | blue = superluminal"
        );
        assert_eq!(
            tesseract.legend(&at(12.0)),
            format!("{mappings} | plane XW | lab frame | window 12s | I/O unreadable for 1")
        );
        assert_eq!(
            tesseract.legend(&at(75.0)),
            format!("{mappings} | plane XW | lab frame | I/O unreadable for 1")
        );
        tesseract.boost.reference = Reference::Rest("worker".into());
        assert!(
            tesseract
                .legend(&at(75.0))
                .contains("| rest frame of worker |")
        );
        tesseract.boost.reference = Reference::None("cc".into(), 2.34);
        assert!(
            tesseract
                .legend(&at(75.0))
                .contains("| no rest frame: cc beta 2.3 |")
        );
    }

    #[test]
    fn the_tour_boosts_into_a_rest_frame_or_says_there_is_none() {
        let tesseract = view("tesseract");
        let (slow, fast) = (busy(10, 1, 0.0, 1 << 24), busy(11, 1, 0.0, 1 << 24));
        let at = |t: f64| {
            let (mut slow, mut fast) = (slow.clone(), fast.clone());
            slow.cpu_time = 0.5 * t as f32;
            fast.cpu_time = 2.3 * t as f32;
            snapshot(t, vec![slow, fast])
        };
        let camera = Camera::default();
        let last = at(60.0);
        let scene = |toured: Option<Identity>, frames: u32| {
            let mut scene = Scene::new();
            for second in 0..60 {
                scene.record(&at(f64::from(second)), second as f32);
            }
            for step in 0..frames {
                scene.render(&RenderInput {
                    toured,
                    ..RenderInput::small(&last, tesseract, &camera, 60.0 + step as f32 * 0.25)
                });
            }
            scene
        };
        let superluminal = scene(Some(fast.id), 1);
        assert_eq!(
            superluminal.callout,
            vec![(fast.id, "no rest frame".to_owned())]
        );
        assert!(
            superluminal
                .legend(tesseract, &last)
                .contains("| no rest frame: ")
        );
        let (boosted, lab) = (scene(Some(slow.id), 24), scene(None, 24));
        assert!(boosted.callout.is_empty());
        assert!(
            boosted
                .legend(tesseract, &last)
                .contains("| rest frame of ")
        );
        let near = |a: Point, b: Point| (0..3).all(|axis| (a[axis] - b[axis]).abs() < 1e-2);
        assert!(
            near(boosted.positions[&slow.id], lab.positions[&slow.id]),
            "the boost turns about the toured head"
        );
        assert!(!near(boosted.positions[&fast.id], lab.positions[&fast.id]));
    }

    #[test]
    fn keys_turn_cycle_boost_and_reset() {
        let mut tesseract = Tesseract::default();
        let snapshot = Snapshot::default();
        assert!(
            tesseract
                .legend(&snapshot)
                .contains("| plane XW | lab frame")
        );
        assert!(!tesseract.key(ViewKey::Local(';')));
        assert!(tesseract.legend(&snapshot).contains("| plane YW |"));
        assert!(!tesseract.key(ViewKey::Escape), "nothing to leave");
        tesseract.key(ViewKey::Local('b'));
        assert!(
            tesseract.key(ViewKey::Escape),
            "the first Esc leaves the boost"
        );
        assert!(!tesseract.key(ViewKey::Escape), "the next one is global");
        tesseract.key(ViewKey::Local('.'));
        tesseract.key(ViewKey::Reset);
        let angles = tesseract.orientation.angles(10.0, 0.0);
        assert_eq!(tesseract.orientation.goal, [0.0; 3]);
        assert!(!tesseract.orientation.stopped, "the drift comes back");
        assert!((angles[1] - CANONICAL[1]).abs() < 1e-6);
        assert!(tesseract.legend(&snapshot).contains("| plane XW |"));
    }
}
