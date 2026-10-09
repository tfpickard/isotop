//! Reef: the machine as a coral reef. System services grow as coral colonies, one per cgroup, with
//! a polyp per process; your session's apps swim in schools; containers are crabs on the sand;
//! kernel threads drift as plankton. I/O rises as bubbles and zombies float belly-up.

use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::model::{Identity, Kind, Process, Snapshot, bounded};
use crate::pack::{Discs, Seats};
use crate::render::{Color, GOLDEN_ANGLE, NONE, Point, Stage, normalize, spin, tint};

/// Height of the water surface above the sand.
const DEPTH: f32 = 14.0;
const CORALS: [Color; 5] = [
    [236, 112, 99],
    [245, 166, 108],
    [214, 120, 190],
    [120, 200, 190],
    [250, 205, 120],
];
const FISH: [Color; 6] = [
    [250, 190, 80],
    [90, 200, 240],
    [255, 140, 100],
    [170, 230, 120],
    [240, 120, 170],
    [150, 160, 255],
];
const CRAB: Color = [178, 136, 232];
const ZOMBIE: Color = [150, 150, 150];

#[derive(Default)]
pub struct Reef {
    homes: Discs,
    seats: Seats,
    fish: HashMap<Identity, Swimmer>,
    last: Option<f32>,
}

#[derive(Clone, Copy)]
struct Swimmer {
    position: Point,
    velocity: Point,
}

