//! Copper runs between tanks: Manhattan routes on a lattice of half a unit, found by A* around
//! the tanks, cached, and recomputed only when an end moves or a new tank stands in the way.
//! Pure, with no drawing and no process data.
//!
//! The cost of a step is 10 (1 unit of the prototype's weights), a turn adds 25 and stepping onto
//! a node an earlier route used adds 70, so runs prefer short, straight, separate paths. The
//! search runs over a window round the two ends and widens once if that finds nothing; a run
//! that still cannot be routed is `Route::crossing`, a plain elbow that may pass over a tank.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashSet};

use crate::model::Identity;

/// The lattice spacing, in world units: half the prototype's cell, so a pipe passes between the
/// widest two tanks of a bay.
pub const STEP: f32 = 0.5;
/// What a pipe and its collars keep clear of a tank, in world units, beyond the tank's radius.
pub const CLEARANCE: f32 = 0.5;
/// How far from a tank's wall a run's first lattice node is, in world units.
const GAP: f32 = 0.75;
/// The most routes `Router::route` finds in one frame; the rest are drawn as elbows until a later
/// frame finds them.
pub const ROUTES_PER_FRAME: usize = 32;

const MOVE: i32 = 10;
const TURN: i32 = 25;
const REUSE: i32 = 70;
/// The search window round the two ends, in lattice steps, first and wider.
const WINDOWS: [i32; 2] = [12, 48];

/// A lattice node: `STEP` units apart.
pub type Node = (i32, i32);

pub fn node_of(point: [f32; 2]) -> Node {
    (
        (point[0] / STEP).round() as i32,
        (point[1] / STEP).round() as i32,
    )
}

pub fn world_of(node: Node) -> [f32; 2] {
    [node.0 as f32 * STEP, node.1 as f32 * STEP]
}

/// Something a run can start or end at, and that runs go round: a tank or a manifold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hull {
    pub owner: Identity,
    /// 0 for the tank of `owner`, 1 for its manifold.
    pub kind: u8,
}

/// A run's two ends and which of the pair's runs it is (a pipe, a hose).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub from: Hull,
    pub to: Hull,
    pub channel: u8,
}

/// A hull's footprint: a disc on the floor. Runs go round `radius`, which a tank keeps whatever
/// its memory, so a run never moves when memory changes; they start and end on `wall`, the
/// tank's true radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    pub center: [f32; 2],
    pub radius: f32,
    pub wall: f32,
}

/// The four sides of a hull a run can leave by: the lattice directions.
const SIDES: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// A found run.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// The corners of the run in world units, from the first hull's port to the last's, every leg
    /// along an axis. Empty for a run that is `crossing`. `Router::polyline` adds the points on
    /// the walls.
    pub corners: Vec<[f32; 2]>,
    /// The sides the run leaves and enters by, indexing `SIDES`.
    sides: (usize, usize),
    /// Every lattice node the run passes over, which later runs avoid.
    nodes: Vec<Node>,
    /// No clear path was found: the run is a plain elbow that may pass over a tank.
    pub crossing: bool,
}

/// The routes, and the tanks they go round.
#[derive(Default)]
pub struct Router {
    hulls: BTreeMap<Hull, Footprint>,
    blocked: HashSet<Node>,
    routes: BTreeMap<Key, Route>,
    /// How many routes pass over each node, for the cost of using one again.
    used: BTreeMap<Node, i32>,
    /// Routes found since `begin_frame`.
    found: usize,
}

impl Router {
    /// Starts a frame: the budget of routes it may find is full again.
    pub fn begin_frame(&mut self) {
        self.found = 0;
    }

