//! The colony, with no drawing: ants that walk from a nest to a goal and home again, and the
//! pheromone they lay. It follows the rules of `docs/mockups/code/anthill.py` (`Colony.step`):
//! steer toward a carrot on a quadratic Bezier with a bend hashed per stream, lean toward the side
//! of the heading that smells stronger, add noise of order sqrt(dt), lay pheromone, and let it
//! evaporate and spread. The rates come from the sample (`flows`), never from the simulation, so
//! the colony cannot invent traffic: how an ant walks is decoration, where it goes and how many
//! go are data.
//!
//! The colony advances in whole steps of `STEP` (a `simulation::FixedStep` drives it), so every
//! frame rate follows the same path, and each ant carries a generator seeded from its nest, its
//! goal and the number of ants its stream had sent.

use std::collections::HashSet;
use std::f32::consts::{PI, TAU as TURN};

use super::flows::{Goal, Way};
use crate::model::Identity;
use crate::simulation::{Rng, hash, hash_identity};

/// How fast an ant walks, in world units per second.
pub const SPEED: f32 = 12.0;

/// How long pheromone takes to fall to 1/e of itself, in seconds.
pub const EVAPORATION: f32 = 2.2;

/// Pheromone cells per world unit, up to `MOST_CELLS` along a side.
pub const CELLS_PER_UNIT: f32 = 4.0;
pub const MOST_CELLS: usize = 256;

/// The most ants alive at once. The scale aims at 3,000, and the rest is room for the ants a
/// change of scale leaves in flight.
pub const MOST_ANTS: usize = 3750;

/// The colony's fixed step, in seconds: 20 steps a second.
pub const STEP: f32 = 0.05;

/// Seeds the ants' streams apart from other uses of an identity.
const SALT: u64 = 0x0a27_0fac_0102_1700;

/// What a spread step moves to each of the two neighbours; the centre keeps the rest.
const BLUR: f32 = 0.1;

/// How much pheromone an ant lays each second, carrying and not.
const LAID_CARRYING: f32 = 1.0;
const LAID_EMPTY: f32 = 0.45;

/// The two sensors lean this far from the heading, and smell this far ahead, in world units.
const SENSE_ANGLE: f32 = 0.5;
const SENSE_REACH: f32 = 1.3;

/// How far from its target an ant heads straight for it, and how near it counts as arrived,
/// at least, in world units.
const FINAL_APPROACH: f32 = 3.2;
const NEAR: f32 = 2.24;
const ARRIVED: f32 = 0.77;

/// How far ahead on the route the carrot is, as a fraction of it.
const LEAD: f32 = 0.14;

/// Where an ant walks, copied into each ant so a stream that ends does not strand it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    pub owner: Identity,
    pub goal_key: Goal,
    pub way: Way,
    /// Bytes per call; None is unsized.
    pub crumb: Option<f32>,
    pub nest: [f32; 2],
    pub nest_radius: f32,
    pub goal: [f32; 2],
    pub goal_radius: f32,
    /// The control point of the route's Bezier: the midpoint pushed sideways by a hashed bend.
    pub control: [f32; 2],
    /// The straight distance from the nest to the goal.
    pub span: f32,
    /// The nest's colour, which the pheromone takes.
    pub color: [f32; 3],
}

impl Route {
    pub fn new(
        owner: Identity,
        goal_key: Goal,
        way: Way,
        crumb: Option<f32>,
        nest: ([f32; 2], f32),
        goal: ([f32; 2], f32),
        color: [u8; 3],
    ) -> Route {
        let (dx, dy) = (goal.0[0] - nest.0[0], goal.0[1] - nest.0[1]);
        let seed = hash_identity(owner, SALT) ^ stream_hash(goal_key);
        let mut rng = Rng::new(seed);
        // A hashed curvature, so streams fan out of a nest instead of overlapping.
        let bend = (if rng.unit() < 0.5 { -1.0 } else { 1.0 }) * (0.10 + 0.14 * rng.unit());
        Route {
            owner,
            goal_key,
            way,
            crumb,
            nest: nest.0,
            nest_radius: nest.1,
            goal: goal.0,
            goal_radius: goal.1,
            control: [
                (nest.0[0] + goal.0[0]) * 0.5 - dy * bend,
                (nest.0[1] + goal.0[1]) * 0.5 + dx * bend,
            ],
            span: dx.hypot(dy),
            color: color.map(|channel| f32::from(channel) / 255.0),
        }
    }

