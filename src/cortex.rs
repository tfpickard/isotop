//! The Cortex view: every process is an Izhikevich (2003) neuron in a layered slice of cortex.
//! Its kind sets the layer and the firing pattern, its CPU sets the current that drives it, its
//! threads are dendrites, and its socket and pipe links are axons that carry delayed spikes. A
//! raster of the last ten seconds runs along the bottom, and the legend reports Golomb's
//! synchrony of the neurons that talk.
//!
//! `neurons` holds the simulation. Here are the layout (a seat in a layer, fixed for the life of
//! the process), the axons built from the sample's links, the drawing, and the raster panel,
//! which is drawn in screen space over the bottom of the frame.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::f32::consts::{FRAC_1_SQRT_2, TAU};

use crate::glyphs;
use crate::layout::Seats;
use crate::model::{Identity, Kind, Process, Snapshot};
use crate::neurons::{self, Firing, Network, STEP_SECONDS, Synapse, TICKS_PER_SECOND};
use crate::palette::{self, mix, tint};
use crate::platform::Source;
use crate::render::{Camera, Color, Frame, Item, Links, NONE, Point, Stage, TourTarget};
use crate::simulation::{FixedStep, Rng, smoothstep, sprout};
use crate::status::missing_note;
use crate::views::ViewModule;

/// Separates this view's per-process random streams from every other use of the identity hash.
const SEAT_SALT: u64 = 0x636f_7274_6578_0001;
const DENDRITE_SALT: u64 = 0x636f_7274_6578_0100;
const APICAL_SALT: u64 = 0x636f_7274_6578_0200;
const DUST_SALT: u64 = 0x636f_7274_6578_0300;
const AXON_SALT: u64 = 0x636f_7274_6578_0400;

/// The slice's height, and its width before it ever widens, in world units.
const SLICE_HEIGHT: f32 = 56.0;
const BASE_WIDTH: f32 = 100.0;
/// A layer that outgrows the slice widens it by this factor, and it never narrows again.
const WIDENING: f32 = 1.25;
/// The room one neuron is given, side to side, when a layer's capacity is counted.
const SPACING: f32 = 2.2;
/// The margin between the slice's side and its neurons, and the label column left of it.
const SIDE_MARGIN: f32 = 4.0;
const LABEL_MARGIN: f32 = 11.0;
/// Axons conduct at the speed that crosses the unwidened slice in 0.8 s, whatever the zoom or
/// the width, in world units per second.
const CONDUCTION: f32 = BASE_WIDTH / 0.8;

/// Seconds the simulation may be asked to catch up in one frame; a longer gap is dropped.
const CATCH_UP: f32 = 0.1;
/// The raster shows this many seconds of spikes, and the simulation keeps a little more.
const RASTER_SECONDS: u64 = 10;
const RASTER_TICKS: u64 = RASTER_SECONDS * TICKS_PER_SECOND;
/// The raster draws a cell at one of this many brightnesses, by how many spikes fell in it.
const RASTER_LEVELS: u8 = 3;
/// The most spikes in flight drawn as pulses.
const MAX_PULSES: usize = 3000;
/// Above this many neurons, or when a soma is smaller than `SMALL_SOMA` pixels, only primary
/// dendrites are drawn.
const CROWD: usize = 400;
const SMALL_SOMA: f32 = 3.0;
/// The most primary dendrites a soma grows, the threads it draws.
const MAX_PRIMARIES: usize = 12;
/// The busiest neurons the tour visits.
const TOUR_STOPS: usize = 16;
/// Processes at or above this CPU, with a link, are the talking set chi is measured over.
const TALKING_CPU: f32 = 5.0;
/// The weight of a pipe, whose throughput neither platform measures: that of a typical socket.
const PIPE_WEIGHT: f32 = 1.0;
/// Seconds an exited soma takes to shrink, leaving its dust.
const DUST_SECONDS: f32 = 1.2;
const DUST_SPECKS: usize = 12;
/// A flash of a firing soma decays with this time constant, in seconds.
const FLASH_TAU: f32 = 0.06;

/// Depth biases, in depth units along the view direction, that stack what is drawn: strokes lie
/// behind the pulses, which lie behind the somas, which hide whatever passes under them.
const FIBRE_DEPTH: f32 = 0.2;
const DENDRITE_DEPTH: f32 = 0.6;
const AXON_DEPTH: f32 = 0.9;
const PULSE_DEPTH: f32 = 1.4;
const SOMA_DEPTH: f32 = 3.0;
/// The raster sits this far in front of the slice's nearest point.
const PANEL_LEAD: f32 = 6.0;
const DENDRITE_WIDTH: f32 = 1.3;
const AXON_WIDTH: f32 = 1.2;

/// One band of the slice: its name, what lives in it, its share of the height and its colour.
struct Layer {
    name: &'static str,
    caption: &'static str,
    kind: Option<Kind>,
    fraction: f32,
    fill: Color,
}

/// From the top: the molecular layer (empty), the layers of the session, containers, system and
/// kernel, and the white matter.
const LAYERS: [Layer; 6] = [
    Layer {
        name: "I",
        caption: "molecular",
        kind: None,
        fraction: 0.10,
        fill: [22, 14, 34],
    },
    Layer {
        name: "II/III",
        caption: "session CH",
        kind: Some(Kind::Session),
        fraction: 0.25,
        fill: [31, 18, 46],
    },
    Layer {
        name: "IV",
        caption: "containers IB",
        kind: Some(Kind::Container),
        fraction: 0.18,
        fill: [37, 22, 55],
    },
    Layer {
        name: "V",
        caption: "system RS",
        kind: Some(Kind::System),
        fraction: 0.25,
        fill: [28, 17, 43],
    },
    Layer {
        name: "VI",
        caption: "kernel FS",
        kind: Some(Kind::Kernel),
        fraction: 0.20,
        fill: [24, 15, 38],
    },
    Layer {
        name: "white matter",
        caption: "",
        kind: None,
        fraction: 0.07,
        fill: [17, 12, 27],
    },
];
/// The fractions add up to this, so the bands are scaled by it.
const FRACTION_SUM: f32 = 1.05;

/// The layer, as an index into `LAYERS`, that `kind` lives in.
fn layer_of(kind: Kind) -> usize {
    LAYERS
        .iter()
        .position(|layer| layer.kind == Some(kind))
        .unwrap_or(1)
}

/// How far below the slice's top a layer starts and ends, in world units.
fn band(layer: usize) -> (f32, f32) {
    let before: f32 = LAYERS[..layer].iter().map(|layer| layer.fraction).sum();
    let top = before / FRACTION_SUM * SLICE_HEIGHT;
    (
        top,
        top + LAYERS[layer].fraction / FRACTION_SUM * SLICE_HEIGHT,
    )
}

/// The world point `along` units along the slice and `down` below its top. The slice is an
/// upright plane facing the default camera: along runs across the screen and down runs down it.
fn world(along: f32, down: f32) -> Point {
    [
        along * FRAC_1_SQRT_2,
        -along * FRAC_1_SQRT_2,
        SLICE_HEIGHT - down,
    ]
}

/// The kind's colour: the palette's base, without the per-pid jitter.
fn kind_base(kind: Kind) -> Color {
    match kind {
        Kind::Kernel => palette::KERNEL,
        Kind::System => palette::SYSTEM,
        Kind::Session => palette::SESSION,
        Kind::Container => palette::CONTAINER,
    }
}

/// The slice, as wide as its fullest layer has needed so far.
#[derive(Clone, Copy)]
struct Slice {
    widenings: u32,
}

impl Slice {
    fn width(self) -> f32 {
        BASE_WIDTH * WIDENING.powi(self.widenings as i32)
    }

    /// How many neurons `layer` holds at this width.
    fn capacity(self, layer: usize) -> usize {
        let (top, bottom) = band(layer);
        let room = (self.width() - 2.0 * SIDE_MARGIN) * (bottom - top) * 0.72;
        (room / (SPACING * SPACING)) as usize
    }

    /// Where seat `seat` of `layer` is, as (along, down): the seat's point of the R2 sequence,
    /// which spreads any number of seats evenly over the band, and a jitter in (-1, 1) each way.
    fn place(self, layer: usize, seat: usize, jitter: [f32; 2]) -> [f32; 2] {
        let (top, bottom) = band(layer);
        let n = (seat + 1) as f32;
        let (u, v) = ((0.754_877_7 * n).fract(), (0.569_840_3 * n).fract());
        let height = bottom - top;
        [
            SIDE_MARGIN + u * (self.width() - 2.0 * SIDE_MARGIN) + jitter[0] * 0.8,
            top + 0.14 * height + v * 0.72 * height + jitter[1] * 0.35,
        ]
    }
}

/// A dendrite branch: a polyline in the slice's plane, offset from its soma, and how many
/// forks deep it is.
struct Branch {
    points: Vec<[f32; 2]>,
    depth: u8,
}

