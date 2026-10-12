//! The process tree as nested Keplerian systems, which Orbit draws as orbits and Flow as wells.
//! A top-level process, or a branch hanging off one, is a system: its children sit on confocal
//! ellipses (`shells`) around it, and theirs around them, down to `MAX_DEPTH`. The systems are
//! placed on the ground once, reserving room for their structure (`Galaxy::place`), and turn
//! about their memory-weighted barycenter at Kepler rates (`Galaxy::revolve`), but only across
//! consecutive frames. A parent isotop cannot read stands as a hollow hub (`hubs`), an
//! unpickable outline with no measured figures, so its readable leaves orbit one star.
//!
//! The view modules (`orbit`, `flow`) draw what `layout` returns and report the systems through
//! `Layout::report`: the tour stops, the titles and the popup line of the memory a system holds.

use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;

use crate::layout::vacant;
use crate::model::{Identity, Process, Snapshot, bytes};
use crate::platform::{self, Shadow};
use crate::render::{Color, Point, Stage, Title, TourTarget, label_name, mass_radius};
use crate::simulation::hash;

pub(super) const MAX_DEPTH: usize = 3;
const ORBIT_GAP: f32 = 0.6;
const SYSTEM_GAP: f32 = 2.5;
/// How much room a system reserves around what it needs when it is placed, so that a change of
/// memory or population does not touch the layout until the system outgrows it. A body's radius
/// follows the cube root of its memory, and shells pack by size, so an extent grows a little
/// faster than that: 30% holds what the view harness does to a system (memory by half to double,
/// a tenth of its processes leaving and as many arriving). A system that outgrows it is placed
/// again.
const HEADROOM: f32 = 1.3;
/// The outline of a hub, a parent isotop cannot read.
pub(super) const HUB: Color = [168, 182, 210];
/// `Identity::start` of a hub. No process starts at the end of time, so a hub's identity never
/// equals a process's, even when it has the pid of the parent it stands for.
const HUB_START: u64 = u64::MAX;

/// A Keplerian ellipse with its focus on the parent body.
#[derive(Clone, Copy)]
pub(crate) struct Orbit {
    pub center: Point,
    pub a: f32,
    pub e: f32,
    pub omega: f32,
}

impl Orbit {
    pub(crate) fn point(&self, eccentric: f32) -> Point {
        let (s, c) = eccentric.sin_cos();
        let x = self.a * (c - self.e);
        let y = self.a * (1.0 - self.e * self.e).sqrt() * s;
        let (ws, wc) = self.omega.sin_cos();
        [
            self.center[0] + x * wc - y * ws,
            self.center[1] + x * ws + y * wc,
            self.center[2],
        ]
    }

    pub(super) fn at(&self, mean: f32) -> Point {
        self.point(eccentric_anomaly(mean, self.e))
    }
}

/// A body's orbit around its parent and where it stands on it.
pub(super) struct Path {
    pub orbit: Orbit,
    pub depth: usize,
    pub id: Identity,
    pub spoke: bool,
    pub mean: f32,
    pub position: Point,
    pub cpu: f32,
}

/// A process placed in the layout: its index in the process list, where it is drawn (flying
/// out of its parent while it grows) and how far grown it is.
pub(crate) struct Body {
    pub index: usize,
    pub position: Point,
    pub grow: f32,
}

pub(super) struct Tree<'a> {
    pub processes: &'a [&'a Process],
    children: Vec<Vec<usize>>,
    extents: Vec<f32>,
    /// Body radius from the memory of the whole subtree a body anchors.
    pub radii: Vec<f32>,
    /// Radius including thread rings, which neighbours must clear.
    reach: Vec<f32>,
    masses: Vec<f32>,
}

/// Where the systems stand: each root's centre and the radius reserved around it, kept while its
/// structure fits, and how they turn about their pivot.
#[derive(Default)]
pub(super) struct Galaxy {
    /// Each system's centre and the radius reserved around it.
    pub systems: HashMap<Identity, ([f32; 2], f32)>,
    /// Point the systems revolve about, the systems it was computed for, the motion time they
    /// were last turned to, and each system's angular speed in radians per second.
    pub center: [f32; 2],
    members: Vec<Identity>,
    time: Option<f32>,
    pub speed: HashMap<Identity, f32>,
    /// The frame the galaxy last turned in: it only turns across consecutive frames, so
    /// switching to another view and back never spins it.
    frame: u64,
}

impl Galaxy {
    fn place(
        &mut self,
        processes: &[&Process],
        snapshot: &Snapshot,
        roots: &[usize],
        extents: &[f32],
    ) {
        let needed: HashMap<Identity, f32> = roots
            .iter()
            .map(|&root| (processes[root].id, extents[root] + SYSTEM_GAP))
            .collect();
        // A system whose root focus or the process limit hides keeps its place and turns on with
        // the others, so clearing the focus brings it back where it was, but only if its root
        // would head a system with every sampled process drawn (a hub counts, as long as a leaf
        // still orbits it). A system that only a focus, or a selection swapped across the limit,
        // made, or whose root left the snapshot, gives its place up.
        let whole = if self.systems.keys().all(|id| needed.contains_key(id)) {
            HashSet::new()
        } else {
            whole_roots(snapshot)
        };
        let before = self.systems.len();
        self.systems
            .retain(|id, (_, reserved)| match needed.get(id) {
                Some(&need) => need <= *reserved,
                None => whole.contains(id),
            });
        let mut pending: Vec<_> = needed
            .iter()
            .filter(|(id, _)| !self.systems.contains_key(id))
            .map(|(&id, &need)| (id, need))
            .collect();
        if self.systems.len() != before || !pending.is_empty() {
            // A system left, arrived or was re-placed: the pivot has to be found again.
            self.members.clear();
        }
        pending.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        for (id, need) in pending {
            // Headroom absorbs memory drift so systems are only re-placed when their structure
            // outgrows it.
            let reserved = need * HEADROOM;
            let placed: Vec<_> = self.systems.values().copied().collect();
            let center = vacant(&placed, reserved);
            self.systems.insert(id, (center, reserved));
        }
    }

