//! The pure yard simulation: Vicsek flocks of chickens in a fenced yard, stepped at a fixed
//! rate. No drawing and no process data, only chickens, houses and fox eyes.

use std::collections::VecDeque;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::geometry::normalize_or_zero;
use crate::model::{Identity, bounded};
use crate::simulation::{FixedStep, Rng, SpatialHash, hash};

/// Fixed simulation step in seconds. The yard advances on wall-clock time in whole steps, so
/// every frame rate follows the same trajectory.
const H: f32 = 0.05;
/// Steps a single frame may catch up; longer gaps (a stall, a pause) are dropped.
const MAX_STEPS: usize = 10;
/// Wall-clock seconds one frame may count; a longer gap (a stall, a pause) is dropped.
const MAX_GAP: f32 = 0.5;
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
/// Trail points kept for chicks, one every TRAIL_EVERY steps (0.1 s).
const TRAIL: usize = 120;
const TRAIL_EVERY: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Foraging,
    Roosting,
    Feeding,
    /// Holding a file lock while not running: sitting beside her own house's nest.
    Brooding,
    /// Blocked on a file lock: queueing beside the nest of the holder's house.
    Waiting,
    Zombie,
    Frozen,
    Stuck,
}

impl Role {
    /// Walks to a target rather than foraging or holding still.
    fn walks(self) -> bool {
        matches!(
            self,
            Role::Roosting | Role::Feeding | Role::Brooding | Role::Waiting
        )
    }

    /// The heading on arriving: queueing hens face the nest to their west, the rest the yard.
    pub(super) fn resting_heading(self) -> f32 {
        if self == Role::Waiting { PI } else { FRAC_PI_2 }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Chicken {
    pub(super) id: Identity,
    pub(super) flock: usize,
    pub(super) position: [f32; 2],
    pub(super) previous: [f32; 2],
    pub(super) theta: f32,
    pub(super) speed: f32,
    pub(super) noise: f32,
    pub(super) role: Role,
    /// Where a roosting, feeding, brooding or waiting chicken walks, and whether it has got there.
    pub(super) target: [f32; 2],
    pub(super) arrived: bool,
    /// Height of the perch bar a roosting chicken sits on once it arrives, or of the straw pad
    /// a brooding one sits on.
    pub(super) perch: f32,
    /// The process holding the file lock a waiting chicken is blocked on, when it is sampled.
    pub(super) holder: Option<Identity>,
    pub(super) rng: Rng,
    pub(super) radius: f32,
    /// Socket peers as (chicken index, weight); only foraging pairs.
    pub(super) peers: Vec<(usize, f32)>,
    /// Recent positions, newest first, for chicks to follow.
    pub(super) trail: VecDeque<[f32; 2]>,
    pub(super) chicks: u32,
    /// Consecutive samples not running, for the feeding hold.
    pub(super) quiet: u8,
    /// Inspector lines fixed at the last decision (the flock line is added per frame).
    pub(super) notes: Vec<String>,
}

impl Chicken {
    pub(super) fn new(
        id: Identity,
        flock: usize,
        position: [f32; 2],
        theta: f32,
        rng: Rng,
    ) -> Self {
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
            holder: None,
            rng,
            radius: 0.3,
            peers: Vec::new(),
            trail: VecDeque::new(),
            chicks: 0,
            quiet: 0,
            notes: Vec::new(),
        }
    }

    pub(super) fn walking(&self) -> bool {
        self.role.walks() && !self.arrived
    }

    /// On the ground and immovable: separation pushes foragers off it but never moves it.
    /// Walking chickens are obstacles in the same way while they walk.
    fn planted(&self) -> bool {
        match self.role {
            Role::Zombie | Role::Stuck => true,
            Role::Frozen => self.perch == 0.0,
            Role::Feeding | Role::Brooding | Role::Waiting => self.arrived,
            Role::Foraging | Role::Roosting => false,
        }
    }

    fn velocity(&self) -> [f32; 2] {
        [self.speed * self.theta.cos(), self.speed * self.theta.sin()]
    }
}

/// A flock's house as the simulation sees it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Home {
    pub(super) center: [f32; 2],
    /// Radius around the henhouse that foragers walk around; they may pass under the perches.
    pub(super) footprint: f32,
    /// Distance from the centre beyond which a forager is drawn home.
    pub(super) range: f32,
}