/// Grows `points` from `start` along `angle`, bending a little at each of `steps` steps.
fn wiggle(
    rng: &mut Rng,
    start: [f32; 2],
    mut angle: f32,
    length: f32,
    steps: usize,
    curl: f32,
) -> (Vec<[f32; 2]>, f32) {
    let mut points = vec![start];
    let mut at = start;
    for _ in 0..steps {
        angle += curl * rng.normal();
        at = [
            at[0] + length / steps as f32 * angle.cos(),
            at[1] + length / steps as f32 * angle.sin(),
        ];
        points.push(at);
    }
    (points, angle)
}

/// A branch from `start` and, until two forks deep, the two it forks into.
fn fork(rng: &mut Rng, start: [f32; 2], angle: f32, length: f32, depth: u8, out: &mut Vec<Branch>) {
    let (points, end) = wiggle(rng, start, angle, length, 4, 0.2);
    let tip = points[points.len() - 1];
    out.push(Branch { points, depth });
    if depth < 2 && length > 0.9 {
        for side in [-1.0, 1.0] {
            let bend = side * (0.45 + 0.15 * rng.normal().abs());
            let shorter = length * (0.55 + 0.2 * rng.unit());
            fork(rng, tip, end + bend, shorter, depth + 1, out);
        }
    }
}

/// The `index`th primary dendrite of a soma of `firing` type and its forks. Its direction is
/// fixed by its index alone, so growing more threads never moves the ones already grown: a fan
/// across the lower half for the layered cells, all round for the fast-spiking kernel cells.
fn grow(id: Identity, index: usize, firing: Firing) -> Vec<Branch> {
    let mut rng = Rng::for_identity(id, DENDRITE_SALT + index as u64);
    let share = (0.618_034 * (index + 1) as f32).fract();
    let (angle, length) = if firing == Firing::FastSpiking {
        (TAU * share, 1.6 + 1.0 * rng.unit())
    } else {
        (
            std::f32::consts::PI * (0.08 + 0.84 * share),
            2.0 + 1.6 * rng.unit(),
        )
    };
    let mut branches = Vec::new();
    fork(&mut rng, [0.0, 0.0], angle, length, 0, &mut branches);
    branches
}

/// The apical dendrite of a layered cell: a trunk up to the molecular layer, a tuft where it
/// ends and a few side branches. It carries no data. `rise` is how far up the layer's top is.
fn apical(id: Identity, rise: f32) -> Vec<Branch> {
    let mut rng = Rng::for_identity(id, APICAL_SALT);
    let up = -std::f32::consts::FRAC_PI_2;
    let reach = (rise - 0.6 - 0.9 * rng.unit()).max(1.0);
    let (trunk, _) = wiggle(&mut rng, [0.0, 0.0], up, reach, 8, 0.05);
    let tip = trunk[trunk.len() - 1];
    let mut branches = Vec::new();
    for tuft in -1..=1 {
        let angle = up + tuft as f32 * 0.9 + 0.2 * rng.normal();
        let length = 1.2 + 1.0 * rng.unit();
        fork(&mut rng, tip, angle, length, 1, &mut branches);
    }
    for knot in (2..trunk.len() - 2).step_by(3) {
        let side = if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        let angle = up + side * (0.7 + 0.4 * rng.unit());
        let length = 1.0 + 0.8 * rng.unit();
        fork(&mut rng, trunk[knot], angle, length, 2, &mut branches);
    }
    branches.insert(
        0,
        Branch {
            points: trunk,
            depth: 0,
        },
    );
    branches
}

/// What the view keeps of one process: its neuron's slot and everything about it that does not
/// change with its CPU.
struct Body {
    id: Identity,
    firing: Firing,
    layer: usize,
    seat: usize,
    jitter: [f32; 2],
    /// Its soma, as (along, down) in the slice.
    at: [f32; 2],
    color: Color,
    /// Its distance into the slice, 0 to 1, which dims the cells at the back.
    far: f32,
    primaries: Vec<Vec<Branch>>,
    apical: Option<Vec<Branch>>,
    /// Its soma's radius in world units, from the memory it holds.
    radius: f32,
}

/// A neuron that left: it shrinks over `DUST_SECONDS` and leaves a handful of specks.
struct Dust {
    at: [f32; 2],
    color: Color,
    radius: f32,
    since: f32,
    seed: u64,
}

/// A link between two neurons, drawn as an arc below them.
struct Axon {
    ends: [Identity; 2],
    slots: [usize; 2],
    /// Where it runs, from `ends[0]`'s soma to `ends[1]`'s, in world units.
    points: Vec<Point>,
    cumulative: Vec<f32>,
}

impl Axon {
    fn length(&self) -> f32 {
        self.cumulative.last().copied().unwrap_or(0.0)
    }

    /// The point `fraction` of the way along it, by length.
    fn at(&self, fraction: f32) -> Point {
        let target = fraction.clamp(0.0, 1.0) * self.length();
        let next = self
            .cumulative
            .partition_point(|&reach| reach < target)
            .clamp(1, self.points.len() - 1);
        let (before, after) = (self.cumulative[next - 1], self.cumulative[next]);
        let t = if after > before {
            (target - before) / (after - before)
        } else {
            0.0
        };
        let (a, b) = (self.points[next - 1], self.points[next]);
        [0, 1, 2].map(|k| a[k] + t * (b[k] - a[k]))
    }
}

/// An arc from `from` to `to`, both (along, down), sagging below them: a cubic curve in the plane
/// of the slice, a little different for every pair of processes.
fn arc(
    ends: [Identity; 2],
    slots: [usize; 2],
    from: [f32; 2],
    to: [f32; 2],
    rng: &mut Rng,
) -> Axon {
    let sag = 2.5 + 0.12 * (from[0] - to[0]).abs();
    let floor = band(4).1;
    let lean = |rng: &mut Rng| 1.5 * rng.signed();
    let (p1, p2) = (
        [from[0] + lean(rng), (from[1] + sag).min(floor)],
        [to[0] + lean(rng), (to[1] + sag).min(floor)],
    );
    let bezier = |t: f32| {
        let u = 1.0 - t;
        [0, 1].map(|k| {
            u * u * u * from[k]
                + 3.0 * u * u * t * p1[k]
                + 3.0 * u * t * t * p2[k]
                + t * t * t * to[k]
        })
    };
    let points: Vec<Point> = (0..=24)
        .map(|step| {
            let [along, down] = bezier(step as f32 / 24.0);
            world(along, down)
        })
        .collect();
    let mut cumulative = vec![0.0];
    for pair in points.windows(2) {
        let step = (0..3)
            .map(|k| (pair[1][k] - pair[0][k]).powi(2))
            .sum::<f32>()
            .sqrt();
        cumulative.push(cumulative[cumulative.len() - 1] + step);
    }
    Axon {
        ends,
        slots,
        points,
        cumulative,
    }
}

/// The weight of a synapse from the bytes per second of its link: a quarter of the log of
/// the kilobytes, at most 2.
fn weight_of_rate(bytes_per_second: f32) -> f32 {
    if bytes_per_second.is_nan() || bytes_per_second <= 0.0 {
        return 0.0;
    }
    (0.25 * (1.0 + bytes_per_second / 1000.0).ln()).min(2.0)
}

/// The weight when traffic is not measured: the same log of the number of sockets of the link.
fn weight_of_sockets(sockets: u32) -> f32 {
    (0.25 * (1.0 + sockets as f32).ln()).min(2.0)
}

/// What the legend reports, from the last frame.
#[derive(Default)]
struct Summary {
    chi: Option<f32>,
    spikes: usize,
}

/// What the view remembers of the selected neuron between frames.
struct Watch {
    id: Identity,
    /// The axons its component was found in: it is found again when they are built again.
    built: Option<(u64, u64)>,
}

pub struct Cortex {
    network: Network,
    clock: FixedStep,
    seats: Seats,
    slice: Slice,
    bodies: Vec<Option<Body>>,
    slots: HashMap<Identity, usize>,
    axons: Vec<Axon>,
    /// The sample the axons were built from, and how many neurons had come and gone by then.
    built: Option<(u64, u64)>,
    changes: u64,
    talking: Vec<usize>,
    watch: Option<Watch>,
    dying: Vec<Dust>,
    summary: Summary,
    /// The raster's cells, how many spikes of their bin fell in them, reused from frame to frame.
    cells: Vec<u8>,
}

impl Default for Cortex {
    fn default() -> Self {
        Self {
            network: Network::default(),
            clock: FixedStep::new(STEP_SECONDS, (CATCH_UP / STEP_SECONDS) as usize, CATCH_UP),
            seats: Seats::default(),
            slice: Slice { widenings: 0 },
            bodies: Vec::new(),
            slots: HashMap::new(),
            axons: Vec::new(),
            built: None,
            changes: 0,
            talking: Vec::new(),
            watch: None,
            dying: Vec::new(),
            summary: Summary::default(),
            cells: Vec::new(),
        }
    }
}

