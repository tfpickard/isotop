//! Arbor: the process tree as a great tree. The trunk is PID 1 and its branches are its
//! descendants, as thick as the memory of the subtree they carry (Leonardo's rule); the leaves are
//! threads, green when idle and red when busy, and the kernel threads grow as roots under a cut-away
//! slab of soil. Blossoms mark processes that just started, leaves fall when threads exit, and the
//! canopy sways in a wind set by CPU pressure.
//!
//! Layout is a pure function of the tree: every parent hands out phyllotaxis slots in the order it
//! first sees its children, and a slot fixes where a child leaves its parent, in which direction
//! and how long it is. Nothing about the geometry depends on CPU, memory or population.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::f32::consts::{PI, TAU};

use crate::geometry::{dot, normalize};
use crate::model::{Identity, Kind, Measured, Process, Snapshot, bounded, bytes};
use crate::palette::{mix, tint};
use crate::platform::{Shadow, Source};
use crate::render::strokes::{Width, lit};
use crate::render::{Color, Item, NONE, Point, SCALE, Stage, TourTarget, label_name, ring_bounds};
use crate::simulation::{FixedStep, Rng, hash_identity, smoothstep};
use crate::status::{missing_note, unreadable_note};
use crate::terminal::clean_text;
use crate::views::ViewModule;

/// Separates this view's draws of the identity hash from every other view's.
const SALT: u64 = 0x6172_626f_7231;

/// The trunk's radius in world units; every other branch is `R_TRUNK * sqrt(area / trunk area)`,
/// never thinner than `RADIUS_FLOOR`, so a twig stays visible.
const R_TRUNK: f32 = 1.6;
const RADIUS_FLOOR: f32 = 0.05;
/// The roots' radius where they leave the trunk, for all the kernel threads together.
const ROOT_COLLAR: f32 = 0.55 * R_TRUNK;

/// A branch of depth `d` is `TRUNK_LENGTH * LENGTH_RATIO^d` long; a root of depth `d` is
/// `ROOT_LENGTH * ROOT_RATIO^d`.
const TRUNK_LENGTH: f32 = 14.0;
const LENGTH_RATIO: f32 = 0.78;
const ROOT_LENGTH: f32 = 8.0;
const ROOT_RATIO: f32 = 0.72;
/// How far a child leaves its parent's axis: the trunk's children, then every deeper branch; and
/// the roots' children of the trunk, then every deeper root (measured from straight down).
const ELEVATION_TRUNK: f32 = 50.0 * PI / 180.0;
const ELEVATION: f32 = 40.0 * PI / 180.0;
const ELEVATION_ROOT_COLLAR: f32 = 60.0 * PI / 180.0;
const ELEVATION_ROOT: f32 = 38.0 * PI / 180.0;
/// The golden angle, which the azimuths of successive slots advance by.
const SLOT_ANGLE: f32 = 137.5 * PI / 180.0;
/// Children whose attachment points lie closer than this fraction of a branch share one step of
/// its radius, so a parent of hundreds of children is still a handful of tube pieces.
const JUNCTION_MERGE: f32 = 0.02;
/// The length of the radius ramp at a junction, as a fraction of the branch.
const RAMP: f32 = 0.05;
/// Longest stretch of a branch drawn as one tube piece, as a fraction of it.
const LONGEST_PIECE: f32 = 0.25;

/// At most this many leaves hang on one process, and in the whole scene.
const LEAVES_PER_TWIG: usize = 32;
const LEAF_CAP: usize = 6000;
/// A leaf's half length and half width in world units.
const LEAF_HALF_LENGTH: f32 = 0.6;
const LEAF_HALF_WIDTH: f32 = 0.3;
/// How far leaves spread round their branch, as `LEAF_SPREAD + LEAF_SPREAD_PER_LENGTH * length`.
const LEAF_SPREAD: f32 = 0.5;
const LEAF_SPREAD_PER_LENGTH: f32 = 0.1;

/// A leaf of an exited thread falls for `FALL_SECONDS`, rests on the soil for `REST_SECONDS` and
/// is gone; at most `FALLEN_CAP` of them are falling or resting at once.
const FALL_SECONDS: f32 = 4.0;
const REST_SECONDS: f32 = 30.0;
const FALLEN_CAP: usize = 400;
/// Leaves that fall at one sample from one process: all of an exited process's, and of a live
/// one, however many of its threads exited, only this many.
const EXIT_FALLS: usize = LEAVES_PER_TWIG;
const THREAD_FALLS: usize = 8;

/// A process that started less than this many seconds ago blossoms.
const BLOSSOM_SECONDS: f32 = 5.0;
const BLOSSOMS_PER_PROCESS: usize = 6;

/// The wind's fixed step, the most steps one frame takes, and the longest gap it counts.
const STEP: f32 = 0.05;
const MAX_STEPS: usize = 10;
const MAX_GAP: f32 = 0.5;
/// Each branch's damping ratio.
const DAMPING: f32 = 0.12;
/// How far a gust at full wind leans a branch, as a fraction of its length.
const LEAN: f32 = 0.09;
/// The wind blows along this heading, in radians from the +x axis.
const WIND_HEADING: f32 = 0.35;
/// Time constant of the wind's easing towards the pressure's reading, in seconds.
const WIND_EASING: f32 = 1.5;

/// The soil slab under the tree: half its width, and its depth.
const SOIL_HALF: f32 = 14.0;
const SOIL_DEPTH: f32 = 10.0;
/// The camera is fitted to a cylinder of this radius from the roots' depth to the crown's top.
const FIT_RADIUS: f32 = 16.0;
const FIT_TOP: f32 = 32.0;

/// Most processes the tour visits above ground, and most roots.
const TOUR_BRANCHES: usize = 16;
const TOUR_ROOTS: usize = 3;
/// Most heavy branches labelled besides the trunk and the roots.
const LABELLED_BRANCHES: usize = 6;

const BARK: Color = [200, 148, 136];
const PALE_BARK: Color = [214, 218, 238];
const ZOMBIE: Color = [128, 128, 128];
const ROOT_BARK: Color = [124, 140, 190];
const ROOT_GLOW: Color = [120, 160, 255];
const BLOSSOM: Color = [250, 206, 224];
const BLOSSOM_HALO: Color = [255, 170, 205];
const GRASS: Color = [44, 70, 50];
const GRASS_EDGE: Color = [74, 112, 76];
const SOIL: [Color; 4] = [[132, 98, 82], [116, 84, 72], [100, 74, 66], [86, 64, 60]];
const HIGHLIGHT: Color = [255, 236, 200];

/// What a node stands for: a process, or a parent isotop cannot read (a pale branch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Key {
    Process(Identity),
    Shadow(u32),
}

impl Key {
    fn pid(self) -> u32 {
        match self {
            Key::Process(id) => id.pid,
            Key::Shadow(pid) => pid,
        }
    }

    /// A start time to order siblings by: a shadow has none and comes first.
    fn start(self) -> u64 {
        match self {
            Key::Process(id) => id.start,
            Key::Shadow(_) => 0,
        }
    }

    /// A stream of this node's own, fixed by what it is.
    fn rng(self, salt: u64) -> Rng {
        // The identity here only seeds a stream; it names no pickable thing.
        Rng::for_identity(
            Identity {
                pid: self.pid(),
                start: self.start(),
            },
            salt,
        )
    }
}

/// Which side of the soil a branch grows on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Side {
    #[default]
    Crown,
    Root,
}

/// Where a child sits on its parent.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Seat {
    parent: Key,
    side: Side,
    slot: u32,
}

/// The slots every parent has handed out. A parent keeps a counter that only rises, so a child
/// that leaves does not free its slot for the next, and no other child moves.
#[derive(Default)]
struct Slots {
    seated: HashMap<Key, Seat>,
    counters: HashMap<(Key, Side), u32>,
}

impl Slots {
    /// The slots of `children` on `parent`, in the order given. A child already seated there keeps
    /// its slot; the others take the next ones in the order of their start times (then pids). A
    /// child seated under another parent or on another side is seated anew.
    fn seat(&mut self, parent: Key, side: Side, children: &[Key]) -> Vec<u32> {
        let mut fresh: Vec<Key> = children
            .iter()
            .copied()
            .filter(|child| {
                self.seated
                    .get(child)
                    .is_none_or(|seat| (seat.parent, seat.side) != (parent, side))
            })
            .collect();
        fresh.sort_by_key(|child| (child.start(), child.pid()));
        let counter = self.counters.entry((parent, side)).or_insert(0);
        for child in fresh {
            self.seated.insert(
                child,
                Seat {
                    parent,
                    side,
                    slot: *counter,
                },
            );
            *counter += 1;
        }
        children
            .iter()
            .map(|child| self.seated[child].slot)
            .collect()
    }

    /// Forgets what is no longer alive: seats, and the counters of parents that are gone.
    fn forget(&mut self, alive: &HashSet<Key>) {
        self.seated.retain(|key, _| alive.contains(key));
        self.counters.retain(|(key, _), _| alive.contains(key));
    }
}