    /// The point `t` of the way along the Bezier from the nest to the goal.
    pub fn at(&self, t: f32) -> [f32; 2] {
        let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * (1.0 - t) * t, t * t);
        [
            a * self.nest[0] + b * self.control[0] + c * self.goal[0],
            a * self.nest[1] + b * self.control[1] + c * self.goal[1],
        ]
    }

    /// Roughly how long a round trip takes, in seconds: the Bezier is about a tenth longer than
    /// the straight line, and each end costs a moment to turn.
    pub fn trip(&self) -> f32 {
        2.0 * 1.1 * self.span / SPEED + 1.0
    }

    /// The key a stream is told from the others by.
    pub fn key(&self) -> (Identity, Goal, Way) {
        (self.owner, self.goal_key, self.way)
    }
}

/// A hash of where a stream goes.
fn stream_hash(goal: Goal) -> u64 {
    match goal {
        Goal::File(key) => key,
        Goal::Blanket(id) => hash_identity(id, SALT ^ 1),
    }
}

/// One ant.
#[derive(Clone, Debug)]
pub struct Ant {
    pub position: [f32; 2],
    /// Where it was one step ago, for drawing between steps.
    pub before: [f32; 2],
    pub heading: f32,
    pub before_heading: f32,
    pub homeward: bool,
    pub route: Route,
    target: [f32; 2],
    speed: f32,
    wander: f32,
    rng: Rng,
}

impl Ant {
    /// Whether it carries a crumb: read ants on the way home, write ants on the way out, and ants
    /// of a stream whose direction is not known both ways.
    pub fn carrying(&self) -> bool {
        match self.route.way {
            Way::Read => self.homeward,
            Way::Write => !self.homeward,
            Way::Either => true,
        }
    }

    /// Where it is `alpha` of the way from its last step to its next.
    pub fn at(&self, alpha: f32) -> [f32; 2] {
        [
            self.before[0] + (self.position[0] - self.before[0]) * alpha,
            self.before[1] + (self.position[1] - self.before[1]) * alpha,
        ]
    }

    /// The way it faces `alpha` of the way from its last step to its next.
    pub fn facing(&self, alpha: f32) -> f32 {
        self.before_heading + wrap(self.heading - self.before_heading) * alpha
    }
}

fn wrap(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TURN) - PI
}

/// The pheromone on the ground: three channels, the nest colour's, on a grid of nodes that the
/// ground field shares.
pub struct Pheromone {
    /// Nodes along a side.
    pub nodes: usize,
    /// World units between nodes.
    pub cell: f32,
    pub origin: [f32; 2],
    planes: [Vec<f32>; 3],
    scratch: Vec<f32>,
}

impl Pheromone {
    /// A grid over a square `size` across and centred on the origin, four cells to the unit and at
    /// most `MOST_CELLS` cells along a side.
    pub fn new(size: f32) -> Pheromone {
        let cells = ((size * CELLS_PER_UNIT).ceil() as usize).clamp(2, MOST_CELLS);
        let nodes = cells + 1;
        Pheromone {
            nodes,
            cell: size / cells as f32,
            origin: [-size * 0.5, -size * 0.5],
            planes: std::array::from_fn(|_| vec![0.0; nodes * nodes]),
            scratch: vec![0.0; nodes * nodes],
        }
    }

    /// The node nearest `at`, clamped to the grid.
    fn node(&self, at: [f32; 2]) -> usize {
        let last = (self.nodes - 1) as f32;
        let column = ((at[0] - self.origin[0]) / self.cell)
            .round()
            .clamp(0.0, last) as usize;
        let row = ((at[1] - self.origin[1]) / self.cell)
            .round()
            .clamp(0.0, last) as usize;
        row * self.nodes + column
    }

    /// How much pheromone lies at `at`, all channels together.
    pub fn smell(&self, at: [f32; 2]) -> f32 {
        let node = self.node(at);
        self.planes.iter().map(|plane| plane[node]).sum()
    }

