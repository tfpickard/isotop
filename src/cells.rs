//! Cells: every cgroup is a living cell in a petri dish, one dish each for system services, your
//! session and containers. A cell's size follows its memory; a dashed ring marks its memory limit
//! and an arc its CPU use against its quota. The membrane trembles under pressure, flashes red
//! when the quota throttles it and bursts when the OOM killer strikes. Processes are organelles,
//! the largest as the nucleus.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, TAU};

use crate::model::{Identity, Kind, Process, Snapshot, Unit, bounded, bytes};
use crate::pack::{Discs, Seats};
use crate::render::{
    Color, GOLDEN_ANGLE, NONE, Point, Stage, WARM, kind_color, mass_radius, ring_bounds, spin,
    tint, without_apple_prefix,
};

const DISHES: [(Kind, &str, Color); 3] = [
    (Kind::System, "system services", [72, 183, 199]),
    (Kind::Session, "your session", [236, 178, 92]),
    (Kind::Container, "containers", [178, 136, 232]),
];
const SEAT: f32 = 0.75;
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
    /// OOM kill count last seen per cgroup, and when its latest burst began.
    kills: HashMap<String, (u64, f32)>,
}

struct Cell<'a> {
    name: String,
    /// The drawn members, as indices into the drawn processes.
    members: Vec<usize>,
    /// How many processes the cell has and what they use between them, drawn or not.
    population: usize,
    cpu: f32,
    /// The largest process, drawn or not, and its place among the drawn ones if it is drawn.
    nucleus: Identity,
    nucleus_memory: u64,
    nucleus_index: Option<usize>,
    unit: Option<&'a Unit>,
    memory: u64,
    radius: f32,
    /// Only the largest cells in a dish are named, so labels don't bury the dish.
    named: bool,
}

impl Dishes {
    pub fn draw(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        snapshot: &Snapshot,
    ) -> usize {
        let time = stage.time;
        // Seats, cell sizes and places are reserved for every sampled process, drawn or not, so
        // focusing a subtree, or `--limit` hiding some, neither frees their seats nor drops their
        // cells: clearing it puts everything back where it was. Only drawn processes are drawn.
        // Sizes use a drawn process's smoothed copy, and the sampled one while it is hidden.
        let drawn: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(index, process)| (process.id, index))
            .collect();
        let sampled: Vec<&Process> = snapshot
            .processes
            .iter()
            .map(|p| drawn.get(&p.id).map_or(p, |&index| processes[index]))
            .collect();
        self.seats.assign(
            sampled
                .iter()
                .filter(|p| p.kind != Kind::Kernel)
                .map(|p| (p.id, cell_name(p))),
        );
        let mut cells: [Vec<Cell>; 3] = Default::default();
        for (dish, (kind, _, _)) in DISHES.iter().enumerate() {
            let mut groups: BTreeMap<&str, Vec<&Process>> = BTreeMap::new();
            for &p in sampled.iter().filter(|p| p.kind == *kind) {
                groups.entry(cell_name(p)).or_default().push(p);
            }
            for (name, everyone) in groups {
                let unit = snapshot.units.get(name);
                let memory = unit.map_or_else(
                    || everyone.iter().map(|p| p.memory).sum(),
                    |unit| unit.memory,
                );
                let nucleus = everyone
                    .iter()
                    .copied()
                    .max_by_key(|p| (p.memory, Reverse(p.id)))
                    .expect("a cell has members");
                let mut members: Vec<usize> = everyone
                    .iter()
                    .filter_map(|p| drawn.get(&p.id).copied())
                    .collect();
                members.sort_unstable();
                let span = self.seats.span(name) as f32;
                let by_memory = 1.3 + 0.16 * (memory as f32 / 1048576.0).sqrt();
                let by_members = SEAT * (span + 1.0).sqrt() + 1.0;
                cells[dish].push(Cell {
                    name: name.to_owned(),
                    members,
                    population: everyone.len(),
                    cpu: everyone.iter().map(|p| p.cpu).sum(),
                    nucleus: nucleus.id,
                    nucleus_memory: nucleus.memory,
                    nucleus_index: drawn.get(&nucleus.id).copied(),
                    unit,
                    memory,
                    radius: by_memory.max(by_members),
                    named: false,
                });
            }
            let mut sizes: Vec<u64> = cells[dish]
                .iter()
                .filter(|c| !c.members.is_empty())
                .map(|c| c.memory)
                .collect();
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            let smallest = sizes.get(NAMED - 1).copied().unwrap_or(0);
            for cell in &mut cells[dish] {
                cell.named = cell.memory >= smallest;
            }
        }
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
        // Kills are watched in every sampled cell, so one that happens while its cell is hidden
        // does not burst when the cell comes back.
        let present: HashSet<&str> = cells.iter().flatten().map(|c| c.name.as_str()).collect();
        self.kills.retain(|name, _| present.contains(name.as_str()));
        for cell in cells.iter().flatten() {
            let kills = cell.unit.map_or(0, |unit| unit.oom_kills);
            let seen = self
                .kills
                .entry(cell.name.clone())
                .or_insert((kills, f32::NEG_INFINITY));
            if kills > seen.0 {
                *seen = (kills, time);
            }
        }
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
        let pressure = unit.pressure.iter().fold(0.0_f32, |a, &b| a.max(b));
        let phase = spin(&cell.name);
        let wobble = 0.03 + 0.22 * bounded(pressure, 15.0);
        let edge = |angle: f32| {
            r * (1.0
                + wobble * (5.0 * angle + time * 2.2 + phase).sin()
                + wobble * 0.5 * (3.0 * angle - time * 1.7).sin())
        };
        let cytoplasm = tint(color, 0.16 + 0.14 * bounded(unit.cpu, 50.0));
        for k in 0..64 {
            let (a, b) = (k as f32 / 64.0 * TAU, (k + 1) as f32 / 64.0 * TAU);
            stage.frame.facet(
                camera,
                [[x, y, 0.05], at(a, edge(a), 0.05), at(b, edge(b), 0.05)],
                cytoplasm,
                pick,
            );
        }
        let flash = if unit.throttled {
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
        let share = unit.cpu / unit.cpu_max.unwrap_or(100.0);
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
        let began = self
            .kills
            .get(&cell.name)
            .map_or(f32::NEG_INFINITY, |&(_, began)| began);
        let age = (time - began) / LYSIS_SECONDS;
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
        // Spread the organelles over most of the cell rather than huddling round the nucleus.
        let spacing = (r * 0.8 / (self.seats.span(&cell.name) as f32 + 1.0).sqrt()).max(SEAT);
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
            let is_nucleus = process.id == cell.nucleus;
            let (position, size) = if is_nucleus {
                ([x, y, 0.4], core)
            } else {
                let seat = self.seats.seat(process.id).unwrap_or(0) as f32;
                let distance = spacing * (seat + 1.0).sqrt();
                let angle = seat * GOLDEN_ANGLE + phase;
                let position = at(angle, distance.min(r - 0.4), 0.3);
                stage
                    .frame
                    .line(camera, [x, y, 0.2], position, tint(color, 0.45));
                (
                    position,
                    (0.8 * mass_radius(process.memory as f32)).min(spacing * 0.42),
                )
            };
            let grown = size * (0.3 + 0.7 * stage.growth(process.id));
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
    let tasks = unit.pids.max(cell.population as u64);
    let tasks = unit.pids_max.map_or_else(
        || format!("{tasks} tasks"),
        |max| format!("{tasks}/{max} tasks"),
    );
    lines.push(format!(
        "cell memory {} ({limit}) | CPU {:.0}%{quota} | {tasks}",
        bytes(cell.memory),
        unit.cpu,
    ));
    let [cpu, memory, io] = unit.pressure;
    let mut status = format!("pressure cpu {cpu:.0}% mem {memory:.0}% io {io:.0}%");
    if unit.throttled {
        status.push_str(" | throttled");
    }
    if unit.oom_kills > 0 {
        status.push_str(&format!(" | {} OOM kills", unit.oom_kills));
    }
    lines.push(status);
    lines
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t) as u8)
}