/// The pure simulation: chickens sorted by identity inside a fence, with houses and fox eyes.
pub(super) struct Yard {
    pub(super) chickens: Vec<Chicken>,
    pub(super) homes: Vec<Home>,
    pub(super) low: [f32; 2],
    pub(super) high: [f32; 2],
    pub(super) eyes: Vec<[f32; 2]>,
    pub(super) steps: u64,
    pub(super) hash: SpatialHash,
    pub(super) found: Vec<usize>,
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
pub(super) fn keep_inside(point: [f32; 2], radius: f32, low: [f32; 2], high: [f32; 2]) -> [f32; 2] {
    [0, 1].map(|axis| {
        let (lo, hi) = centre_range(low[axis], high[axis], radius);
        point[axis].clamp(lo, hi)
    })
}

/// A fixed direction for two coincident chickens, opposite for each of the pair.
fn parting(a: Identity, b: Identity) -> [f32; 2] {
    let (low, high, sign) = if a < b { (a, b, 1.0) } else { (b, a, -1.0) };
    // pid only: kept for pixel stability, see A33
    let angle = (hash::knuth(low.pid) ^ high.pid) as f32 / u32::MAX as f32 * TAU;
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
    pub(super) fn step(&mut self, h: f32) {
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
                let toward =
                    normalize_or_zero([positions[k][0] - here[0], positions[k][1] - here[1]]);
                sum[0] += weight * (headings[k].cos() + 0.5 * toward[0]);
                sum[1] += weight * (headings[k].sin() + 0.5 * toward[1]);
            }
            if let Some(home) = self.homes.get(chicken.flock) {
                let away = [home.center[0] - here[0], home.center[1] - here[1]];
                let distance = away[0].hypot(away[1]);
                if distance > home.range {
                    let pull = 0.6 * ((distance - home.range) / 4.0).min(1.0);
                    let direction = normalize_or_zero(away);
                    sum[0] += pull * direction[0];
                    sum[1] += pull * direction[1];
                }
            }
            for eye in &self.eyes {
                let away = [here[0] - eye[0], here[1] - eye[1]];
                let distance = away[0].hypot(away[1]);
                if distance < FLEE {
                    let push = 1.5 * (1.0 - distance / FLEE);
                    let direction = normalize_or_zero(away);
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
                role if role.walks() && !chicken.arrived => {
                    let away = [
                        chicken.target[0] - chicken.position[0],
                        chicken.target[1] - chicken.position[1],
                    ];
                    let distance = away[0].hypot(away[1]);
                    if distance <= WALK * h {
                        chicken.position = chicken.target;
                        chicken.arrived = true;
                        chicken.theta = role.resting_heading();
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
    /// Two foragers share an overlap; a forager against a planted or walking chicken takes all
    /// of it, so walkers are never displaced and always reach their targets.
    fn separate(&mut self) {
        let members: Vec<usize> = (0..self.chickens.len())
            .filter(|&i| {
                let chicken = &self.chickens[i];
                chicken.role == Role::Foraging || chicken.planted() || chicken.walking()
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
    pub(super) fn order(&self) -> Option<f32> {
        order_of(self.chickens.iter().filter(|c| c.role == Role::Foraging))
    }

    /// The order parameter and forager count of each of `flocks` flocks, in one pass; `None` for
    /// a flock with no foragers.
    pub(super) fn flock_orders(&self, flocks: usize) -> Vec<Option<(f32, usize)>> {
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

    pub(super) fn find(&self, id: Identity) -> Option<&Chicken> {
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

/// The yard's fixed-step clock, which starts with coop's constants.
pub(super) struct YardClock(pub(super) FixedStep);

impl Default for YardClock {
    fn default() -> Self {
        Self(FixedStep::new(H, MAX_STEPS, MAX_GAP))
    }
}

pub(super) fn noise(cv: f32, psi: f32) -> f32 {
    (ETA_BASE + K_CV * cv.min(2.0) + K_PSI * bounded(psi, PSI_KNEE)).min(TAU)
}

pub(super) fn speed(cpu: f32) -> f32 {
    V_MIN + V_SPAN * bounded(cpu, SPEED_KNEE)
}

/// Peer weight from measured loopback traffic in bytes per second, or co-activity without it.
pub(super) fn peer_weight(traffic: Option<f32>, cpu: f32, other: f32) -> f32 {
    match traffic {
        Some(rate) => ((1.0 + rate.max(0.0) / 1024.0).log2() / 10.0).min(1.5),
        None => 0.8 * bounded(cpu.min(other), 25.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::coop::fixtures::{coop_of, forager, render, scatter, state};
    use crate::model::demo;
    use crate::render::Scene;

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
                states.insert(coop_of(&scene).yard.steps, state(&coop_of(&scene).yard));
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
    fn foragers_step_aside_from_walking_chickens_that_keep_their_course() {
        for role in [Role::Roosting, Role::Feeding] {
            let mut yard = Yard::new([-100.0, -100.0], [100.0, 100.0]);
            let mut walker = forager(2, 0, [0.2, 0.0], 0.0, 0.0);
            walker.role = role;
            walker.target = [5.0, 0.0];
            yard.chickens = vec![forager(1, 0, [0.0, 0.0], 0.0, 0.0), walker];
            yard.separate();
            let [a, b] = [&yard.chickens[0], &yard.chickens[1]];
            let distance = (a.position[0] - b.position[0]).hypot(a.position[1] - b.position[1]);
            assert!(
                distance >= a.radius + b.radius - 1e-5,
                "{role:?} walker overlaps a forager at {distance}"
            );
            assert_eq!(b.position, [0.2, 0.0], "the walker is not pushed");
            for _ in 0..200 {
                yard.step(H);
            }
            assert!(
                yard.chickens[1].arrived,
                "{role:?} walker reaches its target"
            );
            assert_eq!(yard.chickens[1].position, [5.0, 0.0]);
        }
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
}
