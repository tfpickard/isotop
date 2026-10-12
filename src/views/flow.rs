//! The Flow view: spacetime weather over the process systems of `systems`. Memory bends a sheet
//! into gravity wells (a bowl as wide as each system, a dimple for each heavy process), CPU spins
//! whirlpools, sockets become two-lane rivers between the wells of processes that talk, and
//! connections leaving the machine send sparks up off the sheet. Particles in each emitter's
//! colour trace it all; births and exits kick the particles near them.
//!
//! The systems are laid out with the bodies held still (`motion` 0), the sheet is a `medium::Flow`
//! that survives small changes of the layout, and its velocity field is worked out again every
//! quarter second.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use super::ViewModule;
use super::systems::{self, Galaxy, HUB, Layout, is_hub};
use crate::medium::{Flow, TRAIL};
use crate::model::{Identity, Process, Snapshot, bounded};
use crate::palette::kind_color;
use crate::render::{Camera, Frame, OUTSIDE, Point, Stage, footprint, glows, similar};

/// The most particles in the sheet.
const PARTICLES: usize = 6000;

/// The Flow view: where its systems stand, the sheet and whether it drew a hub last frame.
#[derive(Default)]
pub struct Wells {
    galaxy: Galaxy,
    /// Whether the last frame drew a hub, for the legend.
    hubs: bool,
    /// The sheet and its particles, kept while the footprint of the layout is about the same.
    sheet: Option<Flow>,
}

#[cfg(test)]
impl Wells {
    pub(super) fn galaxy(&self) -> &Galaxy {
        &self.galaxy
    }
}

/// A socket link as a two-lane river in the flow field.
#[derive(Clone, Copy)]
struct Lane {
    start: [f32; 2],
    direction: [f32; 2],
    length: f32,
    width: f32,
    speed: f32,
}

fn room(flow: &Flow) -> bool {
    flow.particles.len() < PARTICLES
}

impl ViewModule for Wells {
    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let [cpu, memory, io] = snapshot.pressure;
        let stir = bounded(cpu + memory + io, 25.0);
        let hubs = systems::hubs(processes, snapshot, stage.focus);
        self.hubs = !hubs.is_empty();
        let processes: Vec<&Process> = processes.iter().copied().chain(&hubs).collect();
        let layout = systems::layout(&mut self.galaxy, stage, &processes, snapshot, 0.0);
        self.pour(stage, &processes, &layout, snapshot, stir);
        layout.report(stage);
        layout.shown
    }

    fn legend(&self, _snapshot: &Snapshot) -> String {
        format!(
            " Wells = memory | whirlpools + coloured particles = CPU | two-lane rivers = sockets | rising sparks = outside | turbulence = pressure{}",
            systems::hub_note(self.hubs)
        )
    }
}

impl Wells {
    /// Spacetime weather: memory bends a sheet into gravity wells, CPU spins whirlpools, sockets
    /// become two-lane rivers between the wells of processes that talk, and connections leaving
    /// the machine send sparks up off the sheet. Particles in each emitter's colour trace it all.
    fn pour(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        layout: &Layout,
        snapshot: &Snapshot,
        stir: f32,
    ) {
        let time = stage.time;
        let Some((grid, located)) = footprint(stage.positions, processes, &layout.bodies, 140)
        else {
            return;
        };
        let mut flow = match self.sheet.take() {
            Some(flow) if similar(&flow.grid, &grid) => flow,
            _ => Flow::new(grid, time),
        };
        let dt = (time - flow.time).clamp(0.0, 0.1);
        flow.time = time;
        if !(0.0..0.25).contains(&(time - flow.field_time)) {
            weather(&mut flow, processes, layout, &located, snapshot);
            flow.field_time = time;
        }
        let pour = Pour {
            processes,
            layout,
            located: &located,
            snapshot,
            dt,
        };
        pour.background(&mut flow);
        pour.around_bodies(&mut flow);
        pour.along_links(&mut flow);
        pour.sparks(&mut flow);
        let born: Vec<Point> = stage
            .newborn
            .iter()
            .filter_map(|id| stage.positions.get(id).copied())
            .collect();
        disturb(&mut flow, stage.splashes, &born);
        let turbulence = 0.4 + 6.0 * stir;
        flow.advance(dt, |q| {
            [
                turbulence * (q[1] * 0.31 + time * 0.8).sin(),
                turbulence * (q[0] * 0.29 - time * 0.6).cos(),
            ]
        });
        fabric(stage.frame, stage.camera, &flow);
        bodies(stage, processes, layout, &located, &flow);
        particles(stage.frame, stage.camera, &flow);
        self.sheet = Some(flow);
    }
}

