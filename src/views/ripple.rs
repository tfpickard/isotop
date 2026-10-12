//! The Ripple view: the machine as a pond. Each cgroup is a cluster of pebbles on a sunflower
//! spiral, every busy process drives ripples at its own pitch so a cluster's rings interfere and
//! spread over open water to the next, buoys float on the surface sized by memory, every patch of
//! water belongs to its nearest process, births drip and exits splash, and pressure raises a
//! swell.
//!
//! The wave (`medium::Wave`) and the clusters are Ripple's own state. The births and exits that
//! disturb the water are not: the scene notes them for every view (`Stage::newborn`,
//! `Stage::splashes`) and Flow reads the same two lists. A cluster of two or more drawn pebbles
//! is a system to label (`Stage::titles`), tour (`Stage::tour`) and total in its anchor's
//! popup (`Stage::popup`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::f32::consts::TAU;

use crate::geometry::{dot, normalize};
use crate::layout::{spin, spiral, vacant};
use crate::medium::{Grid, Wave, mix};
use crate::model::{Identity, Process, Snapshot, bounded, bytes};
use crate::palette::kind_color;
use crate::platform;
use crate::render::{NONE, Stage, Title, TourTarget, cover, glows, mass_radius, similar};
use crate::simulation::hash;
use crate::views::ViewModule;
use crate::views::systems::system_label;

/// Clearance between neighbouring ripple pebbles, and open water around each cluster.
const PEBBLE_GAP: f32 = 1.0;
const POND_GAP: f32 = 3.0;
const RIPPLE_DEEP: [f32; 3] = [10.0, 30.0, 46.0];
const RIPPLE_CREST: [f32; 3] = [14.0, 34.0, 40.0];
const RIPPLE_MOON: [f32; 3] = [150.0, 190.0, 210.0];
const RIPPLE_SHINE: f32 = 0.6;

/// One cgroup's processes on the ripple pond: pebbles on a sunflower spiral around the centre,
/// slot k at sqrt(k + 1/2) spacings out and k golden angles round.
#[derive(Default)]
struct Cluster {
    center: [f32; 2],
    spacing: f32,
    /// Centre to the far edge of the outermost pebble.
    extent: f32,
    /// Radius of open water reserved around the centre; zero until placed.
    reserved: f32,
    slots: HashMap<usize, Identity>,
}

/// The pond: the wave on the water, the cluster of each cgroup and each process's cluster and
/// slot in it.
#[derive(Default)]
pub struct Ripple {
    wave: Option<Wave>,
    clusters: HashMap<String, Cluster>,
    pebbles: HashMap<Identity, (String, usize)>,
}

/// A cluster of two or more drawn pebbles: its anchor (the drawn pebble of the lowest slot), the
/// pebbles drawn, their memory and the cluster's reach from its centre.
struct System {
    anchor: Identity,
    count: usize,
    memory: u64,
    extent: f32,
}