/// Carries the camera and the viewer's direction, to lift points toward the viewer.
struct Ink<'a> {
    camera: &'a Camera,
    viewer: Point,
}

impl<'a> Ink<'a> {
    fn new(camera: &'a Camera) -> Self {
        Self {
            camera,
            viewer: camera.viewer(),
        }
    }

    /// `point` moved `bias` units toward the viewer, which changes its depth and nothing else on
    /// the screen.
    fn lift(&self, point: Point, bias: f32) -> Point {
        [0, 1, 2].map(|k| point[k] + bias * self.viewer[k])
    }

    fn stroke(&self, frame: &mut Frame, a: Point, b: Point, color: Color, width: f32, bias: f32) {
        frame.hairline(
            self.camera,
            self.lift(a, bias),
            self.lift(b, bias),
            color,
            width,
        );
    }

    /// A ball of `pixels` radius at `point`, never pickable, for pulses and specks.
    fn speck(&self, frame: &mut Frame, point: Point, pixels: f32, color: Color, bias: f32) {
        let center = frame.project(self.camera, self.lift(point, bias));
        frame.items.push(Item::Sphere {
            center,
            radius: pixels,
            depth: 0.1,
            color,
            pick: NONE,
            selected: false,
        });
    }
}

impl Cortex {
    /// Takes in the frame's processes: neurons leave and arrive, seats are given, the slice
    /// widens where a layer is full, and every neuron is driven by its process's CPU.
    fn sync(&mut self, stage: &Stage, processes: &[&Process], snapshot: &Snapshot) {
        let present: HashSet<Identity> = processes.iter().map(|process| process.id).collect();
        let mut gone: Vec<Identity> = self
            .slots
            .keys()
            .filter(|id| !present.contains(id))
            .copied()
            .collect();
        gone.sort();
        if !gone.is_empty() {
            let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
            for id in gone {
                let slot = self.slots.remove(&id).expect("listed from the slots");
                let body = self.bodies[slot].take().expect("a slot holds its body");
                self.network.remove(slot);
                self.changes += 1;
                // A process hidden by the limit or a focus is not dead: only a real exit leaves dust.
                if !alive.contains(&id) && stage.focus.is_none() {
                    self.dying.push(Dust {
                        at: body.at,
                        color: body.color,
                        radius: body.radius,
                        since: stage.time,
                        seed: identity_seed(id),
                    });
                }
            }
        }
        self.seats.assign(
            processes
                .iter()
                .map(|p| (p.id, LAYERS[layer_of(p.kind)].name)),
        );
        let mut widened = false;
        for (layer, named) in LAYERS
            .iter()
            .enumerate()
            .filter(|(_, named)| named.kind.is_some())
        {
            let wanted = self.seats.span(named.name);
            while self.slice.capacity(layer) < wanted {
                self.slice.widenings += 1;
                widened = true;
            }
        }
        for process in processes {
            let layer = layer_of(process.kind);
            let seat = self
                .seats
                .seat(process.id)
                .expect("every process has a seat");
            if let Some(&slot) = self.slots.get(&process.id) {
                if let Some(body) = &mut self.bodies[slot] {
                    body.seat = seat;
                    body.layer = layer;
                }
                continue;
            }
            let firing = Firing::of(process.kind);
            let mut rng = Rng::for_identity(process.id, SEAT_SALT);
            let jitter = [rng.signed(), rng.signed()];
            let far = rng.unit();
            let slot = self.network.add(firing, rng);
            if self.bodies.len() <= slot {
                self.bodies.resize_with(slot + 1, || None);
            }
            self.bodies[slot] = Some(Body {
                id: process.id,
                firing,
                layer,
                seat,
                jitter,
                at: [0.0; 2],
                color: kind_base(process.kind),
                far,
                primaries: Vec::new(),
                apical: None,
                radius: 0.5,
            });
            self.slots.insert(process.id, slot);
            self.changes += 1;
        }
        let slice = self.slice;
        for body in self.bodies.iter_mut().flatten() {
            body.at = slice.place(body.layer, body.seat, body.jitter);
        }
        if widened {
            self.changes += 1;
        }
        for process in processes {
            let slot = self.slots[&process.id];
            let body = self.bodies[slot].as_mut().expect("a slot holds its body");
            body.radius = soma_radius(process.memory);
            let wanted = (process.threads as usize).clamp(1, MAX_PRIMARIES);
            while body.primaries.len() < wanted {
                body.primaries
                    .push(grow(process.id, body.primaries.len(), body.firing));
            }
            if body.firing != Firing::FastSpiking && body.apical.is_none() {
                let rise = body.at[1] - 0.5 * band(0).1;
                body.apical = Some(apical(process.id, rise));
            }
            self.network
                .set_drive(slot, neurons::current(body.firing, process.cpu));
        }
    }

    /// Builds the axons and synapses from the sample's links and pipes when the sample or the
    /// set of neurons changed, and chooses the talking neurons and the selection's component.
    fn connect(&mut self, snapshot: &Snapshot, processes: &[&Process], selected: Option<Identity>) {
        let key = (snapshot.elapsed.to_bits(), self.changes);
        if self.built != Some(key) {
            self.built = Some(key);
            self.rewire(snapshot);
        }
        let cpu: HashMap<Identity, f32> = processes.iter().map(|p| (p.id, p.cpu)).collect();
        let mut linked = HashSet::new();
        for axon in &self.axons {
            linked.extend(axon.slots);
        }
        let mut talking: Vec<usize> = linked
            .into_iter()
            .filter(|&slot| {
                self.bodies[slot]
                    .as_ref()
                    .and_then(|body| cpu.get(&body.id))
                    .is_some_and(|&cpu| cpu >= TALKING_CPU)
            })
            .collect();
        talking.sort_unstable();
        if talking != self.talking {
            self.network.set_group(0, &talking);
            self.talking = talking;
        }
        let watch = selected.and_then(|id| self.slots.get(&id).map(|&slot| (id, slot)));
        self.network.watch(watch.map(|(_, slot)| slot));
        let unchanged = self
            .watch
            .as_ref()
            .is_some_and(|w| Some(w.id) == watch.map(|(id, _)| id) && w.built == self.built);
        if !unchanged {
            let component = watch.map_or_else(Vec::new, |(_, slot)| self.component(slot));
            self.network.set_group(1, &component);
            self.watch = watch.map(|(id, _)| Watch {
                id,
                built: self.built,
            });
        }
    }

    /// The neurons connected to `slot` through axons, itself included; empty when it has none.
    fn component(&self, slot: usize) -> Vec<usize> {
        let mut seen = HashSet::from([slot]);
        let mut frontier = vec![slot];
        while let Some(at) = frontier.pop() {
            for axon in &self.axons {
                for (here, there) in [
                    (axon.slots[0], axon.slots[1]),
                    (axon.slots[1], axon.slots[0]),
                ] {
                    if here == at && seen.insert(there) {
                        frontier.push(there);
                    }
                }
            }
        }
        if seen.len() < 2 {
            return Vec::new();
        }
        let mut members: Vec<usize> = seen.into_iter().collect();
        members.sort_unstable();
        members
    }

    /// One axon for every linked pair whose neurons are both here, with a synapse for each
    /// direction a spike may take: both for a socket, writer to reader for a pipe.
    fn rewire(&mut self, snapshot: &Snapshot) {
        // Weights from the pair's lower identity to its higher, and back.
        let mut pairs: BTreeMap<(Identity, Identity), [Option<f32>; 2]> = BTreeMap::new();
        let socket_traffic = !snapshot.missing.contains(Source::SocketTraffic);
        let mut offer = |from: Identity, to: Identity, weight: f32| {
            let (key, side) = if from < to {
                ((from, to), 0)
            } else {
                ((to, from), 1)
            };
            let entry = pairs.entry(key).or_default();
            entry[side] = Some(entry[side].map_or(weight, |old| old.max(weight)));
        };
        for &(a, b, sockets) in &snapshot.links {
            if a == b || !self.slots.contains_key(&a) || !self.slots.contains_key(&b) {
                continue;
            }
            let rate = socket_traffic
                .then(|| {
                    snapshot
                        .link_traffic
                        .get(&(a, b))
                        .or_else(|| snapshot.link_traffic.get(&(b, a)))
                })
                .flatten();
            let weight =
                rate.map_or_else(|| weight_of_sockets(sockets), |&rate| weight_of_rate(rate));
            offer(a, b, weight);
            offer(b, a, weight);
        }
        for pipe in &snapshot.pipes {
            let (writer, reader) = (pipe.writer, pipe.reader);
            if writer == reader
                || !self.slots.contains_key(&writer)
                || !self.slots.contains_key(&reader)
            {
                continue;
            }
            offer(writer, reader, PIPE_WEIGHT);
            if pipe.two_way {
                offer(reader, writer, PIPE_WEIGHT);
            }
        }
        self.axons.clear();
        let mut synapses = Vec::new();
        for ((a, b), weights) in pairs {
            let (slot_a, slot_b) = (self.slots[&a], self.slots[&b]);
            let (Some(body_a), Some(body_b)) = (&self.bodies[slot_a], &self.bodies[slot_b]) else {
                continue;
            };
            let mut rng = Rng::new(identity_seed(a) ^ identity_seed(b).rotate_left(17) ^ AXON_SALT);
            let from = [body_a.at[0], body_a.at[1] + 0.6 * body_a.radius];
            let to = [body_b.at[0], body_b.at[1] + 0.6 * body_b.radius];
            let axon = arc([a, b], [slot_a, slot_b], from, to, &mut rng);
            let index = self.axons.len();
            let delay = (axon.length() / CONDUCTION / STEP_SECONDS).round().max(1.0) as u32;
            for (side, weight) in weights.into_iter().enumerate() {
                if let Some(weight) = weight {
                    let (from, to) = if side == 0 {
                        (slot_a, slot_b)
                    } else {
                        (slot_b, slot_a)
                    };
                    synapses.push(Synapse {
                        from,
                        to,
                        weight,
                        delay,
                        axon: index,
                        backward: side == 1,
                    });
                }
            }
            self.axons.push(axon);
        }
        self.network.set_synapses(&synapses);
    }