    /// Lays `amount` of each channel at `at`.
    pub fn lay(&mut self, at: [f32; 2], amount: [f32; 3]) {
        let node = self.node(at);
        for (plane, amount) in self.planes.iter_mut().zip(amount) {
            plane[node] += amount;
        }
    }

    /// The three channels at node `index`.
    pub fn at(&self, index: usize) -> [f32; 3] {
        [
            self.planes[0][index],
            self.planes[1][index],
            self.planes[2][index],
        ]
    }

    /// All the pheromone there is, for tests.
    #[cfg(test)]
    pub fn total(&self) -> f32 {
        self.planes.iter().flatten().sum()
    }

    /// Lets it evaporate for `dt` seconds, `exp(-dt / EVAPORATION)`, then spreads it a little: a
    /// three-tap blur along the rows and then the columns.
    pub fn fade(&mut self, dt: f32) {
        let keep = (-dt / EVAPORATION).exp();
        let n = self.nodes;
        for plane in &mut self.planes {
            // Along the rows, scaled by the evaporation.
            for row in 0..n {
                let line = &plane[row * n..(row + 1) * n];
                let out = &mut self.scratch[row * n..(row + 1) * n];
                for column in 0..n {
                    let left = line[column.saturating_sub(1)];
                    let right = line[(column + 1).min(n - 1)];
                    out[column] =
                        keep * (line[column] * (1.0 - 2.0 * BLUR) + (left + right) * BLUR);
                }
            }
            // Down the columns.
            for row in 0..n {
                let (up, down) = (row.saturating_sub(1), (row + 1).min(n - 1));
                for column in 0..n {
                    plane[row * n + column] = self.scratch[row * n + column] * (1.0 - 2.0 * BLUR)
                        + (self.scratch[up * n + column] + self.scratch[down * n + column]) * BLUR;
                }
            }
        }
    }
}

/// A stream the colony sends ants along: a route and how many ants it sends a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sending {
    pub route: Route,
    pub per_second: f32,
}

/// What a stream still owes: the fraction of an ant not yet sent and how many it has sent.
#[derive(Clone, Copy, Debug)]
struct Account {
    sending: Sending,
    owed: f32,
    sent: u64,
}

/// The ants and the pheromone.
pub struct Colony {
    pub pheromone: Pheromone,
    pub ants: Vec<Ant>,
    accounts: Vec<Account>,
    /// Ants a stream was due to send that the cap refused, since the colony began.
    pub refused: u64,
    /// Steps taken, which says where in its trajectory the colony is.
    pub steps: u64,
}

impl Colony {
    pub fn new(size: f32) -> Colony {
        Colony {
            pheromone: Pheromone::new(size),
            ants: Vec::new(),
            accounts: Vec::new(),
            refused: 0,
            steps: 0,
        }
    }

    /// Takes the streams of a new sample. A stream that goes on keeps what it owed and how many
    /// it has sent, so a sample does not restart it; one that stopped sends no more, and the ants
    /// it has on the way finish their trip.
    pub fn send(&mut self, streams: impl IntoIterator<Item = Sending>) {
        let kept: Vec<Account> = std::mem::take(&mut self.accounts);
        self.accounts = streams
            .into_iter()
            .map(|sending| {
                let before = kept
                    .iter()
                    .find(|account| account.sending.route.key() == sending.route.key());
                Account {
                    sending,
                    owed: before.map_or(0.0, |account| account.owed),
                    sent: before.map_or(0, |account| account.sent),
                }
            })
            .collect();
    }

    /// Drops the ants of nests that are gone.
    pub fn retire(&mut self, alive: &HashSet<Identity>) {
        self.ants.retain(|ant| alive.contains(&ant.route.owner));
    }