    /// Turns the systems about the memory-weighted barycenter of the drawn ones (those in
    /// `masses`, or all while none is) by the motion time elapsed since the last call, so a
    /// focused subtree turns about itself rather than about the hidden rest of the galaxy. The
    /// barycenter is taken when systems come or go or are shown or hidden, so the pivot stays
    /// put while they turn. Systems whose distances from it, give or take their reserved radius,
    /// overlap form a band that turns rigidly; bands occupy disjoint annuli, so whatever their
    /// speeds no two systems can collide. Each band turns at its own Kepler rate, outer bands
    /// slower, and phase accumulates, so a band changing speed never makes a system jump.
    fn revolve(&mut self, masses: &HashMap<Identity, f32>, motion: f32, frames: u64) {
        let dt = self.time.map_or(0.0, |last| motion - last);
        self.time = Some(motion);
        let continuous = self.frame + 1 == frames;
        self.frame = frames;
        let weight = |id: &Identity| masses.get(id).copied().unwrap_or(0.0).max(1.0);
        let mut members: Vec<Identity> = self
            .systems
            .keys()
            .filter(|id| masses.contains_key(id))
            .copied()
            .collect();
        if members.is_empty() {
            members = self.systems.keys().copied().collect();
        }
        members.sort();
        if members != self.members {
            let total: f32 = members.iter().map(weight).sum();
            let mut center = [0.0_f32; 2];
            for id in &members {
                let at = self.systems[id].0;
                for k in 0..2 {
                    center[k] += at[k] * weight(id) / total.max(1.0);
                }
            }
            self.center = center;
            self.members = members;
        }
        let center = self.center;
        let mut rings: Vec<(Identity, f32, f32)> = self
            .systems
            .iter()
            .map(|(&id, &(at, reserved))| {
                let distance = (at[0] - center[0]).hypot(at[1] - center[1]);
                (id, distance, reserved)
            })
            .collect();
        rings.sort_by(|a, b| (a.1 - a.2).total_cmp(&(b.1 - b.2)).then(a.0.cmp(&b.0)));
        let mut bands: Vec<(f32, Vec<(Identity, f32)>)> = Vec::new();
        for (id, distance, reserved) in rings {
            match bands.last_mut() {
                Some((outer, members)) if distance - reserved < *outer => {
                    *outer = outer.max(distance + reserved);
                    members.push((id, distance));
                }
                _ => bands.push((distance + reserved, vec![(id, distance)])),
            }
        }
        self.speed.clear();
        let mut enclosed = 0.0;
        let mut inner = f32::INFINITY;
        for (_, members) in &bands {
            let mass: f32 = members.iter().map(|(id, _)| weight(id)).sum();
            enclosed += mass;
            let radius = members.iter().map(|(id, d)| d * weight(id)).sum::<f32>() / mass;
            // A heavy band outside a light one would otherwise outpace it; outer never turns faster.
            let speed = (TAU / galactic_period(radius, enclosed)).min(inner);
            inner = speed;
            for (id, _) in members {
                self.speed.insert(*id, speed);
            }
        }
        // Turn only between consecutive frames and by at most MAX_TURN_STEP (above the 1 s frame
        // interval of --fps 1), so pauses, rewinds and view switches don't spin the galaxy.
        if !(continuous && dt > 0.0 && dt <= MAX_TURN_STEP) {
            return;
        }
        for (id, (at, _)) in &mut self.systems {
            let (s, c) = (self.speed[id] * dt).sin_cos();
            let [x, y] = [at[0] - center[0], at[1] - center[1]];
            *at = [center[0] + x * c - y * s, center[1] + x * s + y * c];
        }
    }
}

/// A system's star: the root of a tree, how many processes the system draws besides a hub, and
/// how far the system reaches from the star in world units.
pub(super) struct Star {
    pub id: Identity,
    /// The star's index in the process list.
    pub root: usize,
    pub members: usize,
    pub reach: f32,
}

/// The systems of one frame: the trees laid out, the orbits and bodies to draw and what to say
/// about the systems.
pub(super) struct Layout<'a> {
    pub tree: Tree<'a>,
    pub paths: Vec<Path>,
    pub bodies: Vec<Body>,
    /// The systems, in the order of their roots.
    pub stars: Vec<Star>,
    /// For bodies with children: memory of the subtree they anchor and its process count.
    pub mass: HashMap<Identity, (u64, usize)>,
    /// How many of the processes the layout drew, hubs not counted: the rest are collapsed.
    pub shown: usize,
}

