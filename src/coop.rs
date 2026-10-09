//! Coop: the machine as a fenced chicken yard. Every process is a chicken in a Vicsek flock, one
//! flock per cgroup around its henhouse; idle processes roost, running ones peck at the feeder of
//! the CPU they ran on, and eggs, chicks, dust and foxes stand for writes, threads, reads and OOM
//! kills.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::sync::OnceLock;

use crate::model::{CoreKind, Cpu, Identity, Kind, Process, Snapshot, bounded, bytes};
use crate::pack::{Discs, Seats};
use crate::render::{Camera, Color, Frame, NONE, Point, SCALE, Stage, kind_color, spin, tint};
use crate::simulation::{Rng, SpatialHash};

/// Fixed simulation step in seconds. The yard advances on wall-clock time in whole steps, so
/// every frame rate follows the same trajectory.
const H: f32 = 0.05;
/// Steps a single frame may catch up; longer gaps (a stall, a pause) are dropped.
const MAX_STEPS: usize = 10;
/// Vicsek interaction radius in world units: a forager aligns with flock mates nearer than this.
const NEIGHBOURHOOD: f32 = 4.0;
/// Foraging speed `V_MIN + V_SPAN * cpu / (cpu + SPEED_KNEE)` in world units per second.
const V_MIN: f32 = 0.6;
const V_SPAN: f32 = 3.4;
const SPEED_KNEE: f32 = 25.0;
/// Speed of a chicken walking straight to its perch or feeder.
const WALK: f32 = 3.0;
/// Heading noise `eta = ETA_BASE + K_CV * min(cv, 2) + K_PSI * psi / (psi + PSI_KNEE)`, capped at
/// a full turn; each step adds `eta * U(-1/2, 1/2)` radians. Calibrated by
/// `flocks_order_at_low_noise_and_dissolve_past_critical_noise`.
const ETA_BASE: f32 = 0.5;
const K_CV: f32 = 0.5;
const K_PSI: f32 = 5.5;
const PSI_KNEE: f32 = 15.0;
/// Foragers within this distance of a fox's eyes turn away from them.
const FLEE: f32 = 6.0;
/// Roosting hysteresis in percent CPU: roost below ENTER, leave above LEAVE; a newcomer roosts
/// below NEWCOMER.
const ROOST_ENTER: f32 = 0.3;
const ROOST_LEAVE: f32 = 0.8;
const ROOST_NEWCOMER: f32 = 0.5;
/// Samples of CPU kept per process for the coefficient of variation.
const HISTORY: usize = 30;
/// Bytes written per egg, how long an egg stays in the nest, and eggs shown per nest.
const EGG: u64 = 1 << 20;
const EGG_SECONDS: f64 = 60.0;
const NEST_EGGS: usize = 24;
/// Reads faster than this raise dust, one puff per doubling, at most MAX_PUFFS.
const DUST_RATE: f32 = 65536.0;
const MAX_PUFFS: usize = 5;
const MAX_CHICKS: u32 = 12;
/// Trail points kept for chicks, one every TRAIL_EVERY steps (0.1 s).
const TRAIL: usize = 120;
const TRAIL_EVERY: u64 = 2;
const FOX_SECONDS: f32 = 3.0;
const MAX_FOXES: u64 = 3;
/// Fox calls held for a view that is not shown; older ones are dropped as stale.
const MAX_PENDING_CALLS: usize = 2 * MAX_FOXES as usize;
/// How long a member that left its cgroup stays a suspect for an OOM kill the counter reports
/// late: the units are read every 2 s on a background thread, so the count can lag the process
/// list by a couple of samples.
const DEPARTED_SECONDS: f64 = 4.0;
const MAX_EYES: usize = 6;
/// Perches: bars of six seats, four bars to a ladder, stepping up and back.
const SEAT: f32 = 1.2;
const BAR_SEATS: usize = 6;
const LADDER_BARS: usize = 4;
const RUNG_STEP: f32 = 0.9;
const RUNG_RISE: f32 = 0.6;
const LADDER_WIDTH: f32 = SEAT * BAR_SEATS as f32 + 1.0;
const LADDER_DEPTH: f32 = RUNG_STEP * LADDER_BARS as f32 + 1.0;
/// Ladder cell rows sit this far in front of the disc centre; the henhouse takes the row behind.
const LADDER_Y: f32 = 2.3;
const HOUSE_Y: f32 = LADDER_Y - LADDER_DEPTH;
const HOUSE_HALF: [f32; 2] = [2.0, 1.3];
const NEST_HALF: [f32; 2] = [0.6, 0.8];
const NEST_HEIGHT: f32 = 0.7;
const WALL: f32 = 1.9;
const RIDGE: f32 = 3.1;
/// Feeders: slot width, depth of one row of troughs with room for a queue, and the trough.
const SLOT: f32 = 1.6;
/// Queue columns per feeder: straight back, one slot to the right and one to the left, which
/// keeps a queue inside its feeder's lane (feeders are at least 3.6 long with 1.2 between).
const QUEUE_COLUMNS: usize = 3;
const FEEDER_ROW: f32 = 6.0;
const TROUGH_DEPTH: f32 = 0.7;
/// Distance of the hedge outside the fence.
const HEDGE: f32 = 2.2;
/// Grass kept between the houses' reserved discs and the fence.
const YARD_MARGIN: f32 = 2.5;
/// The narrowest yard, wide enough for a row of feeders.
const MIN_YARD: f32 = 24.0;

const PERFORMANCE: Color = [236, 178, 92];
const EFFICIENCY: Color = [72, 183, 199];
const UNKNOWN: Color = [140, 160, 196];
const GRASS: Color = [70, 118, 60];
const DIRT: Color = [118, 94, 66];
const WOOD: Color = [156, 120, 82];
const POST: Color = [118, 88, 60];
const LEAVES: Color = [36, 82, 46];
const BARN: Color = [170, 70, 54];
const ROOF: Color = [86, 74, 70];
const DOOR: Color = [56, 38, 30];
const STRAW: Color = [224, 196, 118];
const GRAIN: Color = [246, 214, 128];
const EGGSHELL: Color = [248, 240, 222];
const COMB: Color = [216, 46, 46];
const BEAK: Color = [244, 168, 48];
const LEGS: Color = [232, 160, 64];
const PUPIL: Color = [18, 18, 22];
const MUD: Color = [90, 62, 40];
const SHADOW: Color = [40, 56, 36];
const DUST: Color = [176, 140, 96];
const CHICK: Color = [252, 220, 112];
const FOX: Color = [214, 108, 38];
const FOX_WHITE: Color = [240, 234, 226];
const FOX_DARK: Color = [52, 36, 30];
const EYE_GLOW: Color = [255, 184, 56];
const GHOST: Color = [160, 160, 168];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Foraging,
    Roosting,
    Feeding,
    Zombie,
    Frozen,
    Stuck,
}

#[derive(Clone, Debug)]
struct Chicken {
    id: Identity,
    flock: usize,
    position: [f32; 2],
    previous: [f32; 2],
    theta: f32,
    speed: f32,
    noise: f32,
    role: Role,
    /// Where a roosting or feeding chicken walks, and whether it has got there.
    target: [f32; 2],
    arrived: bool,
    /// Height of the perch bar a roosting chicken sits on once it arrives.
    perch: f32,
    rng: Rng,
    radius: f32,
    /// Socket peers as (chicken index, weight); only foraging pairs.
    peers: Vec<(usize, f32)>,
    /// Recent positions, newest first, for chicks to follow.
    trail: VecDeque<[f32; 2]>,
    chicks: u32,
    /// Consecutive samples not running, for the feeding hold.
    quiet: u8,
    /// Inspector lines fixed at the last decision (the flock line is added per frame).
    notes: Vec<String>,
}

impl Chicken {
    fn new(id: Identity, flock: usize, position: [f32; 2], theta: f32, rng: Rng) -> Self {
        Self {
            id,
            flock,
            position,
            previous: position,
            theta,
            speed: V_MIN,
            noise: ETA_BASE,
            role: Role::Foraging,
            target: position,
            arrived: false,
            perch: 0.0,
            rng,
            radius: 0.3,
            peers: Vec::new(),
            trail: VecDeque::new(),
            chicks: 0,
            quiet: 0,
            notes: Vec::new(),
        }
    }

    fn walking(&self) -> bool {
        matches!(self.role, Role::Roosting | Role::Feeding) && !self.arrived
    }

    /// On the ground and immovable: separation pushes foragers off it but never moves it.
    fn planted(&self) -> bool {
        match self.role {
            Role::Zombie | Role::Stuck => true,
            Role::Frozen => self.perch == 0.0,
            Role::Feeding => self.arrived,
            Role::Foraging | Role::Roosting => false,
        }
    }

    fn velocity(&self) -> [f32; 2] {
        [self.speed * self.theta.cos(), self.speed * self.theta.sin()]
    }
}

/// A flock's house as the simulation sees it.
#[derive(Clone, Copy, Debug)]
struct Home {
    center: [f32; 2],
    /// Radius around the henhouse that foragers walk around; they may pass under the perches.
    footprint: f32,
    /// Distance from the centre beyond which a forager is drawn home.
    range: f32,
}

/// The pure simulation: chickens sorted by identity inside a fence, with houses and fox eyes.
struct Yard {
    chickens: Vec<Chicken>,
    homes: Vec<Home>,
    low: [f32; 2],
    high: [f32; 2],
    eyes: Vec<[f32; 2]>,
    steps: u64,
    hash: SpatialHash,
    found: Vec<usize>,
}

impl Default for Yard {
    fn default() -> Self {
        Self::new([-10.0, -10.0], [10.0, 10.0])
    }
}

/// Where a chicken's centre may be along one axis of the fence: its radius in from each edge, or
/// the middle of a fence narrower than the chicken.
fn centre_range(low: f32, high: f32, radius: f32) -> (f32, f32) {
    let margin = radius.min((high - low) * 0.5);
    (low + margin, high - margin)
}

/// The point nearest `point` where a chicken of `radius` is inside the fence.
fn keep_inside(point: [f32; 2], radius: f32, low: [f32; 2], high: [f32; 2]) -> [f32; 2] {
    [0, 1].map(|axis| {
        let (lo, hi) = centre_range(low[axis], high[axis], radius);
        point[axis].clamp(lo, hi)
    })
}

fn unit(v: [f32; 2]) -> [f32; 2] {
    let length = v[0].hypot(v[1]);
    if length > 1e-6 {
        [v[0] / length, v[1] / length]
    } else {
        [0.0, 0.0]
    }
}

/// A fixed direction for two coincident chickens, opposite for each of the pair.
fn parting(a: Identity, b: Identity) -> [f32; 2] {
    let (low, high, sign) = if a < b { (a, b, 1.0) } else { (b, a, -1.0) };
    let angle = (low.pid.wrapping_mul(2_654_435_761) ^ high.pid) as f32 / u32::MAX as f32 * TAU;
    [sign * angle.cos(), sign * angle.sin()]
}

impl Yard {
    fn new(low: [f32; 2], high: [f32; 2]) -> Self {
        Self {
            chickens: Vec::new(),
            homes: Vec::new(),
            low,
            high,
            eyes: Vec::new(),
            steps: 0,
            hash: SpatialHash::new(NEIGHBOURHOOD),
            found: Vec::new(),
        }
    }

    /// One synchronous Vicsek step: every new heading comes from the old positions and headings,
    /// then all chickens move, are separated, and are kept inside the fence and out of houses.
    fn step(&mut self, h: f32) {
        self.steps += 1;
        let count = self.chickens.len();
        let positions: Vec<[f32; 2]> = self.chickens.iter().map(|c| c.position).collect();
        let headings: Vec<f32> = self.chickens.iter().map(|c| c.theta).collect();
        // Every chicken draws once per step whatever it is doing, so draws never depend on order.
        let draws: Vec<f32> = self
            .chickens
            .iter_mut()
            .map(|c| c.rng.signed() * 0.5)
            .collect();
        let foragers: Vec<usize> = (0..count)
            .filter(|&i| self.chickens[i].role == Role::Foraging)
            .collect();
        let points: Vec<[f32; 2]> = foragers.iter().map(|&i| positions[i]).collect();
        self.hash.rebuild(&points);
        let mut turned = headings.clone();
        let mut mates: Vec<usize> = Vec::new();
        let mut peers: Vec<(usize, f32)> = Vec::new();
        for &i in &foragers {
            let chicken = &self.chickens[i];
            let here = positions[i];
            self.hash.neighbours(here, NEIGHBOURHOOD, &mut self.found);
            mates.clear();
            mates.extend(
                self.found
                    .iter()
                    .map(|&k| foragers[k])
                    .filter(|&j| self.chickens[j].flock == chicken.flock),
            );
            // Sum in identity order so the result is bit-identical however the Vec is ordered.
            mates.sort_unstable_by_key(|&j| self.chickens[j].id);
            let mut sum = [0.0_f32; 2];
            for &j in &mates {
                sum[0] += headings[j].cos();
                sum[1] += headings[j].sin();
            }
            peers.clear();
            peers.extend(
                chicken
                    .peers
                    .iter()
                    .copied()
                    .filter(|&(k, _)| self.chickens[k].role == Role::Foraging),
            );
            peers.sort_unstable_by_key(|&(k, _)| self.chickens[k].id);
            for &(k, weight) in &peers {
                let toward = unit([positions[k][0] - here[0], positions[k][1] - here[1]]);
                sum[0] += weight * (headings[k].cos() + 0.5 * toward[0]);
                sum[1] += weight * (headings[k].sin() + 0.5 * toward[1]);
            }
            if let Some(home) = self.homes.get(chicken.flock) {
                let away = [home.center[0] - here[0], home.center[1] - here[1]];
                let distance = away[0].hypot(away[1]);
                if distance > home.range {
                    let pull = 0.6 * ((distance - home.range) / 4.0).min(1.0);
                    let direction = unit(away);
                    sum[0] += pull * direction[0];
                    sum[1] += pull * direction[1];
                }
            }
            for eye in &self.eyes {
                let away = [here[0] - eye[0], here[1] - eye[1]];
                let distance = away[0].hypot(away[1]);
                if distance < FLEE {
                    let push = 1.5 * (1.0 - distance / FLEE);
                    let direction = unit(away);
                    sum[0] += push * direction[0];
                    sum[1] += push * direction[1];
                }
            }
            let base = if sum[0].hypot(sum[1]) > 1e-6 {
                sum[1].atan2(sum[0])
            } else {
                headings[i]
            };
            turned[i] = (base + chicken.noise * draws[i]).rem_euclid(TAU);
        }
        for (i, chicken) in self.chickens.iter_mut().enumerate() {
            chicken.previous = positions[i];
            match chicken.role {
                Role::Foraging => {
                    chicken.theta = turned[i];
                    chicken.position[0] += chicken.speed * h * chicken.theta.cos();
                    chicken.position[1] += chicken.speed * h * chicken.theta.sin();
                }
                Role::Roosting | Role::Feeding if !chicken.arrived => {
                    let away = [
                        chicken.target[0] - chicken.position[0],
                        chicken.target[1] - chicken.position[1],
                    ];
                    let distance = away[0].hypot(away[1]);
                    if distance <= WALK * h {
                        chicken.position = chicken.target;
                        chicken.arrived = true;
                        chicken.theta = FRAC_PI_2;
                    } else {
                        chicken.theta = away[1].atan2(away[0]);
                        chicken.position[0] += away[0] / distance * WALK * h;
                        chicken.position[1] += away[1] / distance * WALK * h;
                    }
                }
                _ => {}
            }
        }
        self.separate();
        self.confine();
        if self.steps.is_multiple_of(TRAIL_EVERY) {
            for chicken in &mut self.chickens {
                if chicken.chicks == 0 {
                    chicken.trail.clear();
                    continue;
                }
                chicken.trail.push_front(chicken.position);
                chicken.trail.truncate(TRAIL);
            }
        }
    }