    /// Takes the hulls of this frame. A route whose end moved or went is forgotten, and so is one
    /// that a hull which was not there, or has grown, now stands on; every other route stays,
    /// whatever else changed.
    pub fn update(&mut self, hulls: &BTreeMap<Hull, Footprint>) {
        let blocked = blocked_by(hulls);
        let added: HashSet<Node> = blocked.difference(&self.blocked).copied().collect();
        let moved = |hull: &Hull| match (self.hulls.get(hull), hulls.get(hull)) {
            (Some(before), Some(now)) => before.center != now.center,
            _ => true,
        };
        let gone: Vec<Key> = self
            .routes
            .iter()
            .filter(|(key, route)| {
                moved(&key.from)
                    || moved(&key.to)
                    || route.nodes.iter().any(|node| added.contains(node))
            })
            .map(|(&key, _)| key)
            .collect();
        for key in gone {
            if let Some(route) = self.routes.remove(&key) {
                self.release(&route);
            }
        }
        self.hulls = hulls.clone();
        self.blocked = blocked;
    }

    /// The route of `key`, found now if it has none and the frame's budget is not spent. None
    /// while it waits for a later frame, or when an end is not a hull.
    pub fn route(&mut self, key: Key) -> Option<&Route> {
        if !self.routes.contains_key(&key) {
            if self.found >= ROUTES_PER_FRAME {
                return None;
            }
            let route = self.find(key)?;
            self.found += 1;
            for &node in &route.nodes {
                *self.used.entry(node).or_insert(0) += 1;
            }
            self.routes.insert(key, route);
        }
        self.routes.get(&key)
    }

    /// Whether `key` has a route already.
    #[cfg(test)]
    pub fn has(&self, key: Key) -> bool {
        self.routes.contains_key(&key)
    }

    /// The corners of every route found, by key.
    #[cfg(test)]
    pub fn corners(&self) -> Vec<(Key, Vec<[f32; 2]>)> {
        self.routes
            .iter()
            .map(|(&key, route)| (key, route.corners.clone()))
            .collect()
    }

    /// The directions of the sides of `hull` no run uses and that a run could leave by: for a
    /// manifold, not the side its tank is on.
    pub fn free_sides(&self, hull: Hull) -> Vec<(i32, i32)> {
        let Some(footprint) = self.hulls.get(&hull) else {
            return Vec::new();
        };
        let mut taken = [false; 4];
        for (key, route) in &self.routes {
            if key.from == hull {
                taken[route.sides.0] = true;
            }
            if key.to == hull {
                taken[route.sides.1] = true;
            }
        }
        if hull.kind == 1
            && let Some(tank) = self.hulls.get(&Hull { kind: 0, ..hull })
        {
            let (tx, ty) = (
                tank.center[0] - footprint.center[0],
                tank.center[1] - footprint.center[1],
            );
            let towards = |side: usize| SIDES[side].0 as f32 * tx + SIDES[side].1 as f32 * ty;
            let back = (0..4)
                .max_by(|&a, &b| towards(a).total_cmp(&towards(b)).then(b.cmp(&a)))
                .expect("four sides");
            taken[back] = true;
        }
        (0..4)
            .filter(|&side| !taken[side])
            .map(|side| SIDES[side])
            .collect()
    }

    /// The side of `hull` with the fewest runs on it, of those that face `toward` best.
    fn free_side(&self, hull: Hull, toward: [f32; 2]) -> usize {
        let center = self.hulls[&hull].center;
        let (dx, dy) = (toward[0] - center[0], toward[1] - center[1]);
        let mut order: Vec<usize> = vec![0, 1, 2, 3];
        let facing = |side: usize| SIDES[side].0 as f32 * dx + SIDES[side].1 as f32 * dy;
        order.sort_by(|&a, &b| facing(b).total_cmp(&facing(a)).then(a.cmp(&b)));
        // A manifold never lets a run leave by the side its tank is on.
        if hull.kind == 1
            && let Some(tank) = self.hulls.get(&Hull { kind: 0, ..hull })
        {
            let (tx, ty) = (tank.center[0] - center[0], tank.center[1] - center[1]);
            let towards = |side: usize| SIDES[side].0 as f32 * tx + SIDES[side].1 as f32 * ty;
            let back = (0..4)
                .max_by(|&a, &b| towards(a).total_cmp(&towards(b)).then(b.cmp(&a)))
                .expect("four sides");
            order.retain(|&side| side != back);
        }
        let used = |side: usize| {
            self.routes
                .iter()
                .map(|(key, route)| {
                    usize::from(key.from == hull && route.sides.0 == side)
                        + usize::from(key.to == hull && route.sides.1 == side)
                })
                .sum::<usize>()
        };
        *order
            .iter()
            .min_by_key(|&&side| used(side))
            .expect("four sides")
    }