impl Reef {
    pub fn draw(&mut self, stage: &mut Stage, processes: &[&Process], _: &Snapshot) -> usize {
        let time = stage.time;
        let dt = self.last.map_or(0.0, |last| (time - last).clamp(0.0, 0.1));
        self.last = Some(time);
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, process) in processes.iter().enumerate() {
            if process.kind != Kind::Kernel {
                groups.entry(habitat(process)).or_default().push(index);
            }
        }
        self.seats.assign(
            groups.iter().flat_map(|(name, members)| {
                members.iter().map(|&i| (processes[i].id, name.as_str()))
            }),
        );
        let needs: Vec<(String, f32)> = groups
            .keys()
            .map(|name| {
                let span = self.seats.span(name) as f32;
                (name.clone(), 1.6 * span.sqrt() + 2.0)
            })
            .collect();
        let homes = self.homes.arrange(&needs);
        let reach = (self.homes.reach() + 6.0).max(16.0);
        self.draw_seabed(stage, reach, time);
        let alive: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, i))
            .collect();
        self.fish.retain(|id, _| alive.contains_key(id));
        let mut shown = 0;
        let mut names: Vec<&String> = groups.keys().collect();
        names.sort();
        for name in names {
            let members = &groups[name];
            let home = homes[name];
            let room = self.homes.reserved(name) / 1.15;
            let kind = processes[members[0]].kind;
            let palette = (spin(name) / TAU * 997.0) as usize;
            match kind {
                Kind::Session => {
                    shown += self.school(
                        stage,
                        processes,
                        members,
                        home,
                        room,
                        FISH[palette % FISH.len()],
                        dt,
                    );
                }
                Kind::Container => {
                    for &index in members {
                        self.crab(stage, processes, index, home, name);
                        shown += 1;
                    }
                }
                _ => {
                    let color = CORALS[palette % CORALS.len()];
                    for &index in members {
                        self.polyp(stage, processes, index, home, room, name, color);
                        shown += 1;
                    }
                }
            }
        }
        for (index, process) in processes
            .iter()
            .enumerate()
            .filter(|(_, p)| p.kind == Kind::Kernel)
        {
            let h = hash(process.id.pid);
            let unit = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
            let drift = [
                (time * 0.07 + unit(0) * TAU).sin() * 3.0,
                (time * 0.05 + unit(8) * TAU).cos() * 3.0,
                (time * 0.11 + unit(16) * TAU).sin() * 0.8,
            ];
            let position = [
                (unit(4) * 2.0 - 1.0) * reach * 0.8 + drift[0],
                (unit(12) * 2.0 - 1.0) * reach * 0.8 + drift[1],
                2.0 + unit(20) * (DEPTH - 4.0) + drift[2],
            ];
            let heading = [
                (time * 0.07 + unit(0) * TAU).cos(),
                -(time * 0.05 + unit(8) * TAU).sin(),
                0.0,
            ];
            let tail = [0, 1, 2].map(|k| position[k] - heading[k] * 0.5);
            // Idle kernel threads are faint glints; busy ones become motes you can click.
            if process.cpu < 0.5 && stage.selected != Some(process.id) {
                stage
                    .frame
                    .beam(stage.camera, position, tail, [150, 210, 220], 0.12);
                continue;
            }
            let glow = 0.4 + 0.6 * bounded(process.cpu, 5.0);
            stage
                .frame
                .beam(stage.camera, position, tail, [150, 210, 220], glow * 0.6);
            stage.frame.sphere(
                stage.camera,
                position,
                0.05,
                tint([140, 200, 210], 0.6 + glow),
                index as u32,
                stage.selected == Some(process.id),
            );
            stage.positions.insert(process.id, position);
            shown += 1;
        }
        for (index, process) in processes.iter().enumerate() {
            if process.io_rate.is_some_and(|rate| rate > 4096.0)
                && let Some(&from) = stage.positions.get(&process.id)
            {
                let rate = process.io_rate.unwrap_or(0.0);
                let count = 1 + (3.0 * bounded(rate, 1_048_576.0)) as usize;
                for k in 0..count {
                    let rise =
                        (time * 0.25 + k as f32 / count as f32 + index as f32 * 0.37).fract();
                    let wobble = (time * 3.0 + k as f32 * 1.7).sin() * 0.25;
                    let p = [
                        from[0] + wobble,
                        from[1] - wobble,
                        from[2] + rise * (DEPTH - from[2]),
                    ];
                    stage.frame.glow(
                        stage.camera,
                        p,
                        (0.25 * stage.camera.zoom).max(3.0),
                        [200, 235, 255],
                        0.5 * (1.0 - rise),
                    );
                }
            }
        }
        for x in [-reach, reach] {
            for y in [-reach, reach] {
                stage.bounds.push([x, y, 0.0]);
                stage.bounds.push([x, y, DEPTH * 0.6]);
            }
        }
        shown
    }

    /// Rippled sand lit by caustics, kelp swaying at the edges, and light shafts from the surface.
    fn draw_seabed(&self, stage: &mut Stage, reach: f32, time: f32) {
        let camera = stage.camera;
        let cells = (2.0 * reach / 0.7).clamp(60.0, 160.0) as usize;
        let step = 2.0 * reach / cells as f32;
        let height = |x: f32, y: f32| 0.25 * (x * 0.31).sin() * (y * 0.23).cos();
        let caustic = |x: f32, y: f32| {
            let a = (x * 0.35 + time * 0.5 + 1.3 * (y * 0.27 + time * 0.3).sin()).sin();
            let b = (y * 0.31 - time * 0.4 + 1.3 * (x * 0.23 - time * 0.2).sin()).sin();
            (0.5 + 0.5 * a * b).powi(3)
        };
        for j in 0..cells {
            for i in 0..cells {
                let (x0, y0) = (-reach + i as f32 * step, -reach + j as f32 * step);
                let (x1, y1) = (x0 + step, y0 + step);
                let light = caustic(x0 + step * 0.5, y0 + step * 0.5);
                let edge = ((x0.abs().max(y0.abs()) / reach - 0.75) / 0.25).clamp(0.0, 1.0);
                let sand = [0, 1, 2].map(|k| {
                    let base = [38.0, 54.0, 56.0][k] + [70.0, 78.0, 56.0][k] * light;
                    (base * (1.0 - 0.55 * edge)) as u8
                });
                stage.frame.quad(
                    camera,
                    [
                        [x0, y0, height(x0, y0)],
                        [x1, y0, height(x1, y0)],
                        [x1, y1, height(x1, y1)],
                        [x0, y1, height(x0, y1)],
                    ],
                    sand,
                    NONE,
                );
            }
        }
        for strand in 0..28_u32 {
            let h = hash(strand + 1);
            let angle = (h & 0xffff) as f32 / 65536.0 * TAU;
            let distance = reach * (0.82 + 0.12 * ((h >> 16) & 0xff) as f32 / 255.0);
            let root = [distance * angle.cos(), distance * angle.sin(), 0.0];
            let tall = 6.0 + 5.0 * ((h >> 24) & 0xff) as f32 / 255.0;
            let mut last = root;
            for segment in 1..=12 {
                let s = segment as f32 / 12.0;
                let sway = (time * 0.8 + segment as f32 * 0.4 + angle).sin() * s * s * 1.6;
                let next = [root[0] + sway, root[1] + sway * 0.6, tall * s];
                let shade = [46.0 + 50.0 * s, 104.0 + 50.0 * s, 64.0 + 20.0 * s].map(|v| v as u8);
                stage.frame.line(camera, last, next, shade);
                last = next;
            }
        }
        for shaft in 0..6 {
            let x = -reach * 0.6 + shaft as f32 * reach * 0.25;
            let shimmer = 0.04 + 0.03 * (time * 0.5 + shaft as f32).sin();
            stage.frame.beam(
                camera,
                [x, -reach * 0.5, DEPTH],
                [x + 4.0, -reach * 0.5 + 3.0, 0.0],
                [160, 215, 235],
                shimmer,
            );
        }
    }

    /// A school of fish around its home, steered by cohesion, separation and alignment.
    #[allow(clippy::too_many_arguments)]
    fn school(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        members: &[usize],
        home: [f32; 2],
        room: f32,
        color: Color,
        dt: f32,
    ) -> usize {
        let time = stage.time;
        let phase = spin(&habitat(processes[members[0]]));
        let target = [
            home[0] + room * 0.4 * (time * 0.05 + phase).cos(),
            home[1] + room * 0.4 * (time * 0.05 + phase).sin(),
            5.0 + 2.5 * (time * 0.07 + phase).sin(),
        ];
        for &index in members {
            let id = processes[index].id;
            let h = hash(id.pid);
            self.fish.entry(id).or_insert_with(|| Swimmer {
                position: [
                    target[0] + ((h & 0xff) as f32 / 255.0 - 0.5) * 4.0,
                    target[1] + ((h >> 8 & 0xff) as f32 / 255.0 - 0.5) * 4.0,
                    target[2] + ((h >> 16 & 0xff) as f32 / 255.0 - 0.5) * 2.0,
                ],
                velocity: [(phase).cos(), (phase).sin(), 0.0],
            });
        }
        let school: Vec<(Identity, Swimmer)> = members
            .iter()
            .map(|&i| (processes[i].id, self.fish[&processes[i].id]))
            .collect();
        let mean = school.iter().fold([0.0; 3], |a, (_, s)| {
            [0, 1, 2].map(|k| a[k] + s.velocity[k] / school.len() as f32)
        });
        for &index in members {
            let process = processes[index];
            let zombie = process.state == 'Z';
            let mut fish = self.fish[&process.id];
            let speed = if zombie {
                0.3
            } else {
                0.8 + 5.0 * bounded(process.cpu, 40.0)
            };
            let goal = if zombie {
                [fish.position[0], fish.position[1], DEPTH - 1.0]
            } else {
                target
            };
            let mut push = [0, 1, 2].map(|k| (goal[k] - fish.position[k]) * 0.4);
            if !zombie {
                for (other, mate) in &school {
                    if *other == process.id {
                        continue;
                    }
                    let away = [0, 1, 2].map(|k| fish.position[k] - mate.position[k]);
                    let d2 = away.iter().map(|v| v * v).sum::<f32>();
                    if d2 < 6.0 && d2 > 1e-4 {
                        for k in 0..3 {
                            push[k] += away[k] / d2 * 1.5;
                        }
                    }
                }
                for k in 0..3 {
                    push[k] += (mean[k] - fish.velocity[k]) * 0.5;
                }
            }
            for (velocity, push) in fish.velocity.iter_mut().zip(push) {
                *velocity += push * dt;
            }
            let length = fish
                .velocity
                .iter()
                .map(|v| v * v)
                .sum::<f32>()
                .sqrt()
                .max(1e-3);
            let clamped = length.clamp(speed * 0.3, speed);
            fish.velocity = fish.velocity.map(|v| v / length * clamped);
            for k in 0..3 {
                fish.position[k] += fish.velocity[k] * dt;
            }
            fish.position[2] = fish.position[2].clamp(1.0, DEPTH - 0.5);
            self.fish.insert(process.id, fish);
            let forward = if zombie {
                [1.0, 0.0, 0.0]
            } else {
                normalize(fish.velocity)
            };
            let side = normalize([-forward[1], forward[0], 0.0]);
            let up = if zombie {
                [0.0, 0.0, -1.0]
            } else {
                [0.0, 0.0, 1.0]
            };
            let grow = stage.growth(process.id);
            let length = (1.0 + 2.2 * bounded(process.memory as f32 / 1048576.0, 300.0))
                * (0.3 + 0.7 * grow);
            let girth = length * 0.26;
            let body = if zombie { ZOMBIE } else { color };
            let p = fish.position;
            let along = |s: f32| [0, 1, 2].map(|k| p[k] + forward[k] * length * s);
            let pick = index as u32;
            let selected = stage.selected == Some(process.id);
            stage
                .frame
                .sphere(stage.camera, along(0.18), girth, body, pick, selected);
            stage.frame.sphere(
                stage.camera,
                along(-0.12),
                girth * 0.8,
                tint(body, 0.85),
                pick,
                false,
            );
            let wag = (time * (4.0 + 3.0 * clamped)).sin() * girth * 0.6;
            let root = along(-0.35);
            let tip = |s: f32| {
                [0, 1, 2].map(|k| {
                    p[k] - forward[k] * length * 0.75 + up[k] * girth * 1.3 * s + side[k] * wag
                })
            };
            stage.frame.facet(
                stage.camera,
                [root, tip(1.0), tip(-1.0)],
                tint(body, 0.7),
                pick,
            );
            let eye =
                [0, 1, 2].map(|k| along(0.3)[k] + up[k] * girth * 0.35 + side[k] * girth * 0.6);
            stage
                .frame
                .sphere(stage.camera, eye, girth * 0.18, [20, 24, 30], pick, false);
            if process.cpu > 5.0 && !zombie {
                stage.frame.glow(
                    stage.camera,
                    p,
                    length * stage.camera.zoom * 1.4,
                    body,
                    0.2 + 0.4 * bounded(process.cpu, 50.0),
                );
            }
            stage.positions.insert(process.id, p);
            if zombie {
                stage.notes.insert(
                    process.id,
                    vec!["zombie: exited, waiting for its parent to collect it".into()],
                );
            }
        }
        members.len()
    }

    /// A crab scuttling sideways near its container's home, legs stepping as it goes.
    fn crab(
        &self,
        stage: &mut Stage,
        processes: &[&Process],
        index: usize,
        home: [f32; 2],
        group: &str,
    ) {
        let process = processes[index];
        let time = stage.time;
        let seat = self.seats.seat(process.id).unwrap_or(0) as f32;
        let angle = seat * GOLDEN_ANGLE + spin(group);
        let distance = 1.3 * (seat + 0.5).sqrt();
        let pace = 0.4 + 2.0 * bounded(process.cpu, 40.0);
        let stride = (time * pace + seat).sin() * 0.9;
        let heading = angle + std::f32::consts::FRAC_PI_2;
        let size = (0.35 + 0.5 * bounded(process.memory as f32 / 1048576.0, 200.0))
            * (0.3 + 0.7 * stage.growth(process.id));
        let center = [
            home[0] + distance * angle.cos() + stride * heading.cos(),
            home[1] + distance * angle.sin() + stride * heading.sin(),
            size * 0.7,
        ];
        let zombie = process.state == 'Z';
        let shell = if zombie { ZOMBIE } else { CRAB };
        let pick = index as u32;
        let (s, c) = heading.sin_cos();
        let local = |forward: f32, across: f32, z: f32| -> Point {
            [
                center[0] + c * across - s * forward,
                center[1] + s * across + c * forward,
                z,
            ]
        };
        for side in [-1.0_f32, 1.0] {
            for leg in 0..3 {
                let step = (time * pace * 6.0 + leg as f32 * 2.1 + side).sin() * 0.15;
                let knee = local(
                    (leg as f32 - 1.0) * size * 0.6 + step,
                    side * size * 1.3,
                    size * 0.9,
                );
                let foot = local(
                    (leg as f32 - 1.0) * size * 0.8 + step,
                    side * size * 1.9,
                    0.02,
                );
                stage.frame.line(
                    stage.camera,
                    local(0.0, side * size * 0.6, size * 0.6),
                    knee,
                    tint(shell, 0.7),
                );
                stage.frame.line(stage.camera, knee, foot, tint(shell, 0.7));
            }
            let claw = local(size * 1.2, side * size * 0.7, size * 0.8);
            stage.frame.sphere(
                stage.camera,
                claw,
                size * 0.32,
                tint(shell, 1.1),
                pick,
                false,
            );
        }
        stage.frame.sphere(
            stage.camera,
            center,
            size,
            shell,
            pick,
            stage.selected == Some(process.id),
        );
        stage.positions.insert(process.id, center);
    }

    /// One polyp of a coral colony: a stalk rising from the colony's base, taller near the middle,
    /// topped by a polyp sized by memory that glows with CPU.
    #[allow(clippy::too_many_arguments)]
    fn polyp(
        &self,
        stage: &mut Stage,
        processes: &[&Process],
        index: usize,
        home: [f32; 2],
        room: f32,
        group: &str,
        color: Color,
    ) {
        let process = processes[index];
        let time = stage.time;
        let seat = self.seats.seat(process.id).unwrap_or(0) as f32;
        let distance = 1.6 * (seat + 0.5).sqrt();
        let angle = seat * GOLDEN_ANGLE + spin(group);
        let tall = 1.5 + 5.0 * (1.0 - distance / room.max(1.0)).max(0.0);
        let sway = (time * 0.6 + seat).sin() * 0.15;
        let top = [
            home[0] + distance * angle.cos() + sway,
            home[1] + distance * angle.sin(),
            tall,
        ];
        let middle = [
            home[0] + distance * 0.5 * angle.cos(),
            home[1] + distance * 0.5 * angle.sin(),
            tall * 0.45,
        ];
        let zombie = process.state == 'Z';
        let color = if zombie { ZOMBIE } else { color };
        let stalk = tint(color, 0.45);
        stage
            .frame
            .line(stage.camera, [home[0], home[1], 0.0], middle, stalk);
        stage.frame.line(stage.camera, middle, top, stalk);
        let breathe = if process.cpu > 1.0 {
            1.0 + 0.08 * (time * 3.0 + seat).sin()
        } else {
            1.0
        };
        let size = (0.18 + 0.32 * crate::render::mass_radius(process.memory as f32)).min(0.75)
            * breathe
            * (0.3 + 0.7 * stage.growth(process.id));
        if process.cpu > 1.0 {
            stage.frame.glow(
                stage.camera,
                top,
                size * stage.camera.zoom * (2.2 + 2.0 * bounded(process.cpu, 50.0)),
                tint(color, 1.3),
                0.3 + 0.5 * bounded(process.cpu, 40.0),
            );
        }
        stage.frame.sphere(
            stage.camera,
            top,
            size,
            color,
            index as u32,
            stage.selected == Some(process.id),
        );
        stage.positions.insert(process.id, top);
    }
}