    /// Advances the colony by one step of `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.steps += 1;
        self.spawn(dt);
        self.walk(dt);
        self.pheromone.fade(dt);
    }

    fn spawn(&mut self, dt: f32) {
        for account in &mut self.accounts {
            account.owed += account.sending.per_second * dt;
            while account.owed >= 1.0 {
                account.owed -= 1.0;
                if self.ants.len() >= MOST_ANTS {
                    self.refused += 1;
                    continue;
                }
                let ant = leave(&account.sending.route, account.sent);
                account.sent += 1;
                self.ants.push(ant);
            }
        }
    }

    fn walk(&mut self, dt: f32) {
        let mut laid: Vec<([f32; 2], [f32; 3])> = Vec::with_capacity(self.ants.len());
        let pheromone = &self.pheromone;
        self.ants.retain_mut(|ant| {
            ant.before = ant.position;
            ant.before_heading = ant.heading;
            let arrived = steer(ant, dt, pheromone);
            if arrived {
                if ant.homeward {
                    return false;
                }
                turn(ant);
            }
            let weight = if ant.carrying() {
                LAID_CARRYING
            } else {
                LAID_EMPTY
            } * dt;
            laid.push((
                ant.position,
                ant.route.color.map(|channel| channel * weight),
            ));
            true
        });
        for (at, amount) in laid {
            self.pheromone.lay(at, amount);
        }
    }
}

/// A new ant of the stream `route`, the `sent`th it has sent, seeded from its nest, its goal and
/// that number.
fn leave(route: &Route, sent: u64) -> Ant {
    let seed = hash_identity(route.owner, SALT)
        ^ stream_hash(route.goal_key).rotate_left(21)
        ^ hash::golden_gamma_high(sent + 1);
    let mut rng = Rng::new(seed);
    let toward = (route.goal[1] - route.nest[1]).atan2(route.goal[0] - route.nest[0]);
    let angle = toward + 0.55 * rng.normal();
    let inside = route.nest_radius * (0.55 + 0.4 * rng.unit());
    let position = [
        route.nest[0] + inside * angle.cos(),
        route.nest[1] + inside * angle.sin(),
    ];
    let back = toward + PI + 0.8 * rng.normal();
    let reach = route.goal_radius * (0.6 + 0.4 * rng.unit());
    let heading = angle + 0.3 * rng.normal();
    Ant {
        position,
        before: position,
        heading,
        before_heading: heading,
        homeward: false,
        route: *route,
        target: [
            route.goal[0] + reach * back.cos(),
            route.goal[1] + reach * back.sin(),
        ],
        speed: SPEED * (1.0 + 0.13 * rng.normal()).clamp(0.7, 1.3),
        wander: 1.0,
        rng,
    }
}

/// Turns an ant that reached its goal round, to head home to a point inside its nest.
fn turn(ant: &mut Ant) {
    let route = ant.route;
    let toward = (route.goal[1] - route.nest[1]).atan2(route.goal[0] - route.nest[0]);
    let angle = toward + 0.6 * ant.rng.normal();
    let inside = route.nest_radius * 0.35;
    ant.target = [
        route.nest[0] + inside * angle.cos(),
        route.nest[1] + inside * angle.sin(),
    ];
    ant.heading += PI + 0.4 * ant.rng.normal();
    ant.homeward = true;
    ant.wander = ant.wander.min(1.4);
}