    /// Advances the simulation to the frame's time and takes the summary of the spikes.
    fn simulate(&mut self, time: f32) {
        let network = &mut self.network;
        self.clock.advance(time, |_| network.step());
        network.trim(RASTER_TICKS + TICKS_PER_SECOND);
        let mut summary = Summary::default();
        for &slot in self.slots.values() {
            summary.spikes += network.rate(slot, 1.0) as usize;
        }
        summary.chi = network.synchrony(0);
        self.summary = summary;
    }
}

/// A seed for a process, apart from the streams of its own.
fn identity_seed(id: Identity) -> u64 {
    crate::simulation::hash_identity(id, DUST_SALT)
}

/// The radius of a soma holding `memory` bytes, in world units: bigger for the processes that
/// hold more, by the square root of the gibibytes.
fn soma_radius(memory: u64) -> f32 {
    let gibibytes = memory as f32 / (1u64 << 30) as f32;
    0.45 + 0.35 * gibibytes.max(0.0).sqrt().min(2.0)
}

/// How brightly the soma of the neuron in `slot` flashes: each spike of the last 0.3 s counts,
/// fading with `FLASH_TAU`.
fn flash(network: &Network, slot: usize) -> f32 {
    let now = network.tick();
    network
        .spikes(slot)
        .iter()
        .rev()
        .map(|&tick| (now - tick) as f32 * STEP_SECONDS)
        .take_while(|&age| age < 0.3)
        .map(|age| (-age / FLASH_TAU).exp())
        .sum::<f32>()
        .min(1.6)
}

impl ViewModule for Cortex {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        self.sync(stage, processes, snapshot);
        self.connect(snapshot, processes, stage.selected);
        self.simulate(stage.time);
        let ink = Ink::new(stage.camera);
        self.draw_slice(stage, &ink);
        let small = processes.len() > CROWD;
        self.draw_dendrites(stage, &ink, processes, small);
        let drawn = self.draw_axons(stage, &ink);
        self.draw_pulses(stage, &ink, &drawn);
        self.draw_somas(stage, &ink, processes);
        self.draw_dust(stage, &ink);
        self.annotate(stage, processes);
        self.draw_raster(stage, &ink);
        processes.len()
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let chi = self
            .summary
            .chi
            .map_or_else(|| "n/a".to_owned(), |chi| format!("{chi:.2}"));
        let mut text = format!(" chi {chi} talking | {} spikes/s", self.summary.spikes);
        for (source, consequence) in [
            (Source::SocketTraffic, "weight = socket count"),
            (Source::Pipes, "pipes are not drawn"),
        ] {
            if let Some(note) = missing_note(snapshot, source, consequence) {
                text += &note;
            }
        }
        text += " | soma = process, layer by kind | dendrite = thread | axon = link, delay ~ length, weight ~ log traffic | current ~ CPU";
        text
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

impl Cortex {
    fn draw_slice(&self, stage: &mut Stage, ink: &Ink) {
        let width = self.slice.width();
        for (index, layer) in LAYERS.iter().enumerate() {
            let (top, bottom) = band(index);
            stage.frame.quad(
                stage.camera,
                [
                    world(0.0, top),
                    world(width, top),
                    world(width, bottom),
                    world(0.0, bottom),
                ],
                layer.fill,
                NONE,
            );
        }
        // Long faint fibres run through the white matter.
        let (top, bottom) = band(5);
        let mut rng = Rng::new(AXON_SALT);
        for fibre in 0..6 {
            let down = top + (bottom - top) * (0.15 + 0.14 * fibre as f32);
            let period = 9.0 + 7.0 * rng.unit();
            let phase = TAU * rng.unit();
            let mut last = None;
            for step in 0..=24 {
                let along = width * step as f32 / 24.0;
                let at = world(along, down + 0.25 * (along / period + phase).sin());
                if let Some(last) = last {
                    ink.stroke(stage.frame, last, at, [46, 36, 60], 1.2, FIBRE_DEPTH);
                }
                last = Some(at);
            }
        }
    }

    fn draw_dendrites(&self, stage: &mut Stage, ink: &Ink, processes: &[&Process], small: bool) {
        let zoom = stage.camera.zoom;
        for process in processes {
            let Some(body) = self
                .slots
                .get(&process.id)
                .and_then(|&slot| self.bodies[slot].as_ref())
            else {
                continue;
            };
            let grown = sprout(stage.growth(process.id));
            let simple = small || body.radius * zoom * grown < SMALL_SOMA;
            let shade = 0.30 + 0.26 * body.far;
            let wanted = (process.threads as usize).clamp(1, MAX_PRIMARIES);
            let origin = body.at;
            let mut paint = |branch: &Branch, scale: f32| {
                let color = tint(body.color, shade * (1.0 - 0.18 * f32::from(branch.depth)));
                for pair in branch.points.windows(2) {
                    let at = |p: [f32; 2]| {
                        world(
                            origin[0] + p[0] * grown * scale,
                            origin[1] + p[1] * grown * scale,
                        )
                    };
                    ink.stroke(
                        stage.frame,
                        at(pair[0]),
                        at(pair[1]),
                        color,
                        DENDRITE_WIDTH,
                        DENDRITE_DEPTH,
                    );
                }
            };
            for tree in body.primaries.iter().take(wanted) {
                for branch in tree.iter().filter(|branch| !simple || branch.depth == 0) {
                    paint(branch, 1.0);
                }
            }
            if let (Some(apical), false) = (&body.apical, simple) {
                for branch in apical {
                    paint(branch, 1.0);
                }
            }
        }
    }

    /// Draws the axons the link mode shows and returns which those are, by axon.
    fn draw_axons(&self, stage: &mut Stage, ink: &Ink) -> Vec<bool> {
        let hot = |id: &Identity| stage.highlight.contains(id);
        let shown: Vec<bool> = self
            .axons
            .iter()
            .map(|axon| match stage.links {
                Links::All => true,
                Links::Focused => axon.ends.iter().any(hot),
                Links::Off => false,
            })
            .collect();
        for (axon, _) in self.axons.iter().zip(&shown).filter(|(_, shown)| **shown) {
            let lit = axon.ends.iter().any(hot);
            let (a, b) = (self.slots[&axon.ends[0]], self.slots[&axon.ends[1]]);
            let color = match (self.bodies[a].as_ref(), self.bodies[b].as_ref()) {
                (Some(a), Some(b)) => mix(a.color, b.color, 0.5),
                _ => [120, 120, 140],
            };
            let color = tint(color, if lit { 0.8 } else { 0.38 });
            for pair in axon.points.windows(2) {
                ink.stroke(stage.frame, pair[0], pair[1], color, AXON_WIDTH, AXON_DEPTH);
            }
        }
        shown
    }

    /// Draws the spikes in flight, the very events the network will deliver, at most
    /// `MAX_PULSES` of them, as balls on the axons they run along.
    fn draw_pulses(&self, stage: &mut Stage, ink: &Ink, shown: &[bool]) {
        let now = self.network.tick();
        let stride = self.network.flights().div_ceil(MAX_PULSES).max(1);
        for (emit, arrive, synapse) in self.network.in_flight().step_by(stride) {
            let Some(true) = shown.get(synapse.axon) else {
                continue;
            };
            let axon = &self.axons[synapse.axon];
            let travelled = (now.saturating_sub(emit)) as f32 / (arrive - emit).max(1) as f32;
            let fraction = if synapse.backward {
                1.0 - travelled
            } else {
                travelled
            };
            let source = self.bodies[synapse.from]
                .as_ref()
                .map_or([200; 3], |body| body.color);
            let color = mix(source, [255; 3], 0.55);
            ink.speck(
                stage.frame,
                axon.at(fraction),
                1.7 + 0.8 * synapse.weight,
                color,
                PULSE_DEPTH,
            );
        }
    }