    /// Jacobi separation: overlaps measured before any push, then every push applied at once.
    /// Two foragers share an overlap; a forager against a planted chicken takes all of it.
    fn separate(&mut self) {
        let members: Vec<usize> = (0..self.chickens.len())
            .filter(|&i| {
                let chicken = &self.chickens[i];
                chicken.role == Role::Foraging || chicken.planted()
            })
            .collect();
        if members.is_empty() {
            return;
        }
        let points: Vec<[f32; 2]> = members.iter().map(|&i| self.chickens[i].position).collect();
        let widest = members
            .iter()
            .map(|&i| self.chickens[i].radius)
            .fold(0.0, f32::max);
        self.hash.rebuild(&points);
        let mut pushes: Vec<(usize, [f32; 2])> = Vec::new();
        let mut others: Vec<usize> = Vec::new();
        for (slot, &i) in members.iter().enumerate() {
            let chicken = &self.chickens[i];
            if chicken.role != Role::Foraging {
                continue;
            }
            self.hash
                .neighbours(points[slot], chicken.radius + widest, &mut self.found);
            others.clear();
            others.extend(self.found.iter().map(|&k| members[k]).filter(|&j| j != i));
            others.sort_unstable_by_key(|&j| self.chickens[j].id);
            let mut push = [0.0_f32; 2];
            for &j in &others {
                let other = &self.chickens[j];
                let away = [
                    chicken.position[0] - other.position[0],
                    chicken.position[1] - other.position[1],
                ];
                let distance = away[0].hypot(away[1]);
                let overlap = chicken.radius + other.radius - distance;
                if overlap <= 0.0 {
                    continue;
                }
                let share = if other.role == Role::Foraging {
                    0.5
                } else {
                    1.0
                };
                let direction = if distance > 1e-5 {
                    [away[0] / distance, away[1] / distance]
                } else {
                    parting(chicken.id, other.id)
                };
                push[0] += direction[0] * overlap * share;
                push[1] += direction[1] * overlap * share;
            }
            pushes.push((i, push));
        }
        for (i, push) in pushes {
            self.chickens[i].position[0] += push[0];
            self.chickens[i].position[1] += push[1];
        }
    }

    /// The fence reflects: a chicken past an edge is mirrored back and its heading reflected.
    /// Foragers are also pushed out of house footprints, their heading reflected off the wall.
    fn confine(&mut self) {
        let (low, high) = (self.low, self.high);
        for chicken in &mut self.chickens {
            if chicken.role == Role::Foraging {
                for home in &self.homes {
                    let away = [
                        chicken.position[0] - home.center[0],
                        chicken.position[1] - home.center[1],
                    ];
                    let distance = away[0].hypot(away[1]);
                    let limit = home.footprint + chicken.radius;
                    if distance >= limit {
                        continue;
                    }
                    let normal = if distance > 1e-5 {
                        [away[0] / distance, away[1] / distance]
                    } else {
                        [chicken.theta.cos(), chicken.theta.sin()]
                    };
                    chicken.position = [
                        home.center[0] + normal[0] * limit,
                        home.center[1] + normal[1] * limit,
                    ];
                    let heading = [chicken.theta.cos(), chicken.theta.sin()];
                    let inward = heading[0] * normal[0] + heading[1] * normal[1];
                    if inward < 0.0 {
                        let reflected = [
                            heading[0] - 2.0 * inward * normal[0],
                            heading[1] - 2.0 * inward * normal[1],
                        ];
                        chicken.theta = reflected[1].atan2(reflected[0]).rem_euclid(TAU);
                    }
                }
            }
            for axis in 0..2 {
                let (lo, hi) = centre_range(low[axis], high[axis], chicken.radius);
                let value = &mut chicken.position[axis];
                let mut reflected = false;
                if *value < lo {
                    *value = 2.0 * lo - *value;
                    reflected = true;
                } else if *value > hi {
                    *value = 2.0 * hi - *value;
                    reflected = true;
                }
                *value = value.clamp(lo, hi);
                if reflected && chicken.role == Role::Foraging {
                    chicken.theta = if axis == 0 {
                        PI - chicken.theta
                    } else {
                        -chicken.theta
                    }
                    .rem_euclid(TAU);
                }
            }
        }
    }

    /// The Vicsek order parameter over foragers: |sum of velocities| / sum of speeds.
    fn order(&self) -> Option<f32> {
        order_of(self.chickens.iter().filter(|c| c.role == Role::Foraging))
    }

    /// The order parameter and forager count of each of `flocks` flocks, in one pass; `None` for
    /// a flock with no foragers.
    fn flock_orders(&self, flocks: usize) -> Vec<Option<(f32, usize)>> {
        let mut totals = vec![([0.0_f32; 2], 0.0_f32, 0_usize); flocks];
        for chicken in self.chickens.iter().filter(|c| c.role == Role::Foraging) {
            let v = chicken.velocity();
            let (sum, speeds, count) = &mut totals[chicken.flock];
            sum[0] += v[0];
            sum[1] += v[1];
            *speeds += chicken.speed;
            *count += 1;
        }
        totals
            .into_iter()
            .map(|(sum, speeds, count)| {
                (speeds > 0.0).then(|| (sum[0].hypot(sum[1]) / speeds, count))
            })
            .collect()
    }

    fn find(&self, id: Identity) -> Option<&Chicken> {
        self.chickens
            .binary_search_by_key(&id, |c| c.id)
            .ok()
            .map(|i| &self.chickens[i])
    }
}

fn order_of<'a>(chickens: impl Iterator<Item = &'a Chicken>) -> Option<f32> {
    let mut sum = [0.0_f32; 2];
    let mut speeds = 0.0;
    for chicken in chickens {
        let v = chicken.velocity();
        sum[0] += v[0];
        sum[1] += v[1];
        speeds += chicken.speed;
    }
    (speeds > 0.0).then(|| sum[0].hypot(sum[1]) / speeds)
}

/// Bytes written since a process was first seen, and the eggs it has laid for them.
struct Ledger {
    first: u64,
    laid: u64,
}

struct Egg {
    laid: f64,
    slot: usize,
}

#[derive(Default)]
struct Nest {
    eggs: VecDeque<Egg>,
    count: usize,
}

/// An OOM kill recorded but not yet sent into the yard.
struct Call {
    flock: String,
    /// The sample time of the kill, so a call outlived by its fox is dropped.
    at: f64,
    victim: Option<Identity>,
    /// Where the victim was last drawn and its radius, once the yard has been checked.
    seen: Option<([f32; 2], f32)>,
    delay: f32,
}

/// A member that has left its cgroup, kept briefly so a late OOM-kill count can name it.
struct Departed {
    cgroup: String,
    id: Identity,
    memory: u64,
    /// The sample it was first missing from.
    gone: f64,
    /// Where it was last drawn and its radius, once the yard has been checked.
    seen: Option<([f32; 2], f32)>,
}

struct Fox {
    start: f32,
    entry: [f32; 2],
    prey: [f32; 2],
    exit: [f32; 2],
    /// Radius of the departed member it carries off.
    ghost: Option<f32>,
}

struct Feeder {
    cpu: u32,
    kind: CoreKind,
    busy: f32,
    center: [f32; 2],
    slots: usize,
}

impl Feeder {
    fn length(&self) -> f32 {
        self.slots as f32 * SLOT + 0.4
    }

    /// Where eating chickens stand, behind the trough as seen from the yard.
    fn eating_line(&self) -> f32 {
        self.center[1] - TROUGH_DEPTH * 0.5 - 0.7
    }

    fn slot(&self, slot: usize) -> [f32; 2] {
        [
            self.center[0] + (slot as f32 - (self.slots as f32 - 1.0) * 0.5) * SLOT,
            self.eating_line(),
        ]
    }
}

struct Flock {
    key: String,
    label: String,
    members: usize,
    center: [f32; 2],
    span: usize,
}

/// The persistent coop view.
#[derive(Default)]
pub struct Coop {
    recorded: Option<f64>,
    /// Seconds between the last two recorded samples.
    interval: f64,
    history: HashMap<Identity, VecDeque<f32>>,
    ledgers: HashMap<Identity, Ledger>,
    nests: HashMap<String, Nest>,
    kills: HashMap<String, u64>,
    /// Members of each cgroup in the last recorded sample with their memory, for fox victims.
    members: HashMap<String, Vec<(Identity, u64)>>,
    departed: Vec<Departed>,
    calls: Vec<Call>,
    foxes: Vec<Fox>,
    seats: Seats,
    discs: Discs,
    flocks: Vec<Flock>,
    feeders: Vec<Feeder>,
    /// The northern edge of the feeder band, just south of the houses.
    band_top: f32,
    /// The bounds of every reserved house disc seen so far, and the deepest feeder band. The
    /// fence is derived from them and so only ever moves outward.
    extent: Option<([f32; 2], [f32; 2])>,
    band: f32,
    yard: Yard,
    /// The sample the roles were last decided for, and the processes they were decided over.
    decided: Option<f64>,
    drawn: Vec<Identity>,
    clock: Option<f32>,
    accumulator: f32,
    phi: Option<f32>,
    pressure_missing: bool,
    unreadable: usize,
}

/// The flock a process belongs to: its cgroup, or its group without one; kernel threads share one.
fn flock_key(process: &Process) -> String {
    if process.kind == Kind::Kernel {
        "kernel".into()
    } else if process.cgroup.is_empty() {
        process.group.clone()
    } else {
        process.cgroup.clone()
    }
}

fn flock_label(process: &Process) -> String {
    if process.kind == Kind::Kernel {
        "kernel".into()
    } else {
        process.group.clone()
    }
}

/// Body radius from memory: volume proportional to memory, from 0.3 at about 16 MiB and below
/// (0.3 / 0.12 = 2.5, and 2.5 cubed is 15.6) to 1.2 at 1000 MiB and above, so small processes
/// stay visible and large ones keep their spread.
fn body_radius(memory: f32) -> f32 {
    (0.12 * (memory / 1_048_576.0).max(0.0).cbrt()).clamp(0.3, 1.2)
}

/// Ladder centres around a house in the order they fill, nearest first and the house's front
/// before its back. The cell grid is fixed, so a seat never moves while its house stays put.
fn ladder(index: usize) -> [f32; 2] {
    static CELLS: OnceLock<Vec<[f32; 2]>> = OnceLock::new();
    let cells = CELLS.get_or_init(|| {
        let mut cells: Vec<(f32, i32, i32)> = Vec::new();
        for row in -6..=8_i32 {
            for column in -6..=6_i32 {
                if (column, row) == (0, -1) {
                    continue;
                }
                let center = [
                    column as f32 * LADDER_WIDTH,
                    LADDER_Y + row as f32 * LADDER_DEPTH,
                ];
                let behind = if row < 0 { 2.0 } else { 0.0 };
                cells.push((center[0].hypot(center[1]) + behind, -row, -column));
            }
        }
        cells.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.1, a.2).cmp(&(b.1, b.2))));
        cells
            .into_iter()
            .map(|(_, row, column)| {
                [
                    -column as f32 * LADDER_WIDTH,
                    LADDER_Y - row as f32 * LADDER_DEPTH,
                ]
            })
            .collect()
    });
    match cells.get(index) {
        Some(&cell) => cell,
        None => [
            0.0,
            LADDER_Y + (9 + index - cells.len()) as f32 * LADDER_DEPTH,
        ],
    }
}

/// A seat's ground point relative to its house's disc centre, and its perch height.
fn seat_offset(seat: usize) -> ([f32; 2], f32) {
    let cell = ladder(seat / (BAR_SEATS * LADDER_BARS));
    let bar = seat % (BAR_SEATS * LADDER_BARS) / BAR_SEATS;
    let place = seat % BAR_SEATS;
    (
        [
            cell[0] + (place as f32 - (BAR_SEATS as f32 - 1.0) * 0.5) * SEAT,
            cell[1] + RUNG_STEP * (1.5 - bar as f32),
        ],
        bar_height(bar),
    )
}

fn bar_height(bar: usize) -> f32 {
    RUNG_RISE * (bar as f32 + 1.0)
}

/// Radius of the disc a house and its perches for `span` seats need.
fn house_radius(span: usize) -> f32 {
    let ladders = span.div_ceil(BAR_SEATS * LADDER_BARS).max(1);
    let house = (HOUSE_HALF[0] + 2.0 * NEST_HALF[0]).hypot(-HOUSE_Y + HOUSE_HALF[1]);
    (0..ladders)
        .map(|g| {
            let cell = ladder(g);
            (cell[0].abs() + LADDER_WIDTH * 0.5).hypot(cell[1].abs() + LADDER_DEPTH * 0.4)
        })
        .fold(house, f32::max)
        + 0.5
}

/// The distance from home beyond which a forager is drawn back: two and a half house radii.
fn home_range(radius: f32) -> f32 {
    (2.5 * radius).max(6.0)
}

fn feeder_color(kind: CoreKind) -> Color {
    match kind {
        CoreKind::Performance => PERFORMANCE,
        CoreKind::Efficiency => EFFICIENCY,
        CoreKind::Unknown => UNKNOWN,
    }
}

fn kind_name(kind: CoreKind) -> &'static str {
    match kind {
        CoreKind::Performance => "performance",
        CoreKind::Efficiency => "efficiency",
        CoreKind::Unknown => "unknown",
    }
}

/// Feeders in rows along the south edge, sorted by kind then id, and the depth of their band.
/// Feeders in rows below the houses, between `left` and `right`, the first row's troughs just
/// south of `top`. Returns the feeders and the depth of the band they fill.
fn place_feeders(snapshot: &Snapshot, left: f32, right: f32, top: f32) -> (Vec<Feeder>, f32) {
    let mut cpus: Vec<Cpu> = snapshot.cpus.clone();
    if cpus.is_empty() {
        cpus = (0..snapshot.cores.max(1) as u32)
            .map(|id| Cpu {
                id,
                kind: CoreKind::Unknown,
                busy: 0.0,
                mhz: 0.0,
                wait: 0.0,
            })
            .collect();
    }
    cpus.sort_by_key(|cpu| (cpu.kind, cpu.id));
    let mut feeders = Vec::with_capacity(cpus.len());
    let (mut cursor, mut row) = (left + 1.0, 0);
    for cpu in &cpus {
        let slots = if cpu.kind == CoreKind::Performance {
            3
        } else {
            2
        };
        let length = slots as f32 * SLOT + 0.4;
        if cursor + length > right - 1.0 && cursor > left + 1.0 {
            cursor = left + 1.0;
            row += 1;
        }
        feeders.push(Feeder {
            cpu: cpu.id,
            kind: cpu.kind,
            busy: cpu.busy.clamp(0.0, 1.0),
            center: [cursor + length * 0.5, top - 1.2 - row as f32 * FEEDER_ROW],
            slots,
        });
        cursor += length + 1.2;
    }
    (feeders, (row + 1) as f32 * FEEDER_ROW)
}