    /// The first lattice node outside `hull` on `side`.
    fn port(&self, hull: Hull, side: usize) -> Node {
        let footprint = self.hulls[&hull];
        let (dx, dy) = (SIDES[side].0 as f32, SIDES[side].1 as f32);
        let reach = footprint.radius + GAP;
        node_of([
            footprint.center[0] + dx * reach,
            footprint.center[1] + dy * reach,
        ])
    }

    /// The route of `key` as a polyline of points in world units, from a point on the first hull's
    /// wall to one on the last's, every leg along an axis: the route's corners with the walls
    /// the tanks have now. None as `route` is.
    pub fn polyline(&mut self, key: Key) -> Option<Vec<[f32; 2]>> {
        let found = self.route(key)?.clone();
        let (from, to) = (self.hulls[&key.from], self.hulls[&key.to]);
        if found.crossing {
            return Some(elbow(from.center, to.center));
        }
        let wall = |footprint: Footprint, side: usize| {
            let (dx, dy) = (SIDES[side].0 as f32, SIDES[side].1 as f32);
            [
                footprint.center[0] + dx * footprint.wall * 0.9,
                footprint.center[1] + dy * footprint.wall * 0.9,
            ]
        };
        Some(stitched(
            wall(from, found.sides.0),
            &found.corners,
            wall(to, found.sides.1),
        ))
    }

    fn find(&self, key: Key) -> Option<Route> {
        let (from, to) = (self.hulls.get(&key.from)?, self.hulls.get(&key.to)?);
        let sides = (
            self.free_side(key.from, to.center),
            self.free_side(key.to, from.center),
        );
        let (start, goal) = (self.port(key.from, sides.0), self.port(key.to, sides.1));
        let path = WINDOWS.iter().find_map(|&margin| {
            let low = (start.0.min(goal.0) - margin, start.1.min(goal.1) - margin);
            let high = (start.0.max(goal.0) + margin, start.1.max(goal.1) + margin);
            self.search(start, goal, (low, high), &self.used)
        });
        Some(match path {
            Some(nodes) => Route {
                corners: corners_of(&nodes),
                sides,
                nodes,
                crossing: false,
            },
            None => Route {
                corners: Vec::new(),
                sides,
                nodes: Vec::new(),
                crossing: true,
            },
        })
    }

    /// Takes a forgotten route's nodes out of the count of those in use.
    fn release(&mut self, route: &Route) {
        for node in &route.nodes {
            if let Some(count) = self.used.get_mut(node) {
                *count -= 1;
                if *count <= 0 {
                    self.used.remove(node);
                }
            }
        }
    }