/// The colony, school or gang a process belongs to: its cgroup, which leaf names alone can't
/// tell apart (a system and a user `dbus.service`), and its kind, which decides the creature.
fn habitat(process: &Process) -> String {
    let path = if process.cgroup.is_empty() {
        &process.group
    } else {
        &process.cgroup
    };
    format!("{:?} {path}", process.kind)
}

fn hash(value: u32) -> u32 {
    value.wrapping_mul(2_654_435_761) ^ value.wrapping_mul(2_246_822_519).rotate_left(13)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_named_units_of_different_cgroups_live_apart() {
        let unit = |kind: Kind, cgroup: &str| Process {
            id: Identity { pid: 1, start: 1 },
            parent: 0,
            name: "dbus-daemon".into(),
            command: String::new(),
            group: "dbus.service".into(),
            kind,
            state: 'S',
            cpu: 0.0,
            memory: 0,
            io_rate: None,
            read_rate: None,
            write_rate: None,
            written: None,
            priority: 20,
            nice: 0,
            threads: 1,
            gpu_memory: 0,
            core: 0,
            cpu_time: 0.0,
            cgroup: cgroup.into(),
        };
        let system = unit(Kind::System, "/system.slice/dbus.service");
        let session = unit(
            Kind::Session,
            "/user.slice/user-1000.slice/user@1000.service/session.slice/dbus.service",
        );
        assert_ne!(habitat(&system), habitat(&session));
        assert_eq!(habitat(&system), habitat(&system.clone()));
    }
}