/// The status legend, saying what the snapshot's platform leaves out.
pub fn legend(snapshot: &Snapshot) -> String {
    if snapshot.missing.contains(&"cgroups") {
        " Cell = app or user group, size = memory | groups by app and user; macOS has no cgroups, so no limits, quotas or pressure | organelles = processes".into()
    } else {
        " Cell = cgroup, size = memory | dashed ring = memory limit | arc = CPU vs quota | trembling = pressure | red = throttled | burst = OOM kill | organelles = processes".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Identity, demo};
    use crate::render::{Camera, Scene, View};

    #[test]
    fn legend_drops_limits_quotas_and_pressure_without_cgroups() {
        let mut snapshot = demo(1.0, 8);
        assert!(legend(&snapshot).contains("dashed ring = memory limit | arc = CPU vs quota"));
        snapshot.missing = vec!["cgroups"];
        let text = legend(&snapshot);
        assert!(text.contains(
            "groups by app and user; macOS has no cgroups, so no limits, quotas or pressure"
        ));
        assert!(!text.contains("dashed ring") && !text.contains("throttled"));
    }

    #[test]
    fn popups_hold_only_measured_numbers_without_cgroup_accounting() {
        let mut snapshot = demo(10.0, 64);
        let render = |snapshot: &Snapshot| {
            let mut scene = Scene::new();
            scene.render(
                snapshot,
                View::Cells,
                &Camera::default(),
                320,
                180,
                None,
                10.0,
                512,
                None,
            );
            scene.notes
        };
        let with_units = render(&snapshot);
        assert!(
            with_units
                .values()
                .any(|lines| lines.iter().any(|l| l.starts_with("pressure cpu")))
        );
        snapshot.missing = vec!["cgroups"];
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
        let frame = scene.render(
            snapshot,
            View::Cells,
            &Camera::default(),
            160,
            90,
            selected,
            time,
            limit,
            focus,
        );
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
}