/// Where along its parent a child in `slot` leaves it, as a fraction of the parent's length:
/// `0.35 + 0.6 * (1 - 0.8^(slot + 1))`, so successive slots crowd towards the tip.
fn attachment(slot: u32) -> f32 {
    0.35 + 0.6 * (1.0 - 0.8_f32.powi(slot.min(200) as i32 + 1))
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scaled(a: Point, factor: f32) -> Point {
    a.map(|value| value * factor)
}

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Two unit vectors perpendicular to `axis` and to each other, fixed by the axis alone.
fn frame(axis: Point) -> (Point, Point) {
    let reference = if axis[2].abs() < 0.95 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let first = normalize(cross(reference, axis));
    (first, cross(axis, first))
}

/// One process or pale branch of the tree, with the geometry it keeps while at rest.
#[derive(Clone, Debug, Default)]
struct Node {
    key: Option<Key>,
    /// Its index in the frame's process slice; None for a pale branch.
    process: Option<usize>,
    parent: Option<usize>,
    /// Ascending by side and then by slot.
    children: Vec<usize>,
    slot: u32,
    side: Side,
    depth: u32,
    base: Point,
    direction: Point,
    /// The vector the branch bends towards over its length: `point(s)` is
    /// `base + length * (direction * s + bend * s^2)`.
    bend: Point,
    length: f32,
    /// The phase of this branch's gusts.
    phase: f32,
}

impl Node {
    fn key(&self) -> Key {
        self.key.expect("a node has a key")
    }

    /// The point a fraction `s` of the way along the branch, at rest.
    fn point(&self, s: f32) -> Point {
        let along = add(scaled(self.direction, s), scaled(self.bend, s * s));
        add(self.base, scaled(along, self.length))
    }

    fn tangent(&self, s: f32) -> Point {
        normalize(add(self.direction, scaled(self.bend, 2.0 * s)))
    }
}

/// The tree of one sample: who hangs on whom, and where every branch stands at rest.
#[derive(Default)]
struct Tree {
    nodes: Vec<Node>,
    trunk: usize,
    /// Every node, parents before their children.
    order: Vec<usize>,
}

impl Tree {
    /// Builds the tree of `processes` and the pale branches of `shadows`, handing out slots from
    /// `slots`. The trunk is PID 1 (a process, or a shadow such as launchd on macOS); without it,
    /// the root of the subtree with the most nodes. A node whose parent is not drawn hangs on the
    /// trunk, and so does one in a cycle. The kernel processes that are top-level or direct
    /// children of the trunk, with their subtrees, are the roots.
    fn build(processes: &[&Process], shadows: &[Shadow], slots: &mut Slots) -> Tree {
        if processes.is_empty() {
            return Tree::default();
        }
        let mut nodes: Vec<Node> = Vec::with_capacity(processes.len() + shadows.len());
        let mut parent_pids: Vec<u32> = Vec::with_capacity(nodes.capacity());
        let mut by_pid: HashMap<u32, usize> = HashMap::new();
        for (index, process) in processes.iter().enumerate() {
            by_pid.insert(process.id.pid, nodes.len());
            nodes.push(Node {
                key: Some(Key::Process(process.id)),
                process: Some(index),
                ..Node::default()
            });
            parent_pids.push(process.parent);
        }
        for shadow in shadows {
            if by_pid.contains_key(&shadow.pid) {
                continue;
            }
            by_pid.insert(shadow.pid, nodes.len());
            nodes.push(Node {
                key: Some(Key::Shadow(shadow.pid)),
                ..Node::default()
            });
            parent_pids.push(shadow.parent);
        }
        let count = nodes.len();
        let raw: Vec<Option<usize>> = (0..count)
            .map(|index| {
                by_pid
                    .get(&parent_pids[index])
                    .copied()
                    .filter(|&parent| parent != index)
            })
            .collect();
        let trunk = by_pid
            .get(&1)
            .copied()
            .unwrap_or_else(|| largest_subtree(&raw, &nodes));
        let mut parent = raw;
        for (index, link) in parent.iter_mut().enumerate() {
            if index == trunk {
                *link = None;
            } else if link.is_none() {
                *link = Some(trunk);
            }
        }
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (index, link) in parent.iter().enumerate() {
            if let Some(link) = link {
                children[*link].push(index);
            }
        }
        let mut order = Vec::with_capacity(count);
        let mut seen = vec![false; count];
        walk(trunk, &children, &mut seen, &mut order);
        for index in 0..count {
            if !seen[index] {
                // A cycle: its first member in node order is taken to the trunk.
                if let Some(old) = parent[index] {
                    children[old].retain(|&child| child != index);
                }
                parent[index] = Some(trunk);
                children[trunk].push(index);
                walk(index, &children, &mut seen, &mut order);
            }
        }
        let kernel = |index: usize| {
            nodes[index]
                .process
                .is_some_and(|slot| processes[slot].kind == Kind::Kernel)
        };
        let mut sides = vec![Side::Crown; count];
        let mut depths = vec![0_u32; count];
        for &index in &order {
            if let Some(link) = parent[index] {
                sides[index] = if link == trunk && kernel(index) {
                    Side::Root
                } else {
                    sides[link]
                };
                depths[index] = depths[link] + 1;
            }
        }
        for index in 0..count {
            nodes[index].parent = parent[index];
            nodes[index].side = sides[index];
            nodes[index].depth = depths[index];
        }
        for index in 0..count {
            let mut seated = children[index].clone();
            for side in [Side::Crown, Side::Root] {
                let list: Vec<usize> = seated
                    .iter()
                    .copied()
                    .filter(|&child| nodes[child].side == side)
                    .collect();
                if list.is_empty() {
                    continue;
                }
                let keys: Vec<Key> = list.iter().map(|&child| nodes[child].key()).collect();
                let numbers = slots.seat(nodes[index].key(), side, &keys);
                for (&child, slot) in list.iter().zip(numbers) {
                    nodes[child].slot = slot;
                }
            }
            seated.sort_by_key(|&child| (nodes[child].side, nodes[child].slot));
            nodes[index].children = seated;
        }
        let mut tree = Tree {
            nodes,
            trunk,
            order,
        };
        tree.grow();
        tree
    }

    /// Fixes every branch's rest geometry from its slot, its parent and its own seed.
    fn grow(&mut self) {
        let order = self.order.clone();
        for &index in &order {
            let mut random = self.nodes[index].key().rng(SALT);
            let Some(parent) = self.nodes[index].parent else {
                let node = &mut self.nodes[index];
                node.base = [0.0; 3];
                node.direction = normalize([0.03, 0.015, 1.0]);
                node.length = TRUNK_LENGTH;
                node.phase = random.unit() * TAU;
                continue;
            };
            let (side, slot, depth) = {
                let node = &self.nodes[index];
                (node.side, node.slot, node.depth)
            };
            let parent_is_trunk = parent == self.trunk;
            let (base, axis, elevation) = match side {
                Side::Root if parent_is_trunk => {
                    ([0.0, 0.0, -0.05], [0.0, 0.0, -1.0], ELEVATION_ROOT_COLLAR)
                }
                Side::Root => {
                    let along = attachment(slot);
                    let parent = &self.nodes[parent];
                    (parent.point(along), parent.tangent(along), ELEVATION_ROOT)
                }
                Side::Crown => {
                    let along = attachment(slot);
                    let parent_node = &self.nodes[parent];
                    let elevation = if parent_is_trunk {
                        ELEVATION_TRUNK
                    } else {
                        ELEVATION
                    };
                    (
                        parent_node.point(along),
                        parent_node.tangent(along),
                        elevation,
                    )
                }
            };
            let (first, second) = frame(axis);
            let azimuth = slot as f32 * SLOT_ANGLE;
            let direction = normalize(add(
                scaled(axis, elevation.cos()),
                scaled(
                    add(scaled(first, azimuth.cos()), scaled(second, azimuth.sin())),
                    elevation.sin(),
                ),
            ));
            let toward = match side {
                Side::Crown => [0.0, 0.0, 1.0],
                Side::Root => [0.0, 0.0, -1.0],
            };
            let lean = sub(toward, scaled(direction, dot(direction, toward)));
            let lean = if dot(lean, lean) > 1e-6 {
                normalize(lean)
            } else {
                [0.0; 3]
            };
            let magnitude = 0.18 + 0.3 * random.unit();
            let heading = random.unit() * TAU;
            let wobble = 0.12 * random.unit();
            let (across, up) = frame(direction);
            let bend = add(
                scaled(lean, magnitude),
                scaled(
                    add(scaled(across, heading.cos()), scaled(up, heading.sin())),
                    wobble,
                ),
            );
            let length = match side {
                Side::Crown => TRUNK_LENGTH * LENGTH_RATIO.powi(depth as i32),
                Side::Root => ROOT_LENGTH * ROOT_RATIO.powi(depth as i32),
            };
            let node = &mut self.nodes[index];
            node.base = base;
            node.direction = direction;
            node.bend = bend;
            node.length = length;
            node.phase = random.unit() * TAU;
        }
    }

    /// The area of every node's subtree on its own side: the memory in bytes on the crown, the
    /// number of processes among the roots, which have no memory to speak of.
    fn weigh(&self, processes: &[&Process]) -> Weights {
        let count = self.nodes.len();
        let mut own = vec![0.0_f32; count];
        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(slot) = node.process {
                own[index] = match node.side {
                    Side::Crown => processes[slot].memory as f32,
                    Side::Root => 1.0,
                };
            }
        }
        let mut subtree = own.clone();
        for &index in self.order.iter().rev() {
            if let Some(parent) = self.nodes[index].parent
                && self.nodes[parent].side == self.nodes[index].side
            {
                subtree[parent] += subtree[index];
            }
        }
        let root_total = self.nodes[self.trunk]
            .children
            .iter()
            .filter(|&&child| self.nodes[child].side == Side::Root)
            .map(|&child| subtree[child])
            .sum();
        Weights {
            own,
            crown_total: subtree[self.trunk],
            root_total,
            subtree,
        }
    }

    /// The children that draw on `index`'s own side, in slot order: every child, except that the
    /// roots hang from the trunk's base instead of its length.
    fn same_side(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let side = self.nodes[index].side;
        self.nodes[index]
            .children
            .iter()
            .copied()
            .filter(move |&child| self.nodes[child].side == side)
    }

    /// The area a branch carries along its length, as the levels of its stretches from the base:
    /// `levels[0]` is the whole subtree, and each junction takes the area of the child that
    /// leaves there, down to the branch's own area at the tip. Leonardo's rule is the sum of
    /// the child areas and the level after their junction being the level before it.
    fn levels(&self, weights: &Weights, index: usize) -> Vec<f32> {
        let children: Vec<usize> = self.same_side(index).collect();
        let mut levels = vec![0.0; children.len() + 1];
        levels[children.len()] = weights.own[index];
        for step in (0..children.len()).rev() {
            levels[step] = levels[step + 1] + weights.subtree[children[step]];
        }
        levels
    }

    /// Where along the branch its radius changes: `(s, area)` pairs from the base to the tip,
    /// with a short ramp at each junction.
    fn stops(&self, weights: &Weights, index: usize) -> Vec<(f32, f32)> {
        let levels = self.levels(weights, index);
        let junctions: Vec<f32> = self
            .same_side(index)
            .map(|child| attachment(self.nodes[child].slot))
            .collect();
        let mut stops = vec![(0.0, levels[0])];
        let mut step = 0;
        while step < junctions.len() {
            let at = junctions[step];
            let mut end = step;
            while end < junctions.len() && junctions[end] - at < JUNCTION_MERGE {
                end += 1;
            }
            let next = junctions.get(end).copied().unwrap_or(1.0);
            let ramp = (at + RAMP).min((at + next) * 0.5);
            stops.push((at, levels[step]));
            stops.push((ramp, levels[end]));
            step = end;
        }
        stops.push((1.0, levels[junctions.len()]));
        let mut refined = vec![stops[0]];
        for pair in stops.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let pieces = ((to.0 - from.0) / LONGEST_PIECE).ceil().max(1.0) as usize;
            for piece in 1..pieces {
                refined.push((
                    from.0 + (to.0 - from.0) * piece as f32 / pieces as f32,
                    from.1,
                ));
            }
            refined.push(to);
        }
        refined
    }
}

/// Depth-first, parents before children; marks what it visits.
fn walk(start: usize, children: &[Vec<usize>], seen: &mut [bool], order: &mut Vec<usize>) {
    let mut stack = vec![start];
    seen[start] = true;
    while let Some(index) = stack.pop() {
        order.push(index);
        for &child in children[index].iter().rev() {
            if !seen[child] {
                seen[child] = true;
                stack.push(child);
            }
        }
    }
}

/// The top-level node whose tree holds the most nodes, ties to the lowest key.
fn largest_subtree(raw: &[Option<usize>], nodes: &[Node]) -> usize {
    let count = raw.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (index, parent) in raw.iter().enumerate() {
        if let Some(parent) = parent {
            children[*parent].push(index);
        }
    }
    let mut sizes = vec![1_usize; count];
    let mut seen = vec![false; count];
    let mut best: Option<(usize, usize)> = None;
    for top in (0..count).filter(|&index| raw[index].is_none()) {
        let mut order = Vec::new();
        walk(top, &children, &mut seen, &mut order);
        for &index in order.iter().rev() {
            if let Some(parent) = raw[index] {
                sizes[parent] += sizes[index];
            }
        }
        let better = best.is_none_or(|(held, size)| {
            (sizes[top], std::cmp::Reverse(nodes[top].key()))
                > (size, std::cmp::Reverse(nodes[held].key()))
        });
        if better {
            best = Some((top, sizes[top]));
        }
    }
    best.map_or(0, |(top, _)| top)
}

/// The areas of one frame's tree: each node's own, each subtree's, and the totals that scale
/// the radii.
struct Weights {
    own: Vec<f32>,
    subtree: Vec<f32>,
    crown_total: f32,
    root_total: f32,
}

impl Weights {
    /// Leonardo's rule: the radius is `R_TRUNK * sqrt(area / trunk area)` on the crown, and the
    /// same by the roots' collar and the number of kernel threads below the soil. Never under the
    /// floor.
    fn radius(&self, side: Side, area: f32) -> f32 {
        let (scale, total) = match side {
            Side::Crown => (R_TRUNK, self.crown_total),
            Side::Root => (ROOT_COLLAR, self.root_total),
        };
        if total <= 0.0 {
            return RADIUS_FLOOR;
        }
        (scale * (area / total).max(0.0).sqrt()).max(RADIUS_FLOOR)
    }
}

/// One leaf of a thread, or of a bucket of threads, as it hangs at rest.
#[derive(Clone, Copy, Debug)]
struct Leaf {
    /// The node of the process it hangs on, and that process's index in the slice.
    node: usize,
    owner: usize,
    key: (Identity, u64),
    s: f32,
    offset: Point,
    angle: f32,
    size: f32,
    shade: f32,
    /// CPU as a fraction of one core.
    cpu: f32,
}

/// A leaf's id among its process's: the thread's id, a bucket of threads' number, or the number
/// of a leaf drawn without thread data.
const BUCKET: u64 = 1 << 32;
const UNLISTED: u64 = 2 << 32;

/// The leaves a process hangs: `(id, cpu fraction)`, at most `cap`. Threads past the cap are
/// bucketed by tid, and a bucket's leaf shows its mean CPU. A process with no thread list draws
/// one leaf per thread, all shaded by `cpu / threads`.
fn leaf_specs(process: &Process, cap: usize) -> Vec<(u64, f32)> {
    let threads = process.threads.max(1) as f32;
    let each = process.cpu / threads / 100.0;
    match &process.thread_list {
        Measured::Known(list) if !list.threads.is_empty() => {
            let share = |cpu: Option<f32>| cpu.map_or(each, |cpu| cpu / 100.0);
            if list.threads.len() <= cap {
                list.threads
                    .iter()
                    .map(|thread| (u64::from(thread.tid), share(thread.cpu)))
                    .collect()
            } else {
                let mut sums = vec![(0.0_f32, 0_u32); cap];
                for thread in list.threads.iter() {
                    let bucket = &mut sums[thread.tid as usize % cap];
                    bucket.0 += share(thread.cpu);
                    bucket.1 += 1;
                }
                sums.iter()
                    .enumerate()
                    .filter(|(_, (_, members))| *members > 0)
                    .map(|(bucket, &(sum, members))| (BUCKET | bucket as u64, sum / members as f32))
                    .collect()
            }
        }
        _ => (0..(process.threads.max(1) as usize).min(cap))
            .map(|leaf| (UNLISTED | leaf as u64, each))
            .collect(),
    }
}

