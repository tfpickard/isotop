//! The Orbit view: the process tree as nested Keplerian systems (`systems`), drawn as orbits.
//! A body's size is the memory of the subtree it anchors, thread rings circle the ones with many
//! threads, and CPU glows and trails behind it. The star of each system is its top-level
//! process, or a hollow hub where isotop cannot read the parent.
//!
//! The bodies turn: the orbital phase follows the scene's time, and the galaxy of systems
//! revolves about its barycenter.

use std::collections::HashSet;
use std::f32::consts::TAU;

use super::ViewModule;
use super::systems::{self, Galaxy, HUB, Layout, Orbit, is_hub, rings};
use crate::model::{Process, Snapshot, bounded};
use crate::palette::{kind_color, tint};
use crate::platform::Source;
use crate::render::{MIN_BODY_PX, NVIDIA, Stage, WARM};
use crate::simulation::hash;

/// The Orbit view: where its systems stand and whether it drew a hub last frame.
#[derive(Default)]
pub struct Orbits {
    galaxy: Galaxy,
    /// Whether the last frame drew a hub, for the legend.
    hubs: bool,
}

#[cfg(test)]
impl Orbits {
    pub(super) fn galaxy(&self) -> &Galaxy {
        &self.galaxy
    }
}

impl ViewModule for Orbits {
    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let hubs = systems::hubs(processes, snapshot, stage.focus);
        self.hubs = !hubs.is_empty();
        let processes: Vec<&Process> = processes.iter().copied().chain(&hubs).collect();
        let time = stage.time;
        let layout = systems::layout(&mut self.galaxy, stage, &processes, snapshot, time);
        paint(stage, &processes, &layout);
        layout.report(stage);
        layout.shown
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        format!(
            " Size = memory (stars: whole system) | rings = threads | glow + trail = CPU{} | cyan arcs = sockets, pink = outside{}",
            gpu_note(snapshot),
            systems::hub_note(self.hubs)
        )
    }
}

/// The Orbit legend's GPU entry, left out where per-process GPU memory is not measured.
fn gpu_note(snapshot: &Snapshot) -> &'static str {
    if snapshot.missing.contains(Source::GpuMemory) {
        ""
    } else {
        " | green = NVIDIA GPU"
    }
}

