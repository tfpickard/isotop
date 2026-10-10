//! Core track: every CPU is a lane of a circular track, performance cores inside and efficiency
//! cores outside. Lanes glow with how busy their CPU is and chevrons run at its clock speed;
//! waiting tasks queue at the start line. Running processes are marbles that travel one lap per
//! 10 s of CPU time and hop across lanes when the scheduler moves them.
//!
//! Where the platform reports no last CPU but does split each process's CPU time between the
//! performance and efficiency clusters (macOS), a marble rides between the two groups of lanes
//! by that share instead, trailed by beads for its threads waiting to run.

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
/// Time constant of a marble's drift towards the radius of its performance share.
const DRIFT_SECONDS: f32 = 0.8;
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
    /// Distance from the centre when placed by cluster rather than by lane.
    radius: f32,
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
        // The middle of the performance lanes and of the efficiency lanes, when marbles ride
        // between them by their share of performance-core time.
        let bands = by_cluster(snapshot).then(|| {
            [CoreKind::Performance, CoreKind::Efficiency].map(|kind| {
                let radii: Vec<f32> = cpus
                    .iter()
                    .filter(|cpu| cpu.kind == kind)
                    .map(|cpu| lane_radius(cpu.id))
                    .collect();
                radii.iter().sum::<f32>() / radii.len().max(1) as f32
            })
        });
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
        // Without a last CPU per process a marble cannot be placed on a lane, and the running and
        // idle counts would all fall on one of them, so neither is drawn.
        let placed = !snapshot.missing.contains(&"last cpu");
        let mut racing: HashMap<u32, usize> = HashMap::new();
        let mut parked: HashMap<u32, usize> = HashMap::new();
        for process in processes.iter().filter(|_| placed) {
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
            let occupants = if placed {
                format!(
                    " | {} running, {} idle",
                    racing.get(&cpu.id).copied().unwrap_or(0),
                    parked.get(&cpu.id).copied().unwrap_or(0)
                )
            } else {
                String::new()
            };
            stage.places.push((
                ring(-0.05, r, 0.0),
                format!(
                    "cpu{}{kind}{clock} {:.0}%{occupants}",
                    cpu.id,
                    cpu.busy * 100.0
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
        let racing = placed || bands.is_some();
        self.marbles
            .retain(|id, _| racing && alive.get(id).is_some_and(|&i| active(processes[i])));
        let mut shown = 0;
        for (index, process) in processes.iter().enumerate() {
            if !racing || !active(process) {
                continue;
            }
            if let Some([performance, efficiency]) = bands {
                let target = process
                    .performance_share
                    .map(|share| efficiency + (performance - efficiency) * share);
                let marble = self.marbles.entry(process.id).or_insert_with(|| Marble {
                    angle: (TAU * process.cpu_time / LAP).rem_euclid(TAU),
                    core: process.core,
                    hop: None,
                    hops: 0,
                    radius: target.unwrap_or((performance + efficiency) * 0.5),
                });
                marble.angle =
                    (marble.angle + TAU / LAP * process.cpu / 100.0 * dt).rem_euclid(TAU);
                // A process with no CPU time in the last interval has no share, which says
                // nothing about where it runs, so its marble stays where it was (between the
                // bands if it was never measured).
                if let Some(target) = target {
                    marble.radius += (target - marble.radius) * (1.0 - (-dt / DRIFT_SECONDS).exp());
                }
                let r = marble.radius;
                let size = (0.2 + 0.3 * mass_radius(process.memory as f32)).min(LANE * 0.4);
                let position = ring(marble.angle, r, size);
                let color = kind_color(process);
                let trail = (0.05 + 0.5 * bounded(process.cpu, 60.0)) / r * 6.0;
                let mut last = position;
                for k in 1..=8 {
                    let next = ring(marble.angle - trail * k as f32 / 8.0, r, size);
                    frame.beam(camera, last, next, color, 0.5 * (1.0 - k as f32 / 9.0));
                    last = next;
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
                let waiting = process.waiting.unwrap_or(0.0).min(6.0);
                if waiting > 0.05 {
                    frame.glow(
                        camera,
                        ring(marble.angle - (size + 0.6) / r, r, size),
                        (0.5 + 0.4 * waiting) * camera.zoom,
                        QUEUE,
                        0.12 + 0.3 * bounded(waiting, 1.0),
                    );
                }
                for j in 0..waiting.round() as usize {
                    let angle = marble.angle - (size + 0.35 + j as f32 * 0.4) / r;
                    frame.sphere(camera, ring(angle, r, 0.12), 0.12, QUEUE, NONE, false);
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
                let mut note = match process.performance_share {
                    Some(share) => format!(
                        "ran {:.0}% of its CPU time on performance cores",
                        share * 100.0
                    ),
                    None => "used no CPU time in the last sample".to_owned(),
                };
                if let Some(waiting) = process.waiting {
                    note.push_str(&format!(" | {waiting:.1} threads waiting on average"));
                }
                stage.notes.insert(process.id, vec![note]);
                shown += 1;
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
                radius: lane_radius(core),
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

/// The status legend, saying what the snapshot's platform leaves out or measures per cluster.
pub fn legend(snapshot: &Snapshot) -> String {
    let lacks = |name| snapshot.missing.contains(&name);
    let clustered = by_cluster(snapshot);
    let mut line = " Lane = CPU (gold performance, teal efficiency) | brightness = busy".to_owned();
    line.push_str(if snapshot.per_cluster.contains(&"cpu clock") {
        " | chevrons = cluster clock (cycles per CPU second)"
    } else if lacks("cpu clock") {
        " | no clock readings"
    } else {
        " | chevrons = clock"
    });
    line.push_str(if !lacks("run queue") {
        " | red queue = waiting tasks"
    } else if clustered && snapshot.processes.iter().any(|p| p.waiting.is_some()) {
        " | red beads = threads waiting to run"
    } else {
        " | no run queue readings"
    });
    line.push_str(if !lacks("last cpu") {
        " | marble = running process, one lap per 10 s of CPU, hops = migrations"
    } else if clustered {
        " | marble = running process, one lap per 10 s of CPU; nearer the centre = more of it on \
         performance cores (macOS reports the cluster, not the core)"
    } else {
        " | lanes = load; macOS reports no last CPU per process, so no marbles"
    });
    line
}

/// Whether marbles ride by cluster: the platform reports no last CPU per process but does
/// split CPU time between performance and efficiency cores, and the track has lanes of both.
fn by_cluster(snapshot: &Snapshot) -> bool {
    snapshot.missing.contains(&"last cpu")
        && snapshot
            .processes
            .iter()
            .any(|process| process.performance_share.is_some())
        && [CoreKind::Performance, CoreKind::Efficiency]
            .iter()
            .all(|kind| snapshot.cpus.iter().any(|cpu| cpu.kind == *kind))
}

/// Processes on the track; the rest only count towards their lane's idle total.
fn active(process: &Process) -> bool {
    process.cpu >= 1.0 || process.state == 'R'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;
    use crate::render::{Camera, Scene, View};

    fn render(scene: &mut Scene, snapshot: &Snapshot, time: f32) {
        scene.render(
            snapshot,
            View::Cores,
            &Camera::default(),
            320,
            180,
            None,
            time,
            512,
            None,
        );
    }

    /// How many processes the Cores view draws as marbles over the demo workload.
    fn marbles(snapshot: &Snapshot) -> usize {
        let mut scene = Scene::new();
        render(&mut scene, snapshot, 12.0);
        scene.positions.len()
    }

    fn demo_missing(missing: &[&'static str]) -> Snapshot {
        let mut snapshot = demo(12.0, 64);
        snapshot.missing = missing.to_vec();
        snapshot
    }

    /// The demo as macOS reports it: no last CPU, but each process's share of performance-core
    /// time, and the clock per cluster.
    fn by_cluster() -> Snapshot {
        let mut snapshot = demo_missing(&["last cpu", "run queue"]);
        snapshot.per_cluster = vec!["cpu clock"];
        for cpu in &mut snapshot.cpus {
            cpu.wait = 0.0;
        }
        snapshot
    }

    /// Distance of a marble from the centre of the track.
    fn radius(scene: &Scene, id: Identity) -> f32 {
        let [x, y, _] = scene.positions[&id];
        x.hypot(y)
    }

    /// The first three processes the track races, given shares of 1, 0 and 0.5.
    fn three_shares(snapshot: &mut Snapshot) -> [Identity; 3] {
        let mut racing = snapshot.processes.iter_mut().filter(|p| active(p));
        [1.0, 0.0, 0.5].map(|share| {
            let process = racing.next().expect("the demo races three processes");
            process.performance_share = Some(share);
            process.id
        })
    }

    #[test]
    fn marbles_are_not_drawn_without_a_last_cpu_or_a_share_per_cluster() {
        assert!(
            marbles(&demo_missing(&[])) > 0,
            "the demo has running processes"
        );
        let mut snapshot = demo_missing(&["last cpu"]);
        for process in &mut snapshot.processes {
            process.performance_share = None;
        }
        assert_eq!(marbles(&snapshot), 0);
        assert!(
            marbles(&demo_missing(&["cpu clock", "run queue"])) > 0,
            "other gaps keep the marbles"
        );
    }

    #[test]
    fn cluster_marbles_ride_the_band_of_the_cores_they_ran_on() {
        let mut snapshot = by_cluster();
        let [performance, efficiency, mixed] = three_shares(&mut snapshot);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, 12.0);
        let racing = snapshot.processes.iter().filter(|p| active(p)).count();
        assert_eq!(
            scene.positions.len(),
            racing,
            "every running process is a marble"
        );
        // The demo has eight performance lanes inside eight efficiency lanes.
        let inner_edge = INNER - LANE * 0.5;
        let performance_edge = INNER + 7.0 * LANE + LANE * 0.5;
        let efficiency_edge = performance_edge + GROUP_GAP;
        let outer_edge = efficiency_edge + 8.0 * LANE;
        let (p, e, m) = (
            radius(&scene, performance),
            radius(&scene, efficiency),
            radius(&scene, mixed),
        );
        assert!(inner_edge < p && p < performance_edge, "fully on P at {p}");
        assert!(efficiency_edge < e && e < outer_edge, "fully on E at {e}");
        assert!(
            performance_edge < m && m < efficiency_edge,
            "half on P at {m}"
        );
        assert!(
            (p - (INNER + 3.5 * LANE)).abs() < 1e-3,
            "the middle of the P lanes"
        );
        let note = &scene.notes[&performance][0];
        assert!(
            note.starts_with("ran 100% of its CPU time on performance cores | ")
                && note.ends_with(" threads waiting on average"),
            "{note}"
        );
    }

    #[test]
    fn cluster_marbles_drift_to_a_new_share_and_stay_put_without_one() {
        let mut snapshot = by_cluster();
        let [moving, idle, _] = three_shares(&mut snapshot);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, 12.0);
        let (start, kept) = (radius(&scene, moving), radius(&scene, idle));
        for process in &mut snapshot.processes {
            if process.id == moving {
                process.performance_share = Some(0.0);
            } else if process.id == idle {
                process.performance_share = None;
            }
        }
        render(&mut scene, &snapshot, 12.3);
        let after = radius(&scene, moving);
        render(&mut scene, &snapshot, 12.6);
        let later = radius(&scene, moving);
        assert!(start < after && after < later, "{start} {after} {later}");
        let target = INNER + 8.0 * LANE + GROUP_GAP + 3.5 * LANE;
        assert!(later < target - 0.5, "eases rather than hops: {later}");
        assert!((radius(&scene, idle) - kept).abs() < 1e-3);
        assert_eq!(
            scene.notes[&idle][0].split(" | ").next(),
            Some("used no CPU time in the last sample")
        );
    }

    #[test]
    fn no_marbles_ride_by_cluster_when_the_core_kinds_are_unknown() {
        let mut snapshot = by_cluster();
        for cpu in &mut snapshot.cpus {
            cpu.kind = CoreKind::Unknown;
        }
        assert_eq!(marbles(&snapshot), 0);
        let mut snapshot = by_cluster();
        snapshot
            .cpus
            .retain(|cpu| cpu.kind == CoreKind::Performance);
        assert_eq!(marbles(&snapshot), 0, "one kind alone gives no bands");
    }

    #[test]
    fn legend_names_each_missing_source() {
        let mut snapshot = demo(1.0, 8);
        let full = legend(&snapshot);
        assert_eq!(
            full,
            " Lane = CPU (gold performance, teal efficiency) | brightness = busy | chevrons = \
             clock | red queue = waiting tasks | marble = running process, one lap per 10 s of \
             CPU, hops = migrations"
        );
        snapshot.missing = vec!["last cpu"];
        for process in &mut snapshot.processes {
            process.performance_share = None;
        }
        assert!(legend(&snapshot).contains("lanes = load; macOS reports no last CPU per process"));
        snapshot.missing = vec!["cpu clock"];
        let clockless = legend(&snapshot);
        assert!(clockless.contains("no clock readings") && !clockless.contains("chevrons"));
    }

    #[test]
    fn the_legend_explains_cluster_marbles_beads_and_clocks() {
        let snapshot = by_cluster();
        assert_eq!(
            legend(&snapshot),
            " Lane = CPU (gold performance, teal efficiency) | brightness = busy | chevrons = \
             cluster clock (cycles per CPU second) | red beads = threads waiting to run | marble \
             = running process, one lap per 10 s of CPU; nearer the centre = more of it on \
             performance cores (macOS reports the cluster, not the core)"
        );
        let mut unclocked = by_cluster();
        unclocked.per_cluster.clear();
        unclocked.missing.push("cpu clock");
        assert!(legend(&unclocked).contains("| no clock readings | red beads"));
        let mut unmeasured = by_cluster();
        for process in &mut unmeasured.processes {
            process.waiting = None;
        }
        assert!(legend(&unmeasured).contains("| no run queue readings | marble = running"));
    }
}