    /// A* over the lattice inside `window` (low corner, high corner, inclusive), with the cost
    /// of a move, a turn and the reuse of a node. The two ends and their neighbours are free, so
    /// a run can reach a hull whose clearance covers its port.
    fn search(
        &self,
        start: Node,
        goal: Node,
        window: (Node, Node),
        used: &BTreeMap<Node, i32>,
    ) -> Option<Vec<Node>> {
        let (low, high) = window;
        let (width, height) = ((high.0 - low.0 + 1) as usize, (high.1 - low.1 + 1) as usize);
        let cell = |node: Node| -> Option<usize> {
            ((low.0..=high.0).contains(&node.0) && (low.1..=high.1).contains(&node.1))
                .then(|| (node.0 - low.0) as usize * height + (node.1 - low.1) as usize)
        };
        let mut blocked = vec![false; width * height];
        for &node in &self.blocked {
            if let Some(index) = cell(node) {
                blocked[index] = true;
            }
        }
        for end in [start, goal] {
            for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                if let Some(index) = cell((end.0 + dx, end.1 + dy)) {
                    blocked[index] = false;
                }
            }
        }
        let mut penalty = vec![0_i32; width * height];
        for (&node, &count) in used {
            if let Some(index) = cell(node) {
                penalty[index] = count * REUSE;
            }
        }
        let (start_cell, goal_cell) = (cell(start)?, cell(goal)?);
        // A state is a node and the direction it was entered by: 0 to 3, 4 for the start.
        let mut cost = vec![i32::MAX; width * height * 5];
        let mut parent = vec![u32::MAX; width * height * 5];
        let heuristic = |node: Node| ((node.0 - goal.0).abs() + (node.1 - goal.1).abs()) * MOVE;
        let mut queue = BinaryHeap::new();
        let first = start_cell * 5 + 4;
        cost[first] = 0;
        queue.push(Reverse((heuristic(start), first)));
        while let Some(Reverse((_, state))) = queue.pop() {
            let (index, entered) = (state / 5, state % 5);
            let node = (
                low.0 + (index / height) as i32,
                low.1 + (index % height) as i32,
            );
            if index == goal_cell {
                let mut nodes = vec![node];
                let mut at = state;
                while parent[at] != u32::MAX {
                    at = parent[at] as usize;
                    let step = at / 5;
                    nodes.push((
                        low.0 + (step / height) as i32,
                        low.1 + (step % height) as i32,
                    ));
                }
                nodes.reverse();
                return Some(nodes);
            }
            let spent = cost[state];
            for (direction, (dx, dy)) in SIDES.iter().enumerate() {
                let next = (node.0 + dx, node.1 + dy);
                let Some(next_index) = cell(next) else {
                    continue;
                };
                if blocked[next_index] {
                    continue;
                }
                let turn = if entered == 4 || entered == direction {
                    0
                } else {
                    TURN
                };
                let step = spent + MOVE + turn + penalty[next_index];
                let target = next_index * 5 + direction;
                if step < cost[target] {
                    cost[target] = step;
                    parent[target] = state as u32;
                    queue.push(Reverse((step + heuristic(next), target)));
                }
            }
        }
        None
    }
}

/// Every lattice node within a hull's radius and the clearance of its centre.
fn blocked_by(hulls: &BTreeMap<Hull, Footprint>) -> HashSet<Node> {
    let mut blocked = HashSet::new();
    for footprint in hulls.values() {
        let reach = footprint.radius + CLEARANCE;
        let steps = (reach / STEP).ceil() as i32;
        let center = node_of(footprint.center);
        for i in -steps..=steps {
            for j in -steps..=steps {
                let node = (center.0 + i, center.1 + j);
                let at = world_of(node);
                if (at[0] - footprint.center[0]).hypot(at[1] - footprint.center[1]) <= reach {
                    blocked.insert(node);
                }
            }
        }
    }
    blocked
}

/// The corners of the path through `nodes`: its two ends and every node where it turns.
fn corners_of(nodes: &[Node]) -> Vec<[f32; 2]> {
    let mut corners = Vec::new();
    for (index, &node) in nodes.iter().enumerate() {
        let turns = index > 0
            && index + 1 < nodes.len()
            && (node.0 - nodes[index - 1].0, node.1 - nodes[index - 1].1)
                != (nodes[index + 1].0 - node.0, nodes[index + 1].1 - node.1);
        if index == 0 || index + 1 == nodes.len() || turns {
            corners.push(world_of(node));
        }
    }
    corners
}

/// `corners` between two points on the hulls' walls, moved so the first and the last legs run
/// along an axis.
fn stitched(from: [f32; 2], corners: &[[f32; 2]], to: [f32; 2]) -> Vec<[f32; 2]> {
    let mut points = vec![from];
    points.extend(corners);
    points.push(to);
    // The wall point and the port share an axis through the hull, which the first leg follows.
    if let [a, b, ..] = points[..] {
        points[0] = along(a, b);
    }
    let last = points.len() - 1;
    if last >= 1 {
        points[last] = along(points[last], points[last - 1]);
    }
    points.dedup();
    points
}

/// `wall` moved onto the axis it shares with `port`: whichever coordinate differs least stays
/// where `port` is, so the leg between them is straight.
fn along(wall: [f32; 2], port: [f32; 2]) -> [f32; 2] {
    if (port[0] - wall[0]).abs() >= (port[1] - wall[1]).abs() {
        [wall[0], port[1]]
    } else {
        [port[0], wall[1]]
    }
}