/// Moves one ant a step toward its target along its route, leaning toward the stronger smell,
/// and says whether it arrived.
fn steer(ant: &mut Ant, dt: f32, pheromone: &Pheromone) -> bool {
    let route = ant.route;
    let (dx, dy) = (
        ant.target[0] - ant.position[0],
        ant.target[1] - ant.position[1],
    );
    let distance = dx.hypot(dy);
    // A carrot ahead on the route; outbound runs nest to goal and homeward goal to nest.
    let along = ((1.0 - distance / (route.span + 1e-6)).clamp(0.0, 1.0) + LEAD).min(1.0);
    let carrot = if distance < FINAL_APPROACH {
        ant.target
    } else {
        route.at(if ant.homeward { 1.0 - along } else { along })
    };
    let want = (carrot[1] - ant.position[1]).atan2(carrot[0] - ant.position[0]);
    let sensor = |lean: f32| {
        let angle = ant.heading + lean;
        pheromone.smell([
            ant.position[0] + SENSE_REACH * angle.cos(),
            ant.position[1] + SENSE_REACH * angle.sin(),
        ])
    };
    let (left, right) = (sensor(-SENSE_ANGLE), sensor(SENSE_ANGLE));
    let lean = (right - left) / (right + left + 0.08);
    let stiffness = 3.2 / ant.wander;
    ant.heading += dt * (stiffness * wrap(want - ant.heading) + 3.0 * lean)
        + dt.sqrt() * 0.55 * ant.wander * ant.rng.normal();
    if distance < NEAR {
        ant.heading = (dy).atan2(dx);
    }
    ant.position[0] += ant.speed * dt * ant.heading.cos();
    ant.position[1] += ant.speed * dt * ant.heading.sin();
    distance < ARRIVED.max(ant.speed * dt * 1.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(pid: u32) -> Identity {
        Identity {
            pid,
            start: u64::from(pid),
        }
    }

    fn route(way: Way) -> Route {
        Route::new(
            id(1),
            Goal::File(42),
            way,
            Some(4096.0),
            ([0.0, 0.0], 2.0),
            ([30.0, 0.0], 1.0),
            [200, 160, 60],
        )
    }

    fn sending(way: Way, per_second: f32) -> Sending {
        Sending {
            route: route(way),
            per_second,
        }
    }

    fn run(colony: &mut Colony, seconds: f32) {
        for _ in 0..(seconds / STEP).round() as u32 {
            colony.step(STEP);
        }
    }

    #[test]
    fn pheromone_evaporates_with_its_time_constant() {
        let mut field = Pheromone::new(16.0);
        // A flat sheet, so the blur has nothing to move and only the evaporation shows.
        for plane in &mut field.planes {
            plane.fill(1.0);
        }
        let before = field.at(field.nodes * 8 + 8)[0];
        for _ in 0..(EVAPORATION / STEP).round() as u32 {
            field.fade(STEP);
        }
        let after = field.at(field.nodes * 8 + 8)[0];
        assert!(
            (after / before - (-1.0_f32).exp()).abs() < 1e-3,
            "{} is not 1/e",
            after / before
        );
    }

    #[test]
    fn the_blur_spreads_pheromone_without_adding_any() {
        let mut field = Pheromone::new(16.0);
        let middle = field.nodes * 32 + 32;
        field.planes[0][middle] = 100.0;
        // Take the evaporation out of the sum: it is one factor, whatever the spreading did.
        field.fade(0.0);
        assert!((field.total() - 100.0).abs() < 1e-3, "{}", field.total());
        assert!(field.planes[0][middle] < 100.0);
        assert!(field.planes[0][middle + 1] > 0.0 && field.planes[0][middle + field.nodes] > 0.0);
    }

    #[test]
    fn the_pheromone_grid_has_four_cells_to_the_unit_and_never_more_than_256_a_side() {
        let small = Pheromone::new(64.0);
        assert_eq!((small.nodes, small.cell), (257, 0.25));
        let large = Pheromone::new(400.0);
        assert_eq!(large.nodes, 257);
        assert!((large.cell - 400.0 / 256.0).abs() < 1e-6);
    }

    #[test]
    fn a_read_ant_goes_out_empty_and_comes_home_with_a_crumb() {
        let mut colony = Colony::new(64.0);
        colony.send([sending(Way::Read, 2.0)]);
        run(&mut colony, 1.0);
        assert!(!colony.ants.is_empty());
        let mut saw = (false, false);
        for _ in 0..400 {
            colony.step(STEP);
            for ant in &colony.ants {
                match (ant.homeward, ant.carrying()) {
                    (false, false) => saw.0 = true,
                    (true, true) => saw.1 = true,
                    other => panic!("a read ant {other:?}"),
                }
            }
        }
        assert_eq!(saw, (true, true));
    }

    #[test]
    fn a_write_ant_carries_the_crumb_out_and_an_either_ant_both_ways() {
        let mut colony = Colony::new(64.0);
        colony.send([sending(Way::Write, 2.0)]);
        run(&mut colony, 4.0);
        assert!(colony.ants.iter().all(|ant| ant.carrying() != ant.homeward));
        let mut colony = Colony::new(64.0);
        colony.send([sending(Way::Either, 2.0)]);
        run(&mut colony, 4.0);
        assert!(!colony.ants.is_empty() && colony.ants.iter().all(Ant::carrying));
    }

    #[test]
    fn ants_walk_to_their_goal_and_home_again_and_the_trip_takes_about_its_length_over_the_speed() {
        let mut colony = Colony::new(64.0);
        colony.send([sending(Way::Read, 1.0)]);
        // One ant is sent in the first second, and nothing after it.
        run(&mut colony, 1.0);
        colony.send([sending(Way::Read, 0.0)]);
        assert_eq!(colony.ants.len(), 1);
        let mut reached = 0.0_f32;
        let mut seconds = 0.0;
        while !colony.ants.is_empty() && seconds < 20.0 {
            colony.step(STEP);
            seconds += STEP;
            if let Some(ant) = colony.ants.first() {
                reached = reached.max(ant.position[0]);
            }
        }
        assert!(colony.ants.is_empty(), "the ant got home and went in");
        assert!(reached > 26.0, "it reached the pile at 30: {reached}");
        let expected = route(Way::Read).trip();
        assert!(
            (seconds - expected).abs() < 3.5,
            "{seconds} s against {expected} s"
        );
    }

    #[test]
    fn a_stream_sends_the_ants_it_is_owed_and_a_new_sample_does_not_restart_it() {
        let mut colony = Colony::new(64.0);
        colony.send([sending(Way::Read, 10.0)]);
        run(&mut colony, 2.0);
        let sent = colony.accounts[0].sent;
        assert!((19..=20).contains(&sent), "{sent}");
        colony.send([sending(Way::Read, 10.0)]);
        assert_eq!(colony.accounts[0].sent, sent);
        colony.send([]);
        run(&mut colony, 1.0);
        assert!(colony.accounts.is_empty());
    }

    #[test]
    fn the_pool_is_capped_and_the_refused_ants_are_counted() {
        let mut colony = Colony::new(64.0);
        colony.send([sending(Way::Read, 100_000.0)]);
        colony.step(STEP);
        assert_eq!(colony.ants.len(), MOST_ANTS);
        assert!(colony.refused > 0);
        assert_eq!(colony.refused as usize, 5000 - MOST_ANTS);
    }

    #[test]
    fn ants_of_a_nest_that_left_are_dropped() {
        let mut colony = Colony::new(64.0);
        colony.send([sending(Way::Read, 10.0)]);
        run(&mut colony, 1.0);
        assert!(!colony.ants.is_empty());
        colony.retire(&HashSet::new());
        assert!(colony.ants.is_empty());
    }

    #[test]
    fn the_same_seed_sends_the_same_ants_along_the_same_paths() {
        let walk = || {
            let mut colony = Colony::new(64.0);
            colony.send([sending(Way::Read, 8.0), sending(Way::Write, 3.0)]);
            run(&mut colony, 3.0);
            colony
                .ants
                .iter()
                .map(|ant| (ant.position, ant.homeward))
                .collect::<Vec<_>>()
        };
        assert_eq!(walk(), walk());
        assert!(!walk().is_empty());
    }

    #[test]
    fn ants_lay_more_pheromone_carrying_a_crumb_than_without_one() {
        let mut colony = Colony::new(64.0);
        // 20 ants a second for one step of a twentieth: exactly one ant.
        colony.send([sending(Way::Write, 20.0)]);
        colony.step(STEP);
        colony.send([sending(Way::Write, 0.0)]);
        let outbound = colony.ants[0].clone();
        let before = colony.pheromone.total();
        colony.walk(STEP);
        let carrying = colony.pheromone.total() - before;
        assert!(outbound.carrying());
        assert!(carrying > 0.0);
        let mut empty = Colony::new(64.0);
        empty.send([sending(Way::Read, 20.0)]);
        empty.step(STEP);
        empty.send([sending(Way::Read, 0.0)]);
        let before = empty.pheromone.total();
        empty.walk(STEP);
        let without = empty.pheromone.total() - before;
        assert!(!empty.ants[0].carrying());
        // Same colour and step: the deposit is 1.0 against 0.45 of it.
        assert!(
            (without / carrying - LAID_EMPTY / LAID_CARRYING).abs() < 0.02,
            "{without} against {carrying}"
        );
    }
}
