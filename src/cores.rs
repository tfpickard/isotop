//! Core track: every CPU is a lane of a circular track, performance cores inside and efficiency
//! cores outside. Lanes glow with how busy their CPU is and chevrons run at its clock speed;
//! waiting tasks queue at the start line. Running processes are marbles that travel one lap per
//! 10 s of CPU time and hop across lanes when the scheduler moves them.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use crate::model::{CoreKind, Cpu, Identity, Process, Snapshot, bounded};
use crate::render::{Color, NONE, Point, Stage, WARM, kind_color, mass_radius, ring_bounds, tint};

/// Seconds of CPU time per lap, so a process using one full core laps every LAP seconds.
const LAP: f32 = 10.0;
const INNER: f32 = 9.0;
const LANE: f32 = 1.5;
const GROUP_GAP: f32 = 1.2;
/// Rise per unit of radius, banking the track like a velodrome so outer lanes stand higher.
const BANK: f32 = 0.16;
const HOP_SECONDS: f32 = 0.6;
const SEGMENTS: usize = 120;
const PERFORMANCE: Color = [236, 178, 92];
const EFFICIENCY: Color = [72, 183, 199];
const UNKNOWN: Color = [140, 160, 196];
const QUEUE: Color = [240, 96, 84];

#[derive(Default)]
pub struct Track {
    marbles: HashMap<Identity, Marble>,
    /// Chevron phase per CPU, advanced by its clock speed.
    chevrons: HashMap<u32, f32>,
    last: Option<f32>,
}

struct Marble {
    angle: f32,
    core: u32,
    /// Lane the marble left and when, while it hops.
    hop: Option<(f32, f32)>,
    hops: u32,
}

