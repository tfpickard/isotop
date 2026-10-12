//! Cells: every cgroup is a living cell in a petri dish, one dish each for system services, your
//! session and containers. A cell's size follows its memory; a dashed ring marks its memory limit
//! and an arc its CPU use against its quota. The membrane trembles under pressure, flashes red
//! when the quota throttles it and bursts when the OOM killer strikes. Processes are organelles.
//! The nucleus, at the centre, is the largest process of a new cell; it keeps the centre until it
//! exits, and a cell that has lost it gives it to the largest process to arrive.
//!
//! Nothing a measurement moves moves an organelle: each keeps its seat, and the seats are spread
//! for the room the cell reserved (`Room`), within which memory moves the membrane and processes
//! join or leave without moving a seat. The seats are spread over the smallest membrane memory
//! alone is expected to leave the cell (half its memory), so the membrane breathes between half
//! and double its memory without crossing a seat. The room is laid out again only when the
//! membrane grows past `SPARSE` times the room, or when so many processes join that the
//! outermost seat would stand outside the membrane.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, TAU};

use crate::layout::{Discs, Seats, spin, spiral};
use crate::model::{Identity, Kind, Process, Snapshot, Unit, bounded, bytes};
use crate::palette::{CONTAINER, SESSION, SYSTEM, kind_color, mix, tint};
use crate::platform::{GROUP_LABEL, Source, WITHOUT_CGROUPS};
use crate::render::{
    Color, NONE, Point, Stage, WARM, mass_radius, ring_bounds, without_apple_prefix,
};
use crate::simulation::sprout;
use crate::views::ViewModule;

const DISHES: [(Kind, &str, Color); 3] = [
    (Kind::System, "system services", SYSTEM),
    (Kind::Session, "your session", SESSION),
    (Kind::Container, "containers", CONTAINER),
];
const SEAT: f32 = 0.75;
/// The share of the smallest membrane its seats are spread over when its room is laid out.
const SPREAD: f32 = 0.8;
/// The smallest membrane the memory of a cell is expected to leave it, as a share of the radius
/// its room was laid out for. Half the memory is 0.71 of the radius at the least (it goes with
/// the square root of the memory), and a cgroup's memory, page cache and all, swings that much.
const SHRINK: f32 = 0.7;
/// How far inside the membrane the centre of the outermost organelle stays.
const MARGIN: f32 = 0.4;
/// How much its membrane may have grown over its room before a cell lays its seats out again,
/// so they do not huddle round the nucleus.
const SPARSE: f32 = 1.5;
/// Widest a memory limit ring is drawn, as a multiple of the cell radius.
const LIMIT: f32 = 1.6;
const NAMED: usize = 12;
const DISH_GAP: f32 = 4.0;
const LYSIS_SECONDS: f32 = 1.6;
const CRISIS: Color = [255, 96, 80];

#[derive(Default)]
pub struct Dishes {
    cells: [Discs; 3],
    /// Each dish's drawn radius, which only grows, and the dishes' own placement: a dish keeps
    /// its place until it outgrows the room reserved for it, and then only that dish moves.
    radii: [f32; 3],
    dishes: Discs,
    seats: Seats,
    /// The room each cell reserved for its seats, by dish and cgroup.
    rooms: HashMap<(usize, String), Room>,
    /// Each cell's nucleus, and who was in the cell at the last frame, by dish and cgroup: a cell
    /// without a nucleus gives it to the largest process that was not in the cell then.
    nuclei: HashMap<(usize, String), Identity>,
    members: HashMap<(usize, String), HashSet<Identity>>,
    /// OOM kill count last seen per cgroup, and when its latest burst began.
    kills: HashMap<String, (u64, f32)>,
}

/// What a cell reserves for its organelles: the membrane radius and the number of seats its
/// spiral was spread for. A seat is where the room puts it, whatever the membrane measures now.
#[derive(Clone, Copy, Debug)]
struct Room {
    radius: f32,
    seats: usize,
}

impl Room {
    /// The spacing of the seats' spiral: out over most of the smallest membrane the room expects
    /// rather than huddling round the nucleus, so the outermost seat stays inside the membrane
    /// when the memory falls to half.
    fn spacing(self) -> f32 {
        ((self.radius * SHRINK - MARGIN).max(0.0) * SPREAD / (self.seats as f32 + 1.0).sqrt())
            .max(SEAT)
    }

    /// Whether a cell with a membrane of `radius` and `span` seats in use still has this room: the
    /// outermost seat stands inside the membrane, and the membrane has not grown far past the
    /// room. Until it fails, nothing the cell measures within half to double its memory moves a
    /// seat, and a seat that joins goes on outwards from the last.
    fn serves(self, radius: f32, span: usize) -> bool {
        self.spacing() * (span as f32).sqrt() + MARGIN <= radius && radius <= self.radius * SPARSE
    }
}

/// A sampled process with the CPU and memory it is measured by.
#[derive(Clone, Copy)]
struct Sampled<'a> {
    process: &'a Process,
    cpu: f32,
    memory: u64,
}

struct Cell<'a> {
    name: String,
    /// The drawn members, as indices into the drawn processes.
    members: Vec<usize>,
    /// How many processes the cell has and what they use between them, drawn or not.
    population: usize,
    cpu: f32,
    /// The nucleus, drawn or not, if the cell has one, and its place among the drawn ones if it is
    /// drawn.
    nucleus: Option<Identity>,
    nucleus_memory: u64,
    nucleus_index: Option<usize>,
    unit: Option<&'a Unit>,
    memory: u64,
    radius: f32,
    /// Where its seats are: the room reserved for them.
    room: Room,
    /// Only the largest cells in a dish are named, so labels don't bury the dish.
    named: bool,
}