    fn draw_somas(&self, stage: &mut Stage, ink: &Ink, processes: &[&Process]) {
        let height = stage.frame.height as f32;
        for (index, process) in processes.iter().enumerate() {
            let slot = self.slots[&process.id];
            let Some(body) = &self.bodies[slot] else {
                continue;
            };
            let position = world(body.at[0], body.at[1]);
            let grown = sprout(stage.growth(process.id));
            let lit = flash(&self.network, slot);
            let active = (self.network.rate(slot, 1.0) / 40.0).min(1.0);
            let bright = 0.34 + 0.25 * body.far + 0.25 * active + 0.55 * lit.min(1.0);
            let color = mix(
                tint(body.color, bright),
                [255; 3],
                0.35 * (lit - 0.6).max(0.0),
            );
            stage.frame.sphere(
                stage.camera,
                ink.lift(position, SOMA_DEPTH),
                body.radius * grown,
                color,
                index as u32,
                stage.selected == Some(process.id),
            );
            stage.positions.insert(process.id, position);
            if lit > 0.05 {
                let soma = (body.radius * grown * stage.camera.zoom).max(3.5);
                let radius = soma + (6.0 + 9.0 * lit.min(1.0)) * height / 720.0;
                stage.frame.glow(
                    stage.camera,
                    ink.lift(position, SOMA_DEPTH),
                    radius,
                    body.color,
                    0.25 + 0.35 * lit.min(1.0),
                );
            }
        }
    }

    fn draw_dust(&mut self, stage: &mut Stage, ink: &Ink) {
        self.dying
            .retain(|dust| (0.0..DUST_SECONDS).contains(&(stage.time - dust.since)));
        for dust in &self.dying {
            let age = (stage.time - dust.since) / DUST_SECONDS;
            let shrink = 1.0 - smoothstep(age);
            let center = world(dust.at[0], dust.at[1]);
            ink.speck(
                stage.frame,
                center,
                (dust.radius * shrink * stage.camera.zoom).max(0.5),
                tint(dust.color, 0.7),
                SOMA_DEPTH,
            );
            let mut rng = Rng::new(dust.seed);
            for _ in 0..DUST_SPECKS {
                let (angle, speed) = (TAU * rng.unit(), 0.8 + 1.4 * rng.unit());
                let reach = speed * age.sqrt();
                let at = world(
                    dust.at[0] + reach * angle.cos(),
                    dust.at[1] + reach * angle.sin(),
                );
                ink.speck(
                    stage.frame,
                    at,
                    1.4,
                    tint(dust.color, 0.8 * (1.0 - age)),
                    SOMA_DEPTH,
                );
            }
        }
    }

    /// Labels, inspector lines, the tour, the camera's bounds and the neurons' positions.
    fn annotate(&self, stage: &mut Stage, processes: &[&Process]) {
        let width = self.slice.width();
        let below = 0.27 * SLICE_HEIGHT;
        for corner in [
            [-LABEL_MARGIN, -1.0],
            [width, -1.0],
            [width, SLICE_HEIGHT + below],
            [-LABEL_MARGIN, SLICE_HEIGHT + below],
        ] {
            stage.bounds.push(world(corner[0], corner[1]));
        }
        for (index, layer) in LAYERS.iter().enumerate() {
            let (top, bottom) = band(index);
            let middle = 0.5 * (top + bottom);
            let along = -0.5 * LABEL_MARGIN;
            if layer.caption.is_empty() {
                stage
                    .places
                    .push((world(along, middle), layer.name.to_owned()));
            } else {
                stage
                    .places
                    .push((world(along, middle - 0.9), layer.name.to_owned()));
                stage
                    .places
                    .push((world(along, middle + 0.9), layer.caption.to_owned()));
            }
        }
        let mut busiest: Vec<&&Process> = processes.iter().filter(|p| p.cpu > 0.0).collect();
        busiest.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(a.id.cmp(&b.id)));
        for process in busiest.into_iter().take(TOUR_STOPS) {
            let Some(body) = self
                .slots
                .get(&process.id)
                .and_then(|&slot| self.bodies[slot].as_ref())
            else {
                continue;
            };
            stage.tour.push(TourTarget {
                id: process.id,
                position: world(body.at[0], body.at[1]),
                weight: process.cpu,
            });
            let slot = self.slots[&process.id];
            stage.callout.push((
                process.id,
                format!(
                    "{} {} Hz | I {:.1} | layer {}",
                    body.firing.code(),
                    self.network.rate(slot, 2.0).round(),
                    self.network.drive(slot),
                    LAYERS[body.layer].name,
                ),
            ));
        }
        if let Some(watch) = &self.watch
            && let Some(&slot) = self.slots.get(&watch.id)
            && let Some(body) = &self.bodies[slot]
        {
            let params = self.network.firing_params(slot);
            let threads = processes
                .iter()
                .find(|process| process.id == watch.id)
                .map_or(0, |process| process.threads as usize);
            let incoming: Vec<f32> = self
                .network
                .synapses_into(slot)
                .map(|synapse| synapse.delay as f32 * STEP_SECONDS * 1000.0)
                .collect();
            let mut notes = vec![
                format!(
                    "{} {}  a b c d = {} {} {} {}",
                    body.firing.code(),
                    body.firing.name(),
                    params.a,
                    params.b,
                    params.c,
                    params.d
                ),
                format!(
                    "I = {:.1}  rate {:.0} Hz",
                    self.network.drive(slot),
                    self.network.rate(slot, 2.0)
                ),
                format!(
                    "layer {} | {} of {} threads drawn",
                    LAYERS[body.layer].name,
                    threads.clamp(1, MAX_PRIMARIES),
                    threads
                ),
            ];
            notes.push(
                match (
                    incoming.iter().copied().reduce(f32::min),
                    incoming.iter().copied().reduce(f32::max),
                ) {
                    (Some(least), Some(most)) => format!(
                        "synapses in {} | delay {:.0} to {:.0} ms",
                        incoming.len(),
                        least,
                        most
                    ),
                    _ => "synapses in 0".to_owned(),
                },
            );
            notes.push(match self.network.synchrony(1) {
                Some(chi) => format!(
                    "component chi {chi:.2} ({} neurons)",
                    self.network.group_size(1)
                ),
                None => "component chi n/a".to_owned(),
            });
            notes.push("v(t), last 320 ms, -80 to +30 mV".to_owned());
            stage.notes.insert(watch.id, notes);
            stage
                .sparklines
                .insert(watch.id, neurons::sparkline(self.network.trace(), 40, 2));
        }
    }
}

/// How many rows the raster draws for `rows` neurons in a plot `plot` pixels high: one each, until
/// that would be under two pixels a row, and then rows share bins.
fn raster_bins(rows: usize, plot: f32) -> usize {
    rows.min((plot / 2.0) as usize).max(1)
}

/// A screen-space rectangle of whole pixels, from (`x0`, `y0`) to (`x1`, `y1`), at `depth`.
fn rect(frame: &mut Frame, [x0, y0, x1, y1]: [f32; 4], depth: f32, color: Color) {
    let corner = |x: f32, y: f32| [x, y, depth];
    frame.items.push(Item::Triangle(
        [corner(x0, y0), corner(x1, y0), corner(x1, y1)],
        color,
        NONE,
    ));
    frame.items.push(Item::Triangle(
        [corner(x0, y0), corner(x1, y1), corner(x0, y1)],
        color,
        NONE,
    ));
}

/// ASCII text in the bitmap font from (`x`, `y`), `scale` times its size.
fn text(frame: &mut Frame, x: f32, y: f32, scale: u32, color: Color, line: &str) {
    for (index, character) in line.chars().enumerate() {
        frame.items.push(Item::Glyph {
            origin: [x + (index as u32 * glyphs::WIDTH * scale) as f32, y],
            scale,
            bits: glyphs::glyph(character),
            color,
        });
    }
}