impl Layout<'_> {
    /// The place each drawn process keeps: the centre of its system. Its orbit, and where it
    /// stands on that, follow memory and the siblings around it by design (a body's radius is its
    /// memory's cube root, shells pack by size, a parent's mass sets the pace), so the home is
    /// what stays put when CPU, memory or the population changes: a system is only placed
    /// again when its structure outgrows the room reserved for it. Read after `layout`.
    #[cfg(test)]
    pub(super) fn homes(&self, galaxy: &Galaxy) -> HashMap<Identity, Point> {
        let mut homes = HashMap::new();
        let mut first = 0;
        for star in &self.stars {
            // The bodies of a system follow one another in `bodies`: its members, and the hub.
            let size = star.members + usize::from(is_hub(&star.id));
            let [x, y] = galaxy.systems[&star.id].0;
            for body in &self.bodies[first..first + size] {
                homes.insert(self.tree.processes[body.index].id, [x, y, 0.0]);
            }
            first += size;
        }
        homes
    }

    /// Tells the app about the systems once their stars stand where they are drawn: the tour
    /// visits each star (a hub has no callout, so it is passed over), the titles name the
    /// systems, and the popup of a body with children says how much memory it anchors.
    pub(super) fn report(&self, stage: &mut Stage) {
        let processes = self.tree.processes;
        for star in &self.stars {
            let Some(&at) = stage.positions.get(&star.id) else {
                continue;
            };
            let process = processes[star.root];
            let text = if is_hub(&star.id) {
                format!(
                    "{} (unreadable) ({})",
                    label_name(&process.name),
                    star.members
                )
            } else {
                stage.tour.push(TourTarget {
                    id: star.id,
                    position: at,
                    weight: star.members as f32,
                });
                system_label(process, star.members)
            };
            stage.titles.push(Title {
                at,
                reach: star.reach,
                text,
            });
        }
        for (index, kids) in self.tree.children.iter().enumerate() {
            if let Some(&(total, count)) = self
                .mass
                .get(&processes[index].id)
                .filter(|_| !kids.is_empty())
            {
                stage.popup.push((
                    processes[index].id,
                    format!(
                        "system {} {} across {count} processes",
                        platform::MEMORY_LABEL,
                        bytes(total)
                    ),
                ));
            }
        }
    }
}

/// The label of the system whose star is `process` and which holds `members` processes: the
/// star's name (systemd is `init` as process 1 and `systemd --user` otherwise), and the number of
/// processes after it when there are several.
pub(crate) fn system_label(process: &Process, members: usize) -> String {
    let name = match (process.name.as_str(), process.id.pid) {
        ("systemd", 1) => "init",
        ("systemd", _) => "systemd --user",
        (name, _) => name,
    };
    let name = label_name(name);
    if members > 1 {
        format!("{name} ({members})")
    } else {
        name.into_owned()
    }
}

/// Lays out the process tree as nested Keplerian systems, `galaxy` placing and turning them and
/// the stage's time driving spawn growth. `motion` is the orbital phase, so other views can reuse
/// the layout with bodies held still.
pub(super) fn layout<'a>(
    galaxy: &mut Galaxy,
    stage: &mut Stage,
    processes: &'a [&'a Process],
    snapshot: &Snapshot,
    motion: f32,
) -> Layout<'a> {
    let by_pid: HashMap<u32, usize> = processes
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.pid, i))
        .collect();
    let parents: Vec<Option<usize>> = processes
        .iter()
        .enumerate()
        .map(|(i, p)| by_pid.get(&p.parent).copied().filter(|&parent| parent != i))
        .collect();
    let mut child_counts = vec![0; processes.len()];
    for &parent in parents.iter().flatten() {
        child_counts[parent] += 1;
    }
    let mut children = vec![Vec::new(); processes.len()];
    let mut roots = Vec::new();
    for (i, parent) in parents.iter().enumerate() {
        match *parent {
            // Branches hanging off a top-level process (init, kthreadd, a focused root) become
            // their own systems; leaves stay in orbit so kernel threads form one belt.
            Some(parent) if child_counts[i] == 0 || parents[parent].is_some() => {
                children[parent].push(i)
            }
            _ => roots.push(i),
        }
    }
    for list in &mut children {
        list.sort_by_key(|&child| (child_counts[child] == 0, processes[child].id));
    }
    let n = processes.len();
    let mut masses = vec![0.0; n];
    let mut members = vec![0; n];
    let mut weighed = HashSet::new();
    for &root in &roots {
        weigh(
            processes,
            &children,
            root,
            &mut masses,
            &mut members,
            &mut weighed,
        );
    }
    let radii: Vec<f32> = masses.iter().map(|&m| mass_radius(m)).collect();
    let reach: Vec<f32> = radii
        .iter()
        .zip(processes)
        .map(|(&r, p)| r * ring_scale(p.threads))
        .collect();
    let mut mass = HashMap::new();
    for (i, kids) in children.iter().enumerate() {
        if !kids.is_empty() {
            mass.insert(processes[i].id, (masses[i] as u64, members[i]));
        }
    }
    let mut extents = vec![0.0; n];
    let mut measured = HashSet::new();
    for &root in &roots {
        measure(
            &children,
            &reach,
            processes,
            root,
            0,
            &mut extents,
            &mut measured,
        );
    }
    let tree = Tree {
        processes,
        children,
        extents,
        radii,
        reach,
        masses,
    };
    galaxy.place(processes, snapshot, &roots, &tree.extents);
    let weights: HashMap<Identity, f32> = roots
        .iter()
        .map(|&root| (processes[root].id, tree.masses[root]))
        .collect();
    galaxy.revolve(&weights, motion, stage.frames);
    let mut walk = Walk {
        stage,
        tree: &tree,
        motion,
        visited: HashSet::new(),
        paths: Vec::new(),
        bodies: Vec::new(),
    };
    let mut stars = Vec::new();
    for &root in &roots {
        let before = walk.visited.len();
        let [x, y] = galaxy.systems[&processes[root].id].0;
        let point = [x, y, 1.0];
        let grow = walk.stage.growth(processes[root].id);
        walk.branch(root, point, point, grow, 0);
        // A hub's members are the processes around it, not the hub itself.
        let hub = usize::from(is_hub(&processes[root].id));
        stars.push(Star {
            id: processes[root].id,
            root,
            members: walk.visited.len() - before - hub,
            reach: tree.extents[root],
        });
    }
    let Walk {
        visited,
        paths,
        bodies,
        ..
    } = walk;
    let hubs = processes.iter().filter(|p| is_hub(&p.id)).count();
    Layout {
        tree,
        paths,
        bodies,
        stars,
        mass,
        shown: visited.len().saturating_sub(hubs),
    }
}