/// Coefficient of variation of the recorded CPU samples; 0 when too few or too idle to judge.
fn variation(samples: Option<&VecDeque<f32>>) -> f32 {
    let Some(samples) = samples.filter(|s| s.len() >= 3) else {
        return 0.0;
    };
    let count = samples.len() as f32;
    let mean = samples.iter().sum::<f32>() / count;
    if mean < 0.5 {
        return 0.0;
    }
    let spread = samples.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / count;
    spread.sqrt() / mean
}

fn noise(cv: f32, psi: f32) -> f32 {
    (ETA_BASE + K_CV * cv.min(2.0) + K_PSI * bounded(psi, PSI_KNEE)).min(TAU)
}

fn speed(cpu: f32) -> f32 {
    V_MIN + V_SPAN * bounded(cpu, SPEED_KNEE)
}

/// Peer weight from measured loopback traffic in bytes per second, or co-activity without it.
fn peer_weight(traffic: Option<f32>, cpu: f32, other: f32) -> f32 {
    match traffic {
        Some(rate) => ((1.0 + rate.max(0.0) / 1024.0).log2() / 10.0).min(1.5),
        None => 0.8 * bounded(cpu.min(other), 25.0),
    }
}

fn rate(value: f32) -> String {
    bytes(value.max(0.0) as u64)
}

fn hash(value: u32) -> u32 {
    let mut x = value.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    x
}

fn fraction(value: u32) -> f32 {
    (hash(value) >> 8) as f32 / 16_777_216.0
}

impl Coop {
    /// Takes in a new sample: CPU history, eggs, OOM kills and cgroup membership. Older or
    /// repeated samples are ignored, so the scene and the view may both call it.
    pub fn record(&mut self, snapshot: &Snapshot) {
        if self
            .recorded
            .is_some_and(|elapsed| snapshot.elapsed <= elapsed)
        {
            return;
        }
        if let Some(previous) = self.recorded {
            self.interval = snapshot.elapsed - previous;
        }
        self.recorded = Some(snapshot.elapsed);
        let present: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        self.history.retain(|id, _| present.contains(id));
        self.ledgers.retain(|id, _| present.contains(id));
        for process in &snapshot.processes {
            let samples = self.history.entry(process.id).or_default();
            samples.push_back(process.cpu);
            if samples.len() > HISTORY {
                samples.pop_front();
            }
            let Some(written) = process.written else {
                continue;
            };
            let ledger = self.ledgers.entry(process.id).or_insert(Ledger {
                first: written,
                laid: 0,
            });
            let eggs = written.saturating_sub(ledger.first) / EGG;
            if eggs > ledger.laid {
                let fresh = eggs - ledger.laid;
                ledger.laid = eggs;
                let nest = self.nests.entry(flock_key(process)).or_default();
                let shown = fresh.min(NEST_EGGS as u64) as usize;
                nest.count += (fresh as usize).saturating_sub(shown);
                for _ in 0..shown {
                    if nest.eggs.len() == NEST_EGGS {
                        nest.eggs.pop_front();
                    }
                    nest.eggs.push_back(Egg {
                        laid: snapshot.elapsed,
                        slot: nest.count % NEST_EGGS,
                    });
                    nest.count += 1;
                }
            }
        }
        for nest in self.nests.values_mut() {
            while nest
                .eggs
                .front()
                .is_some_and(|egg| snapshot.elapsed - egg.laid >= EGG_SECONDS)
            {
                nest.eggs.pop_front();
            }
        }
        self.nests.retain(|_, nest| !nest.eggs.is_empty());
        // Members that left since the last sample stay suspects for a few seconds, because the
        // kill count comes from the slower background sampler and can rise after they are gone.
        let mut left: Vec<Departed> = self
            .members
            .iter()
            .flat_map(|(cgroup, members)| {
                members
                    .iter()
                    .filter(|(id, _)| !present.contains(id))
                    .map(|&(id, memory)| Departed {
                        cgroup: cgroup.clone(),
                        id,
                        memory,
                        gone: snapshot.elapsed,
                        seen: None,
                    })
            })
            .collect();
        left.sort_by_key(|gone| gone.id);
        self.departed.extend(left);
        // Two and a half sampling intervals cover a kill counted up to two samples late when
        // --sample-ms is longer than the default.
        let window = DEPARTED_SECONDS.max(2.5 * self.interval);
        self.departed
            .retain(|gone| snapshot.elapsed - gone.gone <= window);
        let mut paths: Vec<&String> = snapshot.units.keys().collect();
        paths.sort();
        for path in paths {
            let kills = snapshot.units[path].oom_kills;
            let seen = *self.kills.entry(path.clone()).or_insert(kills);
            if kills > seen {
                for k in 0..(kills - seen).min(MAX_FOXES) as usize {
                    // The largest departed member that no earlier kill has claimed.
                    let victim = self
                        .departed
                        .iter()
                        .enumerate()
                        .filter(|(_, gone)| gone.cgroup == *path)
                        .max_by(|(_, a), (_, b)| a.memory.cmp(&b.memory).then(b.id.cmp(&a.id)))
                        .map(|(index, _)| index)
                        .map(|index| self.departed.remove(index));
                    self.calls.push(Call {
                        flock: path.clone(),
                        at: snapshot.elapsed,
                        victim: victim.as_ref().map(|gone| gone.id),
                        seen: victim.and_then(|gone| gone.seen),
                        delay: k as f32 * 0.8,
                    });
                }
            }
            self.kills.insert(path.clone(), kills);
        }
        self.expire_calls(snapshot.elapsed);
        self.kills
            .retain(|path, _| snapshot.units.contains_key(path));
        self.members.clear();
        for process in snapshot.processes.iter().filter(|p| !p.cgroup.is_empty()) {
            self.members
                .entry(process.cgroup.clone())
                .or_default()
                .push((process.id, process.memory));
        }
    }

    /// Drops fox calls whose fox would already have left, and the oldest beyond the cap, so calls
    /// recorded while another view is shown do not replay on switching to this one.
    fn expire_calls(&mut self, sample: f64) {
        self.calls
            .retain(|call| sample - call.at <= FOX_SECONDS as f64);
        let excess = self.calls.len().saturating_sub(MAX_PENDING_CALLS);
        self.calls.drain(..excess);
    }

    /// The status legend, with the yard's order parameter and what could not be measured.
    pub fn legend(&self) -> String {
        let phi = self
            .phi
            .map_or_else(|| "- (none foraging)".to_owned(), |phi| format!("{phi:.2}"));
        let mut line = format!(
            " Chicken = process, size = memory | flock = cgroup | foraging speed = CPU, roost = idle | feeder = CPU it ran on, pecking order = priority | chicks = threads | eggs = MiB written | dust = reads | fox = OOM kill, eyes = memory pressure | phi {phi}"
        );
        if self.pressure_missing {
            line.push_str(" | no CPU pressure: noise from CPU variation only");
        }
        if self.unreadable > 0 {
            line.push_str(&format!(" | I/O unreadable for {}", self.unreadable));
        }
        line
    }

    pub fn draw(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        snapshot: &Snapshot,
    ) -> usize {
        self.record(snapshot);
        let mut order: Vec<usize> = (0..processes.len()).collect();
        order.sort_by_key(|&i| processes[i].id);
        let ids: Vec<Identity> = order.iter().map(|&i| processes[i].id).collect();
        // Victims are looked up before the new sample removes them from the yard, and the foxes
        // sent after it, so they run to the yard as it is now laid out.
        for call in &mut self.calls {
            if let Some(chicken) = call.victim.and_then(|id| self.yard.find(id)) {
                call.seen = Some((chicken.position, chicken.radius));
            }
        }
        for gone in &mut self.departed {
            if let Some(chicken) = self.yard.find(gone.id) {
                gone.seen = Some((chicken.position, chicken.radius));
            }
        }
        let fresh = self.decided != Some(snapshot.elapsed);
        if fresh || ids != self.drawn {
            self.decide(processes, &order, snapshot, fresh);
            self.decided = Some(snapshot.elapsed);
            self.drawn = ids;
        }
        self.summon(stage.time, snapshot.elapsed);
        let alpha = self.advance(stage.time);
        self.phi = self.yard.order();
        self.paint(stage, processes, &order, snapshot, alpha)
    }

    /// Steps the yard on wall-clock time; returns how far the frame lies into the next step.
    fn advance(&mut self, time: f32) -> f32 {
        let dt = match self.clock {
            Some(last) if time >= last => (time - last).min(0.5),
            _ => 0.0,
        };
        self.clock = Some(time);
        self.accumulator += dt;
        let mut steps = 0;
        while self.accumulator >= H && steps < MAX_STEPS {
            self.yard.step(H);
            self.accumulator -= H;
            steps += 1;
        }
        self.accumulator = self.accumulator.min(H);
        (self.accumulator / H).clamp(0.0, 1.0)
    }

    /// Turns recorded OOM kills into fox runs: to where the victim was last drawn, or to its
    /// flock's house when it was not drawn.
    fn summon(&mut self, time: f32, sample: f64) {
        self.expire_calls(sample);
        for call in std::mem::take(&mut self.calls) {
            let home = self
                .flocks
                .iter()
                .find(|flock| flock.key == call.flock)
                .map(|flock| flock.center);
            let middle = [
                (self.yard.low[0] + self.yard.high[0]) * 0.5,
                (self.yard.low[1] + self.yard.high[1]) * 0.5,
            ];
            let prey = call
                .seen
                .map_or(home.unwrap_or(middle), |(position, _)| position);
            let (entry, exit) = self.hedge_gap(prey);
            self.foxes.push(Fox {
                start: time + call.delay,
                entry,
                prey,
                exit,
                ghost: call.seen.map(|(_, radius)| radius),
            });
        }
        self.foxes
            .retain(|fox| (-10.0..FOX_SECONDS).contains(&(time - fox.start)));
    }

    /// The hedge point nearest `point`, and one further along the same side to leave by.
    fn hedge_gap(&self, point: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (low, high) = (self.yard.low, self.yard.high);
        let distances = [
            point[0] - low[0],
            high[0] - point[0],
            point[1] - low[1],
            high[1] - point[1],
        ];
        let side = (0..4)
            .min_by(|&a, &b| distances[a].total_cmp(&distances[b]))
            .unwrap_or(0);
        let along = |value: f32, lo: f32, hi: f32| {
            let shifted = value + 7.0;
            if shifted > hi - 1.0 {
                value - 7.0
            } else {
                shifted.max(lo + 1.0)
            }
        };
        match side {
            0 => (
                [low[0] - HEDGE, point[1]],
                [low[0] - HEDGE, along(point[1], low[1], high[1])],
            ),
            1 => (
                [high[0] + HEDGE, point[1]],
                [high[0] + HEDGE, along(point[1], low[1], high[1])],
            ),
            2 => (
                [point[0], low[1] - HEDGE],
                [along(point[0], low[0], high[0]), low[1] - HEDGE],
            ),
            _ => (
                [point[0], high[1] + HEDGE],
                [along(point[0], low[0], high[0]), high[1] + HEDGE],
            ),
        }
    }

