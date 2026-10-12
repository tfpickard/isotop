//! Murmuration: every thread is a starling in one flock over a roost at dusk, and every process
//! is a sub-flock inside it. A bird's speed is its thread's CPU use; a falcon (memory pressure or
//! an OOM kill) dives through the flock and a dark wave of agitation crosses it.
//!
//! The flock follows the rules of Ballerini et al. (2008): each bird reacts to its seven nearest
//! neighbours, however far away they are, and the threads of its own process count double. It is
//! drawn as the density of dark splats, so dense regions go black and thin ones stay grey.

use std::collections::HashMap;

use crate::geometry::{dot, normalize};
use crate::model::{Identity, Measured, Process, Snapshot};
use crate::palette::kind_color;
use crate::platform::Source;
use crate::render::{Color, Item, NONE, SCALE, Stage, TourTarget, label_name, ring_bounds};
use crate::simulation::{FixedStep, Rng, SpatialHash3, hash_identity};
use crate::status::missing_note;
use crate::terminal::clean_text;
use crate::views::ViewModule;

/// Separates this view's draws of the identity hash from every other view's.
const SALT: u64 = 0x6d75_726d_7572_6174;

/// Most birds that fly at once; above it each process flies its share of them.
const BIRD_CAP: usize = 8000;
/// Neighbours that steer a bird (alignment and separation), and neighbours it holds together
/// with (cohesion). Cohesion over only seven lets closed groups of eight split off as islands.
const NEIGHBOURS: usize = 7;
const FAR_NEIGHBOURS: usize = 21;
/// Cell of the spatial hash at the start, in world units. The hash answers exactly whatever its
/// cell, so the flock adapts it to a flock's density (see `Grid::adapt`) without changing a bird's
/// path: a query is quickest with a cell about as wide as the distance to the 21st neighbour.
const CELL: f32 = 2.0;
const CELL_RANGE: (f32, f32) = (1.0, 8.0);

/// The flock's fixed step: 20 steps a second, each one a step of the prototype (dt = 1).
const STEP: f32 = 0.05;
const MAX_STEPS: usize = 6;
const MAX_GAP: f32 = 0.5;

/// Forces, in units per step. Alignment pulls the velocity towards the weighted mean of the seven
/// neighbours, cohesion the position towards the centroid of the 21, and separation pushes away
/// from the neighbours closer than `SEPARATION_RADIUS`. A per-process centroid pull of 0.0012 or
/// more splits the flock into separate balls, so only the whole flock is pulled in.
const ALIGNMENT: f32 = 0.30;
const COHESION: f32 = 0.02;
const SEPARATION: f32 = 0.4;
const SEPARATION_RADIUS: f32 = 1.5;
const SEPARATION_FLOOR: f32 = 0.3;
const FLOCK_PULL: f32 = 0.0006;
/// Birds fly level and turn by banking, which flattens the flock into sheets; a damping of 0.22
/// flattens it to a pancake that reads as a smear.
const VERTICAL_DAMPING: f32 = 0.10;
const NOISE: f32 = 0.02;
/// The roost: a horizontal pull back over it beyond `ROOST_RADIUS`, and an altitude band.
const ROOST_RADIUS: f32 = 80.0;
const ROOST_PULL: f32 = 0.004;
const FLOOR_ALTITUDE: f32 = 35.0;
const CEILING_ALTITUDE: f32 = 110.0;
const ALTITUDE_PULL: f32 = 0.01;
/// The roost cylinder the camera is fitted to, which does not follow the flock.
const FIT_RADIUS: f32 = 90.0;

/// The falcon: birds within `FLEE_RADIUS` take fright and flee, and agitation spreads from
/// neighbour to neighbour (times `SPREAD` while above `SPREAD_FLOOR`), fades by `DECAY` a step
/// and adds `JITTER` of noise.
const FLEE_RADIUS: f32 = 14.0;
const FLEE: f32 = 0.9;
const FLEE_FLOOR: f32 = 0.5;
const SPREAD: f32 = 0.93;
const SPREAD_FLOOR: f32 = 0.35;
const DECAY: f32 = 0.86;
const JITTER: f32 = 0.12;
const FALCON_SPEED: f32 = 2.6;
const FALCON_START: Vector = [-140.0, 40.0, 70.0];
/// Steps a falcon flies before it is gone: well past the flock on the far side.
const FALCON_STEPS: u32 = 130;
/// Memory pressure (the 10 s average, percent) that releases a falcon, and the steps between two
/// such falcons: 8 s.
const PRESSURE_FALCON: f32 = 10.0;
const COOLDOWN_STEPS: u64 = 160;

/// A thread leaving the flock falls to the roost over 30 steps (1.5 s).
const FALL_STEPS: u32 = 30;
/// The spread of a process's clump about its centre, and of the clumps about the roost, on the
/// first frame.
const CLUMP_SPREAD: f32 = 14.0;
const CLUMP_CENTRES: f32 = 18.0;
const START_ALTITUDE: f32 = 60.0;

/// A bird is a dark splat; a bird of a lit process is drawn again in the process's kind colour.
const BIRD: Color = [12, 10, 16];
const FALCON: Color = [235, 70, 80];
const TREELINE: Color = [16, 14, 22];
/// A splat's peak alpha is `1 - exp(-DENSITY * weight)`: n of them stack to the prototype's
/// density law. An agitated bird weighs `1 + AGITATED * agitation`, up to 2.6.
const DENSITY: f32 = 1.1;
const AGITATED: f32 = 1.6;
const LIT_DENSITY: f32 = 2.2;
/// A bird's splat has a radius of `BIRD_RADIUS` world units, kept between `MIN_RADIUS` and
/// `MAX_RADIUS` pixels. At the zoom that fits the roost, a splat this big holds the ink of the
/// prototype's bird, a 2 by 2 splat lightly blurred.
const BIRD_RADIUS: f32 = 0.85;
const MIN_RADIUS: f32 = 1.2;
const MAX_RADIUS: f32 = 6.0;
const MIN_PICK: f32 = 2.0;
/// Processes the tour visits first, by thread count.
const TOUR_TARGETS: usize = 12;

const LEGEND: &str = "bird = thread | flock = process (same-process neighbours count double) | speed = thread CPU | falcon = memory pressure or OOM kill | 7 topological neighbours";

type Vector = [f32; 3];

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn subtract(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scaled(a: Vector, factor: f32) -> Vector {
    a.map(|value| value * factor)
}

fn length(a: Vector) -> f32 {
    dot(a, a).sqrt()
}

/// Which thread a bird is. A process whose threads were not read has birds with no thread to
/// name: they are keyed apart from every real thread, so a thread list that arrives later is
/// matched to them by position instead of replacing them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Key {
    process: Identity,
    listed: bool,
    tid: u32,
}

/// A bird the roster asks for.
struct Wanted {
    key: Key,
    speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flight {
    /// Climbing out of the roost towards the altitude band; no other bird reacts to it yet.
    Rising,
    Flying,
}

struct Bird {
    key: Key,
    /// The rank of its process among the roster's, so same-process neighbours compare in one
    /// integer.
    owner: u32,
    position: Vector,
    velocity: Vector,
    speed: f32,
    agitation: f32,
    random: Rng,
    flight: Flight,
}

/// A bird whose thread exited, falling to the roost.
struct Faller {
    process: Identity,
    from: Vector,
    age: u32,
}

impl Faller {
    /// Where it is `fraction` of a step after its last step: falling, so slowly at first.
    fn at(&self, fraction: f32) -> Vector {
        let t = ((self.age as f32 + fraction) / FALL_STEPS as f32).clamp(0.0, 1.0);
        scaled(self.from, 1.0 - t * t)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cause {
    Oom,
    Pressure,
}

impl Cause {
    fn label(self) -> &'static str {
        match self {
            Cause::Oom => "OOM",
            Cause::Pressure => "memory pressure",
        }
    }
}

struct Falcon {
    position: Vector,
    velocity: Vector,
    age: u32,
    cause: Cause,
}

/// The spatial hash, with the width of its cell.
struct Grid {
    hash: SpatialHash3,
    cell: f32,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            hash: SpatialHash3::new(CELL),
            cell: CELL,
        }
    }
}

impl Grid {
    /// Rebuilds the hash with a cell as wide as `typical`, the mean distance of a bird to its
    /// 21st neighbour, when that is more than a quarter off the cell it has.
    fn adapt(&mut self, typical: f32) {
        let wanted = typical.clamp(CELL_RANGE.0, CELL_RANGE.1);
        if wanted > self.cell * 1.25 || wanted < self.cell / 1.25 {
            *self = Self {
                hash: SpatialHash3::new(wanted),
                cell: wanted,
            };
        }
    }
}