/// The walk down the trees of one frame: where each body stands and the orbit it travels.
struct Walk<'a, 'b, 'c> {
    stage: &'a mut Stage<'b>,
    tree: &'a Tree<'c>,
    motion: f32,
    visited: HashSet<usize>,
    paths: Vec<Path>,
    bodies: Vec<Body>,
}

impl Walk<'_, '_, '_> {
    /// Places a body and its descendants. `target` is the settled position used for layout,
    /// picking and labels; `drawn` includes the spawn animation that flies bodies out of parents.
    fn branch(&mut self, index: usize, target: Point, drawn: Point, grow: f32, depth: usize) {
        if depth > MAX_DEPTH || !self.visited.insert(index) {
            return;
        }
        let tree = self.tree;
        let process = tree.processes[index];
        self.stage.positions.insert(process.id, target);
        self.bodies.push(Body {
            index,
            position: drawn,
            grow,
        });
        let kids = &tree.children[index];
        if depth == MAX_DEPTH || kids.is_empty() {
            return;
        }
        let (e, omega) = elements(process.id.pid);
        let sizes: Vec<f32> = kids.iter().map(|&child| tree.extents[child]).collect();
        let (slots, _) = shells(tree.reach[index], &sizes, e);
        for (&child, (a, phase)) in kids.iter().zip(slots) {
            let orbit = Orbit {
                center: target,
                a,
                e,
                omega,
            };
            let mean = phase + TAU * self.motion / period(a, tree.masses[index]);
            let mut settled = orbit.at(mean);
            settled[2] = target[2] + 0.2;
            let child_process = tree.processes[child];
            let child_grow = self.stage.growth(child_process.id);
            let position = [
                drawn[0] + (settled[0] - target[0]) * child_grow,
                drawn[1] + (settled[1] - target[1]) * child_grow,
                drawn[2] + 0.2,
            ];
            self.paths.push(Path {
                orbit: Orbit {
                    center: drawn,
                    ..orbit
                },
                depth,
                id: child_process.id,
                spoke: depth + 1 < MAX_DEPTH && !tree.children[child].is_empty(),
                mean,
                position,
                cpu: child_process.cpu,
            });
            self.branch(child, settled, position, child_grow, depth + 1);
        }
    }
}

/// One hub per parent that isotop can see but not read (a shadow) with readable leaves
/// directly under it, so those leaves orbit one hollow star, as they would orbit the parent,
/// instead of each standing alone as a system of its own: without sudo on macOS, launchd is
/// such a parent to hundreds of daemons. A hub is a process of no CPU, memory or threads,
/// its own parent so it is always a root, in pid order. Branches under it become systems of
/// their own as under init, so a shadow with no leaves gets no hub, and neither does a
/// focused subtree, whose root stands alone by choice.
pub(super) fn hubs(
    processes: &[&Process],
    snapshot: &Snapshot,
    focus: Option<Identity>,
) -> Vec<Process> {
    if snapshot.shadows.is_empty() || focus.is_some() {
        return Vec::new();
    }
    let present: HashSet<u32> = processes.iter().map(|p| p.id.pid).collect();
    let parents: HashSet<u32> = processes
        .iter()
        .filter(|p| p.parent != p.id.pid)
        .map(|p| p.parent)
        .collect();
    let orphaned: HashSet<u32> = processes
        .iter()
        .filter(|p| !present.contains(&p.parent) && !parents.contains(&p.id.pid))
        .map(|p| p.parent)
        .collect();
    let mut hubs: Vec<Process> = snapshot
        .shadows
        .iter()
        .filter(|shadow| orphaned.contains(&shadow.pid) && !present.contains(&shadow.pid))
        .map(hub)
        .collect();
    hubs.sort_by_key(|hub| hub.id);
    hubs.dedup_by_key(|hub| hub.id);
    hubs
}

/// The roots of the systems the Orbit and Flow layouts would make with every sampled process
/// drawn and nothing focused, hubs included, by the rules of `Scene::layout` and `Scene::hubs`.
fn whole_roots(snapshot: &Snapshot) -> HashSet<Identity> {
    let processes = &snapshot.processes;
    let by_pid: HashMap<u32, usize> = processes
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.pid, i))
        .collect();
    let parents: Vec<Option<usize>> = processes
        .iter()
        .enumerate()
        .map(|(i, p)| by_pid.get(&p.parent).copied().filter(|&parent| parent != i))
        .collect();
    let mut child_counts = vec![0; processes.len()];
    for &parent in parents.iter().flatten() {
        child_counts[parent] += 1;
    }
    let shadows: HashSet<u32> = snapshot
        .shadows
        .iter()
        .map(|shadow| shadow.pid)
        .filter(|pid| !by_pid.contains_key(pid))
        .collect();
    let mut roots = HashSet::new();
    for (i, process) in processes.iter().enumerate() {
        let leaf = child_counts[i] == 0;
        match parents[i] {
            Some(parent) if leaf || parents[parent].is_some() => {}
            // A leaf under a shadow orbits the shadow's hub, which is the root instead.
            None if leaf
                && process.parent != process.id.pid
                && shadows.contains(&process.parent) =>
            {
                roots.insert(Identity {
                    pid: process.parent,
                    start: HUB_START,
                });
            }
            _ => {
                roots.insert(process.id);
            }
        }
    }
    roots
}

/// Whether an identity is a hub's rather than a process's.
pub(crate) fn is_hub(id: &Identity) -> bool {
    id.start == HUB_START
}

/// The stand-in for a parent isotop cannot read, with nothing measured.
fn hub(shadow: &Shadow) -> Process {
    Process {
        parent: shadow.pid,
        name: shadow.name.clone(),
        ..Process::placeholder(Identity {
            pid: shadow.pid,
            start: HUB_START,
        })
    }
}