impl Ripple {
    /// Places every process as a pebble in its cgroup's cluster and records the positions of the
    /// drawn ones. A process keeps its slot while it lives, drawn or not; a cluster spreads out
    /// or moves only when it outgrows the water reserved for it. Returns each drawn process's
    /// pebble radius, and the clusters that make a system.
    fn scatter(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        snapshot: &Snapshot,
    ) -> (Vec<f32>, Vec<System>) {
        let radii: Vec<f32> = processes
            .iter()
            .map(|p| mass_radius(p.memory as f32))
            .collect();
        // Slots and clusters are reserved for every sampled process, drawn or not, so focusing a
        // subtree, or the process limit (and a selection swapped across it) hiding some, frees
        // neither their slots nor their cluster's water: clearing it brings everyone back to
        // where they were. Drawn processes are measured as drawn, hidden ones as sampled.
        let mut members: HashMap<Identity, (&str, f32, u64)> = snapshot
            .processes
            .iter()
            .map(|p| {
                let radius = mass_radius(p.memory as f32);
                (p.id, (p.group.as_str(), radius, p.memory))
            })
            .collect();
        members.extend(
            processes
                .iter()
                .zip(&radii)
                .map(|(p, &r)| (p.id, (p.group.as_str(), r, p.memory))),
        );
        let drawn: HashSet<Identity> = processes.iter().map(|p| p.id).collect();
        self.pebbles.retain(|id, (group, slot)| {
            let stays = members.get(id).is_some_and(|m| m.0 == group);
            if !stays && let Some(cluster) = self.clusters.get_mut(group) {
                cluster.slots.remove(slot);
            }
            stays
        });
        self.clusters.retain(|_, cluster| !cluster.slots.is_empty());
        let mut newcomers: Vec<(Identity, &str)> = members
            .iter()
            .filter(|(id, _)| !self.pebbles.contains_key(id))
            .map(|(&id, &(group, _, _))| (id, group))
            .collect();
        newcomers.sort();
        for (id, group) in newcomers {
            let cluster = self.clusters.entry(group.to_owned()).or_default();
            let slot = (0..=cluster.slots.len())
                .find(|slot| !cluster.slots.contains_key(slot))
                .expect("one of len + 1 slots is free");
            cluster.slots.insert(slot, id);
            self.pebbles.insert(id, (group.to_owned(), slot));
        }
        let mut pending = Vec::new();
        for (name, cluster) in &mut self.clusters {
            let largest = cluster
                .slots
                .values()
                .map(|id| members[id].1)
                .fold(0.0, f32::max);
            // Spiral neighbours sit at least ~1.6 spacings apart; jitter takes up to 0.2 of that.
            let needed = (2.0 * largest + PEBBLE_GAP) / 1.4;
            if needed > cluster.spacing {
                cluster.spacing = needed * 1.15;
            }
            let outermost = cluster.slots.keys().max().copied().unwrap_or(0);
            cluster.extent = cluster.spacing * (outermost as f32 + 1.0).sqrt() + largest;
            let reach = cluster.extent + POND_GAP;
            if reach > cluster.reserved {
                cluster.reserved = 0.0;
                pending.push((name.clone(), reach));
            }
        }
        pending.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (name, reach) in pending {
            let placed: Vec<_> = self
                .clusters
                .values()
                .filter(|c| c.reserved > 0.0)
                .map(|c| (c.center, c.reserved))
                .collect();
            let reserved = reach * 1.1;
            let center = vacant(&placed, reserved);
            if let Some(cluster) = self.clusters.get_mut(&name) {
                cluster.center = center;
                cluster.reserved = reserved;
            }
        }
        for process in processes {
            let (group, slot) = &self.pebbles[&process.id];
            let cluster = &self.clusters[group];
            let k = *slot as f32;
            let (distance, angle) = spiral(cluster.spacing, 0.5, k, spin(group));
            // pid only: kept for pixel stability, see A33
            let hash = hash::knuth(process.id.pid);
            let jitter = cluster.spacing * 0.1 * (hash & 0xffff) as f32 / 65536.0;
            let toss = (hash >> 16) as f32 / 65536.0 * TAU;
            stage.positions.insert(
                process.id,
                [
                    cluster.center[0] + distance * angle.cos() + jitter * toss.cos(),
                    cluster.center[1] + distance * angle.sin() + jitter * toss.sin(),
                    0.0,
                ],
            );
        }
        // Only the drawn pebbles of a cluster make it a system to label and tour.
        let mut systems = Vec::new();
        for cluster in self.clusters.values() {
            let shown: Vec<(usize, Identity)> = cluster
                .slots
                .iter()
                .filter(|(_, id)| drawn.contains(id))
                .map(|(&slot, &id)| (slot, id))
                .collect();
            if let Some(&(_, anchor)) = shown.iter().min_by_key(|(slot, _)| *slot)
                && shown.len() > 1
            {
                systems.push(System {
                    anchor,
                    count: shown.len(),
                    memory: shown.iter().map(|(_, id)| members[id].2).sum(),
                    extent: cluster.extent,
                });
            }
        }
        systems.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.anchor.cmp(&b.anchor)));
        (radii, systems)
    }
}