    /// Lays out houses, perches and feeders and decides each chicken's role, target, speed, noise
    /// and peers from the measured sample. Runs once per new sample or when the drawn set changes.
    fn decide(
        &mut self,
        processes: &[&Process],
        order: &[usize],
        snapshot: &Snapshot,
        fresh: bool,
    ) {
        let raw: HashMap<Identity, &Process> =
            snapshot.processes.iter().map(|p| (p.id, p)).collect();
        let measured =
            |i: usize| -> &Process { raw.get(&processes[i].id).copied().unwrap_or(processes[i]) };
        let mut groups: BTreeMap<String, (String, Vec<Identity>)> = BTreeMap::new();
        for &i in order {
            let process = measured(i);
            groups
                .entry(flock_key(process))
                .or_insert_with(|| (flock_label(process), Vec::new()))
                .1
                .push(process.id);
        }
        self.seats.assign(
            groups
                .iter()
                .flat_map(|(key, (_, ids))| ids.iter().map(move |&id| (id, key.as_str()))),
        );
        let needs: Vec<(String, f32)> = groups
            .keys()
            .map(|key| (key.clone(), house_radius(self.seats.span(key))))
            .collect();
        let centers = self.discs.arrange(&needs);
        // The fence hugs the houses' reserved discs, and only grows: a flock at the edge arriving
        // or leaving must not move the fence, the feeders along it or the fox eyes around it.
        // It is at least MIN_YARD wide for the feeders.
        let (mut low, mut high) = self.extent.unwrap_or(([f32::MAX; 2], [f32::MIN; 2]));
        for (key, center) in &centers {
            let reserved = self.discs.reserved(key);
            for axis in 0..2 {
                low[axis] = low[axis].min(center[axis] - reserved - YARD_MARGIN);
                high[axis] = high[axis].max(center[axis] + reserved + YARD_MARGIN);
            }
        }
        if low[0] > high[0] {
            (low, high) = ([-MIN_YARD * 0.5; 2], [MIN_YARD * 0.5; 2]);
        } else {
            self.extent = Some((low, high));
        }
        for axis in 0..2 {
            let spare = (MIN_YARD - (high[axis] - low[axis])).max(0.0) * 0.5;
            low[axis] -= spare;
            high[axis] += spare;
        }
        let (feeders, band) = place_feeders(snapshot, low[0], high[0], low[1]);
        self.feeders = feeders;
        self.band_top = low[1];
        self.band = self.band.max(band);
        self.yard.low = [low[0], low[1] - self.band];
        self.yard.high = high;
        self.flocks = groups
            .iter()
            .map(|(key, (label, ids))| {
                let span = self.seats.span(key);
                Flock {
                    key: key.clone(),
                    label: label.clone(),
                    members: ids.len(),
                    center: centers.get(key).copied().unwrap_or([0.0, 0.0]),
                    span,
                }
            })
            .collect();
        self.yard.homes = self
            .flocks
            .iter()
            .map(|flock| {
                let radius = house_radius(flock.span);
                Home {
                    center: [flock.center[0], flock.center[1] + HOUSE_Y],
                    footprint: 0.6 * radius,
                    range: home_range(radius),
                }
            })
            .collect();
        let flock_of: HashMap<&str, usize> = self
            .flocks
            .iter()
            .enumerate()
            .map(|(index, flock)| (flock.key.as_str(), index))
            .collect();

        let mut old = std::mem::take(&mut self.yard.chickens)
            .into_iter()
            .peekable();
        let first = old.peek().is_none();
        let mut chickens: Vec<Chicken> = Vec::with_capacity(order.len());
        let mut newcomers: Vec<bool> = Vec::with_capacity(order.len());
        for &i in order {
            let process = measured(i);
            while old.peek().is_some_and(|c| c.id < process.id) {
                old.next();
            }
            let flock = flock_of[flock_key(process).as_str()];
            if let Some(chicken) = old.next_if(|c| c.id == process.id) {
                chickens.push(chicken);
                newcomers.push(false);
                continue;
            }
            // Newcomers hatch at their flock's gathering spot beyond the perches, close enough
            // to their mates to align with them; the spot's direction is fixed by the flock name.
            let mut rng = Rng::for_identity(process.id, 0xc0_0b);
            let home = self.yard.homes[flock];
            let toward = spin(&self.flocks[flock].key);
            let spot = home.footprint + 2.5;
            let scatter = 1.0 + 0.35 * (self.flocks[flock].members as f32).sqrt();
            let (angle, distance) = (rng.unit() * TAU, scatter * rng.unit().sqrt());
            let position = [
                home.center[0] + spot * toward.cos() + distance * angle.cos(),
                home.center[1] + spot * toward.sin() + distance * angle.sin(),
            ];
            let theta = rng.unit() * TAU;
            chickens.push(Chicken::new(process.id, flock, position, theta, rng));
            newcomers.push(true);
        }

        let psi = snapshot.pressure[0];
        self.pressure_missing = snapshot.missing.contains(&"pressure");
        for (k, &i) in order.iter().enumerate() {
            let process = measured(i);
            let chicken = &mut chickens[k];
            chicken.flock = flock_of[flock_key(process).as_str()];
            if process.state == 'R' {
                chicken.quiet = 0;
            } else if fresh {
                chicken.quiet = chicken.quiet.saturating_add(1);
            }
            if fresh || newcomers[k] {
                let previous = (!newcomers[k]).then_some(chicken.role);
                let role = match process.state {
                    'Z' => Role::Zombie,
                    'T' | 't' => Role::Frozen,
                    'D' => Role::Stuck,
                    'R' => Role::Feeding,
                    // The feeding hold: a chicken leaves the feeder only after two samples in a
                    // row without running, so R/S flicker on 1 s samples does not shuttle it.
                    _ if previous == Some(Role::Feeding) && chicken.quiet < 2 => Role::Feeding,
                    _ => {
                        let threshold = match previous {
                            None => ROOST_NEWCOMER,
                            Some(Role::Roosting) => ROOST_LEAVE,
                            Some(_) => ROOST_ENTER,
                        };
                        let idle = if previous == Some(Role::Roosting) {
                            process.cpu <= threshold
                        } else {
                            process.cpu < threshold
                        };
                        if idle { Role::Roosting } else { Role::Foraging }
                    }
                };
                if role != chicken.role || newcomers[k] {
                    chicken.arrived = false;
                    if matches!(role, Role::Zombie | Role::Stuck) {
                        chicken.perch = 0.0;
                    }
                    if role == Role::Foraging {
                        chicken.perch = 0.0;
                    }
                }
                chicken.role = role;
            }
            chicken.radius = body_radius(process.memory as f32);
            chicken.speed = speed(process.cpu);
            chicken.chicks = process.threads.saturating_sub(1).min(MAX_CHICKS);
            let cv = variation(self.history.get(&process.id));
            chicken.noise = noise(cv, psi);
            chicken.peers.clear();
            let mut notes = Vec::new();
            notes.push(String::new());
            notes.push(format!(
                "noise +/-{:.0} deg (CPU variation CV {cv:.2}, CPU pressure {})",
                chicken.noise * 0.5 * 180.0 / PI,
                if self.pressure_missing {
                    "unavailable".to_owned()
                } else {
                    format!("{psi:.0}%")
                }
            ));
            match (process.written, self.ledgers.get(&process.id)) {
                (Some(written), Some(ledger)) => {
                    // Tenths of a MiB rounded down, so the count never reads above the eggs.
                    let tenths = written.saturating_sub(ledger.first) * 10 / EGG;
                    let mut line = format!(
                        "eggs {} ({}.{} MiB written)",
                        ledger.laid,
                        tenths / 10,
                        tenths % 10
                    );
                    if let Some(rate) = process.write_rate.filter(|&r| r > 0.0) {
                        line.push_str(&format!(", writing {}/s", self::rate(rate)));
                    }
                    notes.push(line);
                }
                _ => notes.push("I/O unreadable".into()),
            }
            if let Some(read) = process.read_rate.filter(|&r| r > DUST_RATE) {
                notes.push(format!(
                    "dust bathing: {} puff{} (reading {}/s)",
                    puffs(read),
                    if puffs(read) == 1 { "" } else { "s" },
                    rate(read)
                ));
            }
            if process.threads > 1 {
                notes.push(format!(
                    "chicks {} (threads {})",
                    chicken.chicks, process.threads
                ));
            }
            chicken.notes = notes;
        }

        let seat_of = |id: Identity, flock: &Flock| -> ([f32; 2], f32) {
            let (offset, height) = seat_offset(self.seats.seat(id).unwrap_or(0));
            (
                [flock.center[0] + offset[0], flock.center[1] + offset[1]],
                height,
            )
        };
        let mut eating: Vec<Vec<(i32, Identity, usize)>> = vec![Vec::new(); self.feeders.len()];
        let feeder_of: HashMap<u32, usize> = self
            .feeders
            .iter()
            .enumerate()
            .map(|(index, feeder)| (feeder.cpu, index))
            .collect();
        for (k, &i) in order.iter().enumerate() {
            let process = measured(i);
            let chicken = &mut chickens[k];
            match chicken.role {
                Role::Roosting => {
                    let (seat, height) = seat_of(chicken.id, &self.flocks[chicken.flock]);
                    if seat != chicken.target {
                        chicken.arrived = false;
                    }
                    chicken.target = seat;
                    chicken.perch = height;
                }
                Role::Feeding => {
                    let feeder = feeder_of.get(&process.core).copied().unwrap_or(0);
                    eating[feeder].push((process.priority, process.id, k));
                }
                _ => {}
            }
        }
        for (index, line) in eating.iter_mut().enumerate() {
            line.sort();
            let feeder = &self.feeders[index];
            let queue_start = feeder.eating_line() - 1.3;
            let floor = (feeder.center[1] - (FEEDER_ROW - 1.2)).max(self.yard.low[1]);
            // The queue runs south behind the eating line and wraps into at most QUEUE_COLUMNS
            // columns within the feeder's own lane (straight back, then right, then left), so
            // neighbouring feeders' queues never meet. A queue too long for them packs tighter.
            let needed: f32 = line
                .iter()
                .skip(feeder.slots)
                .map(|&(_, _, k)| 2.0 * chickens[k].radius + 0.25)
                .sum();
            let room = QUEUE_COLUMNS as f32 * (queue_start - floor).max(0.1);
            let packing = if needed > room { room / needed } else { 1.0 };
            let (mut cursor, mut column) = (queue_start, 0_usize);
            for (rank, &(priority, _, k)) in line.iter().enumerate() {
                let process = measured(order[k]);
                let chicken = &mut chickens[k];
                let target = if rank < feeder.slots {
                    feeder.slot(rank)
                } else {
                    let step = (2.0 * chicken.radius + 0.25) * packing;
                    if cursor - step < floor - 0.01
                        && cursor < queue_start
                        && column + 1 < QUEUE_COLUMNS
                    {
                        column += 1;
                        cursor = queue_start;
                    }
                    let side = if column % 2 == 1 { 1.0 } else { -1.0 };
                    let across = column.div_ceil(2) as f32 * SLOT * side;
                    cursor -= step * 0.5;
                    let spot = [feeder.center[0] + across, cursor];
                    cursor -= step * 0.5;
                    spot
                };
                let target = keep_inside(target, chicken.radius, self.yard.low, self.yard.high);
                if target != chicken.target {
                    chicken.arrived = false;
                }
                chicken.target = target;
                chicken.perch = 0.0;
                let standing = if process.nice != 0 || priority >= 0 {
                    format!("priority {priority} (nice {})", process.nice)
                } else {
                    format!("priority {priority} (real-time)")
                };
                let mut note = format!(
                    "feeding at cpu{} ({}), pecking rank {} of {} by {standing}",
                    feeder.cpu,
                    kind_name(feeder.kind),
                    rank + 1,
                    line.len()
                );
                if rank >= feeder.slots {
                    note.push_str(", waiting in line");
                }
                if feeder.cpu != process.core {
                    note.push_str(&format!(
                        " (ran on cpu{}, which has no feeder)",
                        process.core
                    ));
                }
                if process.state != 'R' {
                    note.push_str(" (held: not running for one sample)");
                }
                chicken.notes[0] = note;
            }
        }
        for chicken in &mut chickens {
            if chicken.role == Role::Feeding {
                continue;
            }
            chicken.notes[0] = match chicken.role {
                Role::Foraging => "foraging".into(),
                Role::Roosting => "roosting".into(),
                Role::Zombie => "feet up: zombie, waiting for its parent to reap it".into(),
                Role::Frozen => "frozen: stopped (tonic immobility)".into(),
                Role::Stuck => "stuck in the mud: uninterruptible sleep (D)".into(),
                Role::Feeding => unreachable!(),
            };
        }
        if first {
            // The first population starts where it belongs rather than walking in from home.
            for chicken in &mut chickens {
                if chicken.walking() {
                    chicken.position = keep_inside(
                        chicken.target,
                        chicken.radius,
                        self.yard.low,
                        self.yard.high,
                    );
                    chicken.previous = chicken.position;
                    chicken.arrived = true;
                    chicken.theta = FRAC_PI_2;
                }
            }
        }

        let index_of: HashMap<Identity, usize> = chickens
            .iter()
            .enumerate()
            .map(|(k, c)| (c.id, k))
            .collect();
        let mut strongest: Vec<Option<(f32, usize, Option<f32>)>> = vec![None; chickens.len()];
        let mut pairs: Vec<(Identity, Identity)> = snapshot
            .links
            .iter()
            .map(|&(a, b, _)| if a < b { (a, b) } else { (b, a) })
            .collect();
        pairs.sort();
        pairs.dedup();
        for (a, b) in pairs {
            let (Some(&i), Some(&j)) = (index_of.get(&a), index_of.get(&b)) else {
                continue;
            };
            if i == j || chickens[i].role != Role::Foraging || chickens[j].role != Role::Foraging {
                continue;
            }
            let traffic = snapshot
                .link_traffic
                .get(&(a, b))
                .or_else(|| snapshot.link_traffic.get(&(b, a)))
                .copied();
            let weight = peer_weight(traffic, measured(order[i]).cpu, measured(order[j]).cpu);
            if weight <= 0.0 {
                continue;
            }
            chickens[i].peers.push((j, weight));
            chickens[j].peers.push((i, weight));
            for (me, other) in [(i, j), (j, i)] {
                if strongest[me].is_none_or(|(w, _, _)| weight > w) {
                    strongest[me] = Some((weight, other, traffic));
                }
            }
        }
        for (k, best) in strongest.into_iter().enumerate() {
            if let Some((_, other, traffic)) = best {
                let how = match traffic {
                    Some(rate) => format!("{}/s over TCP", self::rate(rate)),
                    None => "both busy".into(),
                };
                let name = &measured(order[other]).name;
                chickens[k].notes.push(format!("walks with {name} ({how})"));
            }
        }
        self.unreadable = order
            .iter()
            .filter(|&&i| measured(i).written.is_none())
            .count();
        self.yard.chickens = chickens;

        let psi_memory = snapshot.pressure[1];
        self.yard.eyes.clear();
        if psi_memory > 0.5 {
            let pairs = (1 + (psi_memory / 10.0) as usize).min(MAX_EYES);
            let outside = 3.0 * (1.0 - bounded(psi_memory, 20.0));
            for k in 0..pairs {
                self.yard
                    .eyes
                    .push(self.perimeter(fraction(k as u32 + 17), outside + 0.4));
            }
        }
    }

    /// A point `outside` units beyond the fence at fraction `t` of the way round it.
    fn perimeter(&self, t: f32, outside: f32) -> [f32; 2] {
        let (low, high) = (self.yard.low, self.yard.high);
        let (width, height) = (high[0] - low[0], high[1] - low[1]);
        let mut d = t.rem_euclid(1.0) * 2.0 * (width + height);
        if d < width {
            return [low[0] + d, high[1] + outside];
        }
        d -= width;
        if d < height {
            return [high[0] + outside, high[1] - d];
        }
        d -= height;
        if d < width {
            return [high[0] - d, low[1] - outside];
        }
        d -= width;
        [low[0] - outside, low[1] + d]
    }