impl ViewModule for Dishes {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let time = stage.time;
        // Seats, cell sizes and places are reserved for every sampled process, drawn or not, so
        // focusing a subtree, or `--limit` hiding some, neither frees their seats nor drops their
        // cells: clearing it puts everything back where it was. Only drawn processes are drawn.
        // Every sampled process is measured as the scene eases it, so hiding one never changes
        // the figures that size and rank the cells.
        let drawn: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(index, process)| (process.id, index))
            .collect();
        let sampled: Vec<Sampled> = snapshot
            .processes
            .iter()
            .map(|process| {
                let (cpu, memory) = stage.eased(process);
                Sampled {
                    process,
                    cpu,
                    memory,
                }
            })
            .collect();
        self.seats.assign(
            sampled
                .iter()
                .filter(|s| s.process.kind != Kind::Kernel)
                .map(|s| (s.process.id, cell_name(s.process))),
        );
        let mut cells: [Vec<Cell>; 3] = Default::default();
        let mut nuclei: HashMap<(usize, String), Identity> = HashMap::new();
        let mut members: HashMap<(usize, String), HashSet<Identity>> = HashMap::new();
        for (dish, (kind, _, _)) in DISHES.iter().enumerate() {
            let mut groups: BTreeMap<&str, Vec<Sampled>> = BTreeMap::new();
            for &s in sampled.iter().filter(|s| s.process.kind == *kind) {
                groups.entry(cell_name(s.process)).or_default().push(s);
            }
            for (name, everyone) in groups {
                let unit = snapshot.units.get(name);
                let memory = unit.map_or_else(
                    || everyone.iter().map(|s| s.memory).sum(),
                    |unit| unit.memory,
                );
                let key = (dish, name.to_owned());
                let nucleus = self.elect(&key, &everyone);
                members.insert(key.clone(), everyone.iter().map(|s| s.process.id).collect());
                if let Some(nucleus) = nucleus {
                    nuclei.insert(key, nucleus.process.id);
                }
                let mut members: Vec<usize> = everyone
                    .iter()
                    .filter_map(|s| drawn.get(&s.process.id).copied())
                    .collect();
                members.sort_unstable();
                let span = self.seats.span(name);
                let by_memory = 1.3 + 0.16 * (memory as f32 / 1048576.0).sqrt();
                let by_members = SEAT * (span as f32 + 1.0).sqrt() + 1.0;
                let radius = by_memory.max(by_members);
                cells[dish].push(Cell {
                    name: name.to_owned(),
                    members,
                    population: everyone.len(),
                    cpu: everyone.iter().map(|s| s.cpu).sum(),
                    nucleus: nucleus.map(|s| s.process.id),
                    nucleus_memory: nucleus.map_or(0, |s| s.memory),
                    nucleus_index: nucleus.and_then(|s| drawn.get(&s.process.id).copied()),
                    unit,
                    memory,
                    radius,
                    room: Room {
                        radius,
                        seats: span,
                    },
                    named: false,
                });
            }
            name_the_largest(&mut cells[dish]);
        }
        self.nuclei = nuclei;
        self.members = members;
        self.reserve(&mut cells);
        let mut offsets: [HashMap<String, [f32; 2]>; 3] = Default::default();
        for dish in 0..3 {
            // Room for the limit ring and CPU gauge drawn outside each membrane.
            let needs: Vec<(String, f32)> = cells[dish]
                .iter()
                .map(|c| (c.name.clone(), c.radius * LIMIT + 0.6))
                .collect();
            offsets[dish] = self.cells[dish].arrange(&needs);
            let needed = self.cells[dish].reach() + 1.0;
            if needed > self.radii[dish] {
                self.radii[dish] = needed * 1.05;
            }
        }
        let needs: Vec<(String, f32)> = DISHES
            .iter()
            .zip(self.radii)
            .map(|(&(_, title, _), radius)| (title.to_owned(), radius + DISH_GAP * 0.5))
            .collect();
        let placed = self.dishes.arrange(&needs);
        let centers: [[f32; 2]; 3] = DISHES.map(|(_, title, _)| placed[title]);
        self.watch_kills(&cells, time);
        let camera = stage.camera;
        let mut shown = 0;
        for (dish, &(_, title, color)) in DISHES.iter().enumerate() {
            let [cx, cy] = centers[dish];
            let r = self.radii[dish];
            let at = |angle: f32, radius: f32, z: f32| -> Point {
                [cx + radius * angle.cos(), cy + radius * angle.sin(), z]
            };
            for k in 0..96 {
                let (a, b) = (k as f32 / 96.0 * TAU, (k + 1) as f32 / 96.0 * TAU);
                stage.frame.facet(
                    camera,
                    [[cx, cy, -0.05], at(a, r, -0.05), at(b, r, -0.05)],
                    [17, 28, 36],
                    NONE,
                );
            }
            for (radius, shade) in [(r, [110, 140, 150]), (r - 0.3, [58, 78, 88])] {
                for k in 0..96 {
                    let (a, b) = (k as f32 / 96.0 * TAU, (k + 1) as f32 / 96.0 * TAU);
                    stage
                        .frame
                        .line(camera, at(a, radius, 0.3), at(b, radius, 0.3), shade);
                }
            }
            let shown_cells = || cells[dish].iter().filter(|c| !c.members.is_empty());
            let count = shown_cells().map(|c| c.members.len()).sum::<usize>();
            stage.places.push((
                camera.front([cx, cy, 0.0], r + 1.5),
                format!(
                    "{title}: {} cells, {count} processes",
                    shown_cells().count()
                ),
            ));
            for cell in shown_cells() {
                let Some(&[ox, oy]) = offsets[dish].get(&cell.name) else {
                    continue;
                };
                let center = [cx + ox, cy + oy];
                shown += self.draw_cell(stage, cell, center, color, processes, time);
            }
        }
        let kernel = processes.iter().filter(|p| p.kind == Kind::Kernel).count();
        if kernel > 0 {
            let [cx, cy] = centers[0];
            stage.places.push((
                stage.camera.front([cx, cy, 0.0], self.radii[0] + 3.0),
                format!("{kernel} kernel threads live outside any cell"),
            ));
        }
        for (center, radius) in centers.iter().zip(self.radii) {
            stage.bounds.extend(ring_bounds(*center, radius, 0.0, 1.5));
        }
        shown
    }

    /// The status legend, saying what the snapshot's platform leaves out.
    fn legend(&self, snapshot: &Snapshot) -> String {
        if snapshot.missing.contains(Source::Cgroups) {
            format!(
                " Cell = {GROUP_LABEL}, size = memory | {WITHOUT_CGROUPS}, so no limits, quotas or pressure | organelles = processes"
            )
        } else {
            " Cell = cgroup, size = memory | dashed ring = memory limit | arc = CPU vs quota | trembling = pressure | red = throttled | burst = OOM kill | organelles = processes".into()
        }
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

impl Dishes {
    /// The nucleus of the cell `key` among `everyone` in it, sampled, drawn or not: the one it
    /// has, or else the largest process that was not in the cell at the last frame (all of them,
    /// for a cell that has just appeared). A cell whose nucleus exited and that gained nobody has
    /// none, and no seated organelle moves to the centre.
    fn elect<'a>(&self, key: &(usize, String), everyone: &[Sampled<'a>]) -> Option<Sampled<'a>> {
        let kept = self.nuclei.get(key);
        if let Some(nucleus) = everyone.iter().find(|s| Some(&s.process.id) == kept) {
            return Some(*nucleus);
        }
        let before = self.members.get(key);
        everyone
            .iter()
            .copied()
            .filter(|s| before.is_none_or(|before| !before.contains(&s.process.id)))
            .max_by_key(|s| (s.memory, Reverse(s.process.id)))
    }

    /// Keeps each cell's room, or lays it out again for the cell as it is now when the cell has
    /// outgrown it. Rooms of cells that are gone are dropped.
    fn reserve(&mut self, cells: &mut [Vec<Cell>; 3]) {
        let mut rooms = HashMap::new();
        for (dish, cells) in cells.iter_mut().enumerate() {
            for cell in cells {
                let key = (dish, cell.name.clone());
                let span = self.seats.span(&cell.name);
                if let Some(&room) = self.rooms.get(&key)
                    && room.serves(cell.radius, span)
                {
                    cell.room = room;
                }
                rooms.insert(key, cell.room);
            }
        }
        self.rooms = rooms;
    }

    /// Remembers when each cell's OOM kill count last rose. Every sampled cell is watched, shown
    /// or not, so a kill that happens while its cell is hidden does not burst when it comes back.
    fn watch_kills(&mut self, cells: &[Vec<Cell>; 3], time: f32) {
        let present: HashSet<&str> = cells.iter().flatten().map(|c| c.name.as_str()).collect();
        self.kills.retain(|name, _| present.contains(name.as_str()));
        for cell in cells.iter().flatten() {
            // An unreadable count changes nothing: the last known count stays the one to compare to.
            let Some(kills) = cell.unit.and_then(|unit| unit.oom_kills) else {
                continue;
            };
            let seen = self
                .kills
                .entry(cell.name.clone())
                .or_insert((kills, f32::NEG_INFINITY));
            if kills > seen.0 {
                *seen = (kills, time);
            }
        }
    }

    #[expect(clippy::too_many_lines, reason = "split in #71 (A12)")]
    fn draw_cell(
        &self,
        stage: &mut Stage,
        cell: &Cell,
        [x, y]: [f32; 2],
        color: Color,
        processes: &[&Process],
        time: f32,
    ) -> usize {
        let camera = stage.camera;
        let r = cell.radius;
        let at = |angle: f32, radius: f32, z: f32| -> Point {
            [x + radius * angle.cos(), y + radius * angle.sin(), z]
        };
        let pick = cell.nucleus_index.map_or(NONE, |index| index as u32);
        let unit = cell.unit.cloned().unwrap_or_default();
        let pressure = unit
            .pressure
            .iter()
            .flatten()
            .fold(0.0_f32, |a, &b| a.max(b));
        let phase = spin(&cell.name);
        let wobble = 0.03 + 0.22 * bounded(pressure, 15.0);
        let edge = |angle: f32| {
            r * (1.0
                + wobble * (5.0 * angle + time * 2.2 + phase).sin()
                + wobble * 0.5 * (3.0 * angle - time * 1.7).sin())
        };
        let cytoplasm = tint(color, 0.16 + 0.14 * bounded(unit.cpu.unwrap_or(0.0), 50.0));
        for k in 0..64 {
            let (a, b) = (k as f32 / 64.0 * TAU, (k + 1) as f32 / 64.0 * TAU);
            stage.frame.facet(
                camera,
                [[x, y, 0.05], at(a, edge(a), 0.05), at(b, edge(b), 0.05)],
                cytoplasm,
                pick,
            );
        }
        let flash = if unit.throttled == Some(true) {
            0.5 + 0.5 * (time * 12.0).sin()
        } else {
            0.0
        };
        let membrane = mix(tint(color, 0.95), CRISIS, flash);
        for inset in [0.0, 0.07] {
            for k in 0..64 {
                let (a, b) = (k as f32 / 64.0 * TAU, (k + 1) as f32 / 64.0 * TAU);
                stage.frame.line(
                    camera,
                    at(a, edge(a) - inset, 0.12),
                    at(b, edge(b) - inset, 0.12),
                    membrane,
                );
            }
        }
        if let Some(max) = unit
            .memory_max
            .filter(|&max| max > 0 && cell.memory * 4 >= max)
        {
            let used = cell.memory as f32 / max as f32;
            let limit = (r * (1.0 / used).sqrt()).clamp(r * 1.04, r * LIMIT);
            let shade = mix(
                [236, 190, 110],
                CRISIS,
                ((used - 0.6) / 0.4).clamp(0.0, 1.0),
            );
            for k in (0..72).step_by(2) {
                let (a, b) = (k as f32 / 72.0 * TAU, (k + 1) as f32 / 72.0 * TAU);
                stage
                    .frame
                    .line(camera, at(a, limit, 0.1), at(b, limit, 0.1), shade);
            }
        }
        let share = unit.cpu.unwrap_or(0.0) / unit.cpu_max.unwrap_or(100.0);
        if share > 0.01 {
            let sweep = share.min(1.0) * TAU;
            let gauge = if unit.cpu_max.is_some() && share > 0.9 {
                CRISIS
            } else {
                WARM
            };
            let steps = (sweep / TAU * 64.0).ceil().max(1.0) as usize;
            for k in 0..steps {
                let a = FRAC_PI_2 - sweep * k as f32 / steps as f32;
                let b = FRAC_PI_2 - sweep * (k + 1) as f32 / steps as f32;
                stage
                    .frame
                    .line(camera, at(a, r + 0.35, 0.15), at(b, r + 0.35, 0.15), gauge);
            }
        }
        let age = self
            .kills
            .get(&cell.name)
            .map_or(f32::INFINITY, |&(_, at)| (time - at) / LYSIS_SECONDS);
        if (0.0..1.0).contains(&age) {
            for k in 0..24 {
                let angle = k as f32 / 24.0 * TAU + phase;
                let p = at(angle, r * (1.0 + 1.5 * age), 0.3);
                stage
                    .frame
                    .glow(camera, p, (0.5 * camera.zoom).max(5.0), CRISIS, 1.0 - age);
            }
        }
        let mut drawn = 0;
        let spacing = cell.room.spacing();
        let core = mass_radius(cell.nucleus_memory as f32).clamp(0.45, r * 0.32);
        if cell.nucleus_index.is_some() {
            stage.frame.glow(
                camera,
                [x, y, 0.4],
                core * camera.zoom * 3.0,
                tint(color, 1.2),
                0.35,
            );
        }
        for &index in &cell.members {
            let process = processes[index];
            let is_nucleus = Some(process.id) == cell.nucleus;
            let (position, size) = if is_nucleus {
                ([x, y, 0.4], core)
            } else {
                let seat = self.seats.seat(process.id).unwrap_or(0) as f32;
                let (distance, angle) = spiral(spacing, 1.0, seat, phase);
                let position = at(angle, distance.min(r - MARGIN), 0.3);
                stage
                    .frame
                    .line(camera, [x, y, 0.2], position, tint(color, 0.45));
                (
                    position,
                    (0.8 * mass_radius(process.memory as f32)).min(spacing * 0.42),
                )
            };
            let grown = size * sprout(stage.growth(process.id));
            if process.cpu > 1.0 {
                stage.frame.glow(
                    camera,
                    position,
                    grown * camera.zoom * (2.0 + 2.0 * bounded(process.cpu, 50.0)),
                    WARM,
                    0.3 + 0.5 * bounded(process.cpu, 40.0),
                );
            }
            stage.frame.sphere(
                camera,
                position,
                grown,
                kind_color(process),
                index as u32,
                stage.selected == Some(process.id),
            );
            stage.positions.insert(process.id, position);
            stage.notes.insert(process.id, describe(cell, is_nucleus));
            drawn += 1;
        }
        if cell.named {
            let label = short(&cell.name);
            stage
                .places
                .push((camera.front([x, y, 0.0], r + 0.6), label));
        }
        drawn
    }
}