/// Assigns every grid node to its nearest body by a multi-source breadth-first flood, larger
/// bodies claiming shared cells first. Returns body indices into the process list, or NONE.
fn territories(grid: &Grid, located: &[[f32; 2]], radii: &[f32]) -> Vec<u32> {
    let mut owners = vec![NONE; grid.len()];
    let mut order: Vec<usize> = (0..located.len()).collect();
    order.sort_by(|&a, &b| radii[b].total_cmp(&radii[a]));
    let mut queue = VecDeque::new();
    for slot in order {
        let [x, y] = grid.locate(located[slot]);
        let (x, y) = (x.round() as usize, y.round() as usize);
        if x < grid.columns && y < grid.rows && owners[y * grid.columns + x] == NONE {
            owners[y * grid.columns + x] = slot as u32;
            queue.push_back(y * grid.columns + x);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % grid.columns, i / grid.columns);
        let neighbours = [
            (x > 0).then(|| i - 1),
            (x + 1 < grid.columns).then(|| i + 1),
            (y > 0).then(|| i - grid.columns),
            (y + 1 < grid.rows).then(|| i + grid.columns),
        ];
        for j in neighbours.into_iter().flatten() {
            if owners[j] == NONE {
                owners[j] = owners[i];
                queue.push_back(j);
            }
        }
    }
    owners
}

/// Names each system for the overlay, the tour and the popup of its anchor: the tour visits the
/// largest first, from where their anchors float.
fn announce(stage: &mut Stage, processes: &[&Process], systems: &[System]) {
    for system in systems {
        let position = stage.positions[&system.anchor];
        if let Some(anchor) = processes.iter().find(|p| p.id == system.anchor) {
            stage.titles.push(Title {
                at: position,
                reach: system.extent,
                text: system_label(anchor, system.count),
            });
        }
        stage.tour.push(TourTarget {
            id: system.anchor,
            position,
            weight: system.count as f32,
        });
        stage.popup.push((
            system.anchor,
            format!(
                "system {} {} across {} processes",
                platform::MEMORY_LABEL,
                bytes(system.memory),
                system.count
            ),
        ));
    }
}