    fn paint(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        order: &[usize],
        snapshot: &Snapshot,
        alpha: f32,
    ) -> usize {
        let camera = stage.camera;
        let time = stage.time;
        let grows: Vec<f32> = order
            .iter()
            .map(|&index| 0.3 + 0.7 * stage.growth(processes[index].id))
            .collect();
        let frame = &mut *stage.frame;
        let (low, high) = (self.yard.low, self.yard.high);
        draw_ground(frame, camera, low, high, self.band_top);
        draw_hedge(frame, camera, low, high);
        let psi_memory = snapshot.pressure[1];
        for (k, eye) in self.yard.eyes.iter().enumerate() {
            // Blinking is decorative.
            let blink = (time * 0.7 + k as f32 * 2.3).sin() > 0.93;
            if blink {
                continue;
            }
            let tangent = if eye[0] < low[0] || eye[0] > high[0] {
                [0.0, 1.0]
            } else {
                [1.0, 0.0]
            };
            for side in [-1.0, 1.0] {
                let p = [
                    eye[0] + tangent[0] * 0.22 * side,
                    eye[1] + tangent[1] * 0.22 * side,
                    0.75,
                ];
                frame.glow(
                    camera,
                    p,
                    (0.35 * camera.zoom).max(3.0),
                    EYE_GLOW,
                    0.55 + 0.4 * bounded(psi_memory, 20.0),
                );
                frame.world_sphere(camera, p, 0.07, [255, 220, 120]);
            }
        }
        draw_fence(frame, camera, low, high);
        for feeder in &self.feeders {
            draw_feeder(frame, camera, feeder);
        }
        let now = snapshot.elapsed;
        for flock in &self.flocks {
            draw_house(frame, camera, flock);
            if let Some(nest) = self.nests.get(&flock.key) {
                let nest_center = [
                    flock.center[0] + HOUSE_HALF[0] + NEST_HALF[0],
                    flock.center[1] + HOUSE_Y,
                ];
                for egg in &nest.eggs {
                    let age = (now - egg.laid) as f32;
                    if !(0.0..EGG_SECONDS as f32).contains(&age) {
                        continue;
                    }
                    let fade = age / EGG_SECONDS as f32;
                    let (column, row) = (egg.slot % 4, egg.slot / 4);
                    let size = 0.15 * (1.0 - 0.5 * fade);
                    let color = mix(EGGSHELL, STRAW, fade);
                    frame.world_sphere(
                        camera,
                        [
                            nest_center[0] + (column as f32 - 1.5) * 0.24,
                            nest_center[1] + (row as f32 - 2.5) * 0.24,
                            NEST_HEIGHT + 0.02 + size,
                        ],
                        size / SCALE,
                        color,
                    );
                }
            }
        }

        let orders = self.yard.flock_orders(self.flocks.len());
        let mut shown = 0;
        for (k, &index) in order.iter().enumerate() {
            let process = processes[index];
            let Some(chicken) = self.yard.chickens.get(k).filter(|c| c.id == process.id) else {
                continue;
            };
            let mut ground = [
                chicken.previous[0] + (chicken.position[0] - chicken.previous[0]) * alpha,
                chicken.previous[1] + (chicken.position[1] - chicken.previous[1]) * alpha,
            ];
            if chicken.role == Role::Frozen {
                ground = chicken.position;
            }
            let r = body_radius(process.memory as f32) * grows[k];
            let selected = stage.selected == Some(process.id);
            let body = draw_chicken(
                frame,
                camera,
                chicken,
                ground,
                r,
                kind_color(process),
                index as u32,
                selected,
                time,
            );
            if let Some(read) = process.read_rate.filter(|&rate| rate > DUST_RATE) {
                for puff in 0..puffs(read) {
                    // The swirl is decorative; the number of puffs is the measurement.
                    let angle = puff as f32 / puffs(read) as f32 * TAU + time * 1.7;
                    let p = [
                        ground[0] + angle.cos() * r * 1.1,
                        ground[1] + angle.sin() * r * 1.1,
                        0.15 + 0.2 * (time * 2.0 + puff as f32).sin().abs(),
                    ];
                    frame.glow(camera, p, (0.55 * r * camera.zoom).max(3.0), DUST, 0.55);
                }
            }
            draw_chicks(frame, camera, chicken, ground, body, r, time);
            stage.positions.insert(process.id, body);
            let flock = &self.flocks[chicken.flock];
            let phi = match orders[chicken.flock] {
                Some((phi, foraging)) => format!("phi {phi:.2} ({foraging} foraging)"),
                None => "(none foraging)".into(),
            };
            let mut notes = vec![format!("flock {}: {phi}", flock.label)];
            notes.extend(chicken.notes.iter().cloned());
            stage.notes.insert(process.id, notes);
            shown += 1;
        }
        for fox in &self.foxes {
            draw_fox(frame, camera, fox, time);
        }
        let mut ranked: Vec<&Flock> = self.flocks.iter().collect();
        ranked.sort_by(|a, b| b.members.cmp(&a.members).then(a.key.cmp(&b.key)));
        let chosen = stage
            .selected
            .and_then(|id| self.yard.find(id))
            .map(|c| c.flock);
        for (rank, flock) in ranked.iter().enumerate() {
            let selected = chosen.is_some_and(|f| self.flocks[f].key == flock.key);
            if rank < 10 || selected {
                stage.places.push((
                    [flock.center[0], flock.center[1] + HOUSE_Y, RIDGE + 0.3],
                    flock.label.clone(),
                ));
            }
        }
        for x in [low[0], high[0]] {
            for y in [low[1], high[1]] {
                stage.bounds.push([x, y, 0.0]);
                stage.bounds.push([x, y, 3.0]);
            }
        }
        shown
    }
}

fn puffs(read: f32) -> usize {
    (1 + (read / DUST_RATE).log2().max(0.0) as usize).min(MAX_PUFFS)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t.clamp(0.0, 1.0)) as u8)
}

/// Shading of a vertical face by its outward direction, from a fixed light in the north west.
fn shade(color: Color, normal: [f32; 2]) -> Color {
    tint(color, 0.72 + 0.2 * (-0.6 * normal[0] + 0.8 * normal[1]))
}