/// Marks the cells of one dish that get a label: only the largest, so labels don't bury the dish.
fn name_the_largest(cells: &mut [Cell]) {
    let mut sizes: Vec<u64> = cells
        .iter()
        .filter(|c| !c.members.is_empty())
        .map(|c| c.memory)
        .collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let smallest = sizes.get(NAMED - 1).copied().unwrap_or(0);
    for cell in cells {
        cell.named = cell.memory >= smallest;
    }
}

/// The cgroup a process's cell stands for; processes outside cgroup v2 share one by group name.
fn cell_name(process: &Process) -> &str {
    if process.cgroup.is_empty() {
        &process.group
    } else {
        &process.cgroup
    }
}

/// The last path component without its unit suffix and systemd's `\x2d` escapes, shortened
/// for a label.
fn short(name: &str) -> String {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    let leaf = without_apple_prefix(leaf)
        .trim_end_matches(".service")
        .trim_end_matches(".scope")
        .replace("\\x2d", "-")
        .replace("\\x40", "@");
    if leaf.chars().count() > 28 {
        format!("{}..", leaf.chars().take(26).collect::<String>())
    } else {
        leaf
    }
}

/// The popup for one organelle. Without cgroup accounting there is no limit, quota or pressure
/// to report, so the cell is described by its members' own summed memory and CPU.
fn describe(cell: &Cell, nucleus: bool) -> Vec<String> {
    let mut lines = vec![format!(
        "{} of cell {}",
        if nucleus { "nucleus" } else { "organelle" },
        cell.name
    )];
    let Some(unit) = cell.unit else {
        lines.push(format!(
            "cell memory {} | CPU {:.0}% | {} tasks",
            bytes(cell.memory),
            cell.cpu,
            cell.population
        ));
        return lines;
    };
    let limit = unit
        .memory_max
        .map_or_else(|| "no limit".into(), |max| format!("limit {}", bytes(max)));
    let quota = unit
        .cpu_max
        .map_or_else(String::new, |max| format!(" of a {max:.0}% quota"));
    let members = cell.population as u64;
    let tasks = unit.pids.map_or(members, |pids| pids.max(members));
    let tasks = unit.pids_max.map_or_else(
        || format!("{tasks} tasks"),
        |max| format!("{tasks}/{max} tasks"),
    );
    lines.push(format!(
        "cell memory {} ({limit}) | CPU {:.0}%{quota} | {tasks}",
        bytes(cell.memory),
        unit.cpu.unwrap_or(0.0),
    ));
    let [cpu, memory, io] = unit.pressure.map(|percent| percent.unwrap_or(0.0));
    let mut status = format!("pressure cpu {cpu:.0}% mem {memory:.0}% io {io:.0}%");
    if unit.throttled == Some(true) {
        status.push_str(" | throttled");
    }
    if let Some(kills) = unit.oom_kills.filter(|&kills| kills > 0) {
        status.push_str(&format!(" | {kills} OOM kills"));
    }
    lines.push(status);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;
    use crate::render::{Camera, RenderInput, Scene, hiding};
    use crate::views::view;

    /// The legend of the view as the scene shows it with no process inspected.
    fn legend(snapshot: &Snapshot) -> String {
        Dishes::default().legend(snapshot)
    }

    #[test]
    fn legend_drops_limits_quotas_and_pressure_without_cgroups() {
        let mut snapshot = demo(1.0, 8);
        assert!(legend(&snapshot).contains("dashed ring = memory limit | arc = CPU vs quota"));
        snapshot.missing = Source::Cgroups.into();
        let text = legend(&snapshot);
        assert!(text.contains(&format!(
            " Cell = {GROUP_LABEL}, size = memory | {WITHOUT_CGROUPS}, so no limits, quotas or \
             pressure | "
        )));
        if cfg!(target_os = "macos") {
            assert!(text.contains(
                "groups by app and user; macOS has no cgroups, so no limits, quotas or pressure"
            ));
        }
        assert!(!text.contains("dashed ring") && !text.contains("throttled"));
    }

    #[test]
    fn popups_hold_only_measured_numbers_without_cgroup_accounting() {
        let mut snapshot = demo(10.0, 64);
        let render = |snapshot: &Snapshot| {
            let mut scene = Scene::new();
            scene.render(&RenderInput::small(
                snapshot,
                view("cells"),
                &Camera::default(),
                10.0,
            ));
            scene.notes
        };
        let with_units = render(&snapshot);
        assert!(
            with_units
                .values()
                .any(|lines| lines.iter().any(|l| l.starts_with("pressure cpu")))
        );
        snapshot.missing = Source::Cgroups.into();
        snapshot.units.clear();
        for process in &mut snapshot.processes {
            process.cgroup.clear();
            process.group = "Safari".into();
            process.cpu = 10.0;
        }
        let notes = render(&snapshot);
        assert!(!notes.is_empty());
        for lines in notes.values() {
            let text = lines.join("\n");
            assert!(
                !text.contains("pressure") && !text.contains("no limit"),
                "{text}"
            );
            assert_eq!(lines.len(), 2, "{text}");
            // Every process reads 10% CPU, so the cell's CPU is ten times its task count.
            let tasks: f32 = lines[1]
                .split(" | ")
                .nth(2)
                .and_then(|part| part.strip_suffix(" tasks"))
                .and_then(|count| count.parse().ok())
                .unwrap_or_else(|| panic!("{text}"));
            assert!(
                lines[1].contains(&format!("CPU {:.0}%", 10.0 * tasks)),
                "{text}"
            );
        }
    }

    type Places = HashMap<Identity, Point>;

    /// Renders one cells frame and returns where every drawn process stands.
    fn draw(
        scene: &mut Scene,
        snapshot: &Snapshot,
        time: f32,
        limit: usize,
        selected: Option<Identity>,
        focus: Option<Identity>,
    ) -> Places {
        let camera = Camera::default();
        let frame = scene.render(&RenderInput {
            width: 160,
            height: 90,
            selected,
            limit,
            focus,
            ..RenderInput::small(snapshot, view("cells"), &camera, time)
        });
        scene.spare = frame.release();
        scene.positions.clone()
    }

    /// Asserts that everything in `now` stands where it stood in `before`.
    fn in_place(now: &Places, before: &Places) {
        for (id, position) in now {
            assert_eq!(before.get(id), Some(position), "{id:?} moved");
        }
    }

    /// The demo dishes grown in two stages, so the cells stand where they arrived and not where a
    /// fresh packing would put them. Returns the scene, the full sample and the time of the last
    /// frame.
    fn settled() -> (Scene, Snapshot, f32) {
        let early = demo(30.0, 32);
        let sample = demo(30.0, 160);
        let mut scene = Scene::new();
        let mut time = 30.0;
        for step in 0..20 {
            let snapshot = if step < 8 { &early } else { &sample };
            draw(&mut scene, snapshot, time, 4096, None, None);
            time += 0.1;
        }
        (scene, sample, time - 0.1)
    }

    #[test]
    fn focusing_and_clearing_focus_keeps_every_cell_in_place() {
        let (mut scene, sample, time) = settled();
        let before = draw(&mut scene, &sample, time, 4096, None, None);
        // One family of sixteen is one whole cell.
        let root = sample.processes[16].id;
        let focused = draw(&mut scene, &sample, time, 4096, None, Some(root));
        assert_eq!(focused.len(), 16);
        in_place(&focused, &before);
        // One organelle of a cell that is not its largest stays an organelle at its own seat
        // instead of becoming the nucleus of a cell of one.
        let family = &sample.processes[32..48];
        let small = family.iter().min_by_key(|p| p.memory).unwrap().id;
        let alone = draw(&mut scene, &sample, time, 4096, None, Some(small));
        assert_eq!(alone.len(), 1);
        in_place(&alone, &before);
        let cleared = draw(&mut scene, &sample, time, 4096, None, None);
        assert_eq!(cleared.len(), before.len());
        in_place(&cleared, &before);
    }

    #[test]
    fn processes_beyond_the_limit_keep_their_cells_places() {
        let (mut scene, sample, time) = settled();
        let before = draw(&mut scene, &sample, time, 4096, None, None);
        let limited = draw(&mut scene, &sample, time, 80, None, None);
        assert!(limited.len() < before.len());
        in_place(&limited, &before);
        // Selecting a process beyond the limit swaps it in for the last one inside it: it comes
        // back to its own seat in its own cell.
        let selected = sample.processes[150].id;
        let swapped = draw(&mut scene, &sample, time, 80, Some(selected), None);
        assert!(swapped.contains_key(&selected));
        in_place(&swapped, &before);
        let restored = draw(&mut scene, &sample, time, 4096, None, None);
        assert_eq!(restored.len(), before.len());
        in_place(&restored, &before);
    }

    #[test]
    fn hiding_a_process_does_not_change_which_process_is_the_nucleus_while_memory_eases() {
        // Leaf 40 is the largest process of its cell and leaf 41 a small one.
        let mut base = demo(30.0, 160);
        base.processes[40].memory = 8 << 30;
        base.processes[41].memory = 1 << 30;
        let (large, small) = (base.processes[40].id, base.processes[41].id);
        let mut scene = Scene::new();
        let mut time = 30.0;
        for _ in 0..10 {
            draw(&mut scene, &base, time, 4096, None, None);
            time += 0.1;
        }
        // The small one balloons past it, but the frames repeat the time, so the scene has not
        // eased towards that yet: the large one is still the nucleus at the centre of its cell.
        let mut grown = base.clone();
        grown.processes[41].memory = 20 << 30;
        let shown = draw(&mut scene, &grown, time, 4096, None, None);
        // With only the large one drawn, the small one is hidden and must be measured as eased
        // too, not by its sampled 20 GiB: the large one stays at the centre.
        let alone = draw(&mut scene, &grown, time, 4096, None, Some(large));
        assert_eq!(alone.len(), 1);
        assert_eq!(alone[&large], shown[&large]);
        assert!(shown.contains_key(&small));
    }

    /// The state of the cells view the scene has drawn.
    fn dishes(scene: &Scene) -> &Dishes {
        scene
            .module::<Dishes>(view("cells"))
            .expect("the cells view has drawn")
    }

    /// The dish and the cgroup a process's cell is kept under.
    fn cell_of(process: &Process) -> (usize, String) {
        let dish = DISHES
            .iter()
            .position(|(kind, ..)| *kind == process.kind)
            .expect("a process of a cell is in a dish");
        (dish, cell_name(process).to_owned())
    }

    /// Draws `snapshot` for three seconds, long enough for the scene's easing of CPU and memory to
    /// settle, and returns where every drawn process stands.
    fn settle(scene: &mut Scene, snapshot: &Snapshot, time: &mut f32) -> Places {
        for _ in 0..30 {
            *time += 0.1;
            draw(scene, snapshot, *time, 4096, None, None);
        }
        scene.positions.clone()
    }

    /// A process that starts in the cell of `template`, with its figures and a fresh identity.
    fn arrival(template: &Process, serial: u32) -> Process {
        let pid = 900_000 + serial;
        Process {
            id: Identity {
                pid,
                start: u64::from(pid),
            },
            ..template.clone()
        }
    }

    /// Asserts that every process in `before` that is not in `except` is drawn where it was.
    fn survivors_in_place(now: &Places, before: &Places, except: &[Identity]) {
        let survivors = before.len() - except.iter().filter(|id| before.contains_key(id)).count();
        let held = before
            .iter()
            .filter(|(id, _)| !except.contains(id))
            .filter(|(id, position)| now.get(id) == Some(position))
            .count();
        assert_eq!(held, survivors, "survivors moved");
    }

    #[test]
    fn the_nucleus_keeps_the_centre_until_it_exits_and_the_next_arrival_takes_it() {
        // Leaf 40 is the largest process of its cell and leaf 41 a small one.
        let mut base = demo(30.0, 160);
        base.processes[40].memory = 8 << 30;
        base.processes[41].memory = 1 << 30;
        let (large, small) = (base.processes[40].id, base.processes[41].id);
        let key = cell_of(&base.processes[40]);
        let (mut scene, mut time) = (Scene::new(), 30.0);
        let before = settle(&mut scene, &base, &mut time);
        assert_eq!(dishes(&scene).nuclei[&key], large);
        assert_eq!(before[&large][2], 0.4, "the nucleus stands at the centre");
        // The small one balloons past it: the large one is still the nucleus, and nothing moves.
        let mut grown = base.clone();
        grown.processes[41].memory = 20 << 30;
        let after = settle(&mut scene, &grown, &mut time);
        assert_eq!(dishes(&scene).nuclei[&key], large);
        assert_eq!(after, before);
        // The nucleus exits. Nobody moves to the centre in its place.
        let mut gone = grown.clone();
        gone.processes.retain(|process| process.id != large);
        let after = settle(&mut scene, &gone, &mut time);
        assert_eq!(after.len(), before.len() - 1);
        survivors_in_place(&after, &before, &[large]);
        assert!(!dishes(&scene).nuclei.contains_key(&key), "no nucleus now");
        assert!(
            after[&small][2] < 0.4,
            "the next largest stays an organelle"
        );
        // A process that arrives takes the vacant centre, the largest of those that arrive. The
        // others stay in their seats.
        let mut arrived = gone.clone();
        let template = arrived.processes[41].clone();
        arrived.processes.push(arrival(&template, 1));
        let heavier = Process {
            memory: 2 << 30,
            ..arrival(&template, 2)
        };
        let heavier_id = heavier.id;
        arrived.processes.push(heavier);
        let after = settle(&mut scene, &arrived, &mut time);
        assert_eq!(dishes(&scene).nuclei[&key], heavier_id);
        assert_eq!(after[&heavier_id][2], 0.4);
        survivors_in_place(&after, &before, &[large]);
        // It keeps it when the cell gains someone larger.
        let mut larger = arrived.clone();
        let template = larger.processes[41].clone();
        larger.processes.push(Process {
            memory: 40 << 30,
            ..arrival(&template, 3)
        });
        let after = settle(&mut scene, &larger, &mut time);
        assert_eq!(dishes(&scene).nuclei[&key], heavier_id);
        survivors_in_place(&after, &before, &[large]);
    }

    #[test]
    fn a_process_joining_or_leaving_a_cell_moves_no_seat() {
        let mut base = demo(30.0, 160);
        base.processes[40].memory = 8 << 30;
        base.processes[41].memory = 1 << 30;
        let (large, leaver) = (base.processes[40].id, base.processes[41].id);
        let (mut scene, mut time) = (Scene::new(), 30.0);
        let before = settle(&mut scene, &base, &mut time);
        // One leaves and three arrive: the first takes the free seat, the others the seats
        // after the last, and the spiral goes on outwards from there.
        let mut changed = base.clone();
        changed.processes.retain(|process| process.id != leaver);
        let template = changed.processes[41].clone();
        let arrivals: Vec<Process> = (1..=3).map(|serial| arrival(&template, serial)).collect();
        let joined: Vec<Identity> = arrivals.iter().map(|process| process.id).collect();
        changed.processes.extend(arrivals);
        let after = settle(&mut scene, &changed, &mut time);
        assert!(joined.iter().all(|id| after.contains_key(id)));
        assert_eq!(after[&large], before[&large]);
        survivors_in_place(&after, &before, &[leaver]);
    }

    #[test]
    fn seats_stay_put_while_memory_moves_the_membrane_until_the_cell_outgrows_its_room() {
        let base = demo(30.0, 160);
        let cgroup = base.processes[40].cgroup.clone();
        let key = cell_of(&base.processes[40]);
        let family: Vec<Identity> = base
            .processes
            .iter()
            .filter(|p| p.kind != Kind::Kernel && cell_of(p) == key)
            .map(|p| p.id)
            .collect();
        let (mut scene, mut time) = (Scene::new(), 30.0);
        let before = settle(&mut scene, &base, &mut time);
        let room = dishes(&scene).rooms[&key];
        let nucleus = dishes(&scene).nuclei[&key];
        // Each organelle of the cell where it is, measured from the cell's own centre.
        let seats = |places: &Places| -> Vec<[f32; 2]> {
            let centre = places[&nucleus];
            family
                .iter()
                .map(|id| [places[id][0] - centre[0], places[id][1] - centre[1]])
                .collect()
        };
        // The membrane breathes with the memory, from half to double, and the organelles stay in
        // their seats, all inside the membrane of the least memory (the draw would otherwise pull
        // the outermost back in). Up to a seventh over its radius the cell keeps its place in the
        // dish as well; a cell that grows more than its dish reserved is placed again whole, so
        // after that only its seats are compared, from its own centre.
        let mut moved = false;
        for factor in [1.2, 0.9, 0.6, 0.5, 2.0, 0.5, 1.0] {
            let mut changed = base.clone();
            let unit = changed.units.get_mut(&cgroup).expect("the cell has a unit");
            unit.memory = (unit.memory as f64 * factor) as u64;
            let after = settle(&mut scene, &changed, &mut time);
            moved |= factor > 1.2;
            // Equal to a rounding error of the subtraction from the centre.
            let moved_by = seats(&after)
                .iter()
                .zip(seats(&before))
                .map(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]))
                .fold(0.0, f32::max);
            assert!(
                moved_by < 1e-3,
                "memory x{factor} moved a seat by {moved_by}"
            );
            assert_eq!(dishes(&scene).rooms[&key].radius, room.radius, "x{factor}");
            if !moved {
                assert_eq!(after, before, "memory x{factor}");
            }
        }
        // Six times the memory is two and a half times the radius: the seats are spread out again.
        let mut changed = base.clone();
        let unit = changed.units.get_mut(&cgroup).expect("the cell has a unit");
        unit.memory *= 6;
        let after = settle(&mut scene, &changed, &mut time);
        let grown = dishes(&scene).rooms[&key];
        assert!(grown.radius > 1.5 * room.radius, "{grown:?}");
        let member = base.processes[41].id;
        assert_ne!(after[&member], before[&member], "its seats moved out");
        // And that is the room the cell keeps from now on.
        let again = settle(&mut scene, &changed, &mut time);
        assert_eq!(again, after);
    }

    #[test]
    fn popups_and_dish_labels_count_the_whole_cell_while_only_part_of_it_is_shown() {
        let (mut scene, mut sample, time) = settled();
        // Without accounting a popup describes the cell by its members' own sums.
        sample.units.clear();
        let family = &sample.processes[32..48];
        let leaf = family[7].id;
        draw(&mut scene, &sample, time, 4096, None, Some(leaf));
        let cpu: f32 = family.iter().map(|p| p.cpu).sum();
        let popup = scene.notes[&leaf].join("\n");
        assert!(
            popup.contains(&format!("CPU {cpu:.0}% | 16 tasks")),
            "{popup}"
        );
        assert!(
            scene
                .places
                .iter()
                .any(|(_, text)| text == "containers: 1 cells, 1 processes"),
            "{:?}",
            scene.places
        );
    }

    #[test]
    fn a_kill_in_a_hidden_cell_is_remembered_so_it_does_not_burst_when_the_cell_returns() {
        // No member is drawn: the cell is hidden, but still sampled.
        fn hidden(unit: &Unit) -> [Vec<Cell<'_>>; 3] {
            let cell = Cell {
                name: "worker.service".into(),
                members: Vec::new(),
                population: 3,
                cpu: 0.0,
                nucleus: Some(Identity { pid: 1, start: 1 }),
                nucleus_memory: 0,
                nucleus_index: None,
                unit: Some(unit),
                memory: 0,
                radius: 1.0,
                room: Room {
                    radius: 1.0,
                    seats: 3,
                },
                named: false,
            };
            [vec![cell], Vec::new(), Vec::new()]
        }
        let killed = |kills| Unit {
            oom_kills: Some(kills),
            ..Unit::default()
        };
        let mut dishes = Dishes::default();
        dishes.watch_kills(&hidden(&killed(1)), 10.0);
        dishes.watch_kills(&hidden(&killed(2)), 20.0);
        assert_eq!(dishes.kills["worker.service"], (2, 20.0));
        // Nothing new, or a count that could not be read, leaves the burst time alone.
        dishes.watch_kills(&hidden(&killed(2)), 25.0);
        dishes.watch_kills(&hidden(&Unit::default()), 30.0);
        assert_eq!(dishes.kills["worker.service"], (2, 20.0));
    }

    #[test]
    fn the_camera_fit_ignores_what_focus_and_the_limit_hide() {
        hiding::bounds_ignore_hiding(view("cells"), &demo(30.0, 160));
    }

    #[test]
    fn cells_without_cgroup_accounting_keep_their_size_while_members_are_hidden() {
        // A cell sized by its members' own memory must count the hidden ones too, or the
        // organelle swapped in across the limit would sit closer to a shrunken nucleus.
        let mut sample = demo(30.0, 160);
        sample.missing = Source::Cgroups.into();
        sample.units.clear();
        for process in &mut sample.processes {
            process.cgroup.clear();
        }
        hiding::limit_and_lift(view("cells"), &sample);
    }

    /// One cell of a big parent and a small child, drawn with `focus`: the glow of a nucleus
    /// core and the triangles of the cytoplasm that a click on the cell's centre would pick.
    fn cell_of_two(focus: impl Fn(Identity, Identity) -> Option<Identity>) -> (usize, Vec<u32>) {
        use crate::render::{Camera, Item, RenderInput, Scene};
        let mut snapshot = demo(30.0, 2);
        let [parent, child] = [0, 1].map(|i| snapshot.processes[i].id);
        assert_eq!(snapshot.processes[1].parent, parent.pid);
        for (process, memory) in snapshot.processes.iter_mut().zip([800 << 20, 8 << 20]) {
            process.memory = memory;
            process.cpu = 0.0;
            process.cgroup = "/system.slice/one.service".into();
        }
        let frame = Scene::new().render(&RenderInput {
            focus: focus(parent, child),
            ..RenderInput::small(&snapshot, view("cells"), &Camera::default(), 30.0)
        });
        let cores = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Glow { strength, .. } if *strength == 0.35))
            .count();
        let picks = frame
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(_, _, pick) if *pick != NONE => Some(*pick),
                _ => None,
            })
            .collect();
        (cores, picks)
    }

    #[test]
    fn a_hidden_nucleus_leaves_the_centre_of_its_cell_dark() {
        assert_eq!(cell_of_two(|_, _| None).0, 1);
        assert_eq!(cell_of_two(|_, child| Some(child)).0, 0);
    }

    #[test]
    fn a_hidden_nucleus_leaves_the_cytoplasm_unpickable() {
        // The cytoplasm picks the nucleus; with the nucleus hidden, the one organelle drawn
        // (index 0) is not the nucleus and a click on the cell's centre must not select it.
        let (_, whole) = cell_of_two(|_, _| None);
        assert!(whole.contains(&0));
        let (_, alone) = cell_of_two(|_, child| Some(child));
        assert!(alone.is_empty(), "{alone:?}");
    }

    #[test]
    fn an_oom_kill_in_a_hidden_cell_bursts_when_it_returns_only_while_the_burst_lasts() {
        use crate::render::{Camera, Item, RenderInput, Scene};
        let mut snapshot = demo(30.0, 4);
        for (index, process) in snapshot.processes.iter_mut().enumerate() {
            process.kind = Kind::System;
            process.parent = process.id.pid;
            process.cgroup = format!("/system.slice/{}.service", index / 2);
        }
        let near = snapshot.processes[2].id;
        // The first cell has the kills, the second never does.
        let kill = |snapshot: &mut Snapshot, kills| {
            for (name, kills) in [
                ("/system.slice/0.service", kills),
                ("/system.slice/1.service", 0),
            ] {
                let unit = Unit {
                    memory: 1 << 30,
                    oom_kills: Some(kills),
                    ..Unit::default()
                };
                snapshot.units.insert(name.into(), unit);
            }
        };
        let bursts = |scene: &mut Scene, snapshot: &Snapshot, time, focus| {
            let frame = scene.render(&RenderInput {
                focus,
                ..RenderInput::small(snapshot, view("cells"), &Camera::default(), time)
            });
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Glow { color, .. } if *color == CRISIS))
                .count()
        };
        for (back, expected) in [(31.2, true), (40.0, false)] {
            let mut scene = Scene::new();
            kill(&mut snapshot, 0);
            assert_eq!(bursts(&mut scene, &snapshot, 30.0, None), 0);
            // The kill lands in the first cell while focus hides it.
            kill(&mut snapshot, 1);
            assert_eq!(bursts(&mut scene, &snapshot, 31.0, Some(near)), 0);
            assert_eq!(
                bursts(&mut scene, &snapshot, back, None) > 0,
                expected,
                "back at {back}"
            );
        }
    }

    #[test]
    fn cells_are_drawn_in_the_same_order_every_time() {
        use crate::render::{Camera, RenderInput, Scene};
        let snapshot = demo(30.0, 160);
        let order = || {
            let mut scene = Scene::new();
            scene.render(&RenderInput::small(
                &snapshot,
                view("cells"),
                &Camera::default(),
                30.0,
            ));
            scene
                .places
                .iter()
                .map(|(_, text)| text.clone())
                .collect::<Vec<_>>()
        };
        let first = order();
        assert!(first.len() > 6);
        for _ in 0..8 {
            assert_eq!(order(), first);
        }
    }

    #[test]
    fn container_dishes_keep_their_places_when_a_session_cell_grows() {
        let render = |scene: &mut Scene, snapshot: &Snapshot, time: f32| {
            let mut frame = scene.render(&RenderInput::small(
                snapshot,
                view("cells"),
                &Camera::default(),
                time,
            ));
            frame.rasterize();
        };
        let mut snapshot = demo(10.0, 128);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, 10.0);
        let containers: Vec<(Identity, Point)> = snapshot
            .processes
            .iter()
            .filter(|p| p.kind == Kind::Container)
            .map(|p| (p.id, scene.positions[&p.id]))
            .collect();
        let session = snapshot
            .processes
            .iter()
            .find(|p| p.kind == Kind::Session)
            .unwrap()
            .cgroup
            .clone();
        snapshot.units.get_mut(&session).unwrap().memory *= 400;
        render(&mut scene, &snapshot, 10.1);
        render(&mut scene, &snapshot, 10.2);
        for (id, at) in &containers {
            assert_eq!(scene.positions[id], *at, "the container dish moved");
        }
    }
}
