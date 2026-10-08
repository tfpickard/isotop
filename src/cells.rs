//! Cells: every cgroup is a living cell in a petri dish, one dish each for system services, your
//! session and containers. A cell's size follows its memory; a dashed ring marks its memory limit
//! and an arc its CPU use against its quota. The membrane trembles under pressure, flashes red
//! when the quota throttles it and bursts when the OOM killer strikes. Processes are organelles,
//! the largest as the nucleus.

use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, TAU};

use crate::model::{Kind, Process, Snapshot, Unit, bounded, bytes};
use crate::pack::{Discs, Seats};
use crate::render::{
    Color, GOLDEN_ANGLE, NONE, Point, Stage, WARM, kind_color, mass_radius, ring_bounds, spin, tint,
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
    members: Vec<usize>,
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
        self.seats.assign(
            processes
                .iter()
                .filter(|p| p.kind != Kind::Kernel)
                .map(|p| (p.id, cell_name(p))),
        );
        let mut cells: [Vec<Cell>; 3] = Default::default();
        for (dish, (kind, _, _)) in DISHES.iter().enumerate() {
            let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
            for (index, p) in processes
                .iter()
                .enumerate()
                .filter(|(_, p)| p.kind == *kind)
            {
                groups.entry(cell_name(p)).or_default().push(index);
            }
            for (name, members) in groups {
                let unit = snapshot.units.get(name);
                let memory = unit.map_or_else(
                    || members.iter().map(|&i| processes[i].memory).sum(),
                    |unit| unit.memory,
                );
                let span = self.seats.span(name) as f32;
                let by_memory = 1.3 + 0.16 * (memory as f32 / 1048576.0).sqrt();
                let by_members = SEAT * (span + 1.0).sqrt() + 1.0;
                cells[dish].push(Cell {
                    name: name.to_owned(),
                    members,
                    unit,
                    memory,
                    radius: by_memory.max(by_members),
                    named: false,
                });
            }
            let mut sizes: Vec<u64> = cells[dish].iter().map(|c| c.memory).collect();
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
        let present: HashSet<&str> = cells.iter().flatten().map(|c| c.name.as_str()).collect();
        self.kills.retain(|name, _| present.contains(name.as_str()));
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
            let count = cells[dish].iter().map(|c| c.members.len()).sum::<usize>();
            stage.places.push((
                camera.front([cx, cy, 0.0], r + 1.5),
                format!("{title}: {} cells, {count} processes", cells[dish].len()),
            ));
            for cell in &cells[dish] {
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
        &mut self,
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
        let nucleus = cell
            .members
            .iter()
            .copied()
            .max_by_key(|&i| (processes[i].memory, std::cmp::Reverse(processes[i].id)))
            .expect("a cell has members");
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
                nucleus as u32,
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
        let seen = self
            .kills
            .entry(cell.name.clone())
            .or_insert((unit.oom_kills, f32::NEG_INFINITY));
        if unit.oom_kills > seen.0 {
            *seen = (unit.oom_kills, time);
        }
        let age = (time - seen.1) / LYSIS_SECONDS;
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
        let core = mass_radius(processes[nucleus].memory as f32).clamp(0.45, r * 0.32);
        stage.frame.glow(
            camera,
            [x, y, 0.4],
            core * camera.zoom * 3.0,
            tint(color, 1.2),
            0.35,
        );
        for &index in &cell.members {
            let process = processes[index];
            let (position, size) = if index == nucleus {
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
            stage
                .notes
                .insert(process.id, describe(cell, &unit, index == nucleus));
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
    let leaf = leaf
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

fn describe(cell: &Cell, unit: &Unit, nucleus: bool) -> Vec<String> {
    let mut lines = vec![format!(
        "{} of cell {}",
        if nucleus { "nucleus" } else { "organelle" },
        cell.name
    )];
    let limit = unit
        .memory_max
        .map_or_else(|| "no limit".into(), |max| format!("limit {}", bytes(max)));
    let quota = unit
        .cpu_max
        .map_or_else(String::new, |max| format!(" of a {max:.0}% quota"));
    let tasks = unit.pids.max(cell.members.len() as u64);
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