/// The flock's fixed-step clock with this view's constants.
struct FlockClock(FixedStep);

impl Default for FlockClock {
    fn default() -> Self {
        Self(FixedStep::new(STEP, MAX_STEPS, MAX_GAP))
    }
}

/// Buffers one step reuses, in the order of the flying birds.
#[derive(Default)]
struct Scratch {
    flying: Vec<usize>,
    points: Vec<Vector>,
    velocities: Vec<Vector>,
    owners: Vec<u32>,
    agitation: Vec<f32>,
    next_velocity: Vec<Vector>,
    next_agitation: Vec<f32>,
    nearest: Vec<(usize, f32)>,
}

/// The flock: every bird, the falcon, and what releases the falcon. Pure simulation: it knows
/// nothing of the camera or the frame.
#[derive(Default)]
struct Flock {
    /// Rising and flying birds, ordered by key.
    birds: Vec<Bird>,
    falling: Vec<Faller>,
    falcon: Option<Falcon>,
    /// Whether memory pressure is at or above `PRESSURE_FALCON` in the sample shown.
    pressure_high: bool,
    /// An OOM kill is waiting for the falcon in flight to leave.
    oom_pending: bool,
    /// Steps taken in all, and the step of the last falcon or of the view's entry.
    steps: u64,
    released: u64,
    /// Whether the first roster has been built: later birds rise from the roost instead of
    /// starting as clumps in a ball.
    settled: bool,
    grid: Grid,
    scratch: Scratch,
}

/// The centre of the clump a process's threads start as: a point of the loose ball, which
/// depends on nothing but the process.
fn clump_centre(process: Identity) -> Vector {
    let mut random = Rng::for_identity(process, SALT);
    let centre = [(); 3].map(|()| random.normal() * CLUMP_CENTRES);
    add(centre, [0.0, 0.0, START_ALTITUDE])
}

/// The heading every bird starts with on the first frame.
fn heading() -> Vector {
    let mut random = Rng::new(SALT);
    normalize([random.normal(), random.normal(), 0.0])
}

/// Matches the birds of one process to the birds it now asks for, both ordered by key: a bird
/// keeps its place when its key is asked for again, and a bird nobody asks for is returned
/// second. When the keys are of the other kind (a thread list arrived, or vanished) the birds
/// are matched to the asks by position, so the process's flock keeps flying.
fn pair(group: Vec<Bird>, wants: Vec<Wanted>) -> (Vec<(Wanted, Option<Bird>)>, Vec<Bird>) {
    let other_kind = group
        .first()
        .zip(wants.first())
        .is_some_and(|(bird, want)| bird.key.listed != want.key.listed);
    let mut kept = Vec::with_capacity(wants.len());
    let mut fallen = Vec::new();
    let mut birds = group.into_iter().peekable();
    for want in wants {
        if !other_kind {
            while let Some(bird) = birds.next_if(|bird| bird.key < want.key) {
                fallen.push(bird);
            }
        }
        let matched = if other_kind {
            birds.next()
        } else {
            birds.next_if(|bird| bird.key == want.key)
        };
        kept.push((want, matched));
    }
    fallen.extend(birds);
    (kept, fallen)
}

impl Flock {
    /// A bird that starts with the flock: in its process's clump, heading as the others do.
    fn clump(key: Key, owner: u32, speed: f32) -> Bird {
        let mut random = Rng::for_identity(key.process, u64::from(key.tid) ^ SALT);
        let centre = clump_centre(key.process);
        let offset = [(); 3].map(|()| random.normal() * CLUMP_SPREAD);
        let noise = [(); 3].map(|()| random.normal() * 0.3);
        Bird {
            key,
            owner,
            position: add(centre, offset),
            velocity: add(heading(), noise),
            speed,
            agitation: 0.0,
            random,
            flight: Flight::Flying,
        }
    }

    /// A bird that rises from the roost: at its foot, climbing.
    fn rising(key: Key, owner: u32, speed: f32) -> Bird {
        let mut random = Rng::for_identity(key.process, u64::from(key.tid) ^ SALT);
        let foot = [
            random.normal() * 3.0,
            random.normal() * 3.0,
            random.unit() * 2.0,
        ];
        let sway = [random.normal() * 0.15, random.normal() * 0.15, 0.0];
        Bird {
            key,
            owner,
            position: foot,
            velocity: add(sway, [0.0, 0.0, speed]),
            speed,
            agitation: 0.0,
            random,
            flight: Flight::Rising,
        }
    }

    /// Brings the birds to the roster `wanted`, which is ordered by key: a bird whose thread is
    /// gone falls, a thread without a bird starts one (in a clump on the first roster, rising
    /// from the roost after), and the birds of a process whose thread list has just arrived or
    /// vanished keep flying as the threads of the other kind.
    fn reroster(&mut self, wanted: Vec<Wanted>) {
        let first = !self.settled;
        self.settled = self.settled || !wanted.is_empty();
        let mut old = std::mem::take(&mut self.birds).into_iter().peekable();
        let mut asked = wanted.into_iter().peekable();
        let mut birds = Vec::with_capacity(asked.len());
        let mut owner = 0_u32;
        loop {
            let process = match (old.peek(), asked.peek()) {
                (Some(bird), Some(want)) => bird.key.process.min(want.key.process),
                (Some(bird), None) => bird.key.process,
                (None, Some(want)) => want.key.process,
                (None, None) => break,
            };
            let mut group = Vec::new();
            while let Some(bird) = old.next_if(|bird| bird.key.process == process) {
                group.push(bird);
            }
            let mut wants = Vec::new();
            while let Some(want) = asked.next_if(|want| want.key.process == process) {
                wants.push(want);
            }
            let (kept, fallen) = pair(group, wants);
            self.falling.extend(fallen.into_iter().map(|bird| Faller {
                process,
                from: bird.position,
                age: 0,
            }));
            for (want, bird) in kept {
                birds.push(match bird {
                    Some(bird) => Bird {
                        key: want.key,
                        owner,
                        speed: want.speed,
                        ..bird
                    },
                    None if first => Self::clump(want.key, owner, want.speed),
                    None => Self::rising(want.key, owner, want.speed),
                });
            }
            owner += 1;
        }
        self.birds = birds;
    }

    /// The mean position of the flying birds, if any fly.
    fn centroid(&self) -> Option<Vector> {
        let mut sum = [0.0; 3];
        let mut count = 0_u32;
        for bird in self.birds.iter().filter(|b| b.flight == Flight::Flying) {
            sum = add(sum, bird.position);
            count += 1;
        }
        (count > 0).then(|| scaled(sum, 1.0 / count as f32))
    }

    /// Lets the falcon go when memory pressure or an OOM kill asks for one and none is flying.
    fn release_falcon(&mut self) {
        if self.falcon.is_some() {
            return;
        }
        let cause = if self.oom_pending {
            Cause::Oom
        } else if self.pressure_high && self.steps - self.released >= COOLDOWN_STEPS {
            Cause::Pressure
        } else {
            return;
        };
        let Some(centre) = self.centroid() else {
            return;
        };
        let start = add(centre, FALCON_START);
        self.falcon = Some(Falcon {
            position: start,
            velocity: scaled(normalize(subtract(centre, start)), FALCON_SPEED),
            age: 0,
            cause,
        });
        if cause == Cause::Oom {
            self.oom_pending = false;
        }
        self.released = self.steps;
    }

    /// One step of the prototype: every flying bird steers by its neighbours, the roost and the
    /// falcon, then flies at its own speed.
    fn step(&mut self) {
        self.release_falcon();
        self.steps += 1;
        for bird in &mut self.birds {
            if bird.flight == Flight::Rising {
                bird.position = add(bird.position, bird.velocity);
                if bird.position[2] >= FLOOR_ALTITUDE {
                    bird.flight = Flight::Flying;
                }
            }
        }
        for faller in &mut self.falling {
            faller.age += 1;
        }
        self.falling.retain(|faller| faller.age < FALL_STEPS);
        self.steer();
        if let Some(falcon) = &mut self.falcon {
            falcon.position = add(falcon.position, falcon.velocity);
            falcon.age += 1;
        }
        if self.falcon.as_ref().is_some_and(|f| f.age >= FALCON_STEPS) {
            self.falcon = None;
        }
    }