impl ViewModule for Ripple {
    /// The machine as a pond: each cgroup is a cluster of pebbles, and every busy process drives
    /// ripples at its own pitch, so a cluster's rings interfere with each other and spread over
    /// open water to the next. Buoys float on the surface sized by memory; every patch of water
    /// belongs to its nearest process; births drip and exits splash. Pressure raises a swell.
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let camera = stage.camera;
        let time = stage.time;
        let selected = stage.selected;
        let [cpu, _, io] = snapshot.pressure;
        let stir = bounded(cpu + io, 20.0);
        let (radii, systems) = self.scatter(stage, processes, snapshot);
        let grows: Vec<f32> = processes
            .iter()
            .map(|process| stage.growth(process.id))
            .collect();
        let Some((grid, located)) = footprint(stage, processes) else {
            return processes.len();
        };
        let mut wave = match self.wave.take() {
            Some(wave) if similar(&wave.grid, &grid) => wave,
            _ => Wave::new(grid, time),
        };
        let grid = wave.grid;
        for &point in stage.splashes {
            wave.drive([point[0], point[1]], -90.0);
        }
        for id in stage.newborn {
            if let Some(p) = stage.positions.get(id) {
                wave.drive([p[0], p[1]], 30.0);
            }
        }
        let sources: Vec<([f32; 2], f32, f32, f32)> = processes
            .iter()
            .zip(&located)
            .filter(|(process, _)| process.cpu > 0.3)
            .map(|(process, &at)| {
                // pid only: kept for pixel stability, see A33
                let hash = hash::knuth(process.id.pid);
                let frequency = 0.35 + 0.9 * (hash >> 24) as f32 / 255.0;
                let phase = (hash >> 8 & 0xffff) as f32 / 65536.0 * TAU;
                (
                    at,
                    900.0 * bounded(process.cpu, 40.0),
                    TAU * frequency,
                    phase,
                )
            })
            .collect();
        wave.advance(time, |wave, now, dt| {
            for &(at, loudness, omega, phase) in &sources {
                wave.drive(at, loudness * (omega * now + phase).sin() * dt);
            }
        });
        let owners = territories(&grid, &located, &radii);
        let swell = 0.01 + 0.12 * stir;
        let surface = |x: f32, y: f32, height: f32| {
            height * 0.09
                + swell
                    * ((x * 0.21 + y * 0.08 + time * 0.9).sin()
                        + 0.7 * (y * 0.17 - x * 0.11 - time * 0.7 + 1.3).sin()
                        + 0.5 * ((x + y) * 0.13 + time * 1.3 + 2.1).sin())
                    / 2.2
        };
        let (columns, rows) = (grid.columns, grid.rows);
        let heights: Vec<f32> = (0..grid.len())
            .map(|i| {
                let [x, y] = grid.world((i % columns) as f32, (i / columns) as f32);
                surface(x, y, wave.height[i])
            })
            .collect();
        let viewer = camera.viewer();
        // Light from the side of the view: calm water stays dark and only slopes catch glints.
        let light = normalize([viewer[1] * 0.75, -viewer[0] * 0.75, 0.65]);
        let half = normalize([
            light[0] + viewer[0],
            light[1] + viewer[1],
            light[2] + viewer[2],
        ]);
        // Lit per node from the height field's gradient and averaged over each facet, so the two
        // triangles of a cell shade alike. Highlights blend towards moonlight by at most
        // RIPPLE_SHINE and never add up to white.
        let shades: Vec<[f32; 3]> = (0..grid.len())
            .map(|i| {
                let (x, y) = (i % columns, i / columns);
                let gradient = |a: usize, b: usize, cells: usize| {
                    (heights[b] - heights[a]) / (cells.max(1) as f32 * grid.cell)
                };
                let (x0, x1) = (x.saturating_sub(1), (x + 1).min(columns - 1));
                let (y0, y1) = (y.saturating_sub(1), (y + 1).min(rows - 1));
                let normal = normalize([
                    -gradient(y * columns + x0, y * columns + x1, x1 - x0),
                    -gradient(y0 * columns + x, y1 * columns + x, y1 - y0),
                    1.0,
                ]);
                let diffuse = dot(normal, light).max(0.0);
                let glint = dot(normal, half).max(0.0).powi(48);
                let crest = (heights[i] * 1.8).clamp(-0.5, 1.0);
                let foam = (wave.velocity[i].abs() * 0.05).min(1.0);
                let tint = processes
                    .get(owners[i] as usize)
                    .map_or(RIPPLE_DEEP, |p| kind_color(p).map(f32::from));
                let shine = (glint * 0.75 + foam * 0.25).min(RIPPLE_SHINE);
                [0, 1, 2].map(|k| {
                    let water = (RIPPLE_DEEP[k] * 0.92 + tint[k] * 0.08) * (0.55 + 0.5 * diffuse)
                        + RIPPLE_CREST[k] * crest;
                    water + (RIPPLE_MOON[k] - water) * shine
                })
            })
            .collect();
        let corner = |node: usize| {
            let [x, y] = grid.world((node % columns) as f32, (node / columns) as f32);
            [x, y, heights[node]]
        };
        for y in 0..rows - 1 {
            for x in 0..columns - 1 {
                let a = y * columns + x;
                let (b, c, d) = (a + 1, a + columns + 1, a + columns);
                for nodes in [[a, b, c], [a, c, d]] {
                    let color = [0, 1, 2].map(|k| {
                        (nodes.iter().map(|&n| shades[n][k]).sum::<f32>() / 3.0).clamp(0.0, 255.0)
                            as u8
                    });
                    stage
                        .frame
                        .facet(camera, nodes.map(corner), color, owners[a]);
                }
            }
        }
        for (index, (process, &at)) in processes.iter().zip(&located).enumerate() {
            let radius = radii[index] * grows[index];
            let z = grid.sample(&heights, at, mix) + radius * 0.35;
            let position = [at[0], at[1], z];
            stage.positions.insert(process.id, position);
            glows(stage.frame, camera, process, position, radii[index]);
            stage.frame.sphere(
                camera,
                position,
                radius,
                kind_color(process),
                index as u32,
                selected == Some(process.id),
            );
        }
        self.wave = Some(wave);
        announce(stage, processes, &systems);
        processes.len()
    }

    fn legend(&self, _snapshot: &Snapshot) -> String {
        " Pebbles = processes, clustered by cgroup | ripples = CPU, each at its own pitch | size = memory | water tint = nearest process | drops = births, splashes = exits | swell = pressure".into()
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// The ground-plane grid covering the pebbles with a margin, and each pebble's ground position.
fn footprint(stage: &Stage, processes: &[&Process]) -> Option<(Grid, Vec<[f32; 2]>)> {
    let located: Vec<[f32; 2]> = processes
        .iter()
        .map(|process| {
            let p = stage.positions[&process.id];
            [p[0], p[1]]
        })
        .collect();
    let grid = cover(&located, 150)?;
    Some((grid, located))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;
    use crate::render::{Camera, Frame, Item, RenderInput, Scene, hiding};
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

    fn pond(scene: &Scene) -> &Ripple {
        scene
            .module::<Ripple>(view("ripple"))
            .expect("the pond has drawn")
    }

    #[test]
    fn ripples_answer_to_activity() {
        let mut snapshot = demo(2.0, 64);
        for p in &mut snapshot.processes {
            p.cpu = 0.0;
        }
        let run = |snapshot: &Snapshot| {
            let mut scene = Scene::new();
            for k in 0..40 {
                render(&mut scene, snapshot, view("ripple"), k as f32 * 0.05);
            }
            scene
        };
        let calm = run(&snapshot);
        assert_eq!(pond(&calm).wave.as_ref().unwrap().energy(), 0.0);
        snapshot.processes[1].cpu = 80.0;
        let busy = run(&snapshot);
        assert!(pond(&busy).wave.as_ref().unwrap().energy() > 1.0);
        assert_eq!(busy.positions.len(), busy.visible);
    }

    #[test]
    fn focusing_and_clearing_focus_keeps_every_ripple_cluster_in_place() {
        hiding::focus_and_clear_by(view("ripple"), &demo(30.0, 160), hiding::ground);
    }

    #[test]
    fn processes_beyond_the_limit_keep_their_ripple_place() {
        hiding::limit_and_lift_by(view("ripple"), &demo(30.0, 160), hiding::ground);
    }

    #[test]
    fn pebbles_cluster_by_cgroup_without_touching_and_stay_put() {
        let mut snapshot = demo(1.0, 64);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, view("ripple"), 1.0);
        let clusters = &pond(&scene).clusters;
        assert_eq!(clusters.len(), 4);
        let flat = |scene: &Scene, id: &Identity| {
            let p = scene.positions[id];
            [p[0], p[1]]
        };
        for (i, a) in snapshot.processes.iter().enumerate() {
            let at = flat(&scene, &a.id);
            let home = clusters[&a.group].center;
            for (name, cluster) in clusters {
                let d = |c: [f32; 2]| (at[0] - c[0]).hypot(at[1] - c[1]);
                assert!(*name == a.group || d(home) < d(cluster.center));
            }
            for b in snapshot.processes[i + 1..]
                .iter()
                .filter(|b| b.group == a.group)
            {
                let gap = (at[0] - flat(&scene, &b.id)[0]).hypot(at[1] - flat(&scene, &b.id)[1]);
                let touching = mass_radius(a.memory as f32) + mass_radius(b.memory as f32);
                assert!(gap >= touching, "{} and {} overlap", a.name, b.name);
            }
        }
        let before: HashMap<Identity, [f32; 2]> = snapshot
            .processes
            .iter()
            .map(|p| (p.id, flat(&scene, &p.id)))
            .collect();
        snapshot.processes.remove(20);
        snapshot.processes[30].memory += snapshot.processes[30].memory / 20;
        snapshot.processes[31].cpu = 400.0;
        render(&mut scene, &snapshot, view("ripple"), 1.1);
        for p in &snapshot.processes {
            assert_eq!(flat(&scene, &p.id), before[&p.id], "{} moved", p.name);
        }
    }

    #[test]
    fn ripple_water_stays_dim_however_hard_it_is_stirred() {
        let mut snapshot = demo(2.0, 64);
        snapshot.pressure = [90.0; 3];
        for p in &mut snapshot.processes {
            p.cpu = 400.0;
        }
        let mut scene = Scene::new();
        let mut frame = render(&mut scene, &snapshot, view("ripple"), 0.0);
        for k in 1..80 {
            if k % 10 == 0 {
                scene.splash([0.0; 3]);
            }
            frame = render(&mut scene, &snapshot, view("ripple"), k as f32 * 0.05);
        }
        assert!(pond(&scene).wave.as_ref().unwrap().energy() > 1.0);
        let brightest = frame
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(_, color, _) => color.iter().max().copied(),
                _ => None,
            })
            .max()
            .unwrap();
        assert!(brightest < 190, "water reached {brightest}");
    }

    #[test]
    fn a_splash_and_a_birth_stir_the_water_and_a_calm_pond_stays_flat() {
        let mut snapshot = demo(2.0, 32);
        for p in &mut snapshot.processes {
            p.cpu = 0.0;
        }
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, view("ripple"), 0.0);
        render(&mut scene, &snapshot, view("ripple"), 0.05);
        assert_eq!(pond(&scene).wave.as_ref().unwrap().energy(), 0.0);
        let mut born = snapshot.clone();
        let mut arrival = born.processes[0].clone();
        arrival.id.pid += 10_000;
        born.processes.push(arrival);
        render(&mut scene, &born, view("ripple"), 0.1);
        render(&mut scene, &born, view("ripple"), 0.15);
        let dripped = pond(&scene).wave.as_ref().unwrap().energy();
        assert!(dripped > 0.0, "a birth drips");
        scene.splash([0.0; 3]);
        render(&mut scene, &born, view("ripple"), 0.2);
        render(&mut scene, &born, view("ripple"), 0.25);
        assert!(
            pond(&scene).wave.as_ref().unwrap().energy() > dripped,
            "a splash stirs more"
        );
    }

    #[test]
    fn a_cluster_of_drawn_pebbles_is_a_system_to_label_tour_and_total() {
        let snapshot = demo(1.0, 64);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, view("ripple"), 1.0);
        let ripple = pond(&scene);
        assert_eq!(scene.titles.len(), ripple.clusters.len());
        assert_eq!(scene.tour.len(), scene.titles.len());
        let counts: Vec<f32> = scene.tour.iter().map(|t| t.weight).collect();
        assert!(
            counts.windows(2).all(|pair| pair[0] >= pair[1]),
            "the largest first: {counts:?}"
        );
        for (title, target) in scene.titles.iter().zip(&scene.tour) {
            let anchor = target.id;
            let members = target.weight as usize;
            assert_eq!(target.position, scene.positions[&anchor]);
            assert_eq!(title.at, target.position);
            let name = snapshot
                .processes
                .iter()
                .find(|p| p.id == anchor)
                .map(|p| system_label(p, members))
                .unwrap();
            assert_eq!(title.text, name);
            let (group, slot) = &ripple.pebbles[&anchor];
            let cluster = &ripple.clusters[group];
            assert_eq!(title.reach, cluster.extent);
            assert!(
                cluster.slots.iter().all(|(&s, _)| s >= *slot),
                "the lowest slot"
            );
            let memory: u64 = snapshot
                .processes
                .iter()
                .filter(|p| &p.group == group)
                .map(|p| p.memory)
                .sum();
            assert!(
                scene.popup.iter().any(|(id, line)| *id == anchor
                    && line.ends_with(&format!("across {members} processes"))
                    && line.contains(&crate::model::bytes(memory))),
                "{anchor:?}: {:?}",
                scene.popup
            );
        }
    }

    #[test]
    fn a_lone_pebble_is_no_system() {
        let mut snapshot = demo(1.0, 8);
        for (i, process) in snapshot.processes.iter_mut().enumerate() {
            process.group = format!("group {i}");
        }
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, view("ripple"), 1.0);
        assert!(scene.titles.is_empty() && scene.tour.is_empty() && scene.popup.is_empty());
    }

    #[test]
    fn the_legend_names_what_the_pond_draws() {
        let scene = Scene::new();
        let legend = scene.legend(view("ripple"), &Snapshot::default());
        assert!(legend.starts_with(" Pebbles = processes, clustered by cgroup"));
        assert!(legend.ends_with("swell = pressure"));
    }
}