/// A run of two legs between two points, for a route that could not be found.
pub fn elbow(from: [f32; 2], to: [f32; 2]) -> Vec<[f32; 2]> {
    if (from[0] - to[0]).abs() < 1e-3 || (from[1] - to[1]).abs() < 1e-3 {
        vec![from, to]
    } else {
        vec![from, [to[0], from[1]], to]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hull(pid: u32) -> Hull {
        Hull {
            owner: Identity {
                pid,
                start: u64::from(pid),
            },
            kind: 0,
        }
    }

    fn key(a: u32, b: u32) -> Key {
        Key {
            from: hull(a),
            to: hull(b),
            channel: 0,
        }
    }

    /// Tanks of radius `radius` on a grid of `columns` by `rows` seats 3 apart, pid = 1 + row *
    /// columns + column.
    fn grid(columns: u32, rows: u32, radius: f32) -> BTreeMap<Hull, Footprint> {
        (0..columns * rows)
            .map(|k| {
                (
                    hull(1 + k),
                    Footprint {
                        center: [
                            1.0 + 3.0 * (k % columns) as f32,
                            1.0 + 3.0 * (k / columns) as f32,
                        ],
                        radius,
                        wall: radius,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn routes_are_manhattan_and_never_cross_a_tank() {
        let hulls = grid(5, 4, 0.9);
        let mut router = Router::default();
        router.update(&hulls);
        let mut found = 0;
        for a in 1..=20 {
            for b in [(a % 20) + 1, ((a + 6) % 20) + 1, ((a + 12) % 20) + 1] {
                if a == b {
                    continue;
                }
                router.begin_frame();
                let route = router.route(key(a, b)).expect("a route").clone();
                assert!(!route.crossing, "{a} to {b}");
                found += 1;
                let points = router.polyline(key(a, b)).expect("a polyline");
                for pair in points.windows(2) {
                    let (p, q) = (pair[0], pair[1]);
                    assert!(
                        (p[0] - q[0]).abs() < 1e-4 || (p[1] - q[1]).abs() < 1e-4,
                        "{a} to {b}: {p:?} to {q:?} is not along an axis"
                    );
                }
                // Every node the run passes is clear of every tank, the two it joins included.
                for &node in &route.nodes {
                    let at = world_of(node);
                    for (owner, footprint) in &hulls {
                        let distance =
                            (at[0] - footprint.center[0]).hypot(at[1] - footprint.center[1]);
                        let own = *owner == hull(a) || *owner == hull(b);
                        assert!(
                            distance
                                >= footprint.radius + if own { GAP - 0.5 } else { CLEARANCE }
                                    - 1e-3,
                            "{a} to {b} passes {distance} from tank {}",
                            owner.owner.pid
                        );
                    }
                }
            }
        }
        assert!(found >= 50);
    }

    #[test]
    fn a_run_leaves_by_the_side_that_faces_its_target_and_by_another_when_that_is_taken() {
        let hulls = grid(3, 1, 0.5);
        let mut router = Router::default();
        router.update(&hulls);
        let first = router.route(key(1, 3)).unwrap().clone();
        assert_eq!(first.sides.0, 0, "tank 3 lies in the +x direction");
        assert_eq!(first.sides.1, 2, "and tank 1 in -x of it");
        // A second run to the same tank takes another side of the first tank, so the two do not
        // lie on top of one another.
        let second = router
            .route(Key {
                channel: 1,
                ..key(1, 3)
            })
            .unwrap();
        assert_ne!(second.sides.0, first.sides.0);
    }

    #[test]
    fn a_route_stays_until_an_end_moves_or_a_new_tank_stands_on_it() {
        let mut hulls = grid(4, 1, 0.5);
        let mut router = Router::default();
        router.update(&hulls);
        router.route(key(1, 4)).unwrap();
        router.route(key(2, 3)).unwrap();
        assert!(router.has(key(1, 4)) && router.has(key(2, 3)));
        // Another tank elsewhere, and a tank that grows, leave both as they are.
        hulls.insert(
            hull(9),
            Footprint {
                center: [40.0, 40.0],
                radius: 0.5,
                wall: 0.5,
            },
        );
        hulls.get_mut(&hull(2)).unwrap().wall = 0.9;
        router.update(&hulls);
        assert!(router.has(key(1, 4)) && router.has(key(2, 3)));
        // A tank standing on the path of the first forgets it, and only it.
        let node = router.routes[&key(1, 4)].nodes[router.routes[&key(1, 4)].nodes.len() / 2];
        hulls.insert(
            hull(10),
            Footprint {
                center: world_of(node),
                radius: 0.3,
                wall: 0.3,
            },
        );
        router.update(&hulls);
        assert!(!router.has(key(1, 4)));
        assert!(router.has(key(2, 3)));
        // An end that moves forgets its runs.
        hulls.get_mut(&hull(3)).unwrap().center = [50.0, 1.0];
        router.update(&hulls);
        assert!(!router.has(key(2, 3)));
        // And a tank that is gone leaves its runs unroutable, not wrongly routed.
        hulls.remove(&hull(4));
        router.update(&hulls);
        assert!(router.route(key(1, 4)).is_none());
    }

    #[test]
    fn runs_avoid_the_nodes_other_runs_use() {
        let hulls = grid(4, 3, 0.4);
        let mut router = Router::default();
        router.update(&hulls);
        let first = router.route(key(1, 4)).unwrap().nodes.clone();
        let second = router
            .route(Key {
                channel: 1,
                ..key(1, 4)
            })
            .unwrap()
            .nodes
            .clone();
        let shared = second.iter().filter(|node| first.contains(node)).count();
        assert!(
            shared <= 2 * 2,
            "{shared} nodes in common: only the stubs by the tanks may overlap"
        );
    }

    #[test]
    fn the_frame_budget_leaves_the_rest_for_later_frames() {
        let hulls = grid(8, 8, 0.4);
        let mut router = Router::default();
        router.update(&hulls);
        router.begin_frame();
        let found = (1..=ROUTES_PER_FRAME as u32 + 8)
            .filter(|&b| router.route(key(1, b + 1)).is_some())
            .count();
        assert_eq!(found, ROUTES_PER_FRAME);
        router.begin_frame();
        assert!(router.route(key(1, ROUTES_PER_FRAME as u32 + 9)).is_some());
    }

    #[test]
    fn a_walled_in_tank_gets_a_crossing_elbow_instead_of_no_run() {
        // Tank 1 inside a ring of tanks too close for any pipe to pass between them.
        let mut hulls = BTreeMap::new();
        hulls.insert(
            hull(1),
            Footprint {
                center: [0.0, 0.0],
                radius: 0.4,
                wall: 0.4,
            },
        );
        for k in 0..16 {
            let angle = k as f32 / 16.0 * std::f32::consts::TAU;
            hulls.insert(
                hull(10 + k),
                Footprint {
                    center: [2.2 * angle.cos(), 2.2 * angle.sin()],
                    radius: 0.9,
                    wall: 0.9,
                },
            );
        }
        hulls.insert(
            hull(2),
            Footprint {
                center: [9.0, 0.0],
                radius: 0.4,
                wall: 0.4,
            },
        );
        let mut router = Router::default();
        router.update(&hulls);
        assert!(router.route(key(1, 2)).unwrap().crossing);
        assert!(router.polyline(key(1, 2)).unwrap().len() >= 2);
    }

    #[test]
    fn a_stress_of_a_few_hundred_runs_routes_within_a_second() {
        let hulls = grid(16, 16, 0.9);
        let mut router = Router::default();
        router.update(&hulls);
        let started = std::time::Instant::now();
        for a in 1..=256_u32 {
            for step in [1, 17, 33] {
                if router.found >= ROUTES_PER_FRAME {
                    router.begin_frame();
                }
                router.route(key(a, (a + step - 1) % 256 + 1));
            }
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{:?} for {} routes",
            started.elapsed(),
            router.routes.len()
        );
    }
}