    fn steer(&mut self) {
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.flying.clear();
        scratch.flying.extend(
            (0..self.birds.len()).filter(|&index| self.birds[index].flight == Flight::Flying),
        );
        let count = scratch.flying.len();
        if count == 0 {
            self.scratch = scratch;
            return;
        }
        scratch.points.clear();
        scratch.velocities.clear();
        scratch.owners.clear();
        scratch.agitation.clear();
        let falcon = self.falcon.as_ref().map(|falcon| falcon.position);
        for &index in &scratch.flying {
            let bird = &self.birds[index];
            scratch.points.push(bird.position);
            scratch.velocities.push(bird.velocity);
            scratch.owners.push(bird.owner);
            // A bird within the flee radius of the falcon is at full agitation this step.
            let frightened =
                falcon.is_some_and(|at| length(subtract(bird.position, at)) < FLEE_RADIUS);
            scratch
                .agitation
                .push(if frightened { 1.0 } else { bird.agitation });
        }
        let centroid = scaled(
            scratch.points.iter().fold([0.0; 3], |sum, &p| add(sum, p)),
            1.0 / count as f32,
        );
        self.grid.hash.rebuild(&scratch.points);
        scratch.next_velocity.clear();
        scratch.next_agitation.clear();
        let mut farthest = 0.0;
        for k in 0..count {
            let bird = &mut self.birds[scratch.flying[k]];
            let (position, velocity) = (scratch.points[k], scratch.velocities[k]);
            self.grid
                .hash
                .nearest(position, FAR_NEIGHBOURS + 1, &mut scratch.nearest);
            farthest += scratch
                .nearest
                .last()
                .map_or(0.0, |&(_, distance)| distance);
            let mut weighted = [0.0; 3];
            let mut weights = 0.0;
            let mut separation = [0.0; 3];
            let mut spread = 0.0_f32;
            let mut centre = [0.0; 3];
            let mut seen = 0;
            for &(other, distance) in scratch.nearest.iter().filter(|&&(other, _)| other != k) {
                if seen == FAR_NEIGHBOURS {
                    break;
                }
                centre = add(centre, scratch.points[other]);
                if seen < NEIGHBOURS {
                    // Threads of the same process count double.
                    let weight = if scratch.owners[other] == bird.owner {
                        2.0
                    } else {
                        1.0
                    };
                    weighted = add(weighted, scaled(scratch.velocities[other], weight));
                    weights += weight;
                    if distance < SEPARATION_RADIUS {
                        let away = subtract(position, scratch.points[other]);
                        let gap = distance.max(SEPARATION_FLOOR);
                        separation = add(separation, scaled(away, 1.0 / (gap * gap)));
                    }
                    spread = spread.max(scratch.agitation[other]);
                }
                seen += 1;
            }
            let mut steer = [0.0; 3];
            if seen > 0 {
                let aligned = subtract(scaled(weighted, 1.0 / weights), velocity);
                let cohered = subtract(scaled(centre, 1.0 / seen as f32), position);
                steer = add(scaled(aligned, ALIGNMENT), scaled(cohered, COHESION));
                steer = add(steer, scaled(separation, SEPARATION));
            }
            steer = add(steer, scaled(subtract(centroid, position), FLOCK_PULL));
            steer = add(steer, roost_pull(position));
            steer = add(steer, [(); 3].map(|()| bird.random.normal() * NOISE));
            steer[2] -= VERTICAL_DAMPING * velocity[2];
            if let Some(at) = falcon {
                let offset = subtract(position, at);
                let distance = length(offset);
                if distance < FLEE_RADIUS {
                    steer = add(steer, scaled(offset, FLEE / distance.max(FLEE_FLOOR)));
                }
            }
            // Agitation passes from neighbour to neighbour and fades: the dark band that ripples
            // across a real murmuration.
            let carried = spread * SPREAD;
            let agitation = (scratch.agitation[k] * DECAY).max(if carried > SPREAD_FLOOR {
                carried
            } else {
                0.0
            });
            if agitation > 0.0 {
                steer = add(
                    steer,
                    [(); 3].map(|()| agitation * bird.random.normal() * JITTER),
                );
            }
            let heading = add(velocity, steer);
            scratch
                .next_velocity
                .push(scaled(heading, bird.speed / length(heading).max(1e-6)));
            scratch.next_agitation.push(agitation);
        }
        self.grid.adapt(farthest / count as f32);
        for k in 0..count {
            let bird = &mut self.birds[scratch.flying[k]];
            bird.velocity = scratch.next_velocity[k];
            bird.agitation = scratch.next_agitation[k];
            bird.position = add(scratch.points[k], bird.velocity);
        }
        self.scratch = scratch;
    }
}

/// The pull of the roost: back over it horizontally beyond `ROOST_RADIUS`, and into the altitude
/// band.
fn roost_pull(position: Vector) -> Vector {
    let radius = position[0].hypot(position[1]);
    let inward = (radius - ROOST_RADIUS).max(0.0) * ROOST_PULL / radius.max(1.0);
    [
        -position[0] * inward,
        -position[1] * inward,
        ((FLOOR_ALTITUDE - position[2]).max(0.0) - (position[2] - CEILING_ALTITUDE).max(0.0))
            * ALTITUDE_PULL,
    ]
}

/// What the roster made of the threads: for the legend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Summary {
    /// Threads the processes have, listed or not.
    threads: usize,
    /// Birds the roster asks for.
    birds: usize,
    /// Birds whose speed comes from their process's mean CPU: their thread's CPU is unknown.
    mean: usize,
}

/// The birds the processes ask for, ordered by key. Every thread of a process flies; above
/// `BIRD_CAP` in all, a process flies its proportional share of them (at least one), the threads
/// with the lowest hash of tid, so the selection holds from sample to sample.
fn roster(processes: &[&Process]) -> (Vec<Wanted>, Summary) {
    struct Candidate {
        key: Key,
        cpu: Option<f32>,
    }
    let mut candidates: Vec<Vec<Candidate>> = Vec::with_capacity(processes.len());
    let mut summary = Summary::default();
    for process in processes {
        let listed = match &process.thread_list {
            Measured::Known(list) if !list.threads.is_empty() => Some(list),
            _ => None,
        };
        let group: Vec<Candidate> = match listed {
            Some(list) => list
                .threads
                .iter()
                .map(|thread| Candidate {
                    key: Key {
                        process: process.id,
                        listed: true,
                        tid: thread.tid,
                    },
                    cpu: thread.cpu,
                })
                .collect(),
            None => (0..process.threads.max(1))
                .map(|tid| Candidate {
                    key: Key {
                        process: process.id,
                        listed: false,
                        tid,
                    },
                    cpu: None,
                })
                .collect(),
        };
        summary.threads += group.len().max(process.threads as usize);
        candidates.push(group);
    }
    let flyable: usize = candidates.iter().map(Vec::len).sum();
    let mut wanted = Vec::with_capacity(flyable.min(BIRD_CAP + processes.len()));
    for (process, mut group) in processes.iter().zip(candidates) {
        if flyable > BIRD_CAP {
            let share = (group.len() as f64 * BIRD_CAP as f64 / flyable as f64).round() as usize;
            group.sort_by_key(|candidate| {
                (
                    hash_identity(process.id, u64::from(candidate.key.tid) ^ SALT),
                    candidate.key.tid,
                )
            });
            group.truncate(share.max(1));
        }
        let mean = process.cpu / process.threads.max(1) as f32;
        for candidate in group {
            let busy = candidate.cpu.unwrap_or_else(|| {
                summary.mean += 1;
                mean
            }) / 100.0;
            // A NaN or infinite reading must not reach the flock's arithmetic.
            let busy = if busy.is_finite() {
                busy.clamp(0.0, 1.0)
            } else {
                0.0
            };
            wanted.push(Wanted {
                key: candidate.key,
                speed: 1.0 + 0.3 * busy,
            });
        }
    }
    wanted.sort_by_key(|want| want.key);
    summary.birds = wanted.len();
    (wanted, summary)
}

/// The kernel's count of OOM kills: the machine's, or else the sum over the cgroups that count
/// them.
fn oom_kills(snapshot: &Snapshot) -> Option<u64> {
    snapshot.oom_kills.or_else(|| {
        snapshot
            .units
            .values()
            .filter_map(|unit| unit.oom_kills)
            .reduce(|a, b| a + b)
    })
}

/// What the view has seen of the samples and the clock, to tell a view entered after a gap from
/// one that has been shown.
struct Watch {
    /// The clock of the last frame drawn.
    time: Option<f32>,
    /// The `elapsed` of the newest sample seen, and the seconds between the last two.
    elapsed: Option<f64>,
    interval: f32,
    /// The OOM kills as of the last sample seen.
    kills: Option<u64>,
    /// The processes and sample the roster was last built from.
    signature: Option<u64>,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            time: None,
            elapsed: None,
            interval: 1.0,
            kills: None,
            signature: None,
        }
    }
}