/// What one frame pours into the sheet: where the bodies stand on the ground and the seconds
/// since the last frame (at most a tenth).
struct Pour<'a> {
    processes: &'a [&'a Process],
    layout: &'a Layout<'a>,
    located: &'a [[f32; 2]],
    snapshot: &'a Snapshot,
    dt: f32,
}

impl Pour<'_> {
    /// Faint particles anywhere, so the field shows even where nothing busy stands.
    fn background(&self, flow: &mut Flow) {
        let grid = flow.grid;
        let mut background = self.dt * 400.0;
        while background > flow.random() && room(flow) {
            background -= 1.0;
            let p = grid.world(
                flow.random() * (grid.columns - 1) as f32,
                flow.random() * (grid.rows - 1) as f32,
            );
            let life = 5.0 + 3.0 * flow.random();
            flow.spawn(p, [90, 130, 190], life, false, false, [0.0; 2]);
        }
    }

    /// Particles in a busy process's colour around it, as many as its CPU asks for.
    fn around_bodies(&self, flow: &mut Flow) {
        for (body, &at) in self.layout.bodies.iter().zip(self.located) {
            let process = self.processes[body.index];
            let mut expected = if process.cpu > 0.5 {
                self.dt * (1.0 + 30.0 * bounded(process.cpu, 40.0))
            } else {
                0.0
            };
            let reach = self.layout.tree.radii[body.index] * 1.5 + 0.5;
            while expected > flow.random() && room(flow) {
                expected -= 1.0;
                let angle = flow.random() * TAU;
                let distance = reach * flow.random().sqrt();
                let p = [
                    at[0] + distance * angle.cos(),
                    at[1] + distance * angle.sin(),
                ];
                let life = 4.0 + 3.0 * flow.random();
                flow.spawn(p, kind_color(process), life, true, false, [0.0; 2]);
            }
        }
    }

    /// Traffic entering the lane that leads away from each end of a socket link.
    fn along_links(&self, flow: &mut Flow) {
        let place: HashMap<Identity, (usize, [f32; 2])> = self
            .layout
            .bodies
            .iter()
            .zip(self.located)
            .map(|(body, &at)| (self.processes[body.index].id, (body.index, at)))
            .collect();
        for &(a, b, count) in &self.snapshot.links {
            let (Some(&(ia, pa)), Some(&(ib, pb))) = (place.get(&a), place.get(&b)) else {
                continue;
            };
            let length = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
            if length < 0.5 {
                continue;
            }
            let direction = [(pb[0] - pa[0]) / length, (pb[1] - pa[1]) / length];
            let lane = 0.6 * (2.5 + 0.8 * (count as f32).ln_1p());
            let normal = [-direction[1] * lane, direction[0] * lane];
            let busy = self.processes[ia].cpu + self.processes[ib].cpu;
            // Traffic enters the lane that leads away from each end, so rivers carry it across.
            for (from, side, color) in [
                (pa, 1.0, kind_color(self.processes[ia])),
                (pb, -1.0, kind_color(self.processes[ib])),
            ] {
                let mut expected =
                    self.dt * (0.4 + 0.3 * count as f32) * (0.5 + 2.0 * bounded(busy, 30.0));
                while expected > flow.random() && room(flow) {
                    expected -= 1.0;
                    let start = [from[0] + normal[0] * side, from[1] + normal[1] * side];
                    flow.spawn(start, color, length / 4.0 + 2.0, true, false, [0.0; 2]);
                }
            }
        }
    }

    /// Sparks rising off the sheet from every process with a connection leaving the machine.
    fn sparks(&self, flow: &mut Flow) {
        // A `HashMap` iterates in a different order every run; the particles draw from one random
        // stream, so a fixed order keeps the frame repeatable.
        let mut outside: Vec<_> = self.snapshot.outside.iter().collect();
        outside.sort_unstable_by_key(|&(id, _)| *id);
        for (id, &count) in outside {
            if let Some(index) = self.processes.iter().position(|p| p.id == *id)
                && let Some(slot) = self.layout.bodies.iter().position(|b| b.index == index)
            {
                let mut expected = self.dt * 2.0 * count as f32;
                while expected > flow.random() && room(flow) {
                    expected -= 1.0;
                    flow.spawn(self.located[slot], OUTSIDE, 2.5, true, true, [0.0; 2]);
                }
            }
        }
    }
}