/// What the Orbit and Flow legends add while the frame draws a hub; nothing otherwise, so a
/// focused subtree or a platform that reads every process gets the legend it always had.
pub(super) fn hub_note(hubs: bool) -> &'static str {
    if hubs {
        " | hollow star = a parent isotop can't read (run with sudo)"
    } else {
        ""
    }
}

/// Thread rings: none below 4 threads, then one more ring per fourfold increase, up to four.
pub(super) fn rings(threads: u32) -> usize {
    match threads {
        0..=3 => 0,
        4..=15 => 1,
        16..=63 => 2,
        64..=255 => 3,
        _ => 4,
    }
}

fn ring_scale(threads: u32) -> f32 {
    match rings(threads) {
        0 => 1.0,
        count => 1.55 + 0.28 * (count - 1) as f32,
    }
}

/// Total memory and process count of each subtree, over every depth including collapsed ones.
fn weigh(
    processes: &[&Process],
    children: &[Vec<usize>],
    index: usize,
    masses: &mut [f32],
    members: &mut [usize],
    weighed: &mut HashSet<usize>,
) -> (f32, usize) {
    if !weighed.insert(index) {
        return (0.0, 0);
    }
    let mut total = (processes[index].memory as f32, 1);
    for &child in &children[index] {
        let (mass, count) = weigh(processes, children, child, masses, members, weighed);
        total.0 += mass;
        total.1 += count;
    }
    masses[index] = total.0;
    members[index] = total.1;
    total
}

fn arc(extent: f32) -> f32 {
    2.0 * extent + ORBIT_GAP
}

/// Eccentricity and periapsis direction shared by all of one parent's shells. Confocal ellipses
/// with the same shape are scaled copies of each other, so neighbouring shells never cross.
fn elements(pid: u32) -> (f32, f32) {
    // pid only: kept for pixel stability, see A33
    let hash = hash::knuth(pid);
    let e = 0.06 + 0.16 * (hash >> 24) as f32 / 255.0;
    let omega = TAU * ((hash >> 8) & 0xffff) as f32 / 65536.0;
    (e, omega)
}

/// Orbital period in seconds from Kepler's third law, T ~ sqrt(a^3 / M): wider orbits are slower
/// and heavier parents (more memory in their tree) pull their children round faster. Normalised
/// to 30 s at a = 3 around 512 MiB and clamped to a watchable range.
fn period(a: f32, parent_bytes: f32) -> f32 {
    let mass = (parent_bytes / (512.0 * 1048576.0)).max(1.0 / 512.0);
    (30.0 * (a / 3.0).powf(1.5) / mass.sqrt()).clamp(8.0, 600.0)
}

/// Longest motion step, in seconds, that the galaxy turns through between two frames.
const MAX_TURN_STEP: f32 = 2.0;

/// Seconds for a system to circle the galaxy once at `radius` world units with `enclosed` bytes
/// of systems inside its band: Kepler's T ~ sqrt(r^3 / M), normalised to four minutes at 30
/// units around 4 GiB and clamped so the galaxy always turns slowly next to its planets.
fn galactic_period(radius: f32, enclosed: f32) -> f32 {
    let mass = (enclosed / (4.0 * 1073741824.0)).max(1.0 / 4096.0);
    (240.0 * (radius.max(1.0) / 30.0).powf(1.5) / mass.sqrt()).clamp(120.0, 900.0)
}

/// Solves Kepler's equation M = E - e sin E for the eccentric anomaly E by Newton's method.
fn eccentric_anomaly(mean: f32, e: f32) -> f32 {
    let mean = mean.rem_euclid(TAU);
    let mut anomaly = mean;
    for _ in 0..6 {
        anomaly -= (anomaly - e * anomaly.sin() - mean) / (1.0 - e * anomaly.cos());
    }
    anomaly
}

/// Packs siblings into concentric confocal shells. Bodies on a shell share one ellipse and are
/// spaced evenly in mean anomaly, so they keep their order as Kepler motion bunches them near
/// apoapsis; capacity uses that slowest stretch. Returns (semi-major axis, starting mean anomaly)
/// per sibling and how far the system reaches from its centre.
fn shells(body: f32, sizes: &[f32], e: f32) -> (Vec<(f32, f32)>, f32) {
    let squeeze = ((1.0 - e) / (1.0 + e)).sqrt();
    let mut widest_after = vec![0.0_f32; sizes.len() + 1];
    for i in (0..sizes.len()).rev() {
        widest_after[i] = widest_after[i + 1].max(sizes[i]);
    }
    let mut slots = Vec::with_capacity(sizes.len());
    let mut a = (body + widest_after[0] + ORBIT_GAP + 1.0) / (1.0 - e);
    let mut reach = 0.0_f32;
    let mut start = 0;
    let mut shell = 0;
    while start < sizes.len() {
        let capacity = TAU * a * squeeze;
        let mut end = start;
        let mut used = 0.0;
        while end < sizes.len() && (end == start || used + arc(sizes[end]) <= capacity) {
            used += arc(sizes[end]);
            end += 1;
        }
        let slack = (capacity - used).max(0.0) / (end - start) as f32;
        let mut mean = shell as f32 * 0.5;
        let mut widest = 0.0_f32;
        for &size in &sizes[start..end] {
            let share = (arc(size) + slack) / (a * squeeze);
            slots.push((a, mean + share * 0.5));
            mean += share;
            widest = widest.max(size);
        }
        reach = reach.max(a * (1.0 + e) + widest);
        start = end;
        if start < sizes.len() {
            a += (widest + widest_after[start] + ORBIT_GAP) / (1.0 - e);
        }
        shell += 1;
    }
    (slots, reach)
}