/// How many leaves a process would hang with no cap on them.
fn leaf_demand(process: &Process) -> usize {
    match &process.thread_list {
        Measured::Known(list) if !list.threads.is_empty() => list.threads.len(),
        _ => process.threads.max(1) as usize,
    }
}

/// The per-twig cap: `LEAVES_PER_TWIG`, lowered for every twig alike until the scene holds at
/// most `LEAF_CAP` leaves.
fn twig_cap(demands: impl Iterator<Item = usize> + Clone) -> usize {
    (1..=LEAVES_PER_TWIG)
        .rev()
        .find(|&cap| demands.clone().map(|demand| demand.min(cap)).sum::<usize>() <= LEAF_CAP)
        .unwrap_or(1)
}

/// The colour of a leaf that is using `cpu` of one core: green idle, through yellow, to red.
fn leaf_color(cpu: f32) -> Color {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [74.0, 150.0, 72.0]),
        (0.08, [104.0, 170.0, 70.0]),
        (0.3, [214.0, 196.0, 60.0]),
        (0.6, [236.0, 120.0, 44.0]),
        (1.0, [220.0, 52.0, 44.0]),
    ];
    let cpu = cpu.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let (low, high) = (pair[0], pair[1]);
        if cpu <= high.0 {
            let along = (cpu - low.0) / (high.0 - low.0);
            return [0, 1, 2].map(|k| (low.1[k] + (high.1[k] - low.1[k]) * along) as u8);
        }
    }
    STOPS[STOPS.len() - 1].1.map(|v| v as u8)
}

/// Where a leaf of a process hangs and how it looks, fixed by the process and the leaf's id.
fn place_leaf(node: &Node, process: &Process, id: u64) -> (f32, Point, f32, f32, f32) {
    let mut random = Rng::for_identity(process.id, SALT ^ id);
    let s = 1.0 - 0.5 * random.unit().powf(1.6);
    let (first, second) = frame(node.tangent(s));
    let spread = LEAF_SPREAD + LEAF_SPREAD_PER_LENGTH * node.length;
    let offset = add(
        scaled(
            add(
                scaled(first, random.normal()),
                scaled(second, random.normal()),
            ),
            spread,
        ),
        [0.0, 0.0, random.normal().abs() * 0.12],
    );
    let angle = random.unit() * PI;
    let size = 0.8 + 0.4 * random.unit();
    let shade = 0.75 + 0.4 * random.unit();
    (s, offset, angle, size, shade)
}

/// What a leaf looked like where it hung, for the sample after to let it fall from.
#[derive(Clone, Copy, Debug)]
struct Memo {
    position: Point,
    color: Color,
}

/// A leaf that fell: where it started, when, and what it looked like.
#[derive(Clone, Copy, Debug)]
struct Fall {
    from: Point,
    color: Color,
    start: f32,
    phase: f32,
    spin: f32,
    size: f32,
}

impl Fall {
    /// Where the leaf is `age` seconds after it left, and how much of its width faces up: None
    /// before it leaves and after it is gone. It drifts downwind and flutters on the way down,
    /// then lies where it landed, which is on the soil.
    fn pose(&self, age: f32) -> Option<(Point, f32)> {
        if !(0.0..FALL_SECONDS + REST_SECONDS).contains(&age) {
            return None;
        }
        let fall = (age / FALL_SECONDS).min(1.0);
        let drift = 2.2 * fall.powf(0.9) * (0.6 + 0.8 * self.phase / TAU);
        let flutter = 0.5 * (2.3 * age + self.phase).sin() * (1.0 - fall);
        let (sine, cosine) = WIND_HEADING.sin_cos();
        let x = self.from[0] + drift * cosine - flutter * sine;
        let y = self.from[1] + drift * sine + flutter * cosine;
        let limit = SOIL_HALF - 0.6;
        let height = (self.from[2] * (1.0 - fall.powf(1.6))).max(0.04);
        let flat = if age < FALL_SECONDS {
            (self.spin * age).cos().abs()
        } else {
            1.0
        };
        Some((
            [x.clamp(-limit, limit), y.clamp(-limit, limit), height],
            flat,
        ))
    }
}

/// A branch's sway: a displacement in the ground plane and its velocity.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Oscillator {
    position: [f32; 2],
    velocity: [f32; 2],
}

/// What drives a branch's sway at one step.
#[derive(Clone, Copy, Debug, Default)]
struct Drive {
    /// Angular frequency in radians a second.
    omega: f32,
    /// The lean at full wind, in world units.
    amplitude: f32,
    phase: f32,
}

impl Oscillator {
    /// One step of `STEP` seconds of a damped oscillator pulled towards the gust's lean:
    /// `x'' = w^2 (target - x) - 2 zeta w x'`, with semi-implicit Euler, stable while `w * STEP`
    /// stays under 2 (the top frequency, 3 Hz, gives 0.94).
    fn step(&mut self, drive: &Drive, wind: f32, time: f32) {
        let gust = gust(time, drive.phase);
        for (axis, push) in gust.into_iter().enumerate() {
            let target = wind * drive.amplitude * push;
            let pull = drive.omega * drive.omega * (target - self.position[axis])
                - 2.0 * DAMPING * drive.omega * self.velocity[axis];
            self.velocity[axis] += pull * STEP;
            self.position[axis] += self.velocity[axis] * STEP;
        }
    }
}

/// The wind's push on a branch at `time`, as a fraction of its full lean: a steady push along the
/// wind's heading with two gusts of unrelated periods along it and across it.
fn gust(time: f32, phase: f32) -> [f32; 2] {
    let along = 0.55 + 0.3 * (0.55 * time + phase).sin() + 0.15 * (1.37 * time + 2.1 * phase).sin();
    let across = 0.3 * (0.83 * time + 1.3 * phase).sin() + 0.12 * (1.9 * time + phase).sin();
    let (sine, cosine) = WIND_HEADING.sin_cos();
    [
        along * cosine - across * sine,
        along * sine + across * cosine,
    ]
}

/// A branch's resonance: `f = clamp(0.6 * sqrt(r / 0.18) / sqrt(L / 6), 0.15, 3)` Hz, higher for
/// thicker branches and lower for longer ones.
fn natural_frequency(radius: f32, length: f32) -> f32 {
    (0.6 * (radius / 0.18).sqrt() / (length / 6.0).sqrt()).clamp(0.15, 3.0)
}

/// The wind's fixed-step clock with this view's constants.
struct WindClock(FixedStep);

impl Default for WindClock {
    fn default() -> Self {
        Self(FixedStep::new(STEP, MAX_STEPS, MAX_GAP))
    }
}

/// The wind and every branch's swing in it.
#[derive(Default)]
struct Sway {
    clock: WindClock,
    /// Steps taken so far, which is the wind's own time.
    steps: u64,
    /// The wind, eased towards the pressure's reading, from 0 to 1.
    wind: f32,
    keys: Vec<Key>,
    motion: Vec<Oscillator>,
    drive: Vec<Drive>,
}

impl Sway {
    /// Gives each node of the new tree the swing its key had, or none.
    fn reindex(&mut self, tree: &Tree) {
        let old: HashMap<Key, Oscillator> = self
            .keys
            .iter()
            .copied()
            .zip(self.motion.iter().copied())
            .collect();
        self.keys = tree.nodes.iter().map(Node::key).collect();
        self.motion = self
            .keys
            .iter()
            .map(|key| old.get(key).copied().unwrap_or_default())
            .collect();
        self.drive = vec![Drive::default(); self.keys.len()];
    }

    /// Takes the steps due at `time` towards `target` wind; roots do not sway.
    fn advance(&mut self, tree: &Tree, weights: &Weights, time: f32, target: f32) {
        for (index, node) in tree.nodes.iter().enumerate() {
            let radius = weights.radius(node.side, weights.subtree[index]);
            self.drive[index] = Drive {
                omega: TAU * natural_frequency(radius, node.length),
                amplitude: if node.side == Side::Crown {
                    LEAN * node.length
                } else {
                    0.0
                },
                phase: node.phase,
            };
        }
        let Self {
            clock,
            steps,
            wind,
            motion,
            drive,
            ..
        } = self;
        clock.0.advance(time, |_| {
            let at = *steps as f32 * STEP;
            *wind += (target - *wind) * (1.0 - (-STEP / WIND_EASING).exp());
            for (state, drive) in motion.iter_mut().zip(drive.iter()) {
                state.step(drive, *wind, at);
            }
            *steps += 1;
        });
    }

    /// Each node's displacement at its base: its parent's displacement where it leaves it.
    fn bases(&self, tree: &Tree) -> Vec<[f32; 2]> {
        let mut bases = vec![[0.0; 2]; tree.nodes.len()];
        for &index in &tree.order {
            let node = &tree.nodes[index];
            let Some(parent) = node.parent else { continue };
            if node.side == Side::Root {
                continue;
            }
            let at = attachment(node.slot);
            let weight = at * at;
            let pushed = self.motion[parent].position;
            bases[index] = [
                bases[parent][0] + pushed[0] * weight,
                bases[parent][1] + pushed[1] * weight,
            ];
        }
        bases
    }
}

/// What the view remembers between frames.
#[derive(Default)]
pub struct Arbor {
    slots: Slots,
    tree: Tree,
    signature: Option<u64>,
    leaves: Vec<Leaf>,
    /// Every leaf of the last sample, by its process and id, and the leaf each listed thread
    /// hung on.
    memo: HashMap<(Identity, u64), Memo>,
    listed: HashMap<Identity, HashMap<u32, u64>>,
    falls: Vec<Fall>,
    sway: Sway,
    /// Pale branches drawn and whether the trunk is PID 1, for the legend.
    pale: usize,
    init_trunk: bool,
}

/// What a sample's tree was built from: the sample, the processes in view with their thread
/// counts and states, and the shadows.
fn signature(processes: &[&Process], snapshot: &Snapshot, shadows: usize) -> u64 {
    processes.iter().fold(
        snapshot.elapsed.to_bits() ^ shadows as u64,
        |hash, process| {
            let listed = match &process.thread_list {
                Measured::Known(list) => list.threads.len() as u64 + 1,
                Measured::Pending | Measured::Unreadable => 0,
            };
            hash.rotate_left(7)
                ^ hash_identity(process.id, SALT)
                ^ (u64::from(process.threads) << 40)
                ^ (listed << 20)
                ^ u64::from(process.state)
        },
    )
}

impl Arbor {
    /// Builds the tree, the leaves and the leaves that fell, when the sample or the processes in
    /// view changed.
    fn observe(&mut self, time: f32, focus: bool, processes: &[&Process], snapshot: &Snapshot) {
        let shadows: &[Shadow] = if focus { &[] } else { &snapshot.shadows };
        let signature = signature(processes, snapshot, shadows.len());
        if self.signature == Some(signature) {
            return;
        }
        self.signature = Some(signature);
        let alive: HashSet<Key> = snapshot
            .processes
            .iter()
            .map(|process| Key::Process(process.id))
            .chain(shadows.iter().map(|shadow| Key::Shadow(shadow.pid)))
            .collect();
        self.slots.forget(&alive);
        self.tree = Tree::build(processes, shadows, &mut self.slots);
        self.sway.reindex(&self.tree);
        self.pale = self
            .tree
            .nodes
            .iter()
            .filter(|node| node.process.is_none())
            .count();
        self.init_trunk = self
            .tree
            .nodes
            .get(self.tree.trunk)
            .is_some_and(|node| node.key().pid() == 1);
        let present: HashSet<Identity> = snapshot
            .processes
            .iter()
            .map(|process| process.id)
            .collect();
        self.hang_leaves(time, processes, &present);
    }