/// What the roster was built from: the sample and the processes in view.
fn signature(processes: &[&Process], snapshot: &Snapshot) -> u64 {
    processes
        .iter()
        .fold(snapshot.elapsed.to_bits(), |hash, p| {
            hash.rotate_left(7) ^ hash_identity(p.id, SALT)
        })
}

/// A bird as a frame draws it.
struct Drawn {
    depth: f32,
    center: [f32; 2],
    strength: f32,
    /// The index of its process in the frame, for picking.
    pick: Option<usize>,
}

/// What the frame's birds say of one process.
#[derive(Clone, Copy, Default)]
struct Group {
    sum: Vector,
    highest: f32,
    birds: usize,
}

#[derive(Default)]
pub struct Murmuration {
    flock: Flock,
    clock: FlockClock,
    watch: Watch,
    summary: Summary,
    drawn: Vec<Drawn>,
}

impl Murmuration {
    /// Takes in the shown sample at `time`: notes a view entered after a gap, and what releases
    /// the falcon, and rebuilds the roster when the sample or the processes changed. While the
    /// clock does not run (`live` is false: paused, rewound) the flock holds as it is.
    fn observe(&mut self, time: f32, live: bool, processes: &[&Process], snapshot: &Snapshot) {
        let entered = self
            .watch
            .time
            .is_none_or(|last| time - last > 2.0 * self.watch.interval);
        self.watch.time = Some(time);
        match self.watch.elapsed {
            Some(previous) if snapshot.elapsed > previous => {
                self.watch.interval = ((snapshot.elapsed - previous) as f32).clamp(0.1, 10.0);
                self.watch.elapsed = Some(snapshot.elapsed);
            }
            None => self.watch.elapsed = Some(snapshot.elapsed),
            _ => {}
        }
        let kills = oom_kills(snapshot);
        if entered {
            // Nothing is released for what happened while the view was away.
            self.flock.oom_pending = false;
            self.flock.released = self.flock.steps;
            self.watch.kills = kills;
        } else if live {
            if let (Some(now), Some(before)) = (kills, self.watch.kills)
                && now > before
            {
                self.flock.oom_pending = true;
            }
            self.watch.kills = kills;
        }
        if entered || live {
            self.flock.pressure_high = snapshot.pressure[1] >= PRESSURE_FALCON;
        }
        let signature = signature(processes, snapshot);
        if (live || !self.flock.settled) && self.watch.signature != Some(signature) {
            self.watch.signature = Some(signature);
            let (wanted, summary) = roster(processes);
            self.flock.reroster(wanted);
            self.summary = summary;
        }
    }

    /// Steps the flock to `time` and returns how far the frame lies into the next step.
    fn advance(&mut self, time: f32) -> f32 {
        let Self { clock, flock, .. } = self;
        clock.0.advance(time, |_| flock.step())
    }

    /// The radius of a bird's splat in pixels.
    fn radius(stage: &Stage) -> f32 {
        (BIRD_RADIUS * SCALE * stage.camera.zoom).clamp(MIN_RADIUS, MAX_RADIUS)
    }

    /// Records the birds, the falcon and the treeline, and tells the app where each flock is.
    fn paint(&mut self, stage: &mut Stage, processes: &[&Process], alpha: f32) {
        let index_of: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(index, process)| (process.id, index))
            .collect();
        let lit: Vec<Option<Color>> = processes
            .iter()
            .map(|process| {
                stage
                    .highlight
                    .contains(&process.id)
                    .then(|| kind_color(process))
            })
            .collect();
        let (width, height) = (stage.frame.width as f32, stage.frame.height as f32);
        let radius = Self::radius(stage);
        let mut groups = vec![Group::default(); processes.len()];
        self.drawn.clear();
        let mut nearest = f32::NEG_INFINITY;
        for bird in &self.flock.birds {
            let Some(&index) = index_of.get(&bird.key.process) else {
                continue;
            };
            let at = add(bird.position, scaled(bird.velocity, alpha));
            let group = &mut groups[index];
            group.sum = add(group.sum, at);
            group.highest = if group.birds == 0 {
                at[2]
            } else {
                group.highest.max(at[2])
            };
            group.birds += 1;
            self.drawn.push(Self::splat(
                stage,
                at,
                1.0 + AGITATED * bird.agitation,
                Some(index),
            ));
        }
        for faller in &self.flock.falling {
            let pick = index_of.get(&faller.process).copied();
            self.drawn
                .push(Self::splat(stage, faller.at(alpha), 1.0, pick));
        }
        self.drawn.retain(|drawn| {
            let [x, y] = drawn.center;
            x >= -radius && x <= width + radius && y >= -radius && y <= height + radius
        });
        // Far to near, so the nearest bird has the last word on a pick.
        self.drawn.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        for drawn in &self.drawn {
            nearest = nearest.max(drawn.depth);
            stage.frame.items.push(Item::Puff {
                center: drawn.center,
                radius,
                color: BIRD,
                strength: drawn.strength,
            });
        }
        let lit_strength = 1.0 - (-LIT_DENSITY).exp();
        for drawn in &self.drawn {
            if let Some(color) = drawn.pick.and_then(|index| lit[index]) {
                stage.frame.items.push(Item::Puff {
                    center: drawn.center,
                    radius: radius * 1.15,
                    color,
                    strength: lit_strength,
                });
            }
        }
        for drawn in &self.drawn {
            if let Some(index) = drawn.pick {
                stage.frame.items.push(Item::Pick {
                    center: drawn.center,
                    radius: radius.max(MIN_PICK),
                    depth: drawn.depth,
                    pick: index as u32,
                });
            }
        }
        let nearest = if nearest.is_finite() { nearest } else { 0.0 };
        self.paint_falcon(stage, alpha, nearest + 1.0);
        self.report(stage, processes, &groups);
        paint_treeline(stage, nearest + 2.0);
        stage
            .bounds
            .extend(ring_bounds([0.0, 0.0], FIT_RADIUS, 0.0, CEILING_ALTITUDE));
    }

    fn splat(stage: &Stage, at: Vector, weight: f32, pick: Option<usize>) -> Drawn {
        let [x, y, depth] = stage.frame.project(stage.camera, at);
        Drawn {
            depth,
            center: [x, y],
            strength: 1.0 - (-DENSITY * weight).exp(),
            pick,
        }
    }

    /// The falcon as a chevron of two triangles that picks nothing, and its label.
    fn paint_falcon(&self, stage: &mut Stage, alpha: f32, depth: f32) {
        let Some(falcon) = &self.flock.falcon else {
            return;
        };
        let at = add(falcon.position, scaled(falcon.velocity, alpha));
        let [x, y, _] = stage.frame.project(stage.camera, at);
        let unit = (stage.frame.height as f32 / 460.0).max(1.0);
        let apex = [x, y - 2.0 * unit, depth];
        let thick = [x, y + 1.5 * unit, depth];
        for side in [-1.0, 1.0] {
            let tip = [x + side * 9.0 * unit, y + 4.0 * unit, depth];
            stage
                .frame
                .items
                .push(Item::Triangle([tip, apex, thick], FALCON, NONE));
        }
        stage
            .places
            .push((add(at, [0.0, 0.0, 6.0]), falcon.cause.label().to_owned()));
    }

    /// Fills the stage's outputs: each flock's centroid, the tour's targets, and the labels and
    /// inspector lines of the lit processes.
    fn report(&self, stage: &mut Stage, processes: &[&Process], groups: &[Group]) {
        let mut targets = Vec::new();
        for (process, group) in processes.iter().zip(groups) {
            let centroid = if group.birds > 0 {
                scaled(group.sum, 1.0 / group.birds as f32)
            } else {
                [0.0, 0.0, FLOOR_ALTITUDE]
            };
            stage.positions.insert(process.id, centroid);
            targets.push(TourTarget {
                id: process.id,
                position: centroid,
                weight: group.birds as f32,
            });
        }
        targets.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
        targets.truncate(TOUR_TARGETS);
        stage.tour.extend(targets);
        for (process, group) in processes.iter().zip(groups) {
            if !stage.highlight.contains(&process.id) || group.birds == 0 {
                continue;
            }
            let threads = match &process.thread_list {
                Measured::Known(list) => list.threads.len().max(process.threads as usize),
                _ => process.threads as usize,
            };
            let centroid = scaled(group.sum, 1.0 / group.birds as f32);
            stage.places.push((
                [centroid[0], centroid[1], group.highest + 4.0],
                format!(
                    "{}  {threads} threads",
                    clean_text(&label_name(&process.name), 15)
                ),
            ));
            let mut notes = vec![format!("{threads} threads, {} flying", group.birds)];
            notes.extend(busiest(process));
            stage.notes.insert(process.id, notes);
            if stage.toured == Some(process.id) {
                stage
                    .callout
                    .push((process.id, format!("{} birds in flight", group.birds)));
            }
        }
    }
}