impl Cortex {
    /// The raster of the last ten seconds, in the bottom of the frame: a row for each neuron
    /// that fired, by layer and then seat, merged into bins where there are more rows than the
    /// plot has room for at two pixels each. Everything is whole pixels, drawn in front of the
    /// slice, so the two rasterizers agree on it.
    fn draw_raster(&mut self, stage: &mut Stage, ink: &Ink) {
        let frame = &mut *stage.frame;
        let (width, height) = (frame.width as f32, frame.height as f32);
        let nearest = [
            world(0.0, 0.0),
            world(self.slice.width(), 0.0),
            world(0.0, SLICE_HEIGHT),
            world(self.slice.width(), SLICE_HEIGHT),
        ]
        .into_iter()
        .map(|corner| frame.project(ink.camera, corner)[2])
        .fold(f32::NEG_INFINITY, f32::max);
        let depth = nearest + SOMA_DEPTH + PANEL_LEAD;
        let top = (0.78 * height).round();
        rect(frame, [0.0, top, width, height], depth, [9, 9, 15]);
        let lettering = height >= 240.0;
        let scale = (height / 540.0).floor().max(1.0) as u32;
        let letter = (glyphs::HEIGHT * scale) as f32;
        let (x0, x1) = if lettering {
            ((0.055 * width).round(), (0.988 * width).round())
        } else {
            (2.0, width - 2.0)
        };
        let y0 = top + if lettering { letter + 8.0 } else { 2.0 };
        let y1 = (0.91 * height).round();
        let cutoff = self.network.tick().saturating_sub(RASTER_TICKS);
        let mut rows: Vec<(usize, usize, usize)> = self
            .slots
            .values()
            .filter(|&&slot| {
                self.network
                    .spikes(slot)
                    .last()
                    .is_some_and(|&tick| tick >= cutoff)
            })
            .filter_map(|&slot| {
                self.bodies[slot]
                    .as_ref()
                    .map(|body| (body.layer, body.seat, slot))
            })
            .collect();
        rows.sort_unstable();
        if lettering {
            let title = format!(
                "RASTER  rows = neurons by layer ({} that fire, of {}), dots = spikes, last 10 s",
                rows.len(),
                self.slots.len()
            );
            let room = ((x1 - x0) / (glyphs::WIDTH * scale) as f32) as usize;
            let title: String = title.chars().take(room.saturating_sub(16)).collect();
            text(frame, x0, top + 4.0, scale, [200, 206, 222], &title);
            let right = "-10 s ... now";
            let at = x1 - (right.len() as u32 * glyphs::WIDTH * scale) as f32;
            text(frame, at, top + 4.0, scale, [120, 126, 150], right);
        }
        if y1 - y0 < 4.0 || x1 - x0 < 16.0 {
            return;
        }
        let plot = y1 - y0;
        let dot = (width / 960.0).round().max(1.0);
        let columns = ((x1 - x0) / dot) as usize;
        for second in 0..=RASTER_SECONDS {
            let x = x0 + ((x1 - x0) * second as f32 / RASTER_SECONDS as f32).round();
            rect(
                frame,
                [x.min(x1 - 1.0), y0, x.min(x1 - 1.0) + 1.0, y1],
                depth + 0.1,
                [30, 30, 44],
            );
        }
        if rows.is_empty() {
            return;
        }
        let bins = raster_bins(rows.len(), plot);
        let edge = |bin: usize| y0 + (plot * bin as f32 / bins as f32).round();
        let bin_of = |row: usize| row * bins / rows.len();
        self.cells.clear();
        self.cells.resize(bins * columns, 0);
        for (row, &(_, _, slot)) in rows.iter().enumerate() {
            for &tick in self.network.spikes(slot).iter().rev() {
                if tick < cutoff {
                    break;
                }
                let column =
                    ((tick - cutoff) as f32 / RASTER_TICKS as f32 * columns as f32) as usize;
                let cell = &mut self.cells[bin_of(row) * columns + column.min(columns - 1)];
                *cell = (*cell + 1).min(RASTER_LEVELS);
            }
        }
        let mut layer_starts: Vec<(usize, usize)> = Vec::new();
        for (row, &(layer, _, _)) in rows.iter().enumerate() {
            if layer_starts.last().is_none_or(|&(last, _)| last != layer) {
                layer_starts.push((layer, bin_of(row)));
            }
        }
        for window in layer_starts.windows(2) {
            let y = edge(window[1].1);
            rect(frame, [x0, y, x1, y + 1.0], depth + 0.1, [40, 40, 58]);
        }
        for bin in 0..bins {
            let layer = layer_starts
                .iter()
                .rev()
                .find(|&&(_, start)| start <= bin)
                .map_or(1, |&(layer, _)| layer);
            let base = LAYERS[layer].kind.map_or([200; 3], kind_base);
            let (low, high) = (edge(bin), edge(bin + 1));
            let high = if high - low >= 4.0 { high - 1.0 } else { high };
            let cells = &self.cells[bin * columns..(bin + 1) * columns];
            let mut column = 0;
            while column < columns {
                let level = cells[column];
                if level == 0 {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < columns && cells[column] == level {
                    column += 1;
                }
                // Cells where several neurons of a bin fired together are brighter.
                let color = tint(base, 0.6 + 0.3 * f32::from(level - 1));
                rect(
                    frame,
                    [x0 + start as f32 * dot, low, x0 + column as f32 * dot, high],
                    depth + 0.2,
                    color,
                );
            }
        }
        if lettering {
            for (index, &(layer, start)) in layer_starts.iter().enumerate() {
                let end = layer_starts.get(index + 1).map_or(bins, |&(_, next)| next);
                let name = LAYERS[layer].name;
                let middle = 0.5 * (edge(start) + edge(end));
                let at = x0 - 8.0 - (name.len() as u32 * glyphs::WIDTH * scale) as f32;
                let color = tint(LAYERS[layer].kind.map_or([200; 3], kind_base), 1.1);
                text(
                    frame,
                    at,
                    (middle - letter * 0.5).round(),
                    scale,
                    color,
                    name,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{churned, rescaled};
    use crate::model::testing::process;
    use crate::model::{Pipe, demo};
    use crate::render::{RenderInput, Scene};
    use crate::views::View;

    fn view() -> View {
        View::named("cortex").expect("cortex is registered")
    }

    /// Draws `snapshot` at `time` in a frame of `width` pixels with the selection `selected`.
    fn draw(
        scene: &mut Scene,
        snapshot: &Snapshot,
        time: f32,
        selected: Option<Identity>,
    ) -> Frame {
        let camera = Camera::default();
        let mut input = RenderInput::small(snapshot, view(), &camera, time);
        (input.width, input.height) = (640, 360);
        input.selected = selected;
        scene.render(&input)
    }

    /// Draws and rasterizes.
    fn render(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> Frame {
        let mut frame = draw(scene, snapshot, time, None);
        frame.rasterize();
        frame
    }

    /// Runs `seconds` of frames at 0.1 s over the same sample, from time 1.
    fn run(scene: &mut Scene, snapshot: &Snapshot, seconds: f32) {
        for k in 0..=(seconds * 10.0) as u32 {
            draw(scene, snapshot, 1.0 + k as f32 * 0.1, None);
        }
    }

    fn state(scene: &Scene) -> &Cortex {
        scene.module::<Cortex>(view()).expect("cortex has drawn")
    }

    /// Two processes at one core each, `reader` and `writer`, and a snapshot of just them.
    fn two(kind: Kind) -> (Snapshot, Identity, Identity) {
        let busy = |pid| Process {
            kind,
            cpu: 100.0,
            ..process(pid, 1)
        };
        let (writer, reader) = (busy(10), busy(11));
        let snapshot = Snapshot {
            processes: vec![writer.clone(), reader.clone()],
            ..Snapshot::default()
        };
        (snapshot, writer.id, reader.id)
    }

    fn weight_between(snapshot: &Snapshot, to: Identity) -> Option<f32> {
        let mut scene = Scene::new();
        run(&mut scene, snapshot, 0.3);
        let cortex = state(&scene);
        let slot = cortex.slots[&to];
        cortex
            .network
            .synapses_into(slot)
            .next()
            .map(|synapse| synapse.weight)
    }

    #[test]
    fn the_cortex_fires_the_same_spikes_at_ten_and_sixty_frames_per_second() {
        let snapshot = demo(12.0, 48);
        let play = |fps: u32| {
            let mut scene = Scene::new();
            for k in 0..=3 * fps {
                draw(&mut scene, &snapshot, 1.0 + k as f32 / fps as f32, None);
            }
            let cortex = state(&scene);
            let spikes: BTreeMap<Identity, Vec<u64>> = cortex
                .slots
                .iter()
                .map(|(&id, &slot)| (id, cortex.network.spikes(slot).to_vec()))
                .collect();
            (cortex.network.tick(), spikes)
        };
        let ((slow_ticks, slow), (fast_ticks, fast)) = (play(10), play(60));
        assert!(
            slow_ticks.abs_diff(fast_ticks) <= 3,
            "{slow_ticks} and {fast_ticks} steps"
        );
        assert!(slow_ticks >= 5_990, "three seconds of steps: {slow_ticks}");
        let common = slow_ticks.min(fast_ticks);
        let upto = |spikes: &[u64]| {
            spikes
                .iter()
                .copied()
                .filter(|&tick| tick <= common)
                .collect::<Vec<_>>()
        };
        let mut total = 0;
        for (id, spikes) in &slow {
            assert_eq!(upto(spikes), upto(&fast[id]), "{id:?}");
            total += spikes.len();
        }
        assert!(total > 200, "the comparison covers {total} spikes");
    }

    #[test]
    fn neurons_keep_their_places_when_cpu_memory_or_population_changes() {
        let base = demo(12.0, 96);
        let (changed, gone) = churned(&base);
        for after in [rescaled(&base), changed] {
            let mut scene = Scene::new();
            draw(&mut scene, &base, 1.0, None);
            let before = scene.positions.clone();
            draw(&mut scene, &after, 2.0, None);
            let mut kept = 0;
            for (id, at) in &before {
                if gone.contains(id) {
                    continue;
                }
                assert_eq!(scene.positions.get(id), Some(at), "{id:?} moved");
                kept += 1;
            }
            assert!(kept > 80);
        }
        // A process that arrives takes a seat a process left, so the slice stays as it was.
        let mut scene = Scene::new();
        draw(&mut scene, &base, 1.0, None);
        let width = state(&scene).slice.width();
        draw(&mut scene, &churned(&base).0, 2.0, None);
        assert_eq!(state(&scene).slice.width(), width);
    }

    #[test]
    fn cortex_renders_the_same_frame_twice_from_the_same_inputs() {
        let snapshot = demo(12.0, 64);
        let frames = || {
            let mut scene = Scene::new();
            for k in 0..30 {
                render(&mut scene, &snapshot, 1.0 + k as f32 * 0.1);
            }
            let frame = render(&mut scene, &snapshot, 4.0);
            (frame.pixels, frame.picks)
        };
        let (first, second) = (frames(), frames());
        assert!(first == second, "two runs differ");
        assert!(first.0.iter().any(|&byte| byte > 0));
    }

    #[test]
    fn pipe_spikes_travel_only_from_writer_to_reader() {
        let (mut snapshot, writer, reader) = two(Kind::System);
        snapshot.pipes = vec![Pipe {
            reader,
            writer,
            two_way: false,
            buffered: None,
            fifo: false,
        }];
        let mut scene = Scene::new();
        run(&mut scene, &snapshot, 2.0);
        let cortex = state(&scene);
        let (from, to) = (cortex.slots[&writer], cortex.slots[&reader]);
        assert_eq!(cortex.network.synapses_into(to).count(), 1);
        assert_eq!(
            cortex.network.synapses_into(from).count(),
            0,
            "nothing runs back"
        );
        assert!(
            !cortex.network.spikes(to).is_empty(),
            "the reader fires too"
        );
        assert!(cortex.network.flights() > 0);
        assert!(
            cortex
                .network
                .in_flight()
                .all(|(_, _, synapse)| synapse.from == from)
        );
        assert_eq!(cortex.axons.len(), 1);
        // A pipe that carries data both ways has a synapse each way along its one axon.
        snapshot.pipes[0].two_way = true;
        let mut scene = Scene::new();
        run(&mut scene, &snapshot, 1.0);
        let cortex = state(&scene);
        assert_eq!(
            cortex.network.synapses_into(cortex.slots[&writer]).count(),
            1
        );
        assert_eq!(
            cortex.network.synapses_into(cortex.slots[&reader]).count(),
            1
        );
        assert_eq!(cortex.axons.len(), 1);
    }

    #[test]
    fn socket_links_run_both_ways() {
        let (mut snapshot, a, b) = two(Kind::Session);
        snapshot.links = vec![(a, b, 1)];
        let mut scene = Scene::new();
        run(&mut scene, &snapshot, 0.3);
        let cortex = state(&scene);
        assert_eq!(cortex.network.synapses_into(cortex.slots[&a]).count(), 1);
        assert_eq!(cortex.network.synapses_into(cortex.slots[&b]).count(), 1);
    }

    #[test]
    fn without_socket_traffic_synapses_weigh_socket_counts_and_the_legend_says_so() {
        let (mut snapshot, a, b) = two(Kind::System);
        snapshot.links = vec![(a, b, 3)];
        snapshot.link_traffic = HashMap::from([((a, b), 1.0e6)]);
        assert_eq!(weight_between(&snapshot, b), Some(weight_of_rate(1.0e6)));
        assert!((weight_of_rate(1.0e6) - 1.7269).abs() < 1e-3);
        let mut scene = Scene::new();
        run(&mut scene, &snapshot, 0.3);
        let legend = scene.legend(view(), &snapshot);
        assert!(!legend.contains("no socket traffic"), "{legend}");
        snapshot.missing = Source::SocketTraffic.into();
        assert_eq!(weight_between(&snapshot, b), Some(weight_of_sockets(3)));
        assert!((weight_of_sockets(3) - 0.3466).abs() < 1e-3);
        let legend = scene.legend(view(), &snapshot);
        assert!(
            legend.contains(" | no socket traffic: weight = socket count"),
            "{legend}"
        );
        // A link whose traffic is not measured (a Unix socket) weighs its sockets as well.
        snapshot.missing = Default::default();
        snapshot.link_traffic.clear();
        assert_eq!(weight_between(&snapshot, b), Some(weight_of_sockets(3)));
    }

    #[test]
    fn weights_follow_the_log_of_the_traffic_and_stop_at_two() {
        assert_eq!(weight_of_rate(0.0), 0.0);
        assert_eq!(weight_of_rate(f32::NAN), 0.0);
        assert!((weight_of_rate(1000.0) - 0.25 * 2.0_f32.ln()).abs() < 1e-6);
        assert!(weight_of_rate(1.0e4) > weight_of_rate(1.0e3));
        assert_eq!(weight_of_rate(1.0e12), 2.0);
        assert!((weight_of_sockets(1) - 0.25 * 2.0_f32.ln()).abs() < 1e-6);
        assert_eq!(weight_of_sockets(u32::MAX), 2.0);
    }

    #[test]
    fn the_slice_widens_in_steps_when_a_layer_is_full_and_never_narrows() {
        let crowd = |count: u32| Snapshot {
            processes: (0..count)
                .map(|pid| Process {
                    kind: Kind::System,
                    ..process(100 + pid, 1)
                })
                .collect(),
            ..Snapshot::default()
        };
        let mut scene = Scene::new();
        draw(&mut scene, &crowd(20), 1.0, None);
        assert_eq!(state(&scene).slice.width(), BASE_WIDTH);
        draw(&mut scene, &crowd(400), 2.0, None);
        let wide = state(&scene).slice.width();
        assert!(wide > BASE_WIDTH);
        let steps = (wide / BASE_WIDTH).ln() / WIDENING.ln();
        assert!((steps - steps.round()).abs() < 1e-3, "{steps} widenings");
        draw(&mut scene, &crowd(20), 3.0, None);
        assert_eq!(state(&scene).slice.width(), wide, "it never narrows");
    }

    #[test]
    fn layers_follow_kind_and_every_soma_sits_inside_its_band() {
        let snapshot = demo(12.0, 192);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 1.0, None);
        for process in &snapshot.processes {
            let at = scene.positions[&process.id];
            let down = SLICE_HEIGHT - at[2];
            let (top, bottom) = band(layer_of(process.kind));
            assert!(
                (top..=bottom).contains(&down),
                "{:?} at {down}",
                process.kind
            );
        }
        let layers: Vec<usize> = [Kind::Session, Kind::Container, Kind::System, Kind::Kernel]
            .into_iter()
            .map(layer_of)
            .collect();
        assert_eq!(layers, [1, 2, 3, 4], "II/III, IV, V and VI from the top");
        assert!(LAYERS[0].kind.is_none() && LAYERS[5].kind.is_none());
        assert!(
            (LAYERS.iter().map(|layer| layer.fraction).sum::<f32>() - FRACTION_SUM).abs() < 1e-6
        );
        assert!((band(5).1 - SLICE_HEIGHT).abs() < 1e-3);
    }

    #[test]
    fn somas_are_pickable_and_nothing_else_is() {
        let snapshot = demo(12.0, 96);
        let mut scene = Scene::new();
        let camera = Camera::default();
        let mut input = RenderInput::small(&snapshot, view(), &camera, 1.0);
        (input.width, input.height) = (1280, 720);
        let mut fitted = camera.clone();
        scene.render(&input);
        scene.fit(&mut fitted, 1280, 720);
        input.camera = &fitted;
        for k in 0..30 {
            input.time = 1.0 + k as f32 * 0.1;
            scene.render(&input);
        }
        let mut frame = scene.render(&input);
        frame.rasterize();
        let mut own = 0;
        for (index, process) in snapshot.processes.iter().enumerate() {
            if let Some([x, y]) = frame.locate(&fitted, scene.positions[&process.id]) {
                let pick = frame.picks[y as usize * 1280 + x as usize];
                own += usize::from(pick == index as u32);
            }
        }
        assert!(
            own >= 90,
            "{own} of 96 somas pick themselves at their centres"
        );
        assert!(
            frame
                .picks
                .iter()
                .all(|&pick| pick == NONE || (pick as usize) < 96)
        );
    }

    #[test]
    fn the_busiest_neurons_lead_the_tour_and_every_neuron_has_a_position() {
        let snapshot = demo(12.0, 96);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 1.0, None);
        assert_eq!(scene.positions.len(), 96);
        assert_eq!(scene.tour.len(), TOUR_STOPS);
        assert!(
            scene
                .tour
                .windows(2)
                .all(|pair| pair[0].weight >= pair[1].weight)
        );
        let busiest = snapshot
            .processes
            .iter()
            .map(|process| process.cpu)
            .fold(0.0, f32::max);
        assert!((scene.tour[0].weight - busiest).abs() < 1e-3 * busiest);
        assert!(
            scene
                .tour
                .iter()
                .all(|stop| scene.positions[&stop.id] == stop.position)
        );
    }

    #[test]
    fn the_inspector_shows_the_selected_neuron_with_a_braille_membrane_trace() {
        let snapshot = demo(12.0, 64);
        let chosen = snapshot
            .processes
            .iter()
            .find(|process| process.kind == Kind::Session && process.cpu > 20.0)
            .expect("a busy session process");
        let mut scene = Scene::new();
        for k in 0..30 {
            draw(&mut scene, &snapshot, 1.0 + k as f32 * 0.1, Some(chosen.id));
        }
        let notes = &scene.notes[&chosen.id];
        assert!(notes[0].starts_with("CH chattering"), "{notes:?}");
        assert!(notes[0].contains("a b c d = 0.02 0.2 -50 2"), "{notes:?}");
        assert!(
            notes[1].starts_with("I = ") && notes[1].contains(" Hz"),
            "{notes:?}"
        );
        assert!(notes[2].starts_with("layer II/III | "), "{notes:?}");
        assert!(notes.iter().any(|line| line.starts_with("synapses in ")));
        assert!(notes.iter().any(|line| line.starts_with("component chi ")));
        let lines = &scene.sparklines[&chosen.id];
        assert_eq!(lines.len(), 2);
        for line in lines {
            assert_eq!(line.chars().count(), 40);
            assert!(
                line.chars()
                    .all(|c| c == ' ' || ('\u{2800}'..='\u{28ff}').contains(&c))
            );
        }
        assert!(lines.iter().any(|line| line.trim() != ""));
        // Only the selected neuron is recorded, and nothing is once the selection goes.
        assert_eq!(scene.notes.len(), 1);
        draw(&mut scene, &snapshot, 5.0, None);
        assert!(scene.notes.is_empty() && scene.sparklines.is_empty());
        assert!(state(&scene).network.trace().is_empty());
    }

    #[test]
    fn an_exited_soma_shrinks_over_a_second_and_a_fifth_and_leaves_twelve_specks() {
        let snapshot = demo(12.0, 64);
        let mut scene = Scene::new();
        run(&mut scene, &snapshot, 1.0);
        let mut left = snapshot.clone();
        let gone = left.processes.pop().expect("a process").id;
        let specks = |frame: &Frame| {
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Sphere { radius, pick: NONE, .. } if *radius == 1.4))
                .count()
        };
        draw(&mut scene, &left, 2.0, None);
        assert_eq!(state(&scene).dying.len(), 1);
        let frame = draw(&mut scene, &left, 2.5, None);
        assert_eq!(specks(&frame), DUST_SPECKS);
        assert!(!scene.positions.contains_key(&gone));
        draw(&mut scene, &left, 2.0 + DUST_SECONDS + 0.1, None);
        assert!(state(&scene).dying.is_empty());
    }

    #[test]
    fn the_link_key_decides_which_axons_are_drawn() {
        let snapshot = demo(12.0, 96);
        let strokes = |links: Links, highlight: Vec<Identity>| {
            let mut scene = Scene::new();
            scene.links = links;
            scene.highlight = highlight;
            draw(&mut scene, &snapshot, 1.0, None);
            let frame = draw(&mut scene, &snapshot, 1.1, None);
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Triangle(..)))
                .count()
        };
        let (a, b, _) = snapshot.links[0];
        let (all, focused, off) = (
            strokes(Links::All, Vec::new()),
            strokes(Links::Focused, vec![a, b]),
            strokes(Links::Off, vec![a, b]),
        );
        assert!(off < focused && focused < all, "{off} < {focused} < {all}");
        assert_eq!(
            strokes(Links::Focused, Vec::new()),
            off,
            "focused on nothing draws no axon"
        );
    }