    /// Hangs this sample's leaves and lets those of the last sample that are gone fall.
    fn hang_leaves(&mut self, time: f32, processes: &[&Process], present: &HashSet<Identity>) {
        let eligible = |node: &Node| {
            node.side == Side::Crown
                && node
                    .process
                    .is_some_and(|slot| processes[slot].state != 'Z')
        };
        let cap = twig_cap(
            self.tree
                .nodes
                .iter()
                .filter(|node| eligible(node))
                .filter_map(|node| node.process)
                .map(|slot| leaf_demand(processes[slot])),
        );
        let mut leaves = Vec::new();
        let mut memo = HashMap::new();
        let mut listed: HashMap<Identity, HashMap<u32, u64>> = HashMap::new();
        for (index, node) in self.tree.nodes.iter().enumerate() {
            if !eligible(node) {
                continue;
            }
            let slot = node.process.expect("an eligible node is a process");
            let process = processes[slot];
            let specs = leaf_specs(process, cap);
            for &(id, cpu) in &specs {
                let (s, offset, angle, size, shade) = place_leaf(node, process, id);
                let leaf = Leaf {
                    node: index,
                    owner: slot,
                    key: (process.id, id),
                    s,
                    offset,
                    angle,
                    size,
                    shade,
                    cpu,
                };
                memo.insert(
                    leaf.key,
                    Memo {
                        position: add(node.point(s), offset),
                        color: leaf_color(cpu),
                    },
                );
                leaves.push(leaf);
            }
            if let Measured::Known(list) = &process.thread_list
                && !list.threads.is_empty()
            {
                let bucketed = list.threads.len() > cap;
                listed.insert(
                    process.id,
                    list.threads
                        .iter()
                        .map(|thread| {
                            let id = if bucketed {
                                BUCKET | (u64::from(thread.tid) % cap as u64)
                            } else {
                                u64::from(thread.tid)
                            };
                            (thread.tid, id)
                        })
                        .collect(),
                );
            }
        }
        let old_memo = std::mem::replace(&mut self.memo, memo);
        let old_listed = std::mem::replace(&mut self.listed, listed);
        self.leaves = leaves;
        let zombies: HashSet<Identity> = processes
            .iter()
            .filter(|process| process.state == 'Z')
            .map(|process| process.id)
            .collect();
        let hung: HashSet<Identity> = self.memo.keys().map(|&(id, _)| id).collect();
        let mut by_process: BTreeMap<Identity, Vec<u64>> = BTreeMap::new();
        for &(id, leaf) in old_memo.keys() {
            by_process.entry(id).or_default().push(leaf);
        }
        let mut fallen = Vec::new();
        for (id, mut ids) in by_process {
            ids.sort_unstable();
            let (falling, most) = if !present.contains(&id) || zombies.contains(&id) {
                // The process exited or was left a zombie, and every thread with it.
                (ids, EXIT_FALLS)
            } else if let (Some(before), Some(after)) = (old_listed.get(&id), self.listed.get(&id))
            {
                // Each listed thread that is gone drops a leaf from where it hung.
                let mut gone: Vec<u32> = before
                    .keys()
                    .filter(|tid| !after.contains_key(tid))
                    .copied()
                    .collect();
                gone.sort_unstable();
                (gone.iter().map(|tid| before[tid]).collect(), THREAD_FALLS)
            } else if hung.contains(&id)
                && !old_listed.contains_key(&id)
                && !self.listed.contains_key(&id)
            {
                // Without thread lists the leaves are numbered, and the highest go first.
                let gone = ids
                    .into_iter()
                    .filter(|&leaf| !self.memo.contains_key(&(id, leaf)))
                    .collect();
                (gone, THREAD_FALLS)
            } else {
                (Vec::new(), 0)
            };
            for leaf in falling.into_iter().take(most) {
                let Some(memo) = old_memo.get(&(id, leaf)) else {
                    continue;
                };
                let mut random = Rng::for_identity(id, SALT ^ leaf ^ u64::from(time.to_bits()));
                fallen.push(Fall {
                    from: memo.position,
                    color: memo.color,
                    start: time,
                    phase: random.unit() * TAU,
                    spin: 2.0 + 3.0 * random.unit(),
                    size: 0.8 + 0.4 * random.unit(),
                });
            }
        }
        self.falls.extend(fallen);
        let over = self.falls.len().saturating_sub(FALLEN_CAP);
        self.falls.drain(..over);
    }
}

/// A branch's place in one frame: its rest point and how far the wind has moved it.
struct Pose<'a> {
    tree: &'a Tree,
    bases: &'a [[f32; 2]],
    motion: &'a [Oscillator],
}

impl Pose<'_> {
    /// How far the wind has moved the point a fraction `s` along branch `index`: the branch's
    /// own swing grows with the square of the distance from its base, on top of what its parent
    /// carried it by.
    fn sway(&self, index: usize, s: f32) -> [f32; 2] {
        let swing = self.motion[index].position;
        [
            self.bases[index][0] + swing[0] * s * s,
            self.bases[index][1] + swing[1] * s * s,
        ]
    }

    /// The point `s` along branch `index`, with `offset` from the branch there, as the wind has it.
    fn point(&self, index: usize, s: f32, offset: Point) -> Point {
        let rest = add(self.tree.nodes[index].point(s), offset);
        let moved = self.sway(index, s);
        [rest[0] + moved[0], rest[1] + moved[1], rest[2]]
    }
}

/// How far from `point`, along the unit vector `viewer`, the soil slab's surface is: moving a
/// point that far toward the viewer puts it just in front of the slab without changing where it
/// appears.
fn surface_distance(point: Point, viewer: Point) -> f32 {
    let mut distance = f32::INFINITY;
    for (axis, (low, high)) in [
        (-SOIL_HALF, SOIL_HALF),
        (-SOIL_HALF, SOIL_HALF),
        (-SOIL_DEPTH, 0.0),
    ]
    .into_iter()
    .enumerate()
    {
        let toward = viewer[axis];
        if toward > 1e-4 {
            distance = distance.min((high - point[axis]) / toward);
        } else if toward < -1e-4 {
            distance = distance.min((low - point[axis]) / toward);
        }
    }
    if distance.is_finite() {
        distance.max(0.0)
    } else {
        0.0
    }
}

/// A leaf as two triangles facing the screen at `center` (x, y, depth), `along` and `across`
/// pixels from its middle to its tip and its side, turned by `angle`.
fn leaf_triangles(center: Point, angle: f32, along: f32, across: f32) -> [[Point; 3]; 2] {
    let (sine, cosine) = angle.sin_cos();
    let at = |forward: f32, side: f32| {
        [
            center[0] + forward * cosine - side * sine,
            center[1] + forward * sine + side * cosine,
            center[2],
        ]
    };
    let (tip, tail) = (at(along, 0.0), at(-along, 0.0));
    let (left, right) = (at(0.0, across), at(0.0, -across));
    [[tip, left, right], [tail, left, right]]
}

fn draw_soil(stage: &mut Stage) {
    let (camera, viewer) = (stage.camera, stage.camera.viewer());
    let cells = 6;
    let step = 2.0 * SOIL_HALF / cells as f32;
    for row in 0..cells {
        for column in 0..cells {
            let (x, y) = (
                -SOIL_HALF + column as f32 * step,
                -SOIL_HALF + row as f32 * step,
            );
            let color = tint(GRASS, if (row + column) % 2 == 0 { 1.0 } else { 1.08 });
            stage.frame.quad(
                camera,
                [
                    [x, y, 0.0],
                    [x + step, y, 0.0],
                    [x + step, y + step, 0.0],
                    [x, y + step, 0.0],
                ],
                lit(color, [0.0, 0.0, 1.0]),
                NONE,
            );
        }
    }
    let heights = [0.0, -0.5, -3.2, -5.8, -8.0, -SOIL_DEPTH];
    let corners = [
        [-SOIL_HALF, -SOIL_HALF],
        [SOIL_HALF, -SOIL_HALF],
        [SOIL_HALF, SOIL_HALF],
        [-SOIL_HALF, SOIL_HALF],
    ];
    for side in 0..4 {
        let (from, to) = (corners[side], corners[(side + 1) % 4]);
        let outward = normalize([(from[0] + to[0]) * 0.5, (from[1] + to[1]) * 0.5, 0.0]);
        let outward = [outward[0].round(), outward[1].round(), 0.0];
        if dot(outward, viewer) <= 0.02 {
            continue;
        }
        for column in 0..cells {
            let at = |t: f32| {
                [
                    from[0] + (to[0] - from[0]) * t,
                    from[1] + (to[1] - from[1]) * t,
                ]
            };
            let (a, b) = (
                at(column as f32 / cells as f32),
                at((column + 1) as f32 / cells as f32),
            );
            for band in 0..heights.len() - 1 {
                let base = if band == 0 {
                    GRASS_EDGE
                } else {
                    SOIL[band - 1]
                };
                let jitter = hash_identity(
                    Identity {
                        pid: (side * 64 + column) as u32,
                        start: band as u64,
                    },
                    SALT,
                ) % 100;
                let color = lit(tint(base, 0.94 + 0.12 * jitter as f32 / 100.0), outward);
                let (top, bottom) = (heights[band], heights[band + 1]);
                stage.frame.quad(
                    camera,
                    [
                        [a[0], a[1], top],
                        [b[0], b[1], top],
                        [b[0], b[1], bottom],
                        [a[0], a[1], bottom],
                    ],
                    color,
                    NONE,
                );
            }
        }
    }
}

/// What a frame's drawing needs besides the stage: the processes, areas and wind.
struct Drawing<'a> {
    processes: &'a [&'a Process],
    weights: &'a Weights,
    pose: &'a Pose<'a>,
}

impl Arbor {
    /// Whether the app highlights `process`: selected, hovered or toured.
    fn lit(stage: &Stage, process: Option<&Process>) -> bool {
        process.is_some_and(|process| stage.highlight.contains(&process.id))
    }

    fn paint_branches(&self, stage: &mut Stage, scene: &Drawing) {
        let Drawing {
            processes,
            weights,
            pose,
        } = scene;
        let (camera, viewer) = (stage.camera, stage.camera.viewer());
        let zoom = camera.zoom * SCALE;
        for (index, node) in self.tree.nodes.iter().enumerate() {
            let process = node.process.map(|slot| processes[slot]);
            let pick = node.process.map_or(NONE, |slot| slot as u32);
            let stops = self.tree.stops(weights, index);
            let radii: Vec<f32> = stops
                .iter()
                .map(|&(_, area)| weights.radius(node.side, area))
                .collect();
            let hot = Self::lit(stage, process);
            match node.side {
                Side::Crown => {
                    let points: Vec<Point> = stops
                        .iter()
                        .map(|&(s, _)| pose.point(index, s, [0.0; 3]))
                        .collect();
                    let base = match process {
                        None => PALE_BARK,
                        Some(process) if process.state == 'Z' && node.children.is_empty() => ZOMBIE,
                        Some(_) => BARK,
                    };
                    let color = if hot {
                        mix(base, HIGHLIGHT, 0.45)
                    } else {
                        base
                    };
                    let widest = radii.iter().copied().fold(0.0, f32::max) * zoom;
                    if widest >= 2.0 {
                        let sides = if widest > 14.0 {
                            8
                        } else if widest > 5.0 {
                            6
                        } else {
                            4
                        };
                        stage
                            .frame
                            .tube(camera, &points, &radii, color, pick, sides, true);
                    } else {
                        stage.frame.ribbon(
                            camera,
                            &points,
                            Width::Pixels((2.0 * widest).max(1.2)),
                            lit(color, viewer),
                            pick,
                            None,
                        );
                    }
                }
                Side::Root => {
                    let cpu = process.map_or(0.0, |process| process.cpu);
                    let glow = bounded(cpu, 6.0);
                    let base = mix(ROOT_BARK, [210, 230, 255], 0.55 * glow);
                    let color = if hot {
                        mix(base, HIGHLIGHT, 0.45)
                    } else {
                        base
                    };
                    let limit = SOIL_HALF - 0.4;
                    let points: Vec<Point> = stops
                        .iter()
                        .map(|&(s, _)| {
                            let at = pose.point(index, s, [0.0; 3]);
                            let at = [
                                at[0].clamp(-limit, limit),
                                at[1].clamp(-limit, limit),
                                at[2].clamp(-(SOIL_DEPTH - 0.5), -0.05),
                            ];
                            let moved =
                                surface_distance(at, viewer) + 0.3 + 0.003 * dot(at, viewer);
                            add(at, scaled(viewer, moved))
                        })
                        .collect();
                    for (pair, radius) in points.windows(2).zip(radii.windows(2)) {
                        let width = (radius[0] + radius[1]) * zoom;
                        stage.frame.ribbon(
                            camera,
                            pair,
                            Width::Pixels(width.max(1.2)),
                            color,
                            pick,
                            None,
                        );
                    }
                    if node.children.is_empty() {
                        let tip = points[points.len() - 1];
                        let [x, y, _] = stage.frame.project(camera, tip);
                        stage.frame.items.push(Item::Puff {
                            center: [x, y],
                            radius: (0.55 * zoom).clamp(3.0, 9.0),
                            color: ROOT_GLOW,
                            strength: 0.2 + 0.5 * glow,
                        });
                    }
                }
            }
        }
    }