/// The inspector line naming a process's three busiest threads, or what stands in for them.
fn busiest(process: &Process) -> Option<String> {
    let Measured::Known(list) = &process.thread_list else {
        return Some("threads not read: speed from the process mean".to_owned());
    };
    let mut threads: Vec<_> = list
        .threads
        .iter()
        .filter_map(|thread| Some((thread.cpu?, thread)))
        .collect();
    threads.sort_by(|a, b| b.0.total_cmp(&a.0));
    let names: Vec<String> = threads
        .iter()
        .take(3)
        .map(|(cpu, thread)| format!("{} {cpu:.0}%", clean_text(&thread.name, 15)))
        .collect();
    (!names.is_empty()).then(|| format!("busiest: {}", names.join(", ")))
}

/// A silhouette of trees along the bottom of the frame, in opaque triangles that pick nothing.
/// It stands for the roost and carries no data. The crown is the prototype's: two sine ridges,
/// scaled to the frame.
fn paint_treeline(stage: &mut Stage, depth: f32) {
    let (width, height) = (stage.frame.width as f32, stage.frame.height as f32);
    let ridge = |x: f32| {
        let u = x / width;
        height * (0.913 - 0.0304 * (u * 21.62).sin().abs() - 0.0196 * (u * 72.73 + 1.0).sin().abs())
    };
    let columns = (width / 4.0).ceil() as usize;
    for column in 0..columns {
        let left = column as f32 * 4.0;
        let right = (left + 4.0).min(width);
        let (top_left, top_right) = (ridge(left), ridge(right));
        let corners = [
            [left, top_left, depth],
            [right, top_right, depth],
            [right, height + 1.0, depth],
            [left, height + 1.0, depth],
        ];
        for triangle in [[0, 1, 2], [0, 2, 3]] {
            stage.frame.items.push(Item::Triangle(
                triangle.map(|corner| corners[corner]),
                TREELINE,
                NONE,
            ));
        }
    }
}

