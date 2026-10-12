//! Cells: every cgroup is a living cell in a petri dish, one dish each for system services, your
//! session and containers. A cell's size follows its memory; a dashed ring marks its memory limit
//! and an arc its CPU use against its quota. The membrane trembles under pressure, flashes red
//! when the quota throttles it and bursts when the OOM killer strikes. Processes are organelles,
//! the largest as the nucleus.

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
                let nucleus = everyone
                    .iter()
                    .copied()
                    .max_by_key(|s| (s.memory, Reverse(s.process.id)))
                    .expect("a cell has members");
                let mut members: Vec<usize> = everyone
                    .iter()
                    .filter_map(|s| drawn.get(&s.process.id).copied())
                    .collect();
                members.sort_unstable();
                let span = self.seats.span(name) as f32;
                let by_memory = 1.3 + 0.16 * (memory as f32 / 1048576.0).sqrt();
                let by_members = SEAT * (span + 1.0).sqrt() + 1.0;
                cells[dish].push(Cell {
                    name: name.to_owned(),
                    members,
                    population: everyone.len(),
                    cpu: everyone.iter().map(|s| s.cpu).sum(),
                    nucleus: nucleus.process.id,
                    nucleus_memory: nucleus.memory,
                    nucleus_index: drawn.get(&nucleus.process.id).copied(),
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
}

impl Dishes {
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
                let (distance, angle) = spiral(spacing, 1.0, seat, phase);
                let position = at(angle, distance.min(r - 0.4), 0.3);
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
                nucleus: Identity { pid: 1, start: 1 },
                nucleus_memory: 0,
                nucleus_index: None,
                unit: Some(unit),
                memory: 0,
                radius: 1.0,
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