    #[test]
    fn a_soma_grows_a_dendrite_for_each_thread_up_to_twelve() {
        let threaded = |threads: u32| Snapshot {
            processes: vec![Process {
                threads,
                ..process(5, 1)
            }],
            ..Snapshot::default()
        };
        let primaries = |threads: u32| {
            let mut scene = Scene::new();
            draw(&mut scene, &threaded(threads), 1.0, None);
            let cortex = state(&scene);
            cortex
                .bodies
                .iter()
                .flatten()
                .next()
                .expect("a body")
                .primaries
                .len()
        };
        assert_eq!(
            (primaries(0), primaries(1), primaries(7), primaries(12)),
            (1, 1, 7, 12)
        );
        assert_eq!(primaries(500), MAX_PRIMARIES);
        // Growing a soma's dendrites does not move the ones it has.
        let id = process(5, 1).id;
        let (few, many) = (
            grow(id, 3, Firing::Chattering),
            grow(id, 3, Firing::Chattering),
        );
        assert_eq!(few.len(), many.len());
        assert!(few.iter().zip(&many).all(|(a, b)| a.points == b.points));
        assert!(few[0].points.len() > 2 && few.iter().all(|branch| branch.depth <= 2));
    }

    #[test]
    fn a_crowd_or_a_small_soma_draws_primary_dendrites_only() {
        let snapshot = demo(12.0, 64);
        let triangles = |count: usize| {
            let snapshot = demo(12.0, count);
            let mut scene = Scene::new();
            draw(&mut scene, &snapshot, 1.0, None);
            let frame = draw(&mut scene, &snapshot, 1.1, None);
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Triangle(..)))
                .count() as f32
                / count as f32
        };
        let (few, many) = (triangles(CROWD), triangles(CROWD + 4));
        assert!(
            many < 0.7 * few,
            "{many} triangles each in a crowd, {few} otherwise"
        );
        assert!(snapshot.processes.len() < CROWD);
    }

    #[test]
    fn at_most_three_thousand_pulses_are_drawn_whatever_is_in_flight() {
        let stride = |flights: usize| flights.div_ceil(MAX_PULSES).max(1);
        assert_eq!(stride(0), 1);
        assert_eq!(stride(MAX_PULSES), 1);
        assert_eq!(stride(MAX_PULSES + 1), 2);
        assert!(
            (0..40_000_usize)
                .step_by(997)
                .all(|flights| flights.div_ceil(stride(flights)) <= MAX_PULSES)
        );
    }

    #[test]
    fn the_raster_merges_rows_into_bins_when_they_outnumber_half_the_plot_height() {
        assert_eq!(raster_bins(0, 60.0), 1);
        assert_eq!(raster_bins(10, 60.0), 10);
        assert_eq!(raster_bins(30, 60.0), 30);
        assert_eq!(raster_bins(31, 60.0), 30);
        assert_eq!(raster_bins(1000, 61.0), 30);
    }

    #[test]
    fn the_raster_title_counts_the_neurons_that_fired() {
        let snapshot = demo(12.0, 64);
        let mut scene = Scene::new();
        run(&mut scene, &snapshot, 3.0);
        let frame = draw(&mut scene, &snapshot, 4.1, None);
        let glyphs = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Glyph { .. }))
            .count();
        assert!(
            glyphs > 60,
            "the title and the layer names are lettered: {glyphs}"
        );
        let legend = scene.legend(view(), &snapshot);
        assert!(
            legend.starts_with(" chi ") && legend.contains(" spikes/s | soma = process"),
            "{legend}"
        );
    }

    #[test]
    fn an_axon_gives_the_point_a_fraction_of_its_length_along() {
        let mut rng = Rng::new(1);
        let ends = [Identity::default(); 2];
        let axon = arc(ends, [0, 1], [10.0, 20.0], [60.0, 30.0], &mut rng);
        assert_eq!(axon.points.len(), 25);
        let (start, end) = (axon.at(0.0), axon.at(1.0));
        assert!((start[2] - (SLICE_HEIGHT - 20.0)).abs() < 1e-3);
        assert!((end[2] - (SLICE_HEIGHT - 30.0)).abs() < 1e-3);
        let (a, b, c) = (axon.at(0.25), axon.at(0.5), axon.at(0.75));
        let gap = |p: Point, q: Point| (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f32>().sqrt();
        assert!(gap(start, a) < gap(start, b) && gap(start, b) < gap(start, c));
        assert!(
            axon.length() > gap(start, end),
            "it sags below the straight line"
        );
        // The delay of the longest crossing is under a second, and a bit under the length of a
        // second's conduction.
        assert!(axon.length() / CONDUCTION < 1.0);
    }
}