/// A burst of particles flung outwards from every place a process left and, more gently, from
/// every process that arrived.
fn disturb(flow: &mut Flow, splashes: &[Point], born: &[Point]) {
    for point in splashes {
        for k in 0..40 {
            let angle = k as f32 / 40.0 * TAU;
            let kick = [9.0 * angle.cos(), 9.0 * angle.sin()];
            flow.spawn(
                [point[0], point[1]],
                [255, 226, 186],
                1.6,
                true,
                false,
                kick,
            );
        }
    }
    for p in born {
        for k in 0..12 {
            let angle = k as f32 / 12.0 * TAU;
            let kick = [3.0 * angle.cos(), 3.0 * angle.sin()];
            flow.spawn([p[0], p[1]], [220, 240, 255], 1.2, true, false, kick);
        }
    }
}

/// The sheet as a net of lines, brighter and more violet with depth so wells read even from
/// above.
fn fabric(frame: &mut Frame, camera: &Camera, flow: &Flow) {
    let grid = flow.grid;
    let deepest = flow.depth.iter().fold(-1.0_f32, |a, &b| a.min(b));
    let color = |depth: f32| {
        let t = (depth / deepest).clamp(0.0, 1.0);
        [
            (28.0 + 70.0 * t) as u8,
            (42.0 + 40.0 * t) as u8,
            (72.0 + 120.0 * t) as u8,
        ]
    };
    let node = |x: usize, y: usize| {
        let [wx, wy] = grid.world(x as f32, y as f32);
        [wx, wy, flow.depth[y * grid.columns + x]]
    };
    for y in (0..grid.rows).step_by(3) {
        for x in 1..grid.columns {
            let (a, b) = (node(x - 1, y), node(x, y));
            frame.line(camera, a, b, color(b[2]));
        }
    }
    for x in (0..grid.columns).step_by(3) {
        for y in 1..grid.rows {
            let (a, b) = (node(x, y - 1), node(x, y));
            frame.line(camera, a, b, color(b[2]));
        }
    }
}

/// The bodies, sunk into the sheet where they stand: a process as a lit sphere with its glows,
/// a hub as an outline. Their positions in the stage are the ones on the sheet.
fn bodies(
    stage: &mut Stage,
    processes: &[&Process],
    layout: &Layout,
    located: &[[f32; 2]],
    flow: &Flow,
) {
    for (body, &at) in layout.bodies.iter().zip(located) {
        let process = processes[body.index];
        let radius = layout.tree.radii[body.index] * 0.7 * body.grow;
        let position = [at[0], at[1], flow.depth_at(at) + radius * 0.6];
        stage.positions.insert(process.id, position);
        if is_hub(&process.id) {
            stage.frame.outline(stage.camera, position, radius, HUB);
            continue;
        }
        glows(stage.frame, stage.camera, process, position, radius);
        stage.frame.sphere(
            stage.camera,
            position,
            radius,
            kind_color(process),
            body.index as u32,
            stage.selected == Some(process.id),
        );
    }
}