/// Draws the orbits, the glows and trails of the busy bodies, the bodies and their rings.
fn paint(stage: &mut Stage, processes: &[&Process], layout: &Layout) {
    let (tree, paths, bodies) = (&layout.tree, &layout.paths, &layout.bodies);
    let frame = &mut *stage.frame;
    let (camera, selected) = (stage.camera, stage.selected);
    let mut drawn = HashSet::new();
    for path in paths {
        let highlighted = selected == Some(path.id);
        let ring = (
            (path.orbit.center[0] * 8.0) as i32,
            (path.orbit.center[1] * 8.0) as i32,
            (path.orbit.a * 8.0) as i32,
        );
        let color = if highlighted {
            [163, 147, 99]
        } else {
            tint([64, 90, 122], 1.0 / (1.0 + path.depth as f32 * 0.3))
        };
        if highlighted || drawn.insert(ring) {
            frame.trace(camera, &path.orbit, color);
        }
        if path.spoke || highlighted {
            frame.line(camera, path.orbit.center, path.position, tint(color, 0.6));
        }
    }
    for path in paths.iter().filter(|path| path.cpu > 3.0) {
        let length = 2 + (14.0 * bounded(path.cpu, 40.0)) as usize;
        let step = (0.6 / path.orbit.a).min(0.25);
        let mut last = path.position;
        for k in 1..=length {
            let point = path.orbit.at(path.mean - k as f32 * step);
            let fade = 1.0 - k as f32 / (length + 1) as f32;
            frame.line(camera, last, point, tint(WARM, 0.2 + 0.8 * fade));
            last = point;
        }
    }
    for body in bodies {
        let process = processes[body.index];
        let core = (tree.radii[body.index] * camera.zoom).max(MIN_BODY_PX);
        if process.cpu > 1.0 {
            let radius = core * (2.2 + 2.0 * bounded(process.cpu, 50.0));
            let strength = 0.3 + 0.6 * bounded(process.cpu, 40.0);
            frame.glow(camera, body.position, radius, WARM, strength);
        }
        if let Some(gpu) = process.gpu_memory {
            let mib = gpu.bytes() as f32 / 1048576.0;
            let radius = core * (2.6 + 0.5 * (mib / 64.0).ln_1p());
            frame.glow(camera, body.position, radius, NVIDIA, 0.6);
        }
    }
    for body in bodies {
        let process = processes[body.index];
        let radius = tree.radii[body.index] * body.grow;
        if is_hub(&process.id) {
            frame.outline(camera, body.position, radius, HUB);
            continue;
        }
        frame.sphere(
            camera,
            body.position,
            radius,
            kind_color(process),
            body.index as u32,
            selected == Some(process.id),
        );
    }
    for body in bodies {
        let process = processes[body.index];
        let radius = tree.radii[body.index] * body.grow;
        if process.gpu_memory.is_some() {
            let halo = Orbit {
                center: body.position,
                a: radius * 1.9,
                e: 0.0,
                omega: 0.0,
            };
            frame.trace(camera, &halo, tint(NVIDIA, 0.8));
        }
        let count = rings(process.threads);
        if count == 0 {
            continue;
        }
        // Saturn-style rings in a plane tilted towards the viewer, so the front arc passes over
        // the body while the depth buffer hides the back arc behind it.
        // pid only: kept for pixel stability, see A33
        let yaw = (hash::xxhash_prime2(process.id.pid) >> 20) as f32 / 4096.0 * TAU;
        let (ys, yc) = yaw.sin_cos();
        let (ts, tc) = 0.45_f32.sin_cos();
        let across = [yc, ys, 0.0];
        let up = [-ys * tc, yc * tc, ts];
        let color = kind_color(process);
        for k in 0..count {
            let r = radius * (1.45 + 0.28 * k as f32);
            let point = |angle: f32| {
                let (s, c) = angle.sin_cos();
                [0, 1, 2].map(|i| body.position[i] + r * (c * across[i] + s * up[i]))
            };
            let shade = tint(color, 1.25 - 0.15 * k as f32);
            let mut last = point(0.0);
            for segment in 1..=48 {
                let next = point(segment as f32 / 48.0 * TAU);
                frame.line(camera, last, next, shade);
                last = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;
    use crate::model::testing::process;
    use crate::render::{Camera, RenderInput, Scene, hiding};
    use crate::views::view;

    fn render(scene: &mut Scene, snapshot: &Snapshot, time: f32) {
        let mut frame = scene.render(&RenderInput::small(
            snapshot,
            view("orbit"),
            &Camera::default(),
            time,
        ));
        frame.rasterize();
    }

    fn galaxy(scene: &Scene) -> &Galaxy {
        scene
            .module::<Orbits>(view("orbit"))
            .expect("Orbit has drawn")
            .galaxy()
    }

    #[test]
    fn focusing_and_clearing_focus_keeps_every_orbit_system_in_place() {
        hiding::focus_and_clear(view("orbit"), &demo(30.0, 160));
    }

    #[test]
    fn a_focused_system_turns_about_what_is_drawn_not_the_hidden_galaxy() {
        let sample = demo(30.0, 160);
        let (mut scene, mut time, _) = hiding::settled(view("orbit"), &sample);
        let root = sample.processes[16].id;
        assert!(
            galaxy(&scene).systems.contains_key(&root),
            "the family's root is a system"
        );
        let mut place = |time| {
            hiding::place(
                &mut scene,
                &sample,
                view("orbit"),
                time,
                4096,
                None,
                Some(root),
            )[&root]
        };
        let start = place(time);
        for _ in 0..300 {
            time += 0.1;
            let now = place(time);
            let moved = (now[0] - start[0]).hypot(now[1] - start[1]);
            assert!(moved < 0.5, "the focused system drifted {moved} at {time}");
        }
    }

    #[test]
    fn processes_beyond_the_limit_keep_their_orbit_system_place() {
        hiding::limit_and_lift_without_selection(view("orbit"), |point| point);
    }

    #[test]
    fn leaves_stay_in_orbit_and_systems_keep_clear_of_each_other() {
        let mut processes = vec![process(1, 0), process(2, 0)];
        processes.extend((10..60).map(|pid| process(pid, 2)));
        processes.extend((200..220).map(|pid| process(pid, 1)));
        for service in (100..160).step_by(10) {
            processes.push(process(service, 1));
            processes.extend((1..4).map(|k| process(service + k, service)));
        }
        let mut snapshot = Snapshot {
            processes,
            cores: 4,
            ..Default::default()
        };
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, 0.0);
        // init, kthreadd and the six services with children; kernel threads and leaf services orbit.
        assert_eq!(galaxy(&scene).systems.len(), 8);
        assert_eq!(scene.titles.len(), 8);
        assert_eq!(scene.tour.len(), 8);
        assert_eq!(scene.visible, snapshot.processes.len());
        let systems: Vec<_> = galaxy(&scene).systems.values().copied().collect();
        for (i, (a, ra)) in systems.iter().enumerate() {
            for (b, rb) in &systems[i + 1..] {
                assert!((a[0] - b[0]).hypot(a[1] - b[1]) >= ra + rb - 1e-3);
            }
        }
        snapshot.processes[5].cpu = 800.0;
        let center = galaxy(&scene).center;
        let reach = |placement: &([f32; 2], f32)| {
            (placement.0[0] - center[0]).hypot(placement.0[1] - center[1])
        };
        let before = galaxy(&scene).systems.clone();
        for step in 1..=240 {
            render(&mut scene, &snapshot, step as f32 * 0.5);
        }
        assert_eq!(
            galaxy(&scene).systems.len(),
            8,
            "turning never re-places a system"
        );
        assert_eq!(
            galaxy(&scene).center,
            center,
            "the pivot stays put while membership is stable"
        );
        let mut turned = 0;
        for (id, placement) in &galaxy(&scene).systems {
            assert!(
                (reach(placement) - reach(&before[id])).abs() < 0.01,
                "{id:?} left its orbit"
            );
            assert_eq!(placement.1, before[id].1);
            turned += usize::from(placement.0 != before[id].0);
        }
        assert!(turned >= 7, "only {turned} systems revolved");
        let service = snapshot
            .processes
            .iter()
            .find(|p| p.id.pid == 100)
            .unwrap()
            .id;
        let reserved = galaxy(&scene).systems[&service].1;
        snapshot
            .processes
            .extend((1000..1040).map(|pid| process(pid, 100)));
        render(&mut scene, &snapshot, 121.0);
        assert!(
            galaxy(&scene).systems[&service].1 > reserved,
            "the grown system was re-placed"
        );
        assert_ne!(
            galaxy(&scene).center,
            center,
            "re-placing a system moves the pivot"
        );
        let systems: Vec<_> = galaxy(&scene).systems.values().copied().collect();
        for (i, (a, ra)) in systems.iter().enumerate() {
            for (b, rb) in &systems[i + 1..] {
                assert!((a[0] - b[0]).hypot(a[1] - b[1]) >= ra + rb - 1e-2);
            }
        }
    }
}