    fn paint_leaves(&self, stage: &mut Stage, scene: &Drawing) {
        let Drawing {
            processes, pose, ..
        } = scene;
        let camera = stage.camera;
        let zoom = camera.zoom * SCALE;
        let (width, height) = (stage.frame.width as f32, stage.frame.height as f32);
        for leaf in &self.leaves {
            let process = processes[leaf.owner];
            let at = pose.point(leaf.node, leaf.s, leaf.offset);
            let [x, y, depth] = stage.frame.project(camera, at);
            let reach = LEAF_HALF_LENGTH * leaf.size * zoom * 1.5;
            if x < -reach || x > width + reach || y < -reach || y > height + reach {
                continue;
            }
            let outward = normalize(sub(at, [0.0, 0.0, TRUNK_LENGTH * 0.9]));
            let mut color = tint(leaf_color(leaf.cpu), leaf.shade * 1.3);
            if Self::lit(stage, Some(process)) {
                color = mix(color, HIGHLIGHT, 0.3);
            }
            let color = lit(color, outward);
            for triangle in leaf_triangles(
                [x, y, depth],
                leaf.angle,
                (LEAF_HALF_LENGTH * leaf.size * zoom).max(1.5),
                (LEAF_HALF_WIDTH * leaf.size * zoom).max(0.8),
            ) {
                stage
                    .frame
                    .items
                    .push(Item::Triangle(triangle, color, leaf.owner as u32));
            }
        }
    }

    fn paint_falls(&self, stage: &mut Stage) {
        let camera = stage.camera;
        let zoom = camera.zoom * SCALE;
        for fall in &self.falls {
            let age = stage.time - fall.start;
            let Some((at, flat)) = fall.pose(age) else {
                continue;
            };
            let [x, y, depth] = stage.frame.project(camera, at);
            let dim = if age < FALL_SECONDS { 0.9 } else { 0.62 };
            let color = tint(fall.color, dim);
            for triangle in leaf_triangles(
                [x, y, depth],
                fall.spin * 0.5 + fall.phase,
                (LEAF_HALF_LENGTH * fall.size * zoom).max(1.5),
                (LEAF_HALF_WIDTH * fall.size * zoom * (0.3 + 0.7 * flat)).max(0.6),
            ) {
                stage
                    .frame
                    .items
                    .push(Item::Triangle(triangle, color, NONE));
            }
        }
    }

    /// Blossoms at the twigs of processes the scene first saw less than `BLOSSOM_SECONDS` ago.
    fn paint_blossoms(&self, stage: &mut Stage, scene: &Drawing) {
        let Drawing {
            processes, pose, ..
        } = scene;
        let (camera, zoom) = (stage.camera, stage.camera.zoom * SCALE);
        for (index, node) in self.tree.nodes.iter().enumerate() {
            let Some(slot) = node.process else { continue };
            let process = processes[slot];
            let Some(age) = stage.age(process.id) else {
                continue;
            };
            if age >= BLOSSOM_SECONDS || node.side == Side::Root {
                continue;
            }
            let open = smoothstep(age / 0.7) * smoothstep((BLOSSOM_SECONDS - age) / 0.8);
            if open < 0.05 {
                continue;
            }
            let mut random = Rng::for_identity(process.id, SALT ^ 0xb105);
            let (first, second) = frame(node.tangent(1.0));
            for _ in 0..BLOSSOMS_PER_PROCESS {
                let offset = add(
                    scaled(
                        add(
                            scaled(first, random.normal()),
                            scaled(second, random.normal()),
                        ),
                        0.22,
                    ),
                    scaled(node.tangent(1.0), -0.5 * random.unit()),
                );
                let size = open * (0.8 + 0.35 * random.unit());
                let at = pose.point(index, 1.0, offset);
                let selected = stage.selected == Some(process.id);
                stage
                    .frame
                    .sphere(camera, at, 0.26 * size, BLOSSOM, slot as u32, selected);
                let [x, y, _] = stage.frame.project(camera, at);
                stage.frame.items.push(Item::Puff {
                    center: [x, y],
                    radius: (0.55 * size * zoom).max(3.0),
                    color: BLOSSOM_HALO,
                    strength: 0.32 * open,
                });
            }
        }
    }

    /// Tells the app where every process is and what the view knows of it.
    fn report(&self, stage: &mut Stage, scene: &Drawing) {
        let Drawing {
            processes,
            weights,
            pose,
        } = scene;
        let tree = &self.tree;
        let tip = |index: usize| pose.point(index, 1.0, [0.0; 3]);
        let mut crown: Vec<usize> = Vec::new();
        let mut roots: Vec<usize> = Vec::new();
        for (index, node) in tree.nodes.iter().enumerate() {
            let Some(slot) = node.process else { continue };
            stage.positions.insert(processes[slot].id, tip(index));
            match node.side {
                Side::Crown => crown.push(index),
                Side::Root => roots.push(index),
            }
        }
        let heavier = |a: &usize, b: &usize| {
            weights.subtree[*b]
                .total_cmp(&weights.subtree[*a])
                .then(tree.nodes[*a].key().cmp(&tree.nodes[*b].key()))
        };
        crown.sort_by(heavier);
        roots.sort_by(heavier);
        let name = |index: usize| {
            let slot = tree.nodes[index].process.expect("a process");
            clean_text(&label_name(&processes[slot].name), 15)
        };
        for &index in crown.iter().take(TOUR_BRANCHES) {
            let slot = tree.nodes[index].process.expect("a process");
            stage.tour.push(TourTarget {
                id: processes[slot].id,
                position: tip(index),
                weight: weights.subtree[index],
            });
        }
        for &index in roots.iter().take(TOUR_ROOTS) {
            let slot = tree.nodes[index].process.expect("a process");
            stage.tour.push(TourTarget {
                id: processes[slot].id,
                position: tip(index),
                weight: weights.subtree[index],
            });
        }
        // Labels: the trunk, the roots below the soil, and the heaviest branches.
        if tree.nodes[tree.trunk].process.is_some() {
            stage.places.push((
                pose.point(tree.trunk, 0.3, [0.0; 3]),
                format!("{} {}", name(tree.trunk), bytes(weights.crown_total as u64)),
            ));
        }
        for &index in roots
            .iter()
            .filter(|&&index| tree.nodes[index].parent == Some(tree.trunk))
            .take(4)
        {
            stage.places.push((
                pose.point(index, 0.6, [0.0; 3]),
                format!("{} {} kernel threads", name(index), weights.subtree[index]),
            ));
        }
        for &index in crown
            .iter()
            .filter(|&&index| index != tree.trunk)
            .take(LABELLED_BRANCHES)
        {
            stage.places.push((
                pose.point(index, 0.55, [0.0; 3]),
                format!("{} {}", name(index), bytes(weights.subtree[index] as u64)),
            ));
        }
        for (index, node) in tree.nodes.iter().enumerate() {
            let Some(slot) = node.process else { continue };
            let process = processes[slot];
            if !stage.highlight.contains(&process.id) {
                continue;
            }
            let notes = self.notes(index, scene);
            let leaves = self.leaves.iter().filter(|leaf| leaf.owner == slot).count();
            stage.notes.insert(process.id, notes);
            if stage.toured == Some(process.id) {
                let line = match node.side {
                    Side::Crown => format!(
                        "{} on this branch, {leaves} leaves",
                        bytes(weights.subtree[index] as u64)
                    ),
                    Side::Root => {
                        format!("{} kernel threads below the soil", weights.subtree[index])
                    }
                };
                stage.callout.push((process.id, line));
            }
        }
    }

    /// The inspector lines of one process: what hangs on it, its leaves, its slot and its three
    /// busiest threads.
    fn notes(&self, index: usize, scene: &Drawing) -> Vec<String> {
        let Drawing {
            processes, weights, ..
        } = scene;
        let tree = &self.tree;
        let node = &tree.nodes[index];
        let process = processes[node.process.expect("a process")];
        let (mut count, mut threads) = (0_usize, 0_u64);
        let mut stack = vec![index];
        while let Some(at) = stack.pop() {
            if let Some(slot) = tree.nodes[at].process {
                count += 1;
                threads += u64::from(processes[slot].threads);
            }
            stack.extend(tree.same_side(at));
        }
        let mut lines = Vec::new();
        lines.push(match node.side {
            Side::Crown => format!(
                "subtree {}: {count} processes, {threads} threads",
                bytes(weights.subtree[index] as u64)
            ),
            Side::Root => format!("{count} kernel threads in this root"),
        });
        let leaves = self
            .leaves
            .iter()
            .filter(|leaf| leaf.owner == node.process.expect("a process"))
            .count();
        lines.push(format!("{} threads, {leaves} leaves", process.threads));
        lines.push(format!("slot {}", node.slot));
        match &process.thread_list {
            Measured::Known(list) => {
                let mut busy: Vec<_> = list
                    .threads
                    .iter()
                    .filter_map(|thread| Some((thread.cpu?, thread)))
                    .collect();
                busy.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.tid.cmp(&b.1.tid)));
                lines.extend(busy.iter().take(3).map(|(cpu, thread)| {
                    format!("{} {} {cpu:.0}%", thread.tid, clean_text(&thread.name, 15))
                }));
            }
            _ => lines.push("threads not read: leaves use process CPU".to_owned()),
        }
        lines
    }
}