fn measure(
    children: &[Vec<usize>],
    reach: &[f32],
    processes: &[&Process],
    index: usize,
    depth: usize,
    extents: &mut [f32],
    measured: &mut HashSet<usize>,
) -> f32 {
    if !measured.insert(index) {
        return 0.0;
    }
    let extent = if depth >= MAX_DEPTH || children[index].is_empty() {
        reach[index]
    } else {
        let sizes: Vec<f32> = children[index]
            .iter()
            .map(|&child| {
                measure(
                    children,
                    reach,
                    processes,
                    child,
                    depth + 1,
                    extents,
                    measured,
                )
            })
            .collect();
        shells(reach[index], &sizes, elements(processes[index].id.pid).0).1
    };
    extents[index] = extent;
    extent
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::harness::{churned, rescaled};
    use crate::model::demo;
    use crate::model::testing::process;
    use crate::render::{Camera, Frame, NONE, RenderInput, Scene, hiding};
    use crate::views::flow::Wells;
    use crate::views::orbit::Orbits;
    use crate::views::{View, view};

    fn render(scene: &mut Scene, snapshot: &Snapshot, view: View, time: f32) -> Frame {
        let mut frame = scene.render(&RenderInput::small(
            snapshot,
            view,
            &Camera::default(),
            time,
        ));
        frame.rasterize();
        frame
    }

    /// Where the systems of the Orbit or Flow view `view` stand in `scene`.
    fn galaxy_of(scene: &Scene, view: View) -> &Galaxy {
        scene
            .module::<Orbits>(view)
            .map(Orbits::galaxy)
            .or_else(|| scene.module::<Wells>(view).map(Wells::galaxy))
            .expect("an Orbit or Flow view that has drawn")
    }

    #[test]
    fn kepler_solution_and_shells_keep_bodies_apart() {
        for &(mean, e) in &[(0.3_f32, 0.1_f32), (2.5, 0.22), (5.9, 0.06)] {
            let anomaly = eccentric_anomaly(mean, e);
            assert!((anomaly - e * anomaly.sin() - mean).abs() < 1e-4);
        }
        let orbit = Orbit {
            center: [0.0; 3],
            a: 10.0,
            e: 0.2,
            omega: 0.0,
        };
        assert!(
            (orbit.at(0.0)[0] - 8.0).abs() < 1e-3,
            "periapsis is a(1 - e)"
        );
        let (slots, reach) = shells(0.5, &[0.3; 200], 0.2);
        assert_eq!(slots.len(), 200);
        let mut axes: Vec<f32> = slots.iter().map(|s| s.0).collect();
        axes.dedup();
        assert!(axes.len() > 1, "200 siblings overflow into outer shells");
        assert!(axes[0] * 0.8 >= 0.5 + 0.3 + ORBIT_GAP + 1.0 - 1e-4);
        for pair in axes.windows(2) {
            assert!((pair[1] - pair[0]) * 0.8 >= 0.6 + ORBIT_GAP - 1e-4);
        }
        assert!(reach >= axes[axes.len() - 1] * 1.2);
        assert!(period(6.0, 8.0 * 1073741824.0) < period(6.0, 64.0 * 1048576.0));
    }

    #[test]
    fn sizes_span_memory_and_stars_carry_their_systems_mass() {
        let mib = |m: f32| m * 1048576.0;
        let small = mass_radius(mib(6.0));
        let large = mass_radius(mib(1024.0));
        assert!(large / small > 5.0, "{small} -> {large}");
        assert_eq!(mass_radius(0.0), 0.25);
        assert_eq!((rings(3), rings(4), rings(16), rings(1000)), (0, 1, 2, 4));
        let mut processes = vec![process(1, 0), process(10, 1)];
        processes[1].memory = 2 << 30;
        processes.push(process(11, 10));
        let snapshot = Snapshot {
            processes,
            ..Default::default()
        };
        let id = snapshot.processes[1].id;
        let line = format!(
            "system {} {} across 2 processes",
            platform::MEMORY_LABEL,
            bytes((2 << 30) + (1 << 20))
        );
        for view in [view("orbit"), view("flow")] {
            let mut scene = Scene::new();
            render(&mut scene, &snapshot, view, 0.0);
            assert_eq!(
                scene
                    .popup
                    .iter()
                    .filter(|(who, _)| *who == id)
                    .collect::<Vec<_>>(),
                [&(id, line.clone())],
                "{view:?}: the star carries its system's memory, once"
            );
        }
    }

    #[test]
    fn an_unreadable_parent_becomes_one_hollow_hub_instead_of_many_systems() {
        let launchd = Shadow {
            pid: 1,
            parent: 0,
            name: "launchd".into(),
        };
        let leaves = Snapshot {
            processes: (10..30).map(|pid| process(pid, 1)).collect(),
            cores: 4,
            ..Default::default()
        };
        let mut scene = Scene::new();
        render(&mut scene, &leaves, view("orbit"), 0.0);
        assert_eq!(
            scene.titles.len(),
            20,
            "without a shadow each leaf stands alone"
        );
        assert_eq!(scene.tour.len(), 20, "and each is a stop of the tour");
        assert!(!scene.legend(view("orbit"), &leaves).contains("hollow star"));
        let shadowed = Snapshot {
            shadows: vec![launchd],
            ..leaves.clone()
        };
        let star = Identity {
            pid: 1,
            start: HUB_START,
        };
        assert!(is_hub(&star));
        for view in [view("orbit"), view("flow")] {
            let mut scene = Scene::new();
            render(&mut scene, &shadowed, view, 0.0);
            let [title] = scene.titles.as_slice() else {
                panic!("{view:?}: {:?}", scene.titles.len());
            };
            assert_eq!(title.text, "launchd (unreadable) (20)", "{view:?}");
            assert_eq!(scene.positions[&star][..2], title.at[..2], "{view:?}");
            assert!(
                scene.tour.is_empty(),
                "{view:?}: a hub has no callout, so the tour passes it over"
            );
            assert!(
                scene
                    .legend(view, &shadowed)
                    .ends_with(" | hollow star = a parent isotop can't read (run with sudo)"),
                "{view:?}"
            );
            assert_eq!(scene.visible, 20, "{view:?}: the hub is not a process");
            let memory = format!(
                "system {} {} across 21 processes",
                platform::MEMORY_LABEL,
                bytes(20 << 20)
            );
            assert_eq!(scene.popup, [(star, memory)], "{view:?}");
            let mut camera = Camera::default();
            scene.fit(&mut camera, 320, 180);
            let mut frame = scene.render(&RenderInput::small(&shadowed, view, &camera, 1.0));
            frame.rasterize();
            assert_eq!(frame.identities.len(), 20);
            assert!(
                frame
                    .picks
                    .iter()
                    .all(|&p| p == NONE || frame.target(p).is_some())
            );
            let [x, y] = frame
                .locate(&camera, scene.positions[&star])
                .expect("the hub is on screen");
            let at = y as usize * 320 + x as usize;
            assert_eq!(frame.picks[at], NONE, "{view:?}: the hub is hollow");
            assert_eq!(frame.pick_near(x, y, 2.0), None, "{view:?}");
        }
        let mut focused = Scene::new();
        focused.render(&RenderInput {
            focus: Some(shadowed.processes[0].id),
            ..RenderInput::small(&shadowed, view("orbit"), &Camera::default(), 0.0)
        });
        assert!(
            focused
                .titles
                .iter()
                .all(|title| !title.text.contains("(unreadable)")),
            "a focused root stands alone"
        );
        assert!(
            !focused
                .legend(view("orbit"), &shadowed)
                .contains("hollow star")
        );
    }

    #[test]
    fn overlapping_rings_turn_together_and_detached_systems_turn_slower() {
        let id = |pid: u32| Identity { pid, start: 1 };
        let mut galaxy = Galaxy {
            systems: HashMap::from([
                (id(1), ([0.0, 0.0], 5.0)),
                (id(2), ([8.5, 0.0], 3.0)),
                (id(3), ([60.0, 0.0], 3.0)),
            ]),
            ..Galaxy::default()
        };
        let masses = HashMap::from([(id(1), 4e9), (id(2), 1e9), (id(3), 1e8)]);
        let mut frames = 1;
        galaxy.revolve(&masses, 0.0, frames);
        let speed = |galaxy: &Galaxy, pid| galaxy.speed[&id(pid)];
        assert_eq!(
            speed(&galaxy, 1),
            speed(&galaxy, 2),
            "touching rings share a band"
        );
        assert!(
            speed(&galaxy, 3) < speed(&galaxy, 2),
            "outer bands turn slower"
        );
        assert!(TAU / speed(&galaxy, 3) <= 900.0 && TAU / speed(&galaxy, 1) >= 120.0);
        for step in 1..=2000 {
            frames += 1;
            galaxy.revolve(&masses, step as f32 * 0.5, frames);
        }
        let [(a, ra), (b, rb), (c, rc)] = [1, 2, 3].map(|pid| galaxy.systems[&id(pid)]);
        let gap = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
        assert!((gap(a, b) - 8.5).abs() < 0.01, "a band turns rigidly");
        assert!(gap(a, c) >= ra + rc && gap(b, c) >= rb + rc);
        assert!(c[1].abs() > 1.0, "the detached system moved round");
        frames += 1;
        galaxy.revolve(&masses, 5000.0, frames);
        assert_eq!(
            galaxy.systems[&id(3)].0,
            c,
            "a jump in time does not spin the galaxy"
        );
    }

    #[test]
    fn the_galaxy_turns_at_one_frame_per_second_but_not_across_view_switches() {
        let id = |pid: u32| Identity { pid, start: 1 };
        let mut galaxy = Galaxy {
            systems: HashMap::from([(id(1), ([0.0, 0.0], 5.0)), (id(2), ([40.0, 0.0], 3.0))]),
            ..Galaxy::default()
        };
        let masses = HashMap::from([(id(1), 4e9), (id(2), 1e8)]);
        let (mut frames, mut motion) = (0, 0.0);
        let mut turn = |galaxy: &mut Galaxy, drawn: u64, seconds: f32| {
            frames += drawn;
            motion += seconds;
            galaxy.revolve(&masses, motion, frames);
            galaxy.systems[&id(2)].0
        };
        turn(&mut galaxy, 1, 0.0);
        let start = turn(&mut galaxy, 1, 1.0);
        let next = turn(&mut galaxy, 1, 1.0);
        assert_ne!(
            start, next,
            "--fps 1 frames are a second apart and still turn it"
        );
        let away = turn(&mut galaxy, 3, 0.3);
        assert_eq!(
            away, next,
            "frames drawn in another view are skipped, however short"
        );
        // A light band close in and a heavy one further out, about a pivot that stays put.
        let mut galaxy = Galaxy {
            systems: HashMap::from([(id(1), ([20.0, 0.0], 3.0)), (id(2), ([60.0, 0.0], 3.0))]),
            members: vec![id(1), id(2)],
            ..Galaxy::default()
        };
        galaxy.revolve(&HashMap::from([(id(1), 1e6), (id(2), 64e9)]), 0.0, 0);
        assert!(
            galaxy.speed[&id(2)] <= galaxy.speed[&id(1)],
            "outer bands never outpace inner ones"
        );
    }

    /// A system is not placed again when the memory and CPU of its processes change by the
    /// factors of the view harness (memory by half to double, CPU by a quarter to fourfold), or
    /// when a tenth of the processes leave and as many arrive: `HEADROOM` is what keeps a
    /// change of resources from moving whole systems. The galaxy still turns, at a pace that
    /// follows the memory, about a pivot that stays put: so what is kept is each reservation,
    /// the pivot and each system's distance from it. The scene eases a change in over a third
    /// of a second, so each run draws three seconds of frames after it.
    #[test]
    fn a_system_keeps_its_place_while_resources_and_the_population_change() {
        let sample = demo(30.0, 160);
        let run = |view: View, next: &Snapshot| {
            let mut scene = Scene::new();
            hiding::place(&mut scene, &sample, view, 30.0, 4096, None, None);
            for frame in 1..=12 {
                let time = 30.0 + frame as f32 * 0.25;
                hiding::place(&mut scene, next, view, time, 4096, None, None);
            }
            let galaxy = galaxy_of(&scene, view);
            (galaxy.systems.clone(), galaxy.center)
        };
        let changes = [
            ("rescaled", rescaled(&sample)),
            ("churned", churned(&sample).0),
        ];
        for view in [view("orbit"), view("flow")] {
            let (steady, center) = run(view, &sample);
            assert!(steady.len() > 1, "{view:?}");
            for (name, next) in &changes {
                let (changed, pivot) = run(view, next);
                assert_eq!(pivot, center, "{view:?} {name}: the pivot moved");
                assert_eq!(
                    changed.keys().collect::<BTreeSet<_>>(),
                    steady.keys().collect::<BTreeSet<_>>()
                );
                let distance = |at: [f32; 2]| (at[0] - center[0]).hypot(at[1] - center[1]);
                for (id, (at, reserved)) in &steady {
                    let (to, now) = changed[id];
                    assert_eq!(now, *reserved, "{view:?} {name}: {id:?} was placed again");
                    assert!(
                        (distance(to) - distance(*at)).abs() < 0.01,
                        "{view:?} {name}: {id:?} left its orbit about the pivot"
                    );
                }
            }
        }
    }

    /// The place a process keeps is its system's centre, where the star of the system stands.
    #[test]
    fn a_process_keeps_the_centre_of_its_system() {
        let sample = demo(30.0, 64);
        for view in [view("orbit"), view("flow")] {
            let mut scene = Scene::new();
            let places = hiding::place(&mut scene, &sample, view, 30.0, 4096, None, None);
            let homes = scene.anchors(view).expect("the view declares its anchors");
            assert_eq!(
                homes.keys().collect::<BTreeSet<_>>(),
                places.keys().collect::<BTreeSet<_>>(),
                "{view:?}: every drawn process has a home"
            );
            let galaxy = galaxy_of(&scene, view);
            for (star, ([x, y], _)) in &galaxy.systems {
                assert_eq!(homes[star], [*x, *y, 0.0], "{view:?}: the star is at home");
                assert_eq!(places[star][..2], [*x, *y], "{view:?}: and stands there");
            }
            let at: BTreeSet<[u32; 2]> = homes
                .values()
                .map(|home| [home[0].to_bits(), home[1].to_bits()])
                .collect();
            assert_eq!(
                at.len(),
                galaxy.systems.len(),
                "{view:?}: one home per system"
            );
        }
    }

    /// The systems of `scene` after each hiding in `hides` (limit, selection, focus), which
    /// must be exactly the whole tree's plus those the hiding itself makes drawn systems, as a
    /// fresh scene shows; lifting the hiding leaves the whole tree's and puts them back.
    fn systems_follow_the_hiding(
        view: View,
        hides: impl Fn(
            &Snapshot,
            &HashMap<Identity, Point>,
        ) -> Vec<(usize, Option<Identity>, Option<Identity>)>,
    ) {
        let sample = demo(30.0, 160);
        let keys = |scene: &Scene| -> BTreeSet<Identity> {
            galaxy_of(scene, view).systems.keys().copied().collect()
        };
        let (mut scene, time, before) = hiding::settled(view, &sample);
        let whole = keys(&scene);
        for (limit, selected, focus) in hides(&sample, &before) {
            hiding::place(&mut scene, &sample, view, time, limit, selected, focus);
            let mut fresh = Scene::new();
            hiding::place(&mut fresh, &sample, view, time, limit, selected, focus);
            let expected: BTreeSet<Identity> = whole.union(&keys(&fresh)).copied().collect();
            assert_eq!(
                keys(&scene),
                expected,
                "{view:?} {limit} {selected:?} {focus:?}"
            );
        }
        let lifted = hiding::place(&mut scene, &sample, view, time, 4096, None, None);
        assert_eq!(lifted, before, "{view:?}");
        assert_eq!(keys(&scene), whole, "{view:?}");
    }

    /// Fails if a hidden system is kept merely because its root is sampled: focusing each
    /// process in turn leaves one system behind for every root a focus made.
    #[test]
    fn focusing_one_process_after_another_leaves_no_orbit_or_flow_system_behind() {
        for view in [view("orbit"), view("flow")] {
            systems_follow_the_hiding(view, |sample, _| {
                sample
                    .processes
                    .iter()
                    .map(|process| (4096, None, Some(process.id)))
                    .collect()
            });
        }
    }

    /// Fails if a hidden system is kept merely because its root is sampled: a selection
    /// swapped across the limit is drawn as a system of its own, which it gives up on
    /// deselection, when it is beyond the limit and not a root of the whole tree.
    #[test]
    fn a_selection_swapped_across_the_limit_leaves_no_orbit_or_flow_system_behind() {
        for view in [view("orbit"), view("flow")] {
            systems_follow_the_hiding(view, |_, before| {
                let mut selected: Vec<_> = before.keys().filter(|id| id.pid >= 1080).collect();
                selected.sort();
                selected
                    .into_iter()
                    .flat_map(|&id| [(80, Some(id), None), (80, None, None)])
                    .collect()
            });
        }
    }
}