impl ViewModule for Murmuration {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        self.observe(stage.time, stage.dt > 0.0, processes, snapshot);
        let alpha = self.advance(stage.time);
        self.paint(stage, processes, alpha);
        processes.len()
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let mut parts: Vec<String> = Vec::new();
        let Summary {
            threads,
            birds,
            mean,
        } = self.summary;
        if birds > 0 && birds < threads {
            let per = threads as f32 / birds as f32;
            parts.push(if per < 10.0 {
                format!("1 bird per {per:.1} threads")
            } else {
                format!("1 bird per {per:.0} threads")
            });
        }
        if mean > 0 {
            parts.push(format!("speed from process mean for {mean}"));
        }
        let notes = [
            missing_note(snapshot, Source::Threads, "speed from process mean"),
            oom_kills(snapshot).map_or_else(
                || {
                    missing_note(
                        snapshot,
                        Source::OomKills,
                        "falcon from memory pressure only",
                    )
                },
                |_| None,
            ),
        ];
        parts.extend(
            notes
                .into_iter()
                .flatten()
                .map(|note| note.trim_start_matches(" | ").to_owned()),
        );
        parts.push(LEGEND.to_owned());
        format!(" {}", parts.join(" | "))
    }

    /// A flock drifts by itself, so the place a process keeps is the clump its threads started
    /// as, which depends on nothing but the process.
    #[cfg(test)]
    fn anchors(&self) -> Option<HashMap<Identity, crate::render::Point>> {
        Some(
            self.flock
                .birds
                .iter()
                .map(|bird| (bird.key.process, clump_centre(bird.key.process)))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::process;
    use crate::model::{DemoOptions, demo_with};
    use crate::platform::Sources;
    use crate::render::{Camera, Frame, RenderInput, Scene};
    use crate::threads::{Thread, ThreadList};
    use crate::views::view;
    use std::sync::Arc;

    /// The prototype's processes: threads and the share of them that is busy.
    const MIX: [(u32, f32); 11] = [
        (1100, 0.15),
        (400, 0.9),
        (260, 0.3),
        (180, 0.2),
        (60, 0.4),
        (220, 0.2),
        (40, 0.02),
        (20, 0.05),
        (480, 0.6),
        (90, 0.1),
        (900, 0.05),
    ];

    fn identity(seed: u64, index: usize) -> Identity {
        Identity {
            pid: 100 + index as u32,
            start: 5_000 + seed * 77 + index as u64,
        }
    }

    /// A flock of the prototype's mix with `scale` of its threads, the busy ones at full speed,
    /// settled for `steps` steps from the clumps it starts as. `seed` picks which threads are
    /// busy and where each process's clump starts.
    fn prototype_flock(seed: u64, scale: f32, steps: usize) -> Flock {
        let mut random = Rng::new(seed);
        let mut wanted = Vec::new();
        for (index, &(threads, share)) in MIX.iter().enumerate() {
            for tid in 0..(threads as f32 * scale) as u32 {
                let busy = random.unit() < share;
                wanted.push(Wanted {
                    key: Key {
                        process: identity(seed, index),
                        listed: true,
                        tid,
                    },
                    speed: 1.0 + 0.3 * f32::from(u8::from(busy)),
                });
            }
        }
        wanted.sort_by_key(|want| want.key);
        let mut flock = Flock::default();
        flock.reroster(wanted);
        for _ in 0..steps {
            flock.step();
        }
        flock
    }

    fn positions(flock: &Flock) -> Vec<Vector> {
        flock.birds.iter().map(|bird| bird.position).collect()
    }

    /// The size of the largest connected component of the graph that joins every bird to its
    /// seven nearest neighbours.
    fn largest_component(points: &[Vector]) -> usize {
        fn root(parent: &mut [usize], mut node: usize) -> usize {
            while parent[node] != node {
                parent[node] = parent[parent[node]];
                node = parent[node];
            }
            node
        }
        let mut grid = SpatialHash3::new(CELL);
        grid.rebuild(points);
        let mut parent: Vec<usize> = (0..points.len()).collect();
        let mut nearest = Vec::new();
        for (index, &point) in points.iter().enumerate() {
            grid.nearest(point, NEIGHBOURS + 1, &mut nearest);
            for &(other, _) in &nearest {
                let (a, b) = (root(&mut parent, index), root(&mut parent, other));
                parent[a] = b;
            }
        }
        let mut sizes = HashMap::new();
        for index in 0..points.len() {
            *sizes.entry(root(&mut parent, index)).or_insert(0) += 1;
        }
        sizes.into_values().max().unwrap_or(0)
    }

    #[test]
    fn the_flock_stays_one_body_under_a_fixed_seed() {
        // A flock is chaotic: a rounding in `ln` or `cos` that differs between platforms changes
        // which of its lobes join and split by the hundredth step. So the property is the
        // flock's, measured over eight fixed seeds, not one lucky run: the largest component of
        // the 7-NN graph holds nearly every bird after 100 steps (five seconds, when cohesion over
        // only seven neighbours would already have split off islands), and after 260 steps, where
        // it turns about the roost and sheds lobes, most of them. (In 80 seeds, 5 held more than
        // 95 % at 260 steps and the median was 84 %.)
        let mut at_100 = Vec::new();
        let mut at_260 = Vec::new();
        for seed in 0..8 {
            let mut flock = prototype_flock(seed, 0.5, 100);
            let share = |flock: &Flock| {
                let points = positions(flock);
                largest_component(&points) as f32 / points.len() as f32
            };
            assert!(flock.birds.len() >= 1800);
            at_100.push(share(&flock));
            for _ in 100..260 {
                flock.step();
            }
            at_260.push(share(&flock));
        }
        let median = |shares: &mut Vec<f32>| {
            shares.sort_by(f32::total_cmp);
            (shares[3] + shares[4]) / 2.0
        };
        let least = |shares: &[f32]| shares.iter().copied().fold(1.0, f32::min);
        assert!(least(&at_100) > 0.9, "after 100 steps: {at_100:?}");
        assert!(median(&mut at_100) > 0.97, "after 100 steps: {at_100:?}");
        assert!(least(&at_260) > 0.55, "after 260 steps: {at_260:?}");
        assert!(median(&mut at_260) > 0.75, "after 260 steps: {at_260:?}");
    }

    #[test]
    fn threads_fly_closer_to_their_own_process_than_to_random_threads() {
        let flock = prototype_flock(12, 0.5, 260);
        let mut random = Rng::new(3);
        let (mut own, mut other) = (0.0_f64, 0.0_f64);
        let mut pairs = 0;
        for bird in &flock.birds {
            // Compare each bird of the larger processes with one of its fellows and one stranger.
            if bird.owner >= 5 {
                continue;
            }
            let mates: Vec<&Bird> = flock
                .birds
                .iter()
                .filter(|b| b.owner == bird.owner && b.key != bird.key)
                .collect();
            let stranger = &flock.birds[random.below(flock.birds.len())];
            let mate = mates[random.below(mates.len())];
            own += f64::from(length(subtract(bird.position, mate.position)));
            other += f64::from(length(subtract(bird.position, stranger.position)));
            pairs += 1;
        }
        assert!(pairs > 500);
        assert!(
            own < 0.9 * other,
            "mean distance to a fellow {:.1}, to a random thread {:.1}",
            own / f64::from(pairs),
            other / f64::from(pairs)
        );
    }

    #[test]
    fn agitation_reaches_beyond_the_flee_radius() {
        let mut flock = prototype_flock(12, 0.25, 260);
        flock.oom_pending = true;
        let mut farthest_agitated = 0.0_f32;
        let mut frightened = 0;
        for _ in 0..140 {
            flock.step();
            let Some(falcon) = &flock.falcon else {
                continue;
            };
            for bird in &flock.birds {
                let distance = length(subtract(bird.position, falcon.position));
                if bird.agitation > SPREAD_FLOOR {
                    farthest_agitated = farthest_agitated.max(distance);
                }
                frightened += usize::from(distance < FLEE_RADIUS);
            }
        }
        assert!(frightened > 0, "the falcon dives through the flock");
        assert!(
            farthest_agitated > 2.0 * FLEE_RADIUS,
            "agitation reached {farthest_agitated:.1} from the falcon"
        );
    }

    /// `count` processes of `threads` threads each, with no thread list.
    fn unlisted(count: u32, threads: u32, cpu: f32) -> Vec<Process> {
        (1..=count)
            .map(|pid| Process {
                threads,
                cpu,
                ..process(pid, 1)
            })
            .collect()
    }

    /// A process whose thread list gives each thread of `cpus` that CPU, with tids from 1000 up.
    fn listed(pid: u32, cpus: &[Option<f32>]) -> Process {
        let threads: Vec<Thread> = cpus
            .iter()
            .enumerate()
            .map(|(index, &cpu)| Thread {
                tid: 1000 * pid + index as u32,
                start: index as u64,
                name: format!("worker-{index}"),
                cpu,
                state: 'S',
                leader: index == 0,
            })
            .collect();
        Process {
            threads: threads.len() as u32,
            thread_list: Measured::Known(ThreadList {
                threads: Arc::from(threads),
                complete: true,
            }),
            ..process(pid, 1)
        }
    }

    fn sample(processes: Vec<Process>, elapsed: f64) -> Snapshot {
        Snapshot {
            processes,
            elapsed,
            ..Snapshot::default()
        }
    }

    /// What a frame at `time` does to the view before it draws: takes in the sample and steps the
    /// flock, with the clock running.
    fn fly(view: &mut Murmuration, snapshot: &Snapshot, time: f32) {
        let processes: Vec<&Process> = snapshot.processes.iter().collect();
        view.observe(time, true, &processes, snapshot);
        view.advance(time);
    }

    /// Every bird's key, position, velocity and agitation, the state two runs must share.
    fn state(flock: &Flock) -> Vec<(Key, Vector, Vector, f32)> {
        flock
            .birds
            .iter()
            .map(|bird| (bird.key, bird.position, bird.velocity, bird.agitation))
            .collect()
    }

    #[test]
    fn the_flock_follows_the_same_path_at_ten_and_sixty_frames_per_second() {
        let mut snapshot = sample(unlisted(40, 8, 12.0), 5.0);
        // Memory pressure releases a falcon after eight seconds, which both runs must see alike.
        snapshot.pressure = [0.0, 15.0, 0.0];
        let run = |fps: u32| {
            let mut view = Murmuration::default();
            let mut states = HashMap::new();
            for k in 0..=11 * fps {
                fly(&mut view, &snapshot, 100.0 + k as f32 / fps as f32);
                let falcon = view.flock.falcon.as_ref().map(|f| f.position);
                states.insert(view.flock.steps, (state(&view.flock), falcon));
            }
            states
        };
        let (slow, fast) = (run(10), run(60));
        let mut compared = 0;
        let mut falcons = 0;
        for (steps, state) in &slow {
            if let Some(other) = fast.get(steps) {
                assert_eq!(state, other, "after {steps} steps");
                compared += 1;
                falcons += usize::from(state.1.is_some());
            }
        }
        assert!(compared >= 100, "compared {compared} states");
        assert!(falcons > 0, "the falcon flew in the compared steps");
        assert!(slow.keys().max() >= Some(&215));
    }

    #[test]
    fn birds_keep_their_flight_when_cpu_memory_or_population_changes() {
        let mut snapshot = sample(unlisted(30, 6, 10.0), 5.0);
        let mut view = Murmuration::default();
        for k in 0..=100 {
            fly(&mut view, &snapshot, k as f32 * STEP);
        }
        let before: HashMap<Key, Vector> = view
            .flock
            .birds
            .iter()
            .map(|bird| (bird.key, bird.position))
            .collect();
        // CPU and memory change, a tenth of the processes exit and as many start.
        for process in &mut snapshot.processes {
            process.cpu *= 4.0;
            process.memory *= 2;
        }
        snapshot.processes.drain(..3);
        snapshot.processes.extend((200..203).map(|pid| Process {
            threads: 6,
            ..process(pid, 1)
        }));
        snapshot.elapsed = 6.0;
        let processes: Vec<&Process> = snapshot.processes.iter().collect();
        view.observe(101.0 * STEP, true, &processes, &snapshot);
        view.flock.step();
        let mut survivors = 0;
        for bird in view
            .flock
            .birds
            .iter()
            .filter(|bird| bird.flight == Flight::Flying)
        {
            let Some(&was) = before.get(&bird.key) else {
                continue;
            };
            survivors += 1;
            let moved = length(subtract(bird.position, was));
            assert!(
                moved <= bird.speed + 1e-3,
                "{:?} moved {moved} in one step at speed {}",
                bird.key,
                bird.speed
            );
        }
        assert!(survivors >= 100, "{survivors} birds flew on");
        let newcomers = view
            .flock
            .birds
            .iter()
            .filter(|bird| bird.key.process.pid >= 200)
            .count();
        assert_eq!(newcomers, 18, "the new processes' threads start too");
    }

    /// The thread list of a process with `count` threads of cpu 1, tids from `from`.
    fn crowd(pid: u32, count: u32) -> Process {
        listed(pid, &vec![Some(1.0); count as usize])
    }

    #[test]
    fn the_bird_cap_samples_threads_per_process_proportionally() {
        let processes = [crowd(1, 6000), crowd(2, 3000), crowd(3, 1), crowd(4, 1)];
        let refs: Vec<&Process> = processes.iter().collect();
        let (wanted, summary) = roster(&refs);
        let flown = |pid: u32| wanted.iter().filter(|w| w.key.process.pid == pid).count();
        // 8000 / 9002 of each, and a single thread always flies.
        assert_eq!(flown(1), (6000.0_f64 * 8000.0 / 9002.0).round() as usize);
        assert_eq!(flown(2), (3000.0_f64 * 8000.0 / 9002.0).round() as usize);
        assert_eq!((flown(3), flown(4)), (1, 1));
        assert_eq!(summary.threads, 9002);
        assert_eq!(summary.birds, wanted.len());
        assert!((7990..=8010).contains(&wanted.len()));
        // The same threads fly in the next sample, whatever their CPU: the lowest hash of tid.
        let louder = [
            listed(1, &vec![Some(50.0); 6000]),
            crowd(2, 3000),
            crowd(3, 1),
            crowd(4, 1),
        ];
        let refs: Vec<&Process> = louder.iter().collect();
        let (again, _) = roster(&refs);
        let keys = |wanted: &[Wanted]| wanted.iter().map(|w| w.key).collect::<Vec<_>>();
        assert_eq!(keys(&wanted), keys(&again));
        // Below the cap every thread flies, and the legend says nothing of a cap.
        let small = [crowd(1, 50)];
        let refs: Vec<&Process> = small.iter().collect();
        assert_eq!(roster(&refs).0.len(), 50);
        let view = Murmuration {
            summary,
            ..Murmuration::default()
        };
        let legend = view.legend(&Snapshot::default());
        assert!(
            legend.starts_with(" 1 bird per 1.1 threads | "),
            "the legend says how thin the flock is: {legend}"
        );
    }

    #[test]
    fn speed_comes_from_thread_cpu_or_else_the_process_mean() {
        let processes = [
            listed(1, &[Some(100.0), Some(50.0), None, Some(0.0)]),
            Process {
                threads: 4,
                cpu: 80.0,
                ..process(2, 1)
            },
        ];
        let refs: Vec<&Process> = processes.iter().collect();
        let (wanted, summary) = roster(&refs);
        let speeds: Vec<f32> = wanted.iter().map(|w| w.speed).collect();
        // Process 1 has no CPU for its third thread, and process 1 itself no mean: cpu is 0.
        assert_eq!(speeds[..4], [1.3, 1.15, 1.0, 1.0]);
        // Process 2's four threads are unread, and share its 80 % as 20 % each.
        assert!(speeds[4..].iter().all(|&speed| (speed - 1.06).abs() < 1e-6));
        assert_eq!(summary.mean, 5);
        let view = Murmuration {
            summary,
            ..Murmuration::default()
        };
        assert!(
            view.legend(&Snapshot::default())
                .starts_with(" speed from process mean for 5 | bird = thread | ")
        );
    }

    #[test]
    fn new_threads_rise_from_the_roost_and_exiting_threads_fall_to_it() {
        let mut snapshot = sample(vec![crowd(1, 20)], 5.0);
        let mut view = Murmuration::default();
        for k in 0..=40 {
            fly(&mut view, &snapshot, k as f32 * STEP);
        }
        assert_eq!(view.flock.birds.len(), 20);
        // One thread starts and two exit.
        snapshot.processes = vec![listed(1, &[Some(1.0); 19]), crowd(2, 1)];
        snapshot.elapsed = 6.0;
        fly(&mut view, &snapshot, 41.0 * STEP);
        let newcomer = view
            .flock
            .birds
            .iter()
            .find(|bird| bird.key.process.pid == 2)
            .expect("the new thread has a bird");
        assert_eq!(newcomer.flight, Flight::Rising);
        assert!(newcomer.position[2] < 3.0, "it starts at the roost");
        assert_eq!(view.flock.falling.len(), 1, "the exited thread falls");
        let height = |view: &Murmuration| {
            view.flock
                .birds
                .iter()
                .find(|bird| bird.key.process.pid == 2)
                .map(|bird| (bird.flight, bird.position[2]))
                .expect("the bird flies")
        };
        let start = view.flock.falling[0].from[2];
        assert!(start > 30.0, "it falls from the flock");
        let mut last = height(&view).1;
        for k in 42..=100 {
            fly(&mut view, &snapshot, k as f32 * STEP);
            let (flight, now) = height(&view);
            if flight == Flight::Flying {
                break;
            }
            assert!(now >= last, "it never sinks while it climbs");
            last = now;
        }
        let (flight, altitude) = height(&view);
        assert_eq!(flight, Flight::Flying, "it joins the flock at {altitude}");
        assert!(altitude >= FLOOR_ALTITUDE, "in the altitude band");
        // The faller is gone after 1.5 s, having fallen to the roost.
        let at = view.flock.falling.first().map(|f| f.at(0.0)[2]);
        assert_eq!(at, None);
        let faller = Faller {
            process: Identity::default(),
            from: [10.0, 0.0, start],
            age: FALL_STEPS - 1,
        };
        assert!(faller.at(0.5)[2] < 0.05 * start, "it ends at the roost");
        assert_eq!(Faller { age: 0, ..faller }.at(0.0)[2], start);
    }

    #[test]
    fn a_thread_list_arriving_keeps_the_flock_flying() {
        let mut snapshot = sample(
            vec![Process {
                threads: 12,
                ..process(1, 1)
            }],
            5.0,
        );
        let mut view = Murmuration::default();
        for k in 0..=60 {
            fly(&mut view, &snapshot, k as f32 * STEP);
        }
        let before: Vec<Vector> = positions(&view.flock);
        assert!(view.flock.birds.iter().all(|bird| !bird.key.listed));
        snapshot.processes = vec![listed(1, &[Some(5.0); 12])];
        snapshot.elapsed = 6.0;
        let processes: Vec<&Process> = snapshot.processes.iter().collect();
        view.observe(61.0 * STEP, true, &processes, &snapshot);
        assert_eq!(positions(&view.flock), before, "the same birds fly on");
        assert!(view.flock.birds.iter().all(|bird| bird.key.listed));
        assert!(view.flock.falling.is_empty());
    }

    /// A flock of 60 settled birds, over `ticks` steps of a calm sample.
    fn calm(view: &mut Murmuration, snapshot: &Snapshot, from: f32, seconds: f32) -> f32 {
        let mut time = from;
        while time < from + seconds {
            fly(view, snapshot, time);
            time += STEP;
        }
        time
    }

    fn falcon_cause(view: &Murmuration) -> Option<Cause> {
        view.flock.falcon.as_ref().map(|falcon| falcon.cause)
    }

    #[test]
    fn a_pressure_rise_or_oom_kill_releases_one_falcon() {
        let mut snapshot = sample(unlisted(10, 6, 5.0), 5.0);
        snapshot.oom_kills = Some(3);
        let mut view = Murmuration::default();
        let mut time = calm(&mut view, &snapshot, 0.0, 12.0);
        assert_eq!(falcon_cause(&view), None, "a calm machine has no falcon");
        // Memory pressure rises: one falcon, and no second one for eight seconds.
        snapshot.pressure = [0.0, 12.0, 0.0];
        snapshot.elapsed = 6.0;
        let mut releases = 0;
        let mut last = view.flock.released;
        let end = time + 7.5;
        while time < end {
            fly(&mut view, &snapshot, time);
            if view.flock.released != last {
                last = view.flock.released;
                releases += 1;
                assert_eq!(falcon_cause(&view), Some(Cause::Pressure));
            }
            time += STEP;
        }
        assert_eq!(releases, 1, "one falcon in 7.5 s of high pressure");
        // The falcon has flown through and gone; the pressure falls, and an OOM kill releases one.
        snapshot.pressure = [0.0; 3];
        snapshot.elapsed = 7.0;
        time = calm(&mut view, &snapshot, time, 3.0);
        assert_eq!(falcon_cause(&view), None);
        snapshot.oom_kills = Some(4);
        snapshot.elapsed = 8.0;
        fly(&mut view, &snapshot, time);
        fly(&mut view, &snapshot, time + STEP);
        assert_eq!(falcon_cause(&view), Some(Cause::Oom));
        // The same count releases nothing more.
        let released = view.flock.released;
        snapshot.elapsed = 9.0;
        calm(&mut view, &snapshot, time + 2.0 * STEP, 8.0);
        assert_eq!(view.flock.released, released);
        assert_eq!(falcon_cause(&view), None);
        // The cgroups' kills count when the kernel's own is not read.
        let mut units = Snapshot::default();
        assert_eq!(oom_kills(&units), None);
        units.units.entry("a.service".into()).or_default().oom_kills = Some(2);
        units.units.entry("b.service".into()).or_default().oom_kills = Some(5);
        assert_eq!(oom_kills(&units), Some(7));
    }

    #[test]
    fn reentering_after_a_gap_releases_no_falcon() {
        let mut snapshot = sample(unlisted(10, 6, 5.0), 5.0);
        snapshot.oom_kills = Some(3);
        let mut view = Murmuration::default();
        let time = calm(&mut view, &snapshot, 0.0, 12.0);
        // The view is away for thirty seconds, in which a kill happened and pressure rose.
        snapshot.oom_kills = Some(4);
        snapshot.pressure = [0.0, 40.0, 0.0];
        snapshot.elapsed = 35.0;
        let back = time + 30.0;
        fly(&mut view, &snapshot, back);
        assert!(
            !view.flock.oom_pending,
            "the kill from before the gap is forgotten"
        );
        let end = back + 7.5;
        let mut now = back;
        while now < end {
            fly(&mut view, &snapshot, now);
            assert_eq!(falcon_cause(&view), None, "nothing is released at {now}");
            now += STEP;
        }
        // Pressure that stays high releases one after the cooldown, as ever.
        let end = now + 1.0;
        let mut seen = false;
        while now < end {
            fly(&mut view, &snapshot, now);
            seen |= falcon_cause(&view) == Some(Cause::Pressure);
            now += STEP;
        }
        assert!(seen, "the cooldown runs from the entry");
        // A kill after the entry still counts.
        snapshot.oom_kills = Some(5);
        snapshot.elapsed = 36.0;
        fly(&mut view, &snapshot, now);
        assert!(view.flock.oom_pending);
    }

    /// Draws `snapshot` through a scene with the camera fitted to the roost.
    fn scene_frame(
        scene: &mut Scene,
        camera: &mut Camera,
        snapshot: &Snapshot,
        time: f32,
        paused: bool,
    ) -> Frame {
        let input = RenderInput {
            paused,
            ..RenderInput::small(snapshot, view("murmuration"), camera, time)
        };
        let frame = scene.render(&input);
        scene.fit(camera, 320, 180);
        frame
    }

    fn demo_sample(time: f64) -> Snapshot {
        demo_with(
            time,
            24,
            DemoOptions {
                threads: true,
                kernel_counters: true,
                ..DemoOptions::NONE
            },
        )
    }

    #[test]
    fn hovering_a_process_lights_only_its_threads() {
        let snapshot = demo_sample(12.0);
        let mut scene = Scene::new();
        let mut camera = Camera::default();
        for k in 0..40 {
            scene_frame(
                &mut scene,
                &mut camera,
                &snapshot,
                12.0 + k as f32 * 0.05,
                false,
            );
        }
        let plain = scene_frame(&mut scene, &mut camera, &snapshot, 14.0, false);
        let colours = |frame: &Frame| {
            frame
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Puff { color, .. } if *color != BIRD => Some(*color),
                    _ => None,
                })
                .count()
        };
        assert_eq!(
            colours(&plain),
            0,
            "nothing is lit until something is hovered"
        );
        // The largest process, hovered.
        let hovered = snapshot
            .processes
            .iter()
            .max_by_key(|p| p.threads)
            .expect("the demo has processes")
            .clone();
        scene.highlight = vec![hovered.id];
        let lit = scene_frame(&mut scene, &mut camera, &snapshot, 14.05, false);
        let index = lit
            .identities
            .iter()
            .position(|id| *id == hovered.id)
            .expect("the process is drawn") as u32;
        let own_picks = lit
            .items
            .iter()
            .filter(|item| matches!(item, Item::Pick { pick, .. } if *pick == index))
            .count();
        let own_lights: Vec<Color> = lit
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Puff { color, .. } if *color != BIRD => Some(*color),
                _ => None,
            })
            .collect();
        assert!(hovered.threads >= 64);
        assert_eq!(
            own_lights.len(),
            own_picks,
            "one light for each of its threads"
        );
        assert!(own_picks > 0);
        assert!(
            own_lights
                .iter()
                .all(|&color| color == kind_color(&hovered))
        );
        let label = format!("{}  {} threads", hovered.name, hovered.threads);
        assert!(
            scene.places.iter().any(|(_, text)| *text == label),
            "the label {label:?} is among {:?}",
            scene.places
        );
        let notes = &scene.notes[&hovered.id];
        assert_eq!(
            notes[0],
            format!("{} threads, {own_picks} flying", hovered.threads)
        );
        assert!(notes[1].starts_with("busiest: "), "{notes:?}");
    }

    #[test]
    fn the_inspector_names_the_busiest_threads_without_control_characters() {
        let mut busy = listed(1, &[Some(10.0), Some(90.0), Some(40.0), Some(70.0), None]);
        if let Measured::Known(list) = &mut busy.thread_list {
            let mut threads = list.threads.to_vec();
            threads[1].name = "bad\x1b[2Jname".into();
            list.threads = Arc::from(threads);
        }
        let line = busiest(&busy).expect("a line for known threads");
        assert_eq!(line, "busiest: bad?[2Jname 90%, worker-3 70%, worker-2 40%");
        assert!(line.chars().all(|c| c.is_ascii_graphic() || c == ' '));
        let unread = process(2, 1);
        assert_eq!(
            busiest(&unread).as_deref(),
            Some("threads not read: speed from the process mean")
        );
    }

    #[test]
    fn murmuration_renders_the_same_frame_twice_from_the_same_inputs() {
        let snapshot = demo_sample(12.0);
        let run = || {
            let mut scene = Scene::new();
            let mut camera = Camera::default();
            let mut last = None;
            for k in 0..60 {
                last = Some(scene_frame(
                    &mut scene,
                    &mut camera,
                    &snapshot,
                    12.0 + k as f32 * 0.05,
                    false,
                ));
            }
            let mut frame = last.expect("frames were drawn");
            frame.rasterize();
            (frame.pixels, frame.picks)
        };
        let (first, second) = (run(), run());
        assert_eq!(first, second);
        assert!(first.1.iter().any(|&pick| pick != NONE), "birds pick");
        assert!(
            first.0.iter().any(|&byte| byte != first.0[0]),
            "the sky is drawn"
        );
    }

    #[test]
    fn a_paused_flock_holds_still() {
        let snapshot = demo_sample(12.0);
        let mut scene = Scene::new();
        let mut camera = Camera::default();
        for k in 0..40 {
            scene_frame(
                &mut scene,
                &mut camera,
                &snapshot,
                12.0 + k as f32 * 0.05,
                false,
            );
        }
        let centres = |scene: &Scene| {
            let mut centres: Vec<_> = scene.positions.iter().map(|(id, p)| (*id, *p)).collect();
            centres.sort_by_key(|(id, _)| *id);
            centres
        };
        scene_frame(&mut scene, &mut camera, &snapshot, 14.0, true);
        let held = centres(&scene);
        scene_frame(&mut scene, &mut camera, &snapshot, 14.0, true);
        scene_frame(&mut scene, &mut camera, &snapshot, 14.0, true);
        assert_eq!(centres(&scene), held);
        scene_frame(&mut scene, &mut camera, &snapshot, 14.5, false);
        assert_ne!(centres(&scene), held, "it flies on when the clock runs");
    }

    #[test]
    fn the_falcon_and_the_treeline_pick_nothing_and_the_falcon_is_labelled() {
        let mut snapshot = demo_sample(12.0);
        snapshot.pressure = [0.0; 3];
        let mut scene = Scene::new();
        let mut camera = Camera::default();
        for k in 0..40 {
            scene_frame(
                &mut scene,
                &mut camera,
                &snapshot,
                12.0 + k as f32 * 0.05,
                false,
            );
        }
        // An OOM kill, as the next sample counts it.
        snapshot.oom_kills = snapshot.oom_kills.map(|kills| kills + 1);
        snapshot.elapsed += 1.0;
        let mut frame = None;
        for k in 40..50 {
            frame = Some(scene_frame(
                &mut scene,
                &mut camera,
                &snapshot,
                12.0 + k as f32 * 0.05,
                false,
            ));
        }
        let frame = frame.expect("frames were drawn");
        assert!(scene.places.iter().any(|(_, text)| text == "OOM"));
        let triangles: Vec<_> = frame
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(_, color, pick) => Some((*color, *pick)),
                _ => None,
            })
            .collect();
        assert!(triangles.iter().all(|&(_, pick)| pick == NONE));
        assert_eq!(
            triangles
                .iter()
                .filter(|(color, _)| *color == FALCON)
                .count(),
            2
        );
        assert!(
            triangles
                .iter()
                .filter(|(color, _)| *color == TREELINE)
                .count()
                > 100
        );
        // The treeline is the last thing drawn.
        assert!(matches!(
            frame.items.last(),
            Some(Item::Triangle(_, color, _)) if *color == TREELINE
        ));
    }

    #[test]
    fn the_legend_composes_the_notes_from_the_sources() {
        let mut view = Murmuration::default();
        let mut snapshot = Snapshot::default();
        assert_eq!(view.legend(&snapshot), format!(" {LEGEND}"));
        snapshot.missing = Sources::from(Source::Threads).union(Sources::from(Source::OomKills));
        view.summary = Summary {
            threads: 24000,
            birds: 8000,
            mean: 31,
        };
        assert_eq!(
            view.legend(&snapshot),
            format!(
                " 1 bird per 3.0 threads | speed from process mean for 31 | no threads: speed from process mean | no oom kills: falcon from memory pressure only | {LEGEND}"
            )
        );
        // Kills the cgroups count stand in for the kernel's, so nothing is missing.
        snapshot
            .units
            .entry("a.service".into())
            .or_default()
            .oom_kills = Some(1);
        assert!(!view.legend(&snapshot).contains("no oom kills"));
    }
}