impl ViewModule for Arbor {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        self.observe(stage.time, stage.focus.is_some(), processes, snapshot);
        stage
            .bounds
            .extend(ring_bounds([0.0, 0.0], FIT_RADIUS, -SOIL_DEPTH, FIT_TOP));
        draw_soil(stage);
        if self.tree.nodes.is_empty() {
            return 0;
        }
        let weights = self.tree.weigh(processes);
        self.sway.advance(
            &self.tree,
            &weights,
            stage.time,
            bounded(snapshot.pressure[0], 20.0),
        );
        let bases = self.sway.bases(&self.tree);
        let pose = Pose {
            tree: &self.tree,
            bases: &bases,
            motion: &self.sway.motion,
        };
        let scene = Drawing {
            processes,
            weights: &weights,
            pose: &pose,
        };
        self.paint_branches(stage, &scene);
        self.paint_leaves(stage, &scene);
        self.paint_falls(stage);
        self.paint_blossoms(stage, &scene);
        self.report(stage, &scene);
        self.tree
            .nodes
            .iter()
            .filter(|node| node.process.is_some())
            .count()
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let trunk = if self.init_trunk || self.signature.is_none() {
            "PID 1"
        } else {
            "largest subtree"
        };
        let mut text = format!(
            " Trunk = {trunk} | branch area = subtree memory | leaves = threads: green idle, red busy | blossoms = started < 5 s | falling leaves = exited threads | roots = kernel threads | wind = CPU pressure {:.0}%",
            snapshot.pressure[0]
        );
        if self.pale > 0 {
            text.push_str(" | pale branch = a parent isotop can't read (run with sudo)");
        }
        if !snapshot
            .processes
            .iter()
            .any(|process| process.kind == Kind::Kernel)
        {
            text.push_str(" | no kernel threads reported: no roots");
        }
        text.extend(missing_note(snapshot, Source::CpuPressure, "still air"));
        if let Some(note) = missing_note(snapshot, Source::Threads, "leaves use process CPU") {
            text.push_str(&note);
        } else {
            let unreadable = snapshot
                .processes
                .iter()
                .filter(|process| matches!(process.thread_list, Measured::Unreadable))
                .count();
            if let Some(note) = unreadable_note("per-thread CPU", unreadable) {
                text.push_str(&note);
                text.push_str(": leaves use process CPU");
            }
        }
        text
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// A twig's tip at rest, which depends on the tree alone: the wind moves the tips drawn.
    #[cfg(test)]
    fn anchors(&self) -> Option<HashMap<Identity, Point>> {
        Some(
            self.tree
                .nodes
                .iter()
                .filter_map(|node| match node.key() {
                    Key::Process(id) => Some((id, node.point(1.0))),
                    Key::Shadow(_) => None,
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::rescaled;
    use crate::model::testing::process;
    use crate::model::{DemoOptions, demo, demo_with};
    use crate::render::{Camera, Frame, RenderInput, Scene};
    use crate::threads::{Thread, ThreadList};
    use crate::views::View;
    use std::sync::Arc;

    fn view() -> View {
        View::named("arbor").expect("arbor is registered")
    }

    fn arbor(scene: &Scene) -> &Arbor {
        scene.module::<Arbor>(view()).expect("arbor has drawn")
    }

    fn snapshot_of(processes: Vec<Process>) -> Snapshot {
        Snapshot {
            processes,
            ..Snapshot::default()
        }
    }

    /// Draws `snapshot` at `time` in a 640 by 360 frame with a camera that fits the tree.
    fn draw(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> Frame {
        let mut camera = Camera::default();
        let mut input = RenderInput::small(snapshot, view(), &camera, time);
        (input.width, input.height) = (640, 360);
        scene.render(&input);
        scene.fit(&mut camera, 640, 360);
        let mut input = RenderInput::small(snapshot, view(), &camera, time);
        (input.width, input.height) = (640, 360);
        scene.render(&input)
    }

    /// A process with the given threads, each using `cpu` percent.
    fn with_threads(process: Process, tids: &[u32], cpu: f32) -> Process {
        let threads: Vec<Thread> = tids
            .iter()
            .map(|&tid| Thread {
                tid,
                start: u64::from(tid),
                name: format!("t{tid}"),
                cpu: Some(cpu),
                state: 'S',
                leader: false,
            })
            .collect();
        Process {
            threads: threads.len() as u32,
            thread_list: Measured::Known(ThreadList {
                threads: Arc::from(threads),
                complete: true,
            }),
            ..process
        }
    }

    fn refs(processes: &[Process]) -> Vec<&Process> {
        processes.iter().collect()
    }

    /// PID 1 with `children` children, each with `grandchildren` of its own, of various memory.
    fn forest(children: u32, grandchildren: u32) -> Vec<Process> {
        let mut processes = vec![Process {
            memory: 14 << 20,
            ..process(1, 0)
        }];
        for child in 0..children {
            let pid = 10 + child * 100;
            processes.push(Process {
                memory: (u64::from(child) * 37 % 400 + 5) << 20,
                ..process(pid, 1)
            });
            for grand in 0..grandchildren {
                processes.push(Process {
                    memory: (u64::from(child + grand) * 53 % 300 + 1) << 20,
                    ..process(pid + 1 + grand, pid)
                });
            }
        }
        processes
    }

    fn build(processes: &[Process]) -> (Tree, Slots) {
        let mut slots = Slots::default();
        let tree = Tree::build(&refs(processes), &[], &mut slots);
        (tree, slots)
    }

    fn node_of(tree: &Tree, pid: u32) -> &Node {
        tree.nodes
            .iter()
            .find(|node| node.key().pid() == pid)
            .expect("the process is in the tree")
    }

    #[test]
    fn every_branch_obeys_leonardos_rule_within_epsilon() {
        let mut shapes = vec![forest(9, 4), forest(40, 0), forest(2, 60)];
        shapes.push(demo(12.0, 192).processes);
        for processes in &shapes {
            let (tree, _) = build(processes);
            let refs = refs(processes);
            let weights = tree.weigh(&refs);
            let close = |a: f32, b: f32| (a - b).abs() <= 1e-4 * a.abs().max(b.abs());
            for (index, node) in tree.nodes.iter().enumerate() {
                let children: Vec<usize> = tree.same_side(index).collect();
                let levels = tree.levels(&weights, index);
                assert_eq!(levels.len(), children.len() + 1);
                // The area before each junction is the node's own plus every child further out.
                for (step, &level) in levels.iter().enumerate() {
                    let beyond: f32 = children[step..]
                        .iter()
                        .map(|&child| weights.subtree[child])
                        .sum();
                    assert!(close(level, weights.own[index] + beyond), "{index} {step}");
                }
                // A junction takes exactly what the child that leaves there carries.
                for (step, &child) in children.iter().enumerate() {
                    assert!(
                        close(levels[step] - levels[step + 1], weights.subtree[child]),
                        "node {index}, child {child}"
                    );
                    assert!(close(
                        tree.levels(&weights, child)[0],
                        weights.subtree[child]
                    ));
                }
                assert!(close(levels[0], weights.subtree[index]));
                // The radius squared is proportional to the area, until the floor.
                let area = weights.subtree[index];
                let radius = weights.radius(node.side, area);
                let total = match node.side {
                    Side::Crown => weights.crown_total,
                    Side::Root => weights.root_total,
                };
                let scale = match node.side {
                    Side::Crown => R_TRUNK,
                    Side::Root => ROOT_COLLAR,
                };
                if radius > RADIUS_FLOOR * 1.001 {
                    assert!(close(radius * radius, scale * scale * area / total));
                }
            }
            assert!(close(
                weights.crown_total,
                weights.own[tree.trunk]
                    + tree
                        .same_side(tree.trunk)
                        .map(|c| weights.subtree[c])
                        .sum::<f32>()
            ));
        }
    }

    #[test]
    fn the_trunk_is_pid_1_else_the_largest_subtree_and_orphans_and_cycles_hang_on_it() {
        let mut processes = forest(3, 2);
        let (tree, _) = build(&processes);
        assert_eq!(tree.nodes[tree.trunk].key().pid(), 1);
        // Without PID 1 the trunk is the root of the largest subtree.
        processes.remove(0);
        processes.push(process(900, 0));
        for process in &mut processes {
            if process.parent == 1 {
                process.parent = 10;
            }
        }
        processes[0].parent = 0;
        let (tree, _) = build(&processes);
        assert_eq!(tree.nodes[tree.trunk].key().pid(), 10);
        assert_eq!(node_of(&tree, 900).parent, Some(tree.trunk), "a lone root");
        // A process whose parent is not drawn, and two that hold each other, hang on the trunk.
        let mut odd = forest(2, 0);
        odd.push(process(7000, 31_337));
        odd.push(process(8001, 8002));
        odd.push(process(8002, 8001));
        let (tree, _) = build(&odd);
        for pid in [7000, 8001] {
            assert_eq!(node_of(&tree, pid).parent, Some(tree.trunk), "{pid}");
        }
        assert_eq!(tree.order.len(), tree.nodes.len(), "every node is reached");
        assert_eq!(tree.order[0], tree.trunk);
    }

    #[test]
    fn slots_follow_the_golden_angle_and_the_attachment_lengths_and_elevations() {
        assert!((attachment(0) - 0.47).abs() < 1e-6);
        assert!((attachment(1) - 0.566).abs() < 1e-6);
        assert!(attachment(500) < 0.951 && attachment(500) > 0.949);
        let processes = forest(6, 3);
        let (tree, _) = build(&processes);
        for node in tree.nodes.iter().filter(|node| node.parent.is_some()) {
            let parent = &tree.nodes[node.parent.expect("a parent")];
            let at = attachment(node.slot);
            assert!((0.0..1.0).contains(&at));
            let tangent = parent.tangent(at);
            let (first, second) = frame(tangent);
            let direction = node.direction;
            let azimuth = dot(direction, second).atan2(dot(direction, first));
            let wanted = (node.slot as f32 * SLOT_ANGLE).rem_euclid(TAU);
            let difference = (azimuth.rem_euclid(TAU) - wanted).rem_euclid(TAU);
            assert!(
                difference.min(TAU - difference) < 1e-3,
                "azimuth {difference}"
            );
            let elevation = dot(direction, tangent).acos();
            let expected = if node.parent == Some(tree.trunk) {
                ELEVATION_TRUNK
            } else {
                ELEVATION
            };
            assert!((elevation - expected).abs() < 1e-3, "elevation {elevation}");
            let length = TRUNK_LENGTH * LENGTH_RATIO.powi(node.depth as i32);
            assert!((node.length - length).abs() < 1e-4);
            let base = parent.point(at);
            assert!(sub(node.base, base).iter().all(|v| v.abs() < 1e-5));
        }
        assert_eq!(tree.nodes[tree.trunk].length, TRUNK_LENGTH);
    }

    #[test]
    fn phyllotaxis_slots_stay_put_through_births_and_exits() {
        let mut slots = Slots::default();
        let before = forest(6, 2);
        let tree = Tree::build(&refs(&before), &[], &mut slots);
        let seats: HashMap<u32, (u32, Point)> = tree
            .nodes
            .iter()
            .map(|node| (node.key().pid(), (node.slot, node.point(1.0))))
            .collect();
        // The trunk's children took slots 0.. in the order of their start times.
        let trunk_slots: Vec<u32> = [10, 110, 210, 310, 410, 510]
            .iter()
            .map(|pid| seats[pid].0)
            .collect();
        assert_eq!(trunk_slots, [0, 1, 2, 3, 4, 5]);
        // One child exits with its own two, and three are born under the trunk, one of them older than the rest
        // by its start time but first seen now: slots only rise.
        let mut after: Vec<Process> = before
            .iter()
            .filter(|process| !(210..=212).contains(&process.id.pid))
            .cloned()
            .collect();
        after.push(process(9000, 1));
        after.push(Process {
            id: Identity {
                pid: 9001,
                start: 2,
            },
            ..process(9001, 1)
        });
        after.push(process(9002, 1));
        let tree = Tree::build(&refs(&after), &[], &mut slots);
        for node in &tree.nodes {
            let pid = node.key().pid();
            if let Some(&(slot, tip)) = seats.get(&pid) {
                assert_eq!(node.slot, slot, "pid {pid} kept its slot");
                assert_eq!(node.point(1.0), tip, "pid {pid} kept its place");
            }
        }
        let mut newcomers: Vec<(u64, u32)> = [9000, 9001, 9002]
            .iter()
            .map(|&pid| {
                let node = node_of(&tree, pid);
                (node.key().start(), node.slot)
            })
            .collect();
        newcomers.sort();
        assert_eq!(
            newcomers.iter().map(|&(_, slot)| slot).collect::<Vec<_>>(),
            [6, 7, 8],
            "the next slots, by start time, and none reuses the one that was freed"
        );
        // A child that changes parent takes a slot on its new one.
        let mut moved = after.clone();
        moved
            .iter_mut()
            .find(|process| process.id.pid == 111)
            .expect("process 111")
            .parent = 310;
        let tree = Tree::build(&refs(&moved), &[], &mut slots);
        let node = node_of(&tree, 111);
        assert_eq!(tree.nodes[node.parent.expect("a parent")].key().pid(), 310);
        assert_eq!(
            node.slot, 2,
            "310 had one child, in slot 0 and 1: 111 is its third"
        );
    }

    #[test]
    fn leaf_count_matches_thread_count_up_to_the_cap() {
        let mut processes = vec![process(1, 0)];
        for (pid, threads) in [(10, 1_u32), (11, 5), (12, 32), (13, 33), (14, 100)] {
            let tids: Vec<u32> = (0..threads).map(|k| 1000 * pid + k).collect();
            processes.push(with_threads(process(pid, 1), &tids, 12.0));
        }
        // Without thread data a process draws one leaf per thread, shaded by cpu / threads.
        processes.push(Process {
            threads: 7,
            cpu: 70.0,
            ..process(15, 1)
        });
        processes.push(Process {
            threads: 90,
            cpu: 90.0,
            ..process(16, 1)
        });
        let snapshot = snapshot_of(processes);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 1.0);
        let state = arbor(&scene);
        let count = |pid: u32| {
            state
                .leaves
                .iter()
                .filter(|leaf| leaf.key.0.pid == pid)
                .count()
        };
        let counts: Vec<usize> = (10..=16).map(count).collect();
        assert_eq!(counts, [1, 5, 32, 32, 32, 7, 32]);
        let fallback: Vec<f32> = state
            .leaves
            .iter()
            .filter(|leaf| leaf.key.0.pid == 15)
            .map(|leaf| leaf.cpu)
            .collect();
        assert!(fallback.iter().all(|cpu| (cpu - 0.1).abs() < 1e-6));
        // Bucketed leaves show the bucket's mean: threads of 12% give 0.12.
        assert!(
            state
                .leaves
                .iter()
                .filter(|leaf| leaf.key.0.pid == 14)
                .all(|leaf| (leaf.cpu - 0.12).abs() < 1e-5 && leaf.key.1 & BUCKET != 0)
        );
        // Over 6,000 leaves, the per-twig cap falls for every twig alike.
        let mut crowd = vec![process(1, 0)];
        for pid in 10..410_u32 {
            let tids: Vec<u32> = (0..40).map(|k| 100_000 * pid + k).collect();
            crowd.push(with_threads(process(pid, 1), &tids, 1.0));
        }
        let snapshot = snapshot_of(crowd);
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 1.0);
        let state = arbor(&scene);
        assert!(
            state.leaves.len() <= LEAF_CAP,
            "{} leaves",
            state.leaves.len()
        );
        assert!(state.leaves.len() > LEAF_CAP - 400);
        let per_twig: HashSet<usize> = (10..410_u32)
            .map(|pid| {
                state
                    .leaves
                    .iter()
                    .filter(|leaf| leaf.key.0.pid == pid)
                    .count()
            })
            .collect();
        assert_eq!(per_twig.len(), 1, "one cap for every twig: {per_twig:?}");
    }

    /// Where every leaf and every branch tip stands at rest, by process.
    fn rest_places(scene: &Scene) -> (HashMap<Identity, Point>, HashMap<(Identity, u64), Point>) {
        let state = arbor(scene);
        let tips = state.anchors().expect("anchors");
        let leaves = state
            .leaves
            .iter()
            .map(|leaf| {
                let node = &state.tree.nodes[leaf.node];
                (leaf.key, add(node.point(leaf.s), leaf.offset))
            })
            .collect();
        (tips, leaves)
    }

    #[test]
    fn branches_keep_their_rest_positions_when_cpu_memory_or_population_change() {
        let base = demo_with(12.0, 192, row_options());
        let mut scene = Scene::new();
        draw(&mut scene, &base, 12.0);
        let (tips, leaves) = rest_places(&scene);
        // The same sample with every process's CPU and memory changed, in a scene that has
        // never seen the first.
        let mut scaled_scene = Scene::new();
        draw(&mut scaled_scene, &rescaled(&base), 12.0);
        let (scaled_tips, scaled_leaves) = rest_places(&scaled_scene);
        assert_eq!(tips, scaled_tips);
        assert_eq!(leaves, scaled_leaves);
        // A tenth of the leaves of the tree exit and as many start: nothing that stays moves.
        let (churned, gone) = crate::harness::churned(&base);
        let mut scene = Scene::new();
        draw(&mut scene, &base, 12.0);
        draw(&mut scene, &churned, 13.0);
        let (after, after_leaves) = rest_places(&scene);
        for (id, tip) in &tips {
            if !gone.contains(id) {
                assert_eq!(after.get(id), Some(tip), "{id:?} stayed");
            }
        }
        for (key, place) in &leaves {
            if !gone.contains(&key.0) && after_leaves.contains_key(key) {
                assert_eq!(after_leaves[key], *place);
            }
        }
        assert!(after.len() >= tips.len() - gone.len() + gone.len() / 2);
    }

    fn row_options() -> DemoOptions {
        view().info().demo
    }

    #[test]
    fn kernel_threads_grow_as_roots_below_the_soil() {
        let mut processes = vec![process(1, 0)];
        processes.push(Process {
            kind: Kind::Kernel,
            ..process(2, 0)
        });
        for pid in 20..36 {
            processes.push(Process {
                kind: Kind::Kernel,
                ..process(pid, 2)
            });
        }
        processes.push(Process {
            kind: Kind::Kernel,
            ..process(3, 1)
        });
        processes.push(Process {
            kind: Kind::Kernel,
            ..process(4, 3)
        });
        for pid in 10..14 {
            processes.push(process(pid, 1));
        }
        let (tree, _) = build(&processes);
        let weights = tree.weigh(&refs(&processes));
        let kthreadd = node_of(&tree, 2);
        let small = node_of(&tree, 3);
        assert_eq!((kthreadd.side, small.side), (Side::Root, Side::Root));
        assert_eq!(node_of(&tree, 4).side, Side::Root, "below a root");
        assert_eq!(node_of(&tree, 10).side, Side::Crown);
        assert!(kthreadd.point(1.0)[2] < -2.0, "the roots grow downwards");
        assert!(node_of(&tree, 10).point(1.0)[2] > 2.0);
        // Thickness is the number of kernel threads: 17 of them against 2.
        let area = |pid| {
            weights.subtree[tree
                .nodes
                .iter()
                .position(|n| n.key().pid() == pid)
                .expect("node")]
        };
        assert_eq!((area(2), area(3)), (17.0, 2.0));
        let ratio = weights.radius(Side::Root, 17.0) / weights.radius(Side::Root, 2.0);
        assert!((ratio - (17.0_f32 / 2.0).sqrt()).abs() < 1e-3, "{ratio}");
        // The crown's areas know nothing of them.
        assert_eq!(
            weights.crown_total,
            weights.own[tree.trunk] + (10..14).map(|_| (1_u64 << 20) as f32).sum::<f32>()
        );
        // Drawn, the roots are visible and pick their kernel threads, though the slab is nearer.
        let snapshot = snapshot_of(processes.clone());
        let mut scene = Scene::new();
        let mut frame = draw(&mut scene, &snapshot, 1.0);
        frame.rasterize();
        let kernel: HashSet<usize> = processes
            .iter()
            .enumerate()
            .filter(|(_, process)| process.kind == Kind::Kernel)
            .map(|(index, _)| index)
            .collect();
        // The scene sorts processes by identity, which for these pids is the order they have.
        let sorted: Vec<Identity> = {
            let mut ids: Vec<Identity> = processes.iter().map(|p| p.id).collect();
            ids.sort();
            ids
        };
        let kernel_ids: HashSet<Identity> =
            kernel.iter().map(|&index| processes[index].id).collect();
        let seen = frame
            .picks
            .iter()
            .filter(|&&pick| pick != NONE && pick & crate::render::THING == 0)
            .filter(|&&pick| kernel_ids.contains(&sorted[pick as usize]))
            .count();
        assert!(seen > 20, "{seen} root pixels pick kernel threads");
        // The roots lie below the ground in the tour and the camera: none rises above it.
        for id in &kernel_ids {
            assert!(scene.positions[id][2] < 0.0, "{id:?}");
        }
    }

    #[test]
    fn wind_follows_cpu_pressure_and_the_air_is_still_without_it() {
        let swing = |pressure: f32| {
            let mut snapshot = snapshot_of(forest(5, 3));
            snapshot.pressure = [pressure, 0.0, 0.0];
            let mut scene = Scene::new();
            for frame in 0..160 {
                draw(&mut scene, &snapshot, 1.0 + frame as f32 * 0.1);
            }
            let state = arbor(&scene);
            let farthest = state
                .sway
                .motion
                .iter()
                .map(|oscillator| oscillator.position[0].hypot(oscillator.position[1]))
                .fold(0.0, f32::max);
            (farthest, state.sway.wind)
        };
        let (still, wind) = swing(0.0);
        assert_eq!((still, wind), (0.0, 0.0), "no pressure, no sway at all");
        let (breeze, breeze_wind) = swing(10.0);
        let (gale, gale_wind) = swing(90.0);
        assert!(breeze > 0.01, "{breeze}");
        assert!(gale > breeze, "{gale} > {breeze}");
        assert!(
            (gale_wind - bounded(90.0, 20.0)).abs() < 0.02,
            "{gale_wind}"
        );
        assert!(gale_wind > breeze_wind);
        // A child carries its parent's displacement along.
        let mut snapshot = snapshot_of(forest(2, 1));
        snapshot.pressure = [50.0, 0.0, 0.0];
        let mut scene = Scene::new();
        for frame in 0..120 {
            draw(&mut scene, &snapshot, 1.0 + frame as f32 * 0.1);
        }
        let state = arbor(&scene);
        let bases = state.sway.bases(&state.tree);
        let child = state
            .tree
            .nodes
            .iter()
            .position(|node| node.key().pid() == 11)
            .expect("a grandchild");
        let parent = state.tree.nodes[child].parent.expect("a parent");
        let at = attachment(state.tree.nodes[child].slot);
        let carried = state.sway.motion[parent].position[0] * at * at + bases[parent][0];
        assert!((bases[child][0] - carried).abs() < 1e-5);
        assert!(bases[child][0].abs() > 0.0);
        // The gust phase is the branch's own, from its identity, so equal trees sway alike.
        let again = {
            let mut scene = Scene::new();
            for frame in 0..120 {
                draw(&mut scene, &snapshot, 1.0 + frame as f32 * 0.1);
            }
            arbor(&scene).sway.motion.clone()
        };
        assert_eq!(again, state.sway.motion);
    }

    #[test]
    fn natural_frequency_falls_with_length_and_rises_with_thickness() {
        let f = natural_frequency;
        assert!(f(0.4, 6.0) > f(0.1, 6.0));
        assert!(f(0.2, 12.0) < f(0.2, 3.0));
        assert!((f(0.18, 6.0) - 0.6).abs() < 1e-6);
        assert_eq!(f(100.0, 0.1), 3.0);
        assert_eq!(f(0.0001, 100.0), 0.15);
    }

    #[test]
    fn sway_follows_the_same_path_at_ten_and_sixty_frames_per_second() {
        let mut snapshot = snapshot_of(forest(4, 3));
        snapshot.pressure = [40.0, 0.0, 0.0];
        let run = |fps: u32| {
            let mut scene = Scene::new();
            let mut states = HashMap::new();
            for k in 0..=3 * fps {
                draw(&mut scene, &snapshot, 100.0 + k as f32 / fps as f32);
                let state = arbor(&scene);
                states.insert(state.sway.steps, state.sway.motion.clone());
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
        // Paused, the wind holds: the same time again takes no step.
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 5.0);
        draw(&mut scene, &snapshot, 5.5);
        let steps = arbor(&scene).sway.steps;
        draw(&mut scene, &snapshot, 5.5);
        assert_eq!(arbor(&scene).sway.steps, steps);
    }

    /// The indices of the spheres in `frame` that pick `slot`.
    fn spheres_of(frame: &Frame, slot: usize) -> usize {
        frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Sphere { pick, .. } if *pick == slot as u32))
            .count()
    }

    #[test]
    fn a_new_process_blossoms_for_five_seconds() {
        let first = snapshot_of(forest(3, 1));
        let mut second = first.clone();
        second.processes.push(process(900, 1));
        let newcomer = {
            let mut ids: Vec<Identity> = second.processes.iter().map(|p| p.id).collect();
            ids.sort();
            ids.iter()
                .position(|id| id.pid == 900)
                .expect("the newcomer")
        };
        let mut scene = Scene::new();
        let frame = draw(&mut scene, &first, 10.0);
        let started: usize = (0..first.processes.len())
            .map(|slot| spheres_of(&frame, slot))
            .sum();
        assert_eq!(
            started, 0,
            "processes present on the first frame never blossom"
        );
        let frame = draw(&mut scene, &second, 11.0);
        assert_eq!(spheres_of(&frame, newcomer), 0, "a bud, not yet open");
        let frame = draw(&mut scene, &second, 12.5);
        assert_eq!(spheres_of(&frame, newcomer), BLOSSOMS_PER_PROCESS);
        let others: usize = (0..second.processes.len())
            .filter(|&slot| slot != newcomer)
            .map(|slot| spheres_of(&frame, slot))
            .sum();
        assert_eq!(others, 0);
        let frame = draw(&mut scene, &second, 11.0 + BLOSSOM_SECONDS + 0.2);
        assert_eq!(spheres_of(&frame, newcomer), 0, "the blossoms are over");
    }

    /// The leaves hanging on process 50.
    fn leaves_of_fifty(scene: &Scene) -> usize {
        arbor(scene)
            .leaves
            .iter()
            .filter(|leaf| leaf.key.0.pid == 50)
            .count()
    }

    /// A process of the listed threads, and a snapshot of it under PID 1.
    fn pool(tids: &[u32]) -> Snapshot {
        snapshot_of(vec![
            process(1, 0),
            with_threads(process(50, 1), tids, 30.0),
        ])
    }

    #[test]
    fn exited_threads_fall_as_leaves() {
        let mut scene = Scene::new();
        draw(&mut scene, &pool(&[50, 51, 52, 53, 54, 55]), 10.0);
        assert!(arbor(&scene).falls.is_empty());
        // Two threads exit: two leaves leave the twig and fall.
        draw(&mut scene, &pool(&[50, 51, 52, 53]), 11.0);
        let state = arbor(&scene);
        assert_eq!(leaves_of_fifty(&scene), 4);
        assert_eq!(state.falls.len(), 2);
        let fall = state.falls[0];
        let ground = |age: f32| fall.pose(age).map(|(at, _)| at[2]);
        assert_eq!(ground(-0.1), None, "not before it leaves");
        let top = ground(0.0).expect("it leaves at once");
        assert!((top - fall.from[2]).abs() < 1e-5);
        assert!(ground(2.0).expect("falling") < top);
        assert!(ground(FALL_SECONDS).expect("landed") < 0.05);
        assert!(ground(FALL_SECONDS + 20.0).expect("resting") < 0.05);
        assert_eq!(
            ground(FALL_SECONDS + REST_SECONDS + 0.1),
            None,
            "gone after 30 s"
        );
        for age in [0.0, 1.0, 3.0, 5.0, 30.0] {
            let (at, _) = fall.pose(age).expect("posed");
            assert!(
                at[0].abs() < SOIL_HALF && at[1].abs() < SOIL_HALF,
                "on the slab"
            );
        }
        // It is drawn while it falls and not after it is gone, and picks nothing.
        let leaf_triangles = |frame: &Frame| {
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Triangle(_, _, pick) if *pick == NONE))
                .count()
        };
        let while_falling = leaf_triangles(&draw(&mut scene, &pool(&[50, 51, 52, 53]), 12.0));
        let gone = leaf_triangles(&draw(&mut scene, &pool(&[50, 51, 52, 53]), 11.0 + 40.0));
        assert!(while_falling >= gone + 4, "{while_falling} against {gone}");
        // A thread that joins grows a leaf, and drops nothing.
        let mut scene = Scene::new();
        draw(&mut scene, &pool(&[50, 51]), 10.0);
        draw(&mut scene, &pool(&[50, 51, 52, 53]), 11.0);
        assert_eq!(leaves_of_fifty(&scene), 4);
        assert!(arbor(&scene).falls.is_empty());
        // A process that exits drops every leaf it had.
        let mut scene = Scene::new();
        draw(&mut scene, &pool(&[50, 51, 52, 53, 54]), 10.0);
        draw(&mut scene, &snapshot_of(vec![process(1, 0)]), 11.0);
        assert_eq!(arbor(&scene).falls.len(), 5);
        // So does one that is left a zombie.
        let mut scene = Scene::new();
        draw(&mut scene, &pool(&[50, 51, 52]), 10.0);
        let mut zombie = pool(&[50, 51, 52]);
        zombie.processes[1].state = 'Z';
        draw(&mut scene, &zombie, 11.0);
        assert_eq!(arbor(&scene).falls.len(), 3);
        // Without thread lists, a drop in the thread count drops that many leaves.
        let counted = |threads: u32| {
            snapshot_of(vec![
                process(1, 0),
                Process {
                    threads,
                    ..process(50, 1)
                },
            ])
        };
        let mut scene = Scene::new();
        draw(&mut scene, &counted(9), 10.0);
        draw(&mut scene, &counted(5), 11.0);
        assert_eq!(arbor(&scene).falls.len(), 4);
        assert_eq!(leaves_of_fifty(&scene), 5);
        // A list that arrives where there was none does not make the numbered leaves fall.
        let mut scene = Scene::new();
        draw(&mut scene, &counted(4), 10.0);
        draw(&mut scene, &pool(&[50, 51, 52, 53]), 11.0);
        assert!(arbor(&scene).falls.is_empty());
        // However many threads exit, at most 400 leaves are falling or resting.
        let mut crowd = vec![process(1, 0)];
        for pid in 10..110_u32 {
            let tids: Vec<u32> = (0..32).map(|k| 10_000 * pid + k).collect();
            crowd.push(with_threads(process(pid, 1), &tids, 5.0));
        }
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot_of(crowd), 10.0);
        draw(&mut scene, &snapshot_of(vec![process(1, 0)]), 11.0);
        assert_eq!(arbor(&scene).falls.len(), FALLEN_CAP);
    }

    /// The pixels of every triangle that picks `slot`: their colours.
    fn colors_of(frame: &Frame, slot: usize) -> Vec<Color> {
        frame
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(_, color, pick) if *pick == slot as u32 => Some(*color),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn zombies_are_bare_grey_twigs() {
        let mut processes = forest(3, 1);
        let zombie = processes.len();
        processes.push(with_threads(process(900, 1), &[900, 901, 902], 40.0));
        processes[zombie].state = 'Z';
        let snapshot = snapshot_of(processes);
        let mut ids: Vec<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        ids.sort();
        let slot = ids.iter().position(|id| id.pid == 900).expect("the zombie");
        let mut scene = Scene::new();
        let frame = draw(&mut scene, &snapshot, 1.0);
        let colors = colors_of(&frame, slot);
        assert!(!colors.is_empty(), "the twig is drawn and pickable");
        assert!(
            colors.iter().all(|c| c[0] == c[1] && c[1] == c[2]),
            "grey: {colors:?}"
        );
        assert!(
            arbor(&scene)
                .leaves
                .iter()
                .all(|leaf| leaf.key.0.pid != 900),
            "no leaves"
        );
        // A live process of the same threads is brown and has its leaves.
        let mut live = snapshot.clone();
        live.processes[zombie].state = 'S';
        let mut scene = Scene::new();
        let frame = draw(&mut scene, &live, 1.0);
        assert!(colors_of(&frame, slot).iter().any(|c| c[0] != c[2]));
        assert_eq!(
            arbor(&scene)
                .leaves
                .iter()
                .filter(|leaf| leaf.key.0.pid == 900)
                .count(),
            3
        );
    }

    #[test]
    fn unreadable_parents_become_pale_unpickable_branches() {
        let mut snapshot = snapshot_of(vec![
            process(1, 0),
            process(100, 500),
            process(101, 500),
            process(102, 1),
        ]);
        snapshot.shadows = vec![Shadow {
            pid: 500,
            parent: 1,
            name: "root-daemon".into(),
        }];
        let mut scene = Scene::new();
        let mut frame = draw(&mut scene, &snapshot, 1.0);
        let state = arbor(&scene);
        assert_eq!(state.pale, 1);
        let pale = state
            .tree
            .nodes
            .iter()
            .position(|node| node.process.is_none())
            .expect("a pale branch");
        assert_eq!(state.tree.nodes[pale].parent, Some(state.tree.trunk));
        let carried: Vec<u32> = state.tree.nodes[pale]
            .children
            .iter()
            .map(|&child| state.tree.nodes[child].key().pid())
            .collect();
        assert_eq!(
            carried,
            [100, 101],
            "the unreadable parent carries its children"
        );
        let weights = state.tree.weigh(&refs(&snapshot.processes));
        assert_eq!(weights.own[pale], 0.0, "no memory of its own");
        assert!(weights.subtree[pale] > 0.0);
        // Nothing it draws is pickable, and nothing but it is pale.
        let paler = |color: &Color| color[2] > 200 && color[0] > 100;
        let pale_pixels = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Triangle(_, color, _) if paler(color)))
            .collect::<Vec<_>>();
        assert!(!pale_pixels.is_empty());
        assert!(
            pale_pixels
                .iter()
                .all(|item| matches!(item, Item::Triangle(_, _, pick) if *pick == NONE))
        );
        frame.rasterize();
        assert!(!scene.positions.keys().any(|id| id.pid == 500));
        let legend = scene.legend(view(), &snapshot);
        assert!(
            legend.contains(" | pale branch = a parent isotop can't read (run with sudo)"),
            "{legend}"
        );
        // Without sudo launchd is the pale trunk, and the processes it started hang on it.
        let mut mac = snapshot_of(vec![process(300, 1), process(301, 1)]);
        mac.shadows = vec![Shadow {
            pid: 1,
            parent: 0,
            name: "launchd".into(),
        }];
        let mut scene = Scene::new();
        draw(&mut scene, &mac, 1.0);
        let state = arbor(&scene);
        let trunk = &state.tree.nodes[state.tree.trunk];
        assert_eq!((trunk.key(), trunk.process), (Key::Shadow(1), None));
        assert_eq!(trunk.children.len(), 2);
        // Focused on a subtree, the shadows are left out.
        let mut scene = Scene::new();
        let camera = Camera::default();
        let mut input = RenderInput::small(&snapshot, view(), &camera, 1.0);
        input.focus = Some(snapshot.processes[1].id);
        scene.render(&input);
        assert_eq!(arbor(&scene).pale, 0);
        let legend = scene.legend(view(), &snapshot);
        assert!(!legend.contains("pale branch"), "{legend}");
    }

    #[test]
    fn arbor_renders_the_same_frame_twice_from_the_same_inputs() {
        let snapshot = demo_with(12.0, 192, row_options());
        let render = || {
            let mut scene = Scene::new();
            draw(&mut scene, &snapshot, 12.0);
            let mut frame = draw(&mut scene, &snapshot, 12.1);
            frame.rasterize();
            (frame.pixels.clone(), frame.picks.clone())
        };
        let (first, second) = (render(), render());
        assert!(first.0 == second.0 && first.1 == second.1);
        assert!(first.1.iter().any(|&pick| pick != NONE));
    }

    #[test]
    fn leaves_run_from_green_when_idle_to_red_when_busy() {
        assert_eq!(leaf_color(0.0), [74, 150, 72]);
        assert_eq!(leaf_color(1.0), [220, 52, 44]);
        assert_eq!(leaf_color(7.0), leaf_color(1.0), "above a core is as red");
        let (idle, middle, busy) = (leaf_color(0.0), leaf_color(0.3), leaf_color(0.9));
        assert!(idle[1] > idle[0] && busy[0] > busy[1] && middle[0] > 200 && middle[1] > 180);
    }

    #[test]
    fn branches_are_lit_by_the_shared_light_and_never_outside_its_range() {
        let snapshot = snapshot_of(forest(4, 2));
        let mut ids: Vec<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        ids.sort();
        let trunk = ids.iter().position(|id| id.pid == 1).expect("the trunk");
        let mut scene = Scene::new();
        let frame = draw(&mut scene, &snapshot, 1.0);
        // Its one leaf is green; the bark is the rest.
        let colors: Vec<Color> = colors_of(&frame, trunk)
            .into_iter()
            .filter(|color| color[0] > color[1])
            .collect();
        assert!(colors.len() > 8);
        let brightest = colors.iter().map(|c| c[0]).max().expect("colours");
        let darkest = colors.iter().map(|c| c[0]).min().expect("colours");
        assert!(brightest > darkest, "faces differ by their light");
        for color in &colors {
            let shade = f32::from(color[0]) / f32::from(BARK[0]);
            assert!((0.34..=1.0).contains(&shade), "{shade}");
        }
    }

    #[test]
    fn the_legend_names_the_mappings_and_each_gap() {
        let mut snapshot = demo_with(12.0, 192, row_options());
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 12.0);
        let legend = scene.legend(view(), &snapshot);
        let pressure = format!("{:.0}", snapshot.pressure[0]);
        assert_eq!(
            legend,
            format!(
                " Trunk = largest subtree | branch area = subtree memory | leaves = threads: green idle, red busy | blossoms = started < 5 s | falling leaves = exited threads | roots = kernel threads | wind = CPU pressure {pressure}% | pale branch = a parent isotop can't read (run with sudo)"
            ),
            "the demo hides one parent, which is drawn pale"
        );
        snapshot.missing = Source::CpuPressure.into();
        assert!(
            scene
                .legend(view(), &snapshot)
                .ends_with(" | no cpu pressure: still air")
        );
        snapshot.missing = Source::Threads.into();
        assert!(
            scene
                .legend(view(), &snapshot)
                .ends_with(" | no threads: leaves use process CPU")
        );
        snapshot.missing = Default::default();
        snapshot.processes[3].thread_list = Measured::Unreadable;
        snapshot.processes[4].thread_list = Measured::Unreadable;
        assert!(
            scene
                .legend(view(), &snapshot)
                .ends_with(" | per-thread CPU unreadable for 2: leaves use process CPU")
        );
        snapshot
            .processes
            .retain(|process| process.kind != Kind::Kernel);
        assert!(
            scene
                .legend(view(), &snapshot)
                .contains(" | no kernel threads reported: no roots")
        );
        // With PID 1 the trunk is named so.
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot_of(forest(2, 1)), 1.0);
        assert!(
            scene
                .legend(view(), &snapshot)
                .starts_with(" Trunk = PID 1 | branch area")
        );
    }

    #[test]
    fn the_inspector_tour_and_labels_come_from_the_tree() {
        let snapshot = demo_with(12.0, 192, row_options());
        let camera = Camera::default();
        let mut scene = Scene::new();
        scene.highlight = vec![snapshot.processes[16].id];
        let mut input = RenderInput::small(&snapshot, view(), &camera, 12.0);
        (input.width, input.height) = (640, 360);
        scene.render(&input);
        let notes = &scene.notes[&snapshot.processes[16].id];
        assert!(notes[0].starts_with("subtree "), "{notes:?}");
        assert!(notes[1].contains(" threads, ") && notes[1].ends_with(" leaves"));
        assert!(notes[2].starts_with("slot "));
        assert!(notes.len() >= 4, "the busiest threads follow: {notes:?}");
        assert!(scene.tour.len() >= TOUR_BRANCHES);
        assert!(
            scene
                .tour
                .windows(2)
                .take(TOUR_BRANCHES - 1)
                .all(|pair| pair[0].weight >= pair[1].weight)
        );
        assert!(scene.places.len() >= 3 + LABELLED_BRANCHES);
        assert!(scene.places.iter().all(|(_, text)| text.is_ascii()));
        // Every drawn process has a place.
        assert_eq!(scene.positions.len(), snapshot.processes.len());
    }
}