/// An axis-aligned box: its top and four sides, each shaded by facing.
fn cuboid(frame: &mut Frame, camera: &Camera, low: Point, high: Point, color: Color) {
    let [x0, y0, z0] = low;
    let [x1, y1, z1] = high;
    frame.quad(
        camera,
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        tint(color, 1.05),
        NONE,
    );
    for (points, normal) in [
        (
            [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
            [0.0, -1.0],
        ),
        (
            [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
            [0.0, 1.0],
        ),
        (
            [[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]],
            [-1.0, 0.0],
        ),
        (
            [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
            [1.0, 0.0],
        ),
    ] {
        frame.quad(camera, points, shade(color, normal), NONE);
    }
}

/// Grass inside and around the fence, with packed earth under the feeders. Decorative.
fn draw_ground(frame: &mut Frame, camera: &Camera, low: [f32; 2], high: [f32; 2], band_top: f32) {
    let margin = HEDGE + 2.5;
    let (x0, x1) = (low[0] - margin, high[0] + margin);
    let (y0, y1) = (low[1] - margin, high[1] + margin);
    let columns = ((x1 - x0) / 1.5).ceil().clamp(8.0, 48.0) as usize;
    let rows = ((y1 - y0) / 1.5).ceil().clamp(8.0, 48.0) as usize;
    let (dx, dy) = ((x1 - x0) / columns as f32, (y1 - y0) / rows as f32);
    for j in 0..rows {
        for i in 0..columns {
            let (ax, ay) = (x0 + i as f32 * dx, y0 + j as f32 * dy);
            let cell = (j * 64 + i) as u32;
            let swirl = (ax * 0.21).sin() * (ay * 0.17).cos();
            let inside = ax + dx > low[0] && ax < high[0] && ay + dy > low[1] && ay < high[1];
            let lush = if inside { 1.0 } else { 0.82 };
            let color = tint(GRASS, lush * (0.9 + 0.1 * swirl + 0.12 * fraction(cell)));
            frame.quad(
                camera,
                [
                    [ax, ay, 0.0],
                    [ax + dx, ay, 0.0],
                    [ax + dx, ay + dy, 0.0],
                    [ax, ay + dy, 0.0],
                ],
                color,
                NONE,
            );
        }
    }
    let (top, bottom) = (band_top + 0.3, low[1]);
    let depth = top - bottom;
    if depth > 0.0 {
        let rows = (depth / 1.5).ceil().clamp(1.0, 24.0) as usize;
        let columns = ((high[0] - low[0]) / 1.5).ceil().clamp(4.0, 48.0) as usize;
        let (dx, dy) = ((high[0] - low[0]) / columns as f32, depth / rows as f32);
        for j in 0..rows {
            for i in 0..columns {
                let (ax, ay) = (low[0] + i as f32 * dx, bottom + j as f32 * dy);
                let color = tint(DIRT, 0.9 + 0.15 * fraction((j * 64 + i) as u32 + 9000));
                frame.quad(
                    camera,
                    [
                        [ax, ay, 0.01],
                        [ax + dx, ay, 0.01],
                        [ax + dx, ay + dy, 0.01],
                        [ax, ay + dy, 0.01],
                    ],
                    color,
                    NONE,
                );
            }
        }
    }
}

/// Posts and two rails around the yard; the fence is the reflecting boundary.
fn draw_fence(frame: &mut Frame, camera: &Camera, low: [f32; 2], high: [f32; 2]) {
    let corners = [
        [low[0], low[1]],
        [high[0], low[1]],
        [high[0], high[1]],
        [low[0], high[1]],
    ];
    for side in 0..4 {
        let (a, b) = (corners[side], corners[(side + 1) % 4]);
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        let posts = (length / 2.4).ceil().max(1.0) as usize;
        for k in 0..posts {
            let t = k as f32 / posts as f32;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            cuboid(
                frame,
                camera,
                [p[0] - 0.09, p[1] - 0.09, 0.0],
                [p[0] + 0.09, p[1] + 0.09, 1.15],
                POST,
            );
        }
        for height in [0.45, 0.9] {
            frame.quad(
                camera,
                [
                    [a[0], a[1], height],
                    [b[0], b[1], height],
                    [b[0], b[1], height + 0.1],
                    [a[0], a[1], height + 0.1],
                ],
                WOOD,
                NONE,
            );
        }
    }
}

/// Dark shrubs in a double row outside the fence. Decorative and never pickable.
fn draw_hedge(frame: &mut Frame, camera: &Camera, low: [f32; 2], high: [f32; 2]) {
    for (layer, offset) in [(0_u32, HEDGE + 0.9), (1, HEDGE - 0.2)] {
        let (a, b) = (
            [low[0] - offset, low[1] - offset],
            [high[0] + offset, high[1] + offset],
        );
        let perimeter = 2.0 * (b[0] - a[0] + b[1] - a[1]);
        let count = (perimeter / 1.6).ceil() as u32;
        for k in 0..count {
            let seed = k * 2 + layer;
            let mut d = (k as f32 + 0.5 * layer as f32) / count as f32 * perimeter;
            let width = b[0] - a[0];
            let height = b[1] - a[1];
            let p = if d < width {
                [a[0] + d, b[1]]
            } else {
                d -= width;
                if d < height {
                    [b[0], b[1] - d]
                } else {
                    d -= height;
                    if d < width {
                        [b[0] - d, a[1]]
                    } else {
                        [a[0], a[1] + d - width]
                    }
                }
            };
            let radius = 0.85 + 0.45 * fraction(seed);
            let color = tint(LEAVES, 0.8 + 0.4 * fraction(seed + 7777));
            frame.world_sphere(camera, [p[0], p[1], radius * 0.8], radius, color);
        }
    }
}

fn draw_feeder(frame: &mut Frame, camera: &Camera, feeder: &Feeder) {
    let [x, y] = feeder.center;
    let half = [feeder.length() * 0.5, TROUGH_DEPTH * 0.5];
    let color = feeder_color(feeder.kind);
    cuboid(
        frame,
        camera,
        [x - half[0], y - half[1], 0.0],
        [x + half[0], y + half[1], 0.45],
        tint(color, 0.75),
    );
    // Grain brightness is how busy the CPU was, as the lanes of the cores view.
    frame.quad(
        camera,
        [
            [x - half[0] + 0.1, y - half[1] + 0.1, 0.46],
            [x + half[0] - 0.1, y - half[1] + 0.1, 0.46],
            [x + half[0] - 0.1, y + half[1] - 0.1, 0.46],
            [x - half[0] + 0.1, y + half[1] - 0.1, 0.46],
        ],
        tint(GRAIN, 0.25 + 0.85 * feeder.busy),
        NONE,
    );
}

/// The henhouse with a pitched roof and door, its nest box, and its perch ladders.
fn draw_house(frame: &mut Frame, camera: &Camera, flock: &Flock) {
    let [cx, cy] = [flock.center[0], flock.center[1] + HOUSE_Y];
    let [hx, hy] = HOUSE_HALF;
    cuboid(
        frame,
        camera,
        [cx - hx, cy - hy, 0.0],
        [cx + hx, cy + hy, WALL],
        BARN,
    );
    let eave = 0.18;
    for side in [-1.0_f32, 1.0] {
        frame.quad(
            camera,
            [
                [cx - hx - eave, cy, RIDGE],
                [cx + hx + eave, cy, RIDGE],
                [cx + hx + eave, cy + side * (hy + eave), WALL - 0.1],
                [cx - hx - eave, cy + side * (hy + eave), WALL - 0.1],
            ],
            tint(ROOF, if side > 0.0 { 1.15 } else { 0.85 }),
            NONE,
        );
    }
    for side in [-1.0_f32, 1.0] {
        frame.facet(
            camera,
            [
                [cx + side * hx, cy - hy, WALL],
                [cx + side * hx, cy + hy, WALL],
                [cx + side * hx, cy, RIDGE - 0.05],
            ],
            shade(BARN, [side, 0.0]),
            NONE,
        );
    }
    frame.quad(
        camera,
        [
            [cx - 0.3, cy + hy + 0.01, 0.0],
            [cx + 0.3, cy + hy + 0.01, 0.0],
            [cx + 0.3, cy + hy + 0.01, 0.85],
            [cx - 0.3, cy + hy + 0.01, 0.85],
        ],
        DOOR,
        NONE,
    );
    let nest = [cx + hx + NEST_HALF[0], cy];
    let [nx, ny] = NEST_HALF;
    cuboid(
        frame,
        camera,
        [nest[0] - nx, nest[1] - ny, 0.0],
        [nest[0] + nx, nest[1] + ny, NEST_HEIGHT - 0.05],
        tint(WOOD, 0.85),
    );
    frame.quad(
        camera,
        [
            [nest[0] - nx + 0.08, nest[1] - ny + 0.08, NEST_HEIGHT],
            [nest[0] + nx - 0.08, nest[1] - ny + 0.08, NEST_HEIGHT],
            [nest[0] + nx - 0.08, nest[1] + ny - 0.08, NEST_HEIGHT],
            [nest[0] - nx + 0.08, nest[1] + ny - 0.08, NEST_HEIGHT],
        ],
        STRAW,
        NONE,
    );
    let seats = flock.span.max(1);
    let per_ladder = BAR_SEATS * LADDER_BARS;
    for g in 0..seats.div_ceil(per_ladder) {
        let cell = ladder(g);
        let bars = (seats - g * per_ladder)
            .div_ceil(BAR_SEATS)
            .min(LADDER_BARS);
        let [lx, ly] = [flock.center[0] + cell[0], flock.center[1] + cell[1]];
        let reach = SEAT * BAR_SEATS as f32 * 0.5;
        let front = ly + RUNG_STEP * 1.5 + 0.3;
        let back = ly + RUNG_STEP * (1.5 - (bars as f32 - 1.0)) - 0.25;
        let top = bar_height(bars - 1);
        for side in [-1.0_f32, 1.0] {
            let x = lx + side * (reach + 0.05);
            frame.quad(
                camera,
                [
                    [x, front, 0.0],
                    [x, back, top + 0.05],
                    [x, back, top + 0.2],
                    [x, front, 0.15],
                ],
                POST,
                NONE,
            );
            frame.quad(
                camera,
                [
                    [x, back - 0.06, 0.0],
                    [x, back + 0.06, 0.0],
                    [x, back + 0.06, top + 0.15],
                    [x, back - 0.06, top + 0.15],
                ],
                tint(POST, 0.85),
                NONE,
            );
        }
        for bar in 0..bars {
            let y = ly + RUNG_STEP * (1.5 - bar as f32);
            let z = bar_height(bar);
            frame.quad(
                camera,
                [
                    [lx - reach, y - 0.1, z],
                    [lx + reach, y - 0.1, z],
                    [lx + reach, y + 0.1, z],
                    [lx - reach, y + 0.1, z],
                ],
                WOOD,
                NONE,
            );
            frame.quad(
                camera,
                [
                    [lx - reach, y + 0.1, z - 0.14],
                    [lx + reach, y + 0.1, z - 0.14],
                    [lx + reach, y + 0.1, z],
                    [lx - reach, y + 0.1, z],
                ],
                tint(WOOD, 0.8),
                NONE,
            );
        }
    }
}

/// A part of a chicken: pickable at full size when large enough to draw true to size, otherwise
/// an unclamped, unpickable sphere, so small parts never swell to the minimum body size.
fn part(frame: &mut Frame, camera: &Camera, p: Point, radius: f32, color: Color, pick: u32) {
    let pixels = radius * camera.zoom;
    if pick != NONE && pixels >= crate::render::MIN_BODY_PX {
        frame.sphere(camera, p, radius, color, pick, false);
    } else if pixels >= 0.45 {
        frame.world_sphere(camera, p, radius / SCALE, color);
    }
}

fn offset(base: Point, forward: [f32; 2], f: f32, side: f32, up: f32) -> Point {
    [
        base[0] + forward[0] * f - forward[1] * side,
        base[1] + forward[1] * f + forward[0] * side,
        base[2] + up,
    ]
}

/// Draws one chicken in the pose of its role and returns its body centre: a plump body of
/// three spheres, neck, head with comb, beak, wattle and eyes, wings, a tail fan and legs.
#[allow(clippy::too_many_arguments)]
fn draw_chicken(
    frame: &mut Frame,
    camera: &Camera,
    chicken: &Chicken,
    ground: [f32; 2],
    r: f32,
    plumage: Color,
    pick: u32,
    selected: bool,
    time: f32,
) -> Point {
    let forward = [chicken.theta.cos(), chicken.theta.sin()];
    let phase = (chicken.id.pid % 97) as f32;
    let seated = chicken.role == Role::Roosting || chicken.perch > 0.0;
    let walking = chicken.role == Role::Foraging || chicken.walking();
    // Hopping up to the perch over the last unit of the walk.
    let lift = if seated {
        let away = (chicken.target[0] - ground[0]).hypot(chicken.target[1] - ground[1]);
        chicken.perch * (1.0 - away).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let perched = seated && lift > 0.0;
    let zombie = chicken.role == Role::Zombie;
    // Body height, then the head's offset forward and up, in body radii.
    let (height, head_forward, head_up, neck) = match chicken.role {
        Role::Zombie => (r * 0.8, 1.05, -0.3, false),
        Role::Frozen => (lift + r * 0.72, 0.85, 0.45, false),
        _ if perched => (lift + r * 0.6, 0.7, 0.5, false),
        Role::Feeding if chicken.arrived => {
            // Pecking is decorative: the head bobs into the trough.
            let peck = (time * 7.0 + phase).sin().abs();
            (r * 1.3, 1.15, 0.25 - 0.45 * peck, true)
        }
        _ => (r * 1.3, 0.85, 0.95, true),
    };
    let up = if zombie { -1.0 } else { 1.0 };
    let body = [ground[0], ground[1], height];
    if !perched && pick != NONE {
        // A soft shadow grounds the bird; decorative.
        let shade = r * 1.05;
        for k in 0..6 {
            let (a, b) = (k as f32 / 6.0 * TAU, (k + 1) as f32 / 6.0 * TAU);
            frame.facet(
                camera,
                [
                    [ground[0], ground[1], 0.015],
                    [
                        ground[0] + shade * a.cos(),
                        ground[1] + shade * a.sin(),
                        0.015,
                    ],
                    [
                        ground[0] + shade * b.cos(),
                        ground[1] + shade * b.sin(),
                        0.015,
                    ],
                ],
                SHADOW,
                NONE,
            );
        }
    }
    if chicken.role == Role::Stuck {
        let puddle = r * 1.8;
        for k in 0..10 {
            let (a, b) = (k as f32 / 10.0 * TAU, (k + 1) as f32 / 10.0 * TAU);
            let wobble = |angle: f32| puddle * (0.85 + 0.15 * (angle * 3.0 + phase).sin());
            frame.facet(
                camera,
                [
                    [ground[0], ground[1], 0.02],
                    [
                        ground[0] + wobble(a) * a.cos(),
                        ground[1] + wobble(a) * 0.8 * a.sin(),
                        0.02,
                    ],
                    [
                        ground[0] + wobble(b) * b.cos(),
                        ground[1] + wobble(b) * 0.8 * b.sin(),
                        0.02,
                    ],
                ],
                if k % 2 == 0 { MUD } else { tint(MUD, 0.88) },
                NONE,
            );
        }
    }
    if zombie {
        for side in [-0.3, 0.3] {
            let hip = offset(body, forward, -0.1 * r, side * r, 0.5 * r);
            let knee = offset(body, forward, -0.15 * r, side * 1.2 * r, 1.25 * r);
            let foot = offset(body, forward, 0.05 * r, side * 1.3 * r, 1.6 * r);
            frame.line(camera, hip, knee, LEGS);
            frame.line(camera, knee, foot, LEGS);
        }
    } else if !perched && chicken.role != Role::Frozen {
        let stride = if walking {
            (time * (4.0 + 3.0 * chicken.speed) + phase).sin() * 0.35 * r
        } else {
            0.0
        };
        for (side, sign) in [(-0.3, 1.0), (0.3, -1.0)] {
            let hip = offset(body, forward, 0.0, side * r, -0.5 * r);
            let foot = [
                ground[0] + forward[0] * stride * sign - forward[1] * side * r,
                ground[1] + forward[1] * stride * sign + forward[0] * side * r,
                0.0,
            ];
            frame.line(camera, hip, foot, LEGS);
        }
    }
    let feathers = tint(plumage, 0.72);
    for side in [-0.3_f32, 0.0, 0.3] {
        frame.facet(
            camera,
            [
                offset(body, forward, -0.6 * r, (side - 0.25) * r, up * 0.2 * r),
                offset(body, forward, -0.6 * r, (side + 0.25) * r, up * 0.2 * r),
                offset(body, forward, -1.05 * r, side * 1.1 * r, up * 1.05 * r),
            ],
            feathers,
            pick,
        );
    }
    frame.sphere(camera, body, r, plumage, pick, selected);
    part(
        frame,
        camera,
        offset(body, forward, -0.5 * r, 0.0, up * 0.15 * r),
        0.72 * r,
        tint(plumage, 0.92),
        pick,
    );
    part(
        frame,
        camera,
        offset(body, forward, 0.45 * r, 0.0, up * 0.05 * r),
        0.75 * r,
        tint(plumage, 1.05),
        pick,
    );
    for side in [-1.0_f32, 1.0] {
        frame.facet(
            camera,
            [
                offset(body, forward, 0.3 * r, side * 0.9 * r, up * 0.3 * r),
                offset(body, forward, -0.75 * r, side * 0.8 * r, up * 0.4 * r),
                offset(body, forward, -0.35 * r, side * 0.82 * r, -up * 0.2 * r),
            ],
            tint(plumage, 0.8),
            pick,
        );
    }
    let head = offset(body, forward, head_forward * r, 0.0, up * head_up * r);
    let head_radius = 0.45 * r;
    if neck {
        part(
            frame,
            camera,
            offset(body, forward, 0.65 * r, 0.0, (head_up * 0.5 + 0.1) * r),
            0.38 * r,
            plumage,
            pick,
        );
    }
    part(frame, camera, head, head_radius, tint(plumage, 1.08), pick);
    let comb_radius = 0.18 * r;
    if comb_radius * camera.zoom >= 1.2 {
        for (along, rise) in [(0.35, 0.8), (-0.05, 0.95), (-0.45, 0.75)] {
            part(
                frame,
                camera,
                offset(
                    head,
                    forward,
                    along * head_radius,
                    0.0,
                    up * rise * head_radius,
                ),
                comb_radius * if along.abs() < 0.1 { 1.0 } else { 0.8 },
                COMB,
                NONE,
            );
        }
    } else {
        frame.facet(
            camera,
            [
                offset(
                    head,
                    forward,
                    0.5 * head_radius,
                    0.0,
                    up * 0.5 * head_radius,
                ),
                offset(
                    head,
                    forward,
                    -0.7 * head_radius,
                    0.0,
                    up * 0.5 * head_radius,
                ),
                offset(
                    head,
                    forward,
                    -0.1 * head_radius,
                    0.0,
                    up * 1.5 * head_radius,
                ),
            ],
            COMB,
            NONE,
        );
    }
    let tip = offset(
        head,
        forward,
        1.8 * head_radius,
        0.0,
        -up * 0.15 * head_radius,
    );
    frame.facet(
        camera,
        [
            offset(
                head,
                forward,
                0.8 * head_radius,
                0.0,
                up * 0.25 * head_radius,
            ),
            offset(
                head,
                forward,
                0.8 * head_radius,
                0.0,
                -up * 0.35 * head_radius,
            ),
            tip,
        ],
        BEAK,
        NONE,
    );
    frame.facet(
        camera,
        [
            offset(head, forward, 0.8 * head_radius, -0.3 * head_radius, 0.0),
            offset(head, forward, 0.8 * head_radius, 0.3 * head_radius, 0.0),
            tip,
        ],
        tint(BEAK, 0.85),
        NONE,
    );
    part(
        frame,
        camera,
        offset(
            head,
            forward,
            0.75 * head_radius,
            0.0,
            -up * 0.75 * head_radius,
        ),
        0.25 * head_radius,
        COMB,
        NONE,
    );
    for side in [-0.6, 0.6] {
        part(
            frame,
            camera,
            offset(
                head,
                forward,
                0.55 * head_radius,
                side * head_radius,
                up * 0.25 * head_radius,
            ),
            0.16 * head_radius,
            PUPIL,
            NONE,
        );
    }
    body
}

/// Chicks for threads: in a file behind a moving hen along its trail, tucked beside a still one.
fn draw_chicks(
    frame: &mut Frame,
    camera: &Camera,
    chicken: &Chicken,
    ground: [f32; 2],
    body: Point,
    r: f32,
    time: f32,
) {
    if chicken.chicks == 0 {
        return;
    }
    let size = 0.2;
    let moving = chicken.role == Role::Foraging || chicken.walking();
    let forward = [chicken.theta.cos(), chicken.theta.sin()];
    let mut cursor = ground;
    let mut points = chicken.trail.iter();
    let mut travelled = 0.0;
    for k in 0..chicken.chicks {
        let p = if moving {
            let wanted = r + 0.45 + k as f32 * 0.6;
            let mut spot = None;
            while travelled < wanted {
                let Some(&next) = points.next() else {
                    break;
                };
                let step = (next[0] - cursor[0]).hypot(next[1] - cursor[1]);
                if travelled + step >= wanted && step > 1e-6 {
                    let t = (wanted - travelled) / step;
                    let at = [
                        cursor[0] + (next[0] - cursor[0]) * t,
                        cursor[1] + (next[1] - cursor[1]) * t,
                    ];
                    travelled = wanted;
                    cursor = at;
                    spot = Some(at);
                    break;
                }
                travelled += step;
                cursor = next;
            }
            let at = spot.unwrap_or_else(|| {
                // A short trail: the rest of the brood bunches up at its end.
                let angle = k as f32 * 2.4;
                [
                    cursor[0] + 0.15 * angle.cos(),
                    cursor[1] + 0.15 * angle.sin(),
                ]
            });
            let hop = 0.05 * (time * 9.0 + k as f32).sin().abs();
            [at[0], at[1], size + hop]
        } else {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let along = (k / 2) as f32;
            let base = [body[0], body[1], (body[2] - r * 0.45).max(size)];
            offset(
                base,
                forward,
                0.35 * r - along * 0.25,
                side * (0.75 * r + 0.15 + along * 0.12),
                0.0,
            )
        };
        let look = if moving { forward } else { [0.0, 1.0] };
        part(frame, camera, p, size, CHICK, NONE);
        part(
            frame,
            camera,
            offset(p, look, 0.15, 0.0, 0.14),
            0.11,
            tint(CHICK, 1.05),
            NONE,
        );
    }
}

/// The fox: in through the hedge to its victim, then away with a grey ghost in its mouth.
fn draw_fox(frame: &mut Frame, camera: &Camera, fox: &Fox, time: f32) {
    let u = (time - fox.start) / FOX_SECONDS;
    if !(0.0..1.0).contains(&u) {
        return;
    }
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let lerp =
        |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let (at, heading, carrying) = if u < 0.42 {
        (
            lerp(fox.entry, fox.prey, ease(u / 0.42)),
            unit([fox.prey[0] - fox.entry[0], fox.prey[1] - fox.entry[1]]),
            false,
        )
    } else if u < 0.55 {
        (
            fox.prey,
            unit([fox.prey[0] - fox.entry[0], fox.prey[1] - fox.entry[1]]),
            true,
        )
    } else {
        (
            lerp(fox.prey, fox.exit, ease((u - 0.55) / 0.45)),
            unit([fox.exit[0] - fox.prey[0], fox.exit[1] - fox.prey[1]]),
            true,
        )
    };
    let forward = if heading == [0.0, 0.0] {
        [1.0, 0.0]
    } else {
        heading
    };
    let running = !(0.42..0.55).contains(&u);
    let size = 1.4;
    let gallop = if running { (time * 14.0).sin() } else { 0.0 };
    let body = [at[0], at[1], size * 0.95 + 0.08 * gallop.abs()];
    for (along, side) in [(0.55, -0.25), (0.55, 0.25), (-0.55, -0.25), (-0.55, 0.25)] {
        let swing = gallop * 0.3 * if along > 0.0 { 1.0 } else { -1.0 };
        let hip = offset(body, forward, along * size, side * size, -0.3 * size);
        let foot = offset(
            [at[0], at[1], 0.0],
            forward,
            (along + swing) * size,
            side * size,
            0.0,
        );
        frame.line(camera, hip, foot, FOX_DARK);
    }
    let mut tail = offset(body, forward, -0.95 * size, 0.0, 0.05 * size);
    for (k, girth) in [0.24_f32, 0.34, 0.4, 0.36, 0.26].into_iter().enumerate() {
        let color = if k == 4 { FOX_WHITE } else { FOX };
        frame.world_sphere(camera, tail, girth * size / SCALE, color);
        let sway = 0.06 * gallop * size * k as f32;
        tail = offset(
            tail,
            forward,
            -0.3 * size,
            sway,
            (0.1 + 0.03 * k as f32) * size,
        );
    }
    for (along, girth) in [(-0.5_f32, 0.48_f32), (0.0, 0.52), (0.5, 0.55)] {
        frame.world_sphere(
            camera,
            offset(body, forward, along * size, 0.0, 0.0),
            girth * size / SCALE,
            FOX,
        );
    }
    frame.world_sphere(
        camera,
        offset(body, forward, 0.8 * size, 0.0, -0.12 * size),
        0.32 * size / SCALE,
        FOX_WHITE,
    );
    let head = offset(body, forward, 1.2 * size, 0.0, 0.45 * size);
    frame.world_sphere(camera, head, 0.4 * size / SCALE, FOX);
    let snout = offset(head, forward, 0.85 * size, 0.0, -0.15 * size);
    for side in [-0.22, 0.22] {
        frame.facet(
            camera,
            [
                offset(head, forward, 0.2 * size, side * size, 0.08 * size),
                offset(head, forward, 0.2 * size, 0.0, -0.28 * size),
                snout,
            ],
            tint(FOX, 0.9),
            NONE,
        );
        frame.facet(
            camera,
            [
                offset(head, forward, 0.2 * size, side * size, 0.08 * size),
                offset(head, forward, 0.2 * size, 0.0, 0.2 * size),
                snout,
            ],
            FOX,
            NONE,
        );
        frame.facet(
            camera,
            [
                offset(head, forward, -0.12 * size, side * 0.8 * size, 0.25 * size),
                offset(head, forward, 0.12 * size, side * 1.5 * size, 0.25 * size),
                offset(head, forward, 0.0, side * 1.3 * size, 0.85 * size),
            ],
            FOX_DARK,
            NONE,
        );
    }
    frame.world_sphere(camera, snout, 0.08 * size / SCALE, FOX_DARK);
    if carrying && let Some(radius) = fox.ghost {
        let mouth = offset(head, forward, 0.7 * size + radius * 0.8, 0.0, -0.35 * size);
        frame.world_sphere(camera, mouth, radius / SCALE, GHOST);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Unit, demo};
    use crate::render::{Camera, Scene, View};

    fn identity(pid: u32) -> Identity {
        Identity {
            pid,
            start: pid as u64,
        }
    }

    fn process(pid: u32, cgroup: &str) -> Process {
        Process {
            id: identity(pid),
            parent: 1,
            name: format!("hen-{pid}"),
            command: String::new(),
            group: cgroup.rsplit('/').next().unwrap_or("").into(),
            kind: Kind::System,
            state: 'S',
            cpu: 5.0,
            memory: 20 << 20,
            io_rate: Some(0.0),
            read_rate: Some(0.0),
            write_rate: Some(0.0),
            written: Some(0),
            priority: 20,
            nice: 0,
            threads: 1,
            gpu_memory: 0,
            core: 0,
            cpu_time: 0.0,
            cgroup: cgroup.into(),
        }
    }

    fn snapshot(processes: Vec<Process>, elapsed: f64) -> Snapshot {
        Snapshot {
            processes,
            cores: 2,
            elapsed,
            ..Snapshot::default()
        }
    }

    fn forager(pid: u32, flock: usize, position: [f32; 2], theta: f32, noise: f32) -> Chicken {
        let mut chicken = Chicken::new(
            identity(pid),
            flock,
            position,
            theta,
            Rng::for_identity(identity(pid), 1),
        );
        chicken.noise = noise;
        chicken.speed = speed(0.0);
        chicken.radius = 0.3;
        chicken
    }

    /// A flock of `count` foragers scattered over a square of side `spread` around `center`.
    fn scatter(
        count: u32,
        first: u32,
        flock: usize,
        center: [f32; 2],
        spread: f32,
        noise: f32,
    ) -> Vec<Chicken> {
        let mut rng = Rng::new(first as u64 * 7 + 3);
        (0..count)
            .map(|k| {
                let position = [
                    center[0] + rng.signed() * spread * 0.5,
                    center[1] + rng.signed() * spread * 0.5,
                ];
                forager(first + k, flock, position, rng.unit() * TAU, noise)
            })
            .collect()
    }

    fn state(yard: &Yard) -> Vec<(Identity, [u32; 2], u32)> {
        let mut state: Vec<_> = yard
            .chickens
            .iter()
            .map(|c| {
                (
                    c.id,
                    [c.position[0].to_bits(), c.position[1].to_bits()],
                    c.theta.to_bits(),
                )
            })
            .collect();
        state.sort();
        state
    }

    fn render(scene: &mut Scene, snapshot: &Snapshot, time: f32) {
        let frame = scene.render(
            snapshot,
            View::Coop,
            &Camera::default(),
            160,
            90,
            None,
            time,
            4096,
            None,
        );
        scene.spare = frame.release();
    }

    #[test]
    fn vicsek_step_does_not_depend_on_update_order() {
        let mut chickens = scatter(40, 100, 0, [0.0, 0.0], 12.0, 1.2);
        chickens.extend(scatter(20, 300, 1, [2.0, 1.0], 10.0, 0.8));
        chickens[3].peers.push((45, 0.7));
        chickens[45].peers.push((3, 0.7));
        let mut sorted = Yard::new([-30.0, -30.0], [30.0, 30.0]);
        sorted.chickens = chickens.clone();
        sorted.homes = vec![Home {
            center: [5.0, 5.0],
            footprint: 2.0,
            range: 6.0,
        }];
        sorted.eyes = vec![[-8.0, 0.0]];
        let mut permuted = Yard::new(sorted.low, sorted.high);
        permuted.homes = sorted.homes.clone();
        permuted.eyes = sorted.eyes.clone();
        let mut order: Vec<usize> = (0..chickens.len()).collect();
        order.reverse();
        order.swap(5, 17);
        let place: HashMap<usize, usize> = order
            .iter()
            .enumerate()
            .map(|(to, &from)| (from, to))
            .collect();
        permuted.chickens = order
            .iter()
            .map(|&from| {
                let mut chicken = chickens[from].clone();
                for peer in &mut chicken.peers {
                    peer.0 = place[&peer.0];
                }
                chicken
            })
            .collect();
        for _ in 0..40 {
            sorted.step(H);
            permuted.step(H);
            assert_eq!(state(&sorted), state(&permuted));
        }
    }

    #[test]
    fn order_parameter_is_the_magnitude_of_the_summed_velocity() {
        let mut yard = Yard {
            chickens: (0..5)
                .map(|k| forager(k, 0, [k as f32, 0.0], 0.7, 0.0))
                .collect(),
            ..Yard::default()
        };
        yard.chickens = (0..5)
            .map(|k| forager(k, 0, [k as f32, 0.0], 0.7, 0.0))
            .collect();
        assert!((yard.order().unwrap() - 1.0).abs() < 1e-5);
        yard.chickens = vec![
            forager(1, 0, [0.0, 0.0], 0.0, 0.0),
            forager(2, 0, [1.0, 0.0], PI, 0.0),
        ];
        assert!(yard.order().unwrap() < 1e-5);
        // Speeds 1 and 3 in opposite directions: |3 - 1| / (1 + 3) = 0.5.
        yard.chickens[0].speed = 1.0;
        yard.chickens[1].speed = 3.0;
        assert!((yard.order().unwrap() - 0.5).abs() < 1e-5);
        // At right angles with speeds 3 and 4: 5 / 7.
        yard.chickens[0].speed = 3.0;
        yard.chickens[1].speed = 4.0;
        yard.chickens[1].theta = FRAC_PI_2;
        assert!((yard.order().unwrap() - 5.0 / 7.0).abs() < 1e-5);
        yard.chickens[0].role = Role::Roosting;
        yard.chickens[1].role = Role::Roosting;
        assert_eq!(yard.order(), None);
    }

    #[test]
    fn flock_follows_the_same_path_at_ten_and_sixty_frames_per_second() {
        let snapshot = demo(30.0, 96);
        let run = |fps: u32| {
            let mut scene = Scene::new();
            let mut states = HashMap::new();
            for k in 0..=3 * fps {
                render(&mut scene, &snapshot, 100.0 + k as f32 / fps as f32);
                states.insert(scene.coop.yard.steps, state(&scene.coop.yard));
            }
            states
        };
        let (slow, fast) = (run(10), run(60));
        let mut compared = 0;
        for (steps, state) in &slow {
            if let Some(other) = fast.get(steps) {
                assert_eq!(state, other, "after {steps} steps");
                compared += 1;
            }
        }
        assert!(compared >= 25, "compared {compared} states");
        assert!(slow.keys().max() >= Some(&58));
    }

    /// The mean order parameter over the last five of sixty simulated seconds for one flock of 60
    /// idle foragers around its house, at CPU pressure `psi` and steady CPU.
    fn settled_order(psi: f32) -> f32 {
        let mut yard = Yard::new([-30.0, -30.0], [30.0, 30.0]);
        yard.homes = vec![Home {
            center: [0.0, 0.0],
            footprint: 4.0,
            range: 10.0,
        }];
        yard.chickens = scatter(60, 1000, 0, [8.0, 8.0], 18.0, noise(0.0, psi));
        let mut total = 0.0;
        let mut count = 0;
        for step in 0..1200 {
            yard.step(H);
            if step >= 1100 {
                total += yard.order().unwrap();
                count += 1;
            }
        }
        total / count as f32
    }

    #[test]
    fn flocks_order_at_low_noise_and_dissolve_past_critical_noise() {
        let calm = settled_order(0.0);
        let stressed = settled_order(40.0);
        assert!(calm > 0.8, "calm phi {calm}");
        assert!(stressed < 0.4, "stressed phi {stressed}");
    }

    #[test]
    fn chickens_never_leave_the_fence() {
        let mut yard = Yard::new([-6.0, -9.0], [6.0, 4.0]);
        yard.chickens = scatter(80, 1, 0, [0.0, -2.0], 11.0, 2.5);
        for (k, chicken) in yard.chickens.iter_mut().enumerate() {
            chicken.speed = speed(400.0);
            chicken.radius = 0.2 + (k % 5) as f32 * 0.2;
        }
        yard.homes = vec![Home {
            center: [0.0, 0.0],
            footprint: 2.0,
            range: 6.0,
        }];
        for _ in 0..2000 {
            yard.step(H);
            for chicken in &yard.chickens {
                let [x, y] = chicken.position;
                assert!(
                    (-6.0..=6.0).contains(&x) && (-9.0..=4.0).contains(&y),
                    "{x} {y}"
                );
            }
        }
    }

    #[test]
    fn chickens_align_only_with_their_own_flock() {
        let mut yard = Yard::new([-200.0, -200.0], [200.0, 200.0]);
        let mut rng = Rng::new(5);
        for k in 0..60 {
            let flock = k % 2;
            let base = if flock == 0 { 0.0 } else { PI };
            let position = [rng.signed() * 6.0, rng.signed() * 6.0];
            let theta = base + rng.signed() * 0.5;
            yard.chickens
                .push(forager(k as u32 + 1, flock, position, theta, 0.0));
        }
        for _ in 0..100 {
            yard.step(H);
        }
        let orders = yard.flock_orders(2);
        assert!(orders[0].unwrap().0 > 0.98);
        assert!(orders[1].unwrap().0 > 0.98);
        assert!(
            yard.order().unwrap() < 0.2,
            "the flocks keep their own ways"
        );
    }

    #[test]
    fn talking_processes_walk_together() {
        let run = |weight: f32| {
            let mut yard = Yard::new([-100.0, -100.0], [100.0, 100.0]);
            yard.chickens = vec![
                forager(1, 0, [0.0, 0.0], 0.3, 0.2),
                forager(2, 1, [10.0, 0.0], 2.4, 0.2),
            ];
            if weight > 0.0 {
                yard.chickens[0].peers.push((1, weight));
                yard.chickens[1].peers.push((0, weight));
            }
            for _ in 0..600 {
                yard.step(H);
            }
            let [a, b] = [&yard.chickens[0], &yard.chickens[1]];
            let distance = (a.position[0] - b.position[0]).hypot(a.position[1] - b.position[1]);
            (distance, (a.theta - b.theta).cos())
        };
        let traffic = peer_weight(Some(200_000.0), 0.0, 0.0);
        let (apart, apart_alignment) = run(0.0);
        let (together, together_alignment) = run(traffic);
        assert!(together < apart * 0.5, "{together} vs {apart}");
        assert!(together_alignment > apart_alignment + 0.3);
        assert!(together_alignment > 0.7);
        assert_eq!(peer_weight(Some(0.0), 50.0, 50.0), 0.0, "idle measured TCP");
        assert!(
            peer_weight(None, 50.0, 0.0) == 0.0,
            "co-activity needs both busy"
        );
    }

    #[test]
    fn higher_priority_chicken_takes_the_feeder() {
        let mut processes: Vec<Process> = (1..=3)
            .map(|pid| {
                let mut p = process(pid, "/system.slice/a.service");
                p.state = 'R';
                p.cpu = 90.0;
                p
            })
            .collect();
        let mut sample = snapshot(processes.clone(), 1.0);
        sample.cpus = vec![Cpu {
            id: 0,
            kind: CoreKind::Efficiency,
            busy: 1.0,
            mhz: 0.0,
            wait: 0.0,
        }];
        let mut scene = Scene::new();
        render(&mut scene, &sample, 1.0);
        let notes = |scene: &Scene, pid: u32| scene.notes[&identity(pid)][1].clone();
        assert!(notes(&scene, 1).contains("rank 1 of 3"));
        assert!(notes(&scene, 3).contains("rank 3 of 3") && notes(&scene, 3).contains("waiting"));
        processes[2].priority = -51;
        sample.processes = processes;
        sample.elapsed = 2.0;
        render(&mut scene, &sample, 2.0);
        let feeder = &scene.coop.feeders[0];
        let chicken = scene.coop.yard.find(identity(3)).unwrap();
        assert_eq!(chicken.target, feeder.slot(0), "real-time pushes in first");
        assert!(notes(&scene, 3).contains("rank 1 of 3 by priority -51 (real-time)"));
        let displaced = scene.coop.yard.find(identity(2)).unwrap();
        assert!(displaced.target[1] < feeder.eating_line(), "now queueing");
    }

    #[test]
    fn one_egg_is_laid_per_mebibyte_written() {
        let mut coop = Coop::default();
        let mib = 1u64 << 20;
        let writes = [
            1234,
            1234 + mib - 1,
            1234 + mib,
            1234 + 3 * mib + 777,
            1234 + 7 * mib + mib / 2,
            1234 + 7 * mib + mib - 1,
        ];
        for (k, written) in writes.into_iter().enumerate() {
            let mut p = process(10, "/system.slice/writer.service");
            p.written = Some(written);
            let mut quiet = process(11, "/system.slice/writer.service");
            quiet.written = None;
            coop.record(&snapshot(vec![p, quiet], k as f64));
        }
        assert_eq!(coop.ledgers[&identity(10)].laid, 7);
        assert!(
            !coop.ledgers.contains_key(&identity(11)),
            "unreadable I/O lays nothing"
        );
        let nest = &coop.nests["/system.slice/writer.service"];
        assert_eq!(nest.eggs.len(), 7);
        assert_eq!(nest.eggs.iter().filter(|e| e.laid == 3.0).count(), 2);
    }

    #[test]
    fn an_oom_kill_sends_a_fox_for_the_departed_member() {
        let path = "/system.slice/greedy.service";
        let unit = |kills| {
            HashMap::from([(
                path.to_string(),
                Unit {
                    oom_kills: kills,
                    ..Unit::default()
                },
            )])
        };
        let mut small = process(1, path);
        small.memory = 50 << 20;
        let mut large = process(2, path);
        large.memory = 900 << 20;
        let mut bystander = process(3, path);
        bystander.memory = 100 << 20;
        let mut before = snapshot(vec![small.clone(), large.clone(), bystander.clone()], 1.0);
        before.units = unit(2);
        let mut scene = Scene::new();
        render(&mut scene, &before, 1.0);
        render(&mut scene, &before, 1.5);
        let victim = scene.coop.yard.find(identity(2)).unwrap().position;
        assert!(
            scene.coop.foxes.is_empty(),
            "the first sight is the baseline"
        );
        let mut after = snapshot(vec![small, bystander], 2.0);
        after.units = unit(3);
        render(&mut scene, &after, 2.0);
        assert_eq!(scene.coop.foxes.len(), 1);
        let fox = &scene.coop.foxes[0];
        assert_eq!(fox.prey, victim);
        assert!(
            fox.ghost
                .is_some_and(|r| (r - body_radius((900 << 20) as f32)).abs() < 1e-5)
        );
        let mut again = after.clone();
        again.elapsed = 3.0;
        again.units = unit(4);
        render(&mut scene, &again, 3.0);
        assert!(
            scene.coop.foxes.iter().any(|fox| fox.ghost.is_none()),
            "a kill with nobody gone sends a fox that leaves with nothing"
        );
    }

    fn units(path: &str, kills: u64) -> HashMap<String, Unit> {
        HashMap::from([(
            path.to_string(),
            Unit {
                oom_kills: kills,
                ..Unit::default()
            },
        )])
    }

    /// Three members of one cgroup rendered at samples 1 and 1.5, ready to lose the large one.
    fn greedy_flock(scene: &mut Scene, path: &str) -> ([f32; 2], Vec<Process>) {
        let mut small = process(1, path);
        small.memory = 50 << 20;
        let mut large = process(2, path);
        large.memory = 900 << 20;
        let mut bystander = process(3, path);
        bystander.memory = 100 << 20;
        let mut before = snapshot(vec![small.clone(), large, bystander.clone()], 1.0);
        before.units = units(path, 2);
        render(scene, &before, 1.0);
        render(scene, &before, 1.5);
        let victim = scene.coop.yard.find(identity(2)).unwrap().position;
        (victim, vec![small, bystander])
    }

    #[test]
    fn a_kill_counted_one_sample_after_the_member_vanished_still_names_it() {
        let path = "/system.slice/greedy.service";
        let mut scene = Scene::new();
        let (victim, survivors) = greedy_flock(&mut scene, path);
        let mut gone = snapshot(survivors.clone(), 2.0);
        gone.units = units(path, 2);
        render(&mut scene, &gone, 2.0);
        assert!(scene.coop.foxes.is_empty(), "the counter has not moved yet");
        let mut counted = snapshot(survivors, 3.0);
        counted.units = units(path, 3);
        render(&mut scene, &counted, 3.0);
        assert_eq!(scene.coop.foxes.len(), 1);
        let fox = &scene.coop.foxes[0];
        assert_eq!(fox.prey, victim);
        assert!(
            fox.ghost
                .is_some_and(|r| (r - body_radius((900 << 20) as f32)).abs() < 1e-5)
        );
    }

    #[test]
    fn a_kill_counted_two_samples_after_the_member_vanished_still_names_it() {
        let path = "/system.slice/greedy.service";
        let mut scene = Scene::new();
        let (victim, survivors) = greedy_flock(&mut scene, path);
        for elapsed in [2.0, 3.0] {
            let mut gone = snapshot(survivors.clone(), elapsed);
            gone.units = units(path, 2);
            render(&mut scene, &gone, elapsed as f32);
        }
        let mut counted = snapshot(survivors.clone(), 4.0);
        counted.units = units(path, 3);
        render(&mut scene, &counted, 4.0);
        assert_eq!(scene.coop.foxes.len(), 1);
        assert_eq!(scene.coop.foxes[0].prey, victim);
        assert!(scene.coop.foxes[0].ghost.is_some());
        // The victim is claimed once: a second kill finds nobody left to take.
        let mut again = snapshot(survivors, 5.0);
        again.units = units(path, 4);
        render(&mut scene, &again, 5.0);
        assert!(scene.coop.foxes.iter().any(|fox| fox.ghost.is_none()));
    }

    #[test]
    fn a_member_that_left_long_before_the_kill_is_not_blamed_for_it() {
        let path = "/system.slice/greedy.service";
        let mut scene = Scene::new();
        let (_, survivors) = greedy_flock(&mut scene, path);
        for elapsed in [2.0, 4.0, 6.0] {
            let mut gone = snapshot(survivors.clone(), elapsed);
            gone.units = units(path, 2);
            render(&mut scene, &gone, elapsed as f32);
        }
        let mut counted = snapshot(survivors, 7.0);
        counted.units = units(path, 3);
        render(&mut scene, &counted, 7.0);
        assert_eq!(scene.coop.foxes.len(), 1);
        assert!(scene.coop.foxes[0].ghost.is_none());
    }

    #[test]
    fn kills_recorded_while_another_view_is_shown_do_not_replay_on_switching_to_coop() {
        let path = "/system.slice/greedy.service";
        let mut scene = Scene::new();
        let processes = vec![process(1, path), process(2, path)];
        for k in 0..40 {
            let mut sample = snapshot(processes.clone(), 60.0 * k as f64);
            sample.units = units(path, k);
            scene.record(&sample, 60.0 * k as f32);
        }
        assert!(
            scene.coop.calls.len() <= 1,
            "an hour of kills leaves only the newest pending, not {}",
            scene.coop.calls.len()
        );
        let mut now = snapshot(processes, 60.0 * 40.0);
        now.units = units(path, 39);
        render(&mut scene, &now, 2400.0);
        assert!(scene.coop.foxes.is_empty());
        assert!(scene.coop.calls.is_empty());
    }

    #[test]
    fn pending_fox_calls_are_capped() {
        let mut coop = Coop::default();
        let paths: Vec<String> = (0..10)
            .map(|k| format!("/system.slice/unit{k}.service"))
            .collect();
        let at = |kills| {
            let mut sample = snapshot(Vec::new(), 1.0 + kills as f64 * 0.1);
            for path in &paths {
                sample.units.insert(
                    path.clone(),
                    Unit {
                        oom_kills: kills,
                        ..Unit::default()
                    },
                );
            }
            sample
        };
        coop.record(&at(0));
        coop.record(&at(5));
        assert_eq!(coop.calls.len(), MAX_PENDING_CALLS);
    }

    #[test]
    fn neighbouring_feeders_keep_their_queues_in_their_own_lanes() {
        let path = "/system.slice/busy.service";
        let processes = (1..=16)
            .map(|pid| {
                let mut p = process(pid, path);
                p.state = 'R';
                p.cpu = 90.0;
                p.memory = 20 << 20;
                p.core = u32::from(pid > 9);
                p
            })
            .collect();
        let mut sample = snapshot(processes, 1.0);
        sample.cores = 2;
        let mut scene = Scene::new();
        render(&mut scene, &sample, 1.0);
        let coop = &scene.coop;
        assert_eq!(coop.feeders.len(), 2);
        let chickens = &coop.yard.chickens;
        for chicken in chickens {
            let feeder = &coop.feeders[usize::from(chicken.id.pid > 9)];
            assert!(
                (chicken.target[0] - feeder.center[0]).abs() <= SLOT + 0.01,
                "pid {} queues outside cpu{}'s lane",
                chicken.id.pid,
                feeder.cpu
            );
        }
        for (a, b) in chickens
            .iter()
            .flat_map(|a| chickens.iter().map(move |b| (a, b)))
        {
            if a.id < b.id {
                let apart = (a.target[0] - b.target[0]).hypot(a.target[1] - b.target[1]);
                assert!(
                    apart > 0.2,
                    "pids {} and {} share a spot",
                    a.id.pid,
                    b.id.pid
                );
            }
        }
    }

    #[test]
    fn queued_feeders_wrap_into_columns_and_every_target_stays_inside_the_fence() {
        let path = "/system.slice/busy.service";
        let inside = |scene: &Scene| {
            let yard = &scene.coop.yard;
            for c in &yard.chickens {
                for point in [c.position, c.target] {
                    assert_eq!(
                        keep_inside(point, c.radius, yard.low, yard.high),
                        point,
                        "pid {} at {point:?} is outside {:?} to {:?}",
                        c.id.pid,
                        yard.low,
                        yard.high
                    );
                }
            }
        };
        let running = |state: char, elapsed: f64| {
            let processes = (1..=12)
                .map(|pid| {
                    let mut p = process(pid, path);
                    p.state = state;
                    p.cpu = 90.0;
                    p
                })
                .collect();
            let mut sample = snapshot(processes, elapsed);
            sample.cores = 1;
            sample
        };
        // The first population is placed straight at its targets, which must not be in the hedge.
        let mut scene = Scene::new();
        render(&mut scene, &running('R', 1.0), 1.0);
        assert_eq!(scene.coop.feeders.len(), 1);
        inside(&scene);
        // A population that starts walking must be able to arrive, one chicken to a spot.
        let mut scene = Scene::new();
        render(&mut scene, &running('S', 1.0), 1.0);
        let sample = running('R', 2.0);
        for frame in 0..400 {
            render(&mut scene, &sample, 2.0 + frame as f32 * 0.05);
        }
        inside(&scene);
        let chickens = &scene.coop.yard.chickens;
        assert!(
            chickens
                .iter()
                .all(|c| c.role == Role::Feeding && c.arrived)
        );
        for (a, b) in chickens
            .iter()
            .flat_map(|a| chickens.iter().map(move |b| (a, b)))
        {
            if a.id < b.id {
                let apart = (a.target[0] - b.target[0]).hypot(a.target[1] - b.target[1]);
                assert!(
                    apart > 0.2,
                    "pids {} and {} share a spot",
                    a.id.pid,
                    b.id.pid
                );
            }
        }
    }

    #[test]
    fn the_yard_and_feeders_stay_put_when_the_outermost_flock_leaves() {
        let cgroups: Vec<String> = (0..6)
            .map(|k| format!("/system.slice/f{k}.service"))
            .collect();
        let sample = |count: usize, elapsed: f64| {
            let processes = cgroups[..count]
                .iter()
                .enumerate()
                .flat_map(|(k, cgroup)| {
                    (0..30).map(move |n| process(1000 * (k as u32 + 1) + n, cgroup))
                })
                .collect();
            snapshot(processes, elapsed)
        };
        let mut scene = Scene::new();
        render(&mut scene, &sample(6, 1.0), 1.0);
        let layout = |scene: &Scene| {
            (
                scene.coop.yard.low,
                scene.coop.yard.high,
                scene.coop.band_top,
                scene
                    .coop
                    .feeders
                    .iter()
                    .map(|f| f.center)
                    .collect::<Vec<_>>(),
                scene.coop.yard.eyes.clone(),
            )
        };
        let before = layout(&scene);
        // Drop whichever flock stands furthest south, the one that sets the yard's southern edge.
        let outermost = scene
            .coop
            .flocks
            .iter()
            .min_by(|a, b| a.center[1].total_cmp(&b.center[1]))
            .map(|flock| flock.key.clone())
            .unwrap();
        let mut smaller = sample(6, 2.0);
        smaller.processes.retain(|p| p.cgroup != outermost);
        render(&mut scene, &smaller, 2.0);
        assert_eq!(scene.coop.flocks.len(), 5);
        assert_eq!(layout(&scene), before);
        // A flock arriving beyond the fence grows it, and the fence does not shrink back.
        let mut scene = Scene::new();
        render(&mut scene, &sample(2, 1.0), 1.0);
        let small = layout(&scene);
        render(&mut scene, &sample(6, 2.0), 2.0);
        let grown = layout(&scene);
        assert!(grown.0[0] <= small.0[0] && grown.0[1] <= small.0[1]);
        assert!(grown.1[0] >= small.1[0] && grown.1[1] >= small.1[1]);
        assert!(grown.0 != small.0 || grown.1 != small.1);
        render(&mut scene, &sample(2, 3.0), 3.0);
        assert_eq!(layout(&scene), grown);
    }

    #[test]
    fn flock_orders_count_foragers_and_match_the_order_of_each_flock() {
        let mut yard = Yard::new([-30.0, -30.0], [30.0, 30.0]);
        yard.chickens = scatter(30, 100, 0, [0.0, 0.0], 12.0, 1.2);
        yard.chickens
            .extend(scatter(10, 300, 2, [2.0, 1.0], 10.0, 0.8));
        yard.chickens[4].role = Role::Roosting;
        yard.chickens.sort_by_key(|c| c.id);
        let orders = yard.flock_orders(3);
        assert_eq!(orders.len(), 3);
        assert_eq!(orders[1], None, "a flock with nobody has no order");
        for flock in [0, 2] {
            let foragers = yard
                .chickens
                .iter()
                .filter(|c| c.role == Role::Foraging && c.flock == flock);
            let (phi, count) = orders[flock].unwrap();
            assert_eq!(Some(phi), order_of(foragers.clone()));
            assert_eq!(count, foragers.count());
        }
        assert_eq!(orders[0].unwrap().1, 29);
    }

    #[test]
    fn houses_and_perches_stay_put_when_resources_or_population_change() {
        let mut sample = demo(30.0, 128);
        let mut scene = Scene::new();
        render(&mut scene, &sample, 30.0);
        let houses = |scene: &Scene| -> BTreeMap<String, [f32; 2]> {
            scene
                .coop
                .flocks
                .iter()
                .map(|f| (f.key.clone(), f.center))
                .collect()
        };
        let seats = |scene: &Scene| -> HashMap<Identity, [f32; 2]> {
            scene
                .coop
                .yard
                .chickens
                .iter()
                .map(|c| {
                    let flock = &scene.coop.flocks[c.flock];
                    let (offset, _) = seat_offset(scene.coop.seats.seat(c.id).unwrap());
                    (
                        c.id,
                        [flock.center[0] + offset[0], flock.center[1] + offset[1]],
                    )
                })
                .collect()
        };
        let (before_houses, before_seats) = (houses(&scene), seats(&scene));
        for (k, p) in sample.processes.iter_mut().enumerate() {
            p.cpu = (k % 7) as f32 * 13.0;
            p.memory = (k as u64 % 11 + 1) * (90 << 20);
        }
        let removed = sample.processes.remove(37).id;
        sample.elapsed = 31.0;
        render(&mut scene, &sample, 31.0);
        assert_eq!(houses(&scene), before_houses);
        let after_seats = seats(&scene);
        for (id, seat) in &after_seats {
            assert_eq!(before_seats[id], *seat);
        }
        assert!(!after_seats.contains_key(&removed));
    }

    #[test]
    fn coop_renders_the_same_frame_twice_from_the_same_inputs() {
        let draw = || {
            let mut scene = Scene::new();
            let mut pixels = Vec::new();
            for k in 0..6 {
                let sample = demo(20.0 + k as f64, 160);
                scene.record(&sample, 20.0 + k as f32);
                let mut frame = scene.render(
                    &sample,
                    View::Coop,
                    &Camera::default(),
                    320,
                    180,
                    None,
                    20.0 + k as f32 * 0.7,
                    512,
                    None,
                );
                frame.rasterize();
                pixels = frame.pixels.clone();
                scene.spare = frame.release();
            }
            pixels
        };
        assert!(draw() == draw());
    }
}