/// The particles, each a trail of beams that fades in and out, with a glow on the bright ones.
fn particles(frame: &mut Frame, camera: &Camera, flow: &Flow) {
    for particle in &flow.particles {
        let fade =
            (particle.age / 0.3).min(1.0) * (1.0 - particle.age / particle.life).max(0.0).sqrt();
        let strength = if particle.bright { 0.9 } else { 0.6 } * fade;
        for k in 0..TRAIL - 1 {
            let (a, b) = (particle.trail[k], particle.trail[k + 1]);
            if a != b {
                let weight = strength * (1.0 - k as f32 / TRAIL as f32);
                frame.beam(camera, a, b, particle.color, weight);
            }
        }
        if particle.bright {
            frame.glow(camera, particle.trail[0], 3.0, particle.color, 0.6 * fade);
        }
    }
}

/// Recomputes the flow's velocity field and well depths from the current processes.
fn weather(
    flow: &mut Flow,
    processes: &[&Process],
    layout: &Layout,
    located: &[[f32; 2]],
    snapshot: &Snapshot,
) {
    let (bodies, radii) = (&layout.bodies, &layout.tree.radii);
    let at: HashMap<Identity, [f32; 2]> = bodies
        .iter()
        .zip(located)
        .map(|(body, &p)| (processes[body.index].id, p))
        .collect();
    // Each solar system bends a bowl as wide as the system, deeper for more memory; heavy
    // processes add their own dimples. (centre, depth, width, inflow)
    let mut wells: Vec<([f32; 2], f32, f32, f32)> = layout
        .stars
        .iter()
        .filter_map(|star| {
            let (id, extent) = (star.id, star.reach);
            let bytes = layout.mass.get(&id).map_or_else(
                || {
                    processes
                        .iter()
                        .find(|p| p.id == id)
                        .map_or(0, |p| p.memory)
                },
                |&(bytes, _)| bytes,
            );
            let depth = 2.0 + 14.0 * bounded(bytes as f32 / 1048576.0, 2048.0);
            Some((*at.get(&id)?, depth, (extent * 0.5).max(4.0), 0.0))
        })
        .collect();
    wells.extend(
        bodies
            .iter()
            .zip(located)
            .filter(|(body, _)| radii[body.index] > 0.9)
            .map(|(body, &p)| {
                let r = radii[body.index];
                (p, 0.4 * r, 1.0 + 2.0 * r, 0.6 * r * r * r)
            }),
    );
    let vortices: Vec<([f32; 2], f32, f32)> = bodies
        .iter()
        .zip(located)
        .filter(|(body, _)| processes[body.index].cpu > 0.5)
        .map(|(body, &at)| {
            let process = processes[body.index];
            let spin = if process.id.pid.is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            let strength = (6.0 + 60.0 * bounded(process.cpu, 40.0)) * spin;
            (at, strength, (1.0 + 2.0 * radii[body.index]).powi(2))
        })
        .collect();
    let place: HashMap<Identity, ([f32; 2], f32)> = bodies
        .iter()
        .zip(located)
        .map(|(body, &at)| (processes[body.index].id, (at, processes[body.index].cpu)))
        .collect();
    let jets: Vec<Lane> = snapshot
        .links
        .iter()
        .filter_map(|(a, b, count)| {
            let (&(pa, ca), &(pb, cb)) = (place.get(a)?, place.get(b)?);
            let length = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
            (length > 0.5).then(|| {
                let direction = [(pb[0] - pa[0]) / length, (pb[1] - pa[1]) / length];
                let width = 2.5 + 0.8 * (*count as f32).ln_1p();
                let speed =
                    (8.0 + 3.0 * (*count as f32).ln_1p()) * (0.6 + 1.4 * bounded(ca + cb, 30.0));
                Lane {
                    start: pa,
                    direction,
                    length,
                    width,
                    speed,
                }
            })
        })
        .collect();
    let grid = flow.grid;
    flow.velocity.fill([0.0; 2]);
    flow.depth.fill(0.0);
    // Every influence fades with distance, so each one only visits the cells within its reach.
    let around = |center: [f32; 2], reach: f32, apply: &mut dyn FnMut([f32; 2], usize)| {
        let low = grid.locate([center[0] - reach, center[1] - reach]);
        let high = grid.locate([center[0] + reach, center[1] + reach]);
        let span = |a: f32, b: f32, limit: usize| {
            (a.floor().max(0.0) as usize)..((b.ceil() + 1.0).max(0.0) as usize).min(limit)
        };
        for j in span(low[1], high[1], grid.rows) {
            for i in span(low[0], high[0], grid.columns) {
                apply(grid.world(i as f32, j as f32), j * grid.columns + i);
            }
        }
    };
    let (velocity, depth) = (&mut flow.velocity, &mut flow.depth);
    for &(c, deep, width, inflow) in &wells {
        around(c, width * 3.0, &mut |p, cell| {
            let d = [p[0] - c[0], p[1] - c[1]];
            let distance2 = d[0] * d[0] + d[1] * d[1];
            // Gaussian bowls stay local, so the sheet is flat between systems.
            depth[cell] -= deep * (-distance2 / (width * width)).exp();
            let r2 = distance2 + width * width;
            let pull = inflow / r2 / r2.sqrt();
            velocity[cell][0] -= d[0] * pull;
            velocity[cell][1] -= d[1] * pull;
        });
    }
    for &(c, strength, core) in &vortices {
        // Beyond this radius the swirl is below 0.1 units per second.
        let reach = (strength.abs() * 10.0).sqrt().max(core.sqrt() * 3.0);
        around(c, reach, &mut |p, cell| {
            let d = [p[0] - c[0], p[1] - c[1]];
            let swirl = strength / (d[0] * d[0] + d[1] * d[1] + core);
            velocity[cell][0] -= d[1] * swirl;
            velocity[cell][1] += d[0] * swirl;
        });
    }
    for &Lane {
        start: a,
        direction,
        length,
        width,
        speed,
    } in &jets
    {
        let middle = [
            a[0] + direction[0] * length * 0.5,
            a[1] + direction[1] * length * 0.5,
        ];
        around(middle, length * 0.5 + width * 2.0, &mut |p, cell| {
            let d = [p[0] - a[0], p[1] - a[1]];
            let along = d[0] * direction[0] + d[1] * direction[1];
            if !(0.0..length).contains(&along) {
                return;
            }
            // Two lanes, one each way: the side of the line decides the direction.
            let side = direction[0] * d[1] - direction[1] * d[0];
            let lane = (side.abs() - width * 0.6) / (width * 0.5);
            let weight = (-lane * lane).exp() * (PI * along / length).sin().sqrt();
            let sign = if side > 0.0 { 1.0 } else { -1.0 };
            velocity[cell][0] += direction[0] * speed * weight * sign;
            velocity[cell][1] += direction[1] * speed * weight * sign;
        });
    }
    for v in velocity.iter_mut() {
        let speed = v[0].hypot(v[1]);
        if speed > 20.0 {
            *v = v.map(|c| c * 20.0 / speed);
        }
    }
    for d in depth.iter_mut() {
        *d = d.max(-20.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;
    use crate::render::{Camera, RenderInput, Scene, hiding};
    use crate::views::view;

    #[test]
    fn the_sheet_answers_to_activity() {
        let mut snapshot = demo(2.0, 64);
        for p in &mut snapshot.processes {
            p.cpu = 0.0;
        }
        snapshot.processes[1].cpu = 80.0;
        let mut scene = Scene::new();
        for k in 0..40 {
            let mut frame = scene.render(&RenderInput::small(
                &snapshot,
                view("flow"),
                &Camera::default(),
                k as f32 * 0.05,
            ));
            frame.rasterize();
        }
        let flow = scene.module::<Wells>(view("flow")).expect("Flow has drawn");
        let particles = &flow.sheet.as_ref().expect("the sheet is poured").particles;
        assert!(
            particles.iter().any(|p| p.bright),
            "busy process and links emit"
        );
        assert!(
            particles.iter().any(|p| !p.bright),
            "background reveals the field"
        );
    }

    #[test]
    fn focusing_and_clearing_focus_keeps_every_flow_system_in_place() {
        hiding::focus_and_clear_by(view("flow"), &demo(30.0, 160), hiding::ground);
    }

    #[test]
    fn processes_beyond_the_limit_keep_their_flow_system_place() {
        hiding::limit_and_lift_without_selection(view("flow"), hiding::ground);
    }
}