impl Track {
    pub fn draw(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        snapshot: &Snapshot,
    ) -> usize {
        let time = stage.time;
        let dt = self.last.map_or(0.0, |last| (time - last).clamp(0.0, 0.5));
        self.last = Some(time);
        let mut cpus: Vec<Cpu> = snapshot.cpus.clone();
        if cpus.is_empty() {
            cpus = (0..snapshot.cores.max(1) as u32)
                .map(|id| Cpu {
                    id,
                    kind: CoreKind::Unknown,
                    busy: 0.0,
                    mhz: 0.0,
                    wait: 0.0,
                })
                .collect();
        }
        cpus.sort_by_key(|cpu| (cpu.kind, cpu.id));
        let mut radius = INNER;
        let mut lanes: HashMap<u32, (usize, f32)> = HashMap::new();
        for (index, cpu) in cpus.iter().enumerate() {
            if index > 0 && cpus[index - 1].kind != cpu.kind {
                radius += GROUP_GAP;
            }
            lanes.insert(cpu.id, (index, radius));
            radius += LANE;
        }
        let outer = radius - LANE * 0.5;
        let lane_radius = |core: u32| lanes.get(&core).map_or(INNER, |&(_, r)| r);
        let frame = &mut *stage.frame;
        let camera = stage.camera;
        let ring = |angle: f32, r: f32, z: f32| -> Point {
            [
                r * angle.cos(),
                r * angle.sin(),
                z + BANK * (r - INNER).max(0.0),
            ]
        };
        let infield = INNER - LANE * 0.5 - 0.4;
        for k in 0..SEGMENTS {
            let (a, b) = (
                k as f32 / SEGMENTS as f32 * TAU,
                (k + 1) as f32 / SEGMENTS as f32 * TAU,
            );
            frame.facet(
                camera,
                [
                    [0.0, 0.0, -0.02],
                    ring(a, infield, -0.02),
                    ring(b, infield, -0.02),
                ],
                [12, 18, 28],
                NONE,
            );
        }
        let mut racing: HashMap<u32, usize> = HashMap::new();
        let mut parked: HashMap<u32, usize> = HashMap::new();
        for process in processes {
            let core = if lanes.contains_key(&process.core) {
                process.core
            } else {
                cpus[0].id
            };
            if active(process) {
                *racing.entry(core).or_default() += 1;
            } else {
                *parked.entry(core).or_default() += 1;
            }
        }
        for cpu in &cpus {
            let r = lane_radius(cpu.id);
            let base = match cpu.kind {
                CoreKind::Performance => PERFORMANCE,
                CoreKind::Efficiency => EFFICIENCY,
                CoreKind::Unknown => UNKNOWN,
            };
            let surface = tint(base, 0.10 + 0.45 * cpu.busy);
            let (inside, outside) = (r - LANE * 0.5 + 0.08, r + LANE * 0.5 - 0.08);
            for k in 0..SEGMENTS {
                let a = k as f32 / SEGMENTS as f32 * TAU;
                let b = (k + 1) as f32 / SEGMENTS as f32 * TAU;
                frame.quad(
                    camera,
                    [
                        ring(a, inside, 0.0),
                        ring(b, inside, 0.0),
                        ring(b, outside, 0.0),
                        ring(a, outside, 0.0),
                    ],
                    surface,
                    NONE,
                );
            }
            // Chevrons lap at 0.1 rad/s per GHz: a 4 GHz core turns its pattern about every 16 s.
            let phase = self.chevrons.entry(cpu.id).or_insert(cpu.id as f32 * 1.9);
            *phase = (*phase + dt * 0.1 * cpu.mhz / 1000.0).rem_euclid(TAU);
            let mark = tint(base, 0.55 + 0.8 * cpu.busy);
            for k in 0..12 {
                let tip = k as f32 / 12.0 * TAU + *phase;
                let tail = tip - 0.6 / r;
                for side in [-0.35, 0.35] {
                    frame.line(camera, ring(tail, r + side, 0.02), ring(tip, r, 0.02), mark);
                }
            }
            let waiting = cpu.wait.min(6.0);
            if waiting > 0.05 {
                frame.glow(
                    camera,
                    ring(-0.5 / r, r, 0.1),
                    (0.8 + waiting) * camera.zoom,
                    QUEUE,
                    0.25 + 0.5 * bounded(waiting, 1.0),
                );
                for j in 0..waiting.round() as usize {
                    let angle = -(0.9 + j as f32 * 0.55) / r;
                    frame.sphere(camera, ring(angle, r, 0.18), 0.18, QUEUE, NONE, false);
                }
            }
            let kind = match cpu.kind {
                CoreKind::Performance => "P",
                CoreKind::Efficiency => "E",
                CoreKind::Unknown => "",
            };
            let clock = if cpu.mhz > 0.0 {
                format!(" {:.1}GHz", cpu.mhz / 1000.0)
            } else {
                String::new()
            };
            stage.places.push((
                ring(-0.05, r, 0.0),
                format!(
                    "cpu{}{kind}{clock} {:.0}% | {} running, {} idle",
                    cpu.id,
                    cpu.busy * 100.0,
                    racing.get(&cpu.id).copied().unwrap_or(0),
                    parked.get(&cpu.id).copied().unwrap_or(0)
                ),
            ));
        }
        frame.line(
            camera,
            ring(0.0, infield, 0.03),
            ring(0.0, outer, 0.03),
            [230, 236, 240],
        );
        for k in 0..SEGMENTS {
            let (a, b) = (
                k as f32 / SEGMENTS as f32 * TAU,
                (k + 1) as f32 / SEGMENTS as f32 * TAU,
            );
            let (top_a, top_b) = (ring(a, outer, 0.0), ring(b, outer, 0.0));
            frame.quad(
                camera,
                [
                    [top_a[0], top_a[1], 0.0],
                    [top_b[0], top_b[1], 0.0],
                    top_b,
                    top_a,
                ],
                [22, 30, 42],
                NONE,
            );
            frame.line(camera, top_a, top_b, [70, 86, 104]);
        }
        let alive: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, i))
            .collect();
        self.marbles
            .retain(|id, _| alive.get(id).is_some_and(|&i| active(processes[i])));
        let mut shown = 0;
        for (index, process) in processes.iter().enumerate() {
            if !active(process) {
                continue;
            }
            let core = if lanes.contains_key(&process.core) {
                process.core
            } else {
                cpus[0].id
            };
            let marble = self.marbles.entry(process.id).or_insert_with(|| Marble {
                angle: (TAU * process.cpu_time / LAP).rem_euclid(TAU),
                core,
                hop: None,
                hops: 0,
            });
            marble.angle = (marble.angle + TAU / LAP * process.cpu / 100.0 * dt).rem_euclid(TAU);
            if marble.core != core {
                marble.hop = Some((lane_radius(marble.core), time));
                marble.core = core;
                marble.hops += 1;
            }
            let target = lane_radius(core);
            let (r, lift, progress) = match marble.hop {
                Some((from, start)) => {
                    let u = ((time - start) / HOP_SECONDS).clamp(0.0, 1.0);
                    let eased = u * u * (3.0 - 2.0 * u);
                    let height = 0.8 + 0.3 * (target - from).abs();
                    (
                        from + (target - from) * eased,
                        height * (PI * u).sin(),
                        Some((from, u)),
                    )
                }
                None => (target, 0.0, None),
            };
            if progress.is_some_and(|(_, u)| u >= 1.0) {
                marble.hop = None;
            }
            let size = (0.2 + 0.3 * mass_radius(process.memory as f32)).min(LANE * 0.4);
            let position = ring(marble.angle, r, size + lift);
            let color = kind_color(process);
            let trail = (0.05 + 0.5 * bounded(process.cpu, 60.0)) / r * 6.0;
            let mut last = position;
            for k in 1..=8 {
                let next = ring(marble.angle - trail * k as f32 / 8.0, target, size);
                frame.beam(camera, last, next, color, 0.5 * (1.0 - k as f32 / 9.0));
                last = next;
            }
            if let Some((from, u)) = progress {
                let mut last = ring(marble.angle, from, size);
                for k in 1..=12 {
                    let v = u * k as f32 / 12.0;
                    let eased = v * v * (3.0 - 2.0 * v);
                    let rr = from + (target - from) * eased;
                    let height = (0.8 + 0.3 * (target - from).abs()) * (PI * v).sin();
                    let next = ring(marble.angle, rr, size + height);
                    frame.beam(camera, last, next, [255, 236, 200], 0.6);
                    last = next;
                }
            }
            if process.cpu > 20.0 {
                frame.glow(
                    camera,
                    position,
                    size * camera.zoom * (2.0 + 2.0 * bounded(process.cpu, 100.0)),
                    WARM,
                    0.25 + 0.5 * bounded(process.cpu, 100.0),
                );
            }
            frame.sphere(
                camera,
                position,
                size,
                color,
                index as u32,
                stage.selected == Some(process.id),
            );
            stage.positions.insert(process.id, position);
            let kind = match cpus.iter().find(|c| c.id == core).map(|c| c.kind) {
                Some(CoreKind::Performance) => " (performance core)",
                Some(CoreKind::Efficiency) => " (efficiency core)",
                _ => "",
            };
            stage.notes.insert(
                process.id,
                vec![format!(
                    "on cpu{core}{kind} | {} hop{} between cores while watched",
                    marble.hops,
                    if marble.hops == 1 { "" } else { "s" }
                )],
            );
            shown += 1;
        }
        let rim = BANK * (outer - INNER);
        stage
            .bounds
            .extend(ring_bounds([0.0, 0.0], outer + 1.0, 0.0, rim + 2.0));
        shown
    }
}

/// Processes on the track; the rest only count towards their lane's idle total.
fn active(process: &Process) -> bool {
    process.cpu >= 1.0 || process.state == 'R'
}
