//! Feeding: the chickens that are running line up at the feeder of the CPU they ran on,
//! highest priority first, and the rest of the line queues behind.

use std::collections::HashMap;

use crate::model::{Identity, Process};
use crate::palette::core_name;

use super::Coop;
use super::layout::{FEEDER_ROW, Feeder, Flock, SLOT, seat_offset};
use super::roles::Pass;
use super::sim::{Chicken, Role, keep_inside};

/// Queue columns per feeder: straight back, one slot to the right and one to the left, which
/// keeps a queue inside its feeder's lane (feeders are at least 3.6 long with 1.2 between).
const QUEUE_COLUMNS: usize = 3;

/// The chickens queueing or eating at one feeder: priority, identity and index in the roll.
type Line = Vec<(i32, Identity, usize)>;

impl Coop {
    /// Where a roosting chicken's seat is: the ground point on its flock's perches and the
    /// perch height.
    fn seat_of(&self, id: Identity, flock: &Flock) -> ([f32; 2], f32) {
        let (offset, height) = seat_offset(self.seats.seat(id).unwrap_or(0));
        (
            [flock.center[0] + offset[0], flock.center[1] + offset[1]],
            height,
        )
    }

    /// Sends roosting chickens to their seats and gathers the feeding ones at the feeder of the
    /// CPU they ran on (the shared trough where the OS cannot say). Returns each feeder's line.
    pub(super) fn roost_and_gather(&self, pass: &Pass, chickens: &mut [Chicken]) -> Vec<Line> {
        let mut eating: Vec<Line> = vec![Vec::new(); self.feeders.len()];
        let feeder_of: HashMap<u32, usize> = self
            .feeders
            .iter()
            .enumerate()
            .map(|(index, feeder)| (feeder.cpu, index))
            .collect();
        for (k, process) in pass.drawn.iter().enumerate() {
            let chicken = &mut chickens[k];
            match chicken.role {
                Role::Roosting => {
                    let (seat, height) = self.seat_of(chicken.id, &self.flocks[chicken.flock]);
                    if seat != chicken.target {
                        chicken.arrived = false;
                    }
                    chicken.target = seat;
                    chicken.perch = height;
                }
                Role::Feeding => {
                    let feeder = if self.trough_shared {
                        self.feeders.len() - 1
                    } else {
                        process
                            .core
                            .and_then(|core| feeder_of.get(&core).copied())
                            .unwrap_or(0)
                    };
                    eating[feeder].push((process.priority, process.id, k));
                }
                _ => {}
            }
        }
        eating
    }

    /// Lines the feeding chickens up at their feeders, highest priority first.
    pub(super) fn queue_feeders(&self, pass: &Pass, chickens: &mut [Chicken], eating: Vec<Line>) {
        for (index, mut line) in eating.into_iter().enumerate() {
            line.sort();
            self.queue_line(pass, index, &line, chickens);
        }
    }

    /// Gives each chicken in the line of feeder `index` its slot at the trough or its place in
    /// the queue behind it.
    fn queue_line(
        &self,
        pass: &Pass,
        index: usize,
        line: &[(i32, Identity, usize)],
        chickens: &mut [Chicken],
    ) {
        let feeder = &self.feeders[index];
        let queue_start = feeder.eating_line() - 1.3;
        let floor = (feeder.center[1] - (FEEDER_ROW - 1.2)).max(self.yard.low[1]);
        // The queue runs south behind the eating line and wraps into at most QUEUE_COLUMNS
        // columns within the feeder's own lane (straight back, then right, then left), so
        // neighbouring feeders' queues never meet. A queue too long for them packs tighter.
        let needed: f32 = line
            .iter()
            .skip(feeder.slots)
            .map(|&(_, _, k)| 2.0 * chickens[k].radius + 0.25)
            .sum();
        let room = QUEUE_COLUMNS as f32 * (queue_start - floor).max(0.1);
        let packing = if needed > room { room / needed } else { 1.0 };
        let (mut cursor, mut column) = (queue_start, 0_usize);
        for (rank, &(priority, _, k)) in line.iter().enumerate() {
            let process = pass.drawn.at(k);
            let chicken = &mut chickens[k];
            let target = if rank < feeder.slots {
                feeder.slot(rank)
            } else {
                let step = (2.0 * chicken.radius + 0.25) * packing;
                if cursor - step < floor - 0.01
                    && cursor < queue_start
                    && column + 1 < QUEUE_COLUMNS
                {
                    column += 1;
                    cursor = queue_start;
                }
                let side = if column % 2 == 1 { 1.0 } else { -1.0 };
                let across = column.div_ceil(2) as f32 * SLOT * side;
                cursor -= step * 0.5;
                let spot = [feeder.center[0] + across, cursor];
                cursor -= step * 0.5;
                spot
            };
            let target = keep_inside(target, chicken.radius, self.yard.low, self.yard.high);
            if target != chicken.target {
                chicken.arrived = false;
            }
            chicken.target = target;
            chicken.perch = 0.0;
            chicken.notes[0] = feeding_note(feeder, process, rank, line.len(), priority);
        }
    }
}

/// What a feeding chicken says about its place in the `line` of chickens at `feeder`.
fn feeding_note(
    feeder: &Feeder,
    process: &Process,
    rank: usize,
    line: usize,
    priority: i32,
) -> String {
    let standing = if process.nice != 0 || priority >= 0 {
        format!("priority {priority} (nice {})", process.nice)
    } else {
        format!("priority {priority} (real-time)")
    };
    let place = if feeder.shared {
        "the shared trough (no last CPU on this OS)".to_owned()
    } else {
        format!("cpu{} ({})", feeder.cpu, core_name(feeder.kind))
    };
    let mut note = format!(
        "feeding at {place}, pecking rank {} of {line} by {standing}",
        rank + 1
    );
    if rank >= feeder.slots {
        note.push_str(", waiting in line");
    }
    if let Some(core) = process
        .core
        .filter(|&core| !feeder.shared && feeder.cpu != core)
    {
        note.push_str(&format!(" (ran on cpu{core}, which has no feeder)"));
    }
    if process.state != 'R' {
        note.push_str(" (held: not running for one sample)");
    }
    note
}

#[cfg(test)]
mod tests {
    use crate::coop::fixtures::{coop_of, identity, legend_of, process, render, snapshot};
    use crate::coop::layout::{SLOT, TROUGH_DEPTH};
    use crate::coop::sim::{Role, keep_inside};
    use crate::model::{CoreKind, Cpu, Process, Snapshot};
    use crate::platform::Source;
    use crate::render::Scene;
    use crate::views::ViewModule;

    #[test]
    fn higher_priority_chicken_takes_the_feeder() {
        let mut processes: Vec<Process> = (1..=3)
            .map(|pid| {
                let mut p = process(pid, "/system.slice/a.service");
                p.state = 'R';
                p.cpu = 90.0;
                p
            })
            .collect();
        let mut sample = snapshot(processes.clone(), 1.0);
        sample.cpus = vec![Cpu {
            id: 0,
            kind: CoreKind::Efficiency,
            busy: Some(1.0),
            mhz: None,
            wait: None,
        }];
        let mut scene = Scene::new();
        render(&mut scene, &sample, 1.0);
        let notes = |scene: &Scene, pid: u32| scene.notes[&identity(pid)][1].clone();
        assert!(notes(&scene, 1).contains("rank 1 of 3"));
        assert!(notes(&scene, 3).contains("rank 3 of 3") && notes(&scene, 3).contains("waiting"));
        processes[2].priority = -51;
        sample.processes = processes;
        sample.elapsed = 2.0;
        render(&mut scene, &sample, 2.0);
        let feeder = &coop_of(&scene).feeders[0];
        let chicken = coop_of(&scene).yard.find(identity(3)).unwrap();
        assert_eq!(chicken.target, feeder.slot(0), "real-time pushes in first");
        assert!(notes(&scene, 3).contains("rank 1 of 3 by priority -51 (real-time)"));
        let displaced = coop_of(&scene).yard.find(identity(2)).unwrap();
        assert!(displaced.target[1] < feeder.eating_line(), "now queueing");
    }

    #[test]
    fn neighbouring_feeders_keep_their_queues_in_their_own_lanes() {
        let path = "/system.slice/busy.service";
        let processes = (1..=16)
            .map(|pid| {
                let mut p = process(pid, path);
                p.state = 'R';
                p.cpu = 90.0;
                p.memory = 20 << 20;
                p.core = Some(u32::from(pid > 9));
                p
            })
            .collect();
        let mut sample = snapshot(processes, 1.0);
        sample.cores = 2;
        let mut scene = Scene::new();
        render(&mut scene, &sample, 1.0);
        let coop = coop_of(&scene);
        assert_eq!(coop.feeders.len(), 2);
        let chickens = &coop.yard.chickens;
        for chicken in chickens {
            let feeder = &coop.feeders[usize::from(chicken.id.pid > 9)];
            assert!(
                (chicken.target[0] - feeder.center[0]).abs() <= SLOT + 0.01,
                "pid {} queues outside cpu{}'s lane",
                chicken.id.pid,
                feeder.cpu
            );
        }
        for (a, b) in chickens
            .iter()
            .flat_map(|a| chickens.iter().map(move |b| (a, b)))
        {
            if a.id < b.id {
                let apart = (a.target[0] - b.target[0]).hypot(a.target[1] - b.target[1]);
                assert!(
                    apart > 0.2,
                    "pids {} and {} share a spot",
                    a.id.pid,
                    b.id.pid
                );
            }
        }
    }

    #[test]
    fn running_chickens_share_one_trough_when_the_os_reports_no_last_cpu() {
        let path = "/system.slice/busy.service";
        let processes = (1..=16)
            .map(|pid| {
                let mut p = process(pid, path);
                p.state = 'R';
                p.cpu = 90.0;
                p.memory = 20 << 20;
                p.core = Some(u32::from(pid > 9));
                p
            })
            .collect();
        let mut sample = snapshot(processes, 1.0);
        sample.cpus = [1.0, 0.0]
            .into_iter()
            .enumerate()
            .map(|(id, busy)| Cpu {
                id: id as u32,
                kind: CoreKind::Unknown,
                busy: Some(busy),
                mhz: None,
                wait: None,
            })
            .collect();
        sample.missing = [Source::CpuPressure, Source::LastCpu].into();
        let mut scene = Scene::new();
        render(&mut scene, &sample, 1.0);
        let coop = coop_of(&scene);
        assert_eq!(
            coop.feeders.len(),
            3,
            "two per-CPU feeders and the shared one"
        );
        let shared = &coop.feeders[2];
        assert!(shared.shared && !coop.feeders[0].shared && !coop.feeders[1].shared);
        assert_eq!(
            coop.feeders[0].busy, 1.0,
            "per-CPU feeders keep their busy level"
        );
        assert_eq!(shared.busy, 0.5, "the shared trough shows the mean");
        let lowest = coop.feeders[..2]
            .iter()
            .map(|feeder| feeder.center[1])
            .fold(f32::MAX, f32::min);
        assert!(shared.center[1] < lowest, "its own row below the others");
        assert!(
            coop.yard.low[1] < shared.center[1] - TROUGH_DEPTH,
            "inside the fence"
        );
        for chicken in &coop.yard.chickens {
            assert!(
                (chicken.target[0] - shared.center[0]).abs() <= SLOT + 0.01
                    && chicken.target[1] <= shared.eating_line() + 0.01,
                "pid {} is not at the shared trough: {:?}",
                chicken.id.pid,
                chicken.target
            );
            assert!(
                scene.notes[&chicken.id][1].contains("the shared trough"),
                "{}",
                scene.notes[&chicken.id][1]
            );
            assert!(!scene.notes[&chicken.id][1].contains("ran on cpu"));
        }
        let at_slots = coop
            .yard
            .chickens
            .iter()
            .filter(|chicken| chicken.target[1] == shared.eating_line())
            .count();
        assert_eq!(at_slots, shared.slots, "the rest wait in line");
        let legend = coop.legend(&Snapshot::default());
        assert!(legend.contains("no last CPU: running chickens share one trough"));
        assert!(legend.contains("no CPU pressure"));
    }

    #[test]
    fn coop_keeps_per_cpu_feeders_when_only_cpu_pressure_is_missing() {
        let mut sample = snapshot(vec![process(1, "/system.slice/a.service")], 1.0);
        sample.cores = 2;
        sample.missing = Source::CpuPressure.into();
        let mut scene = Scene::new();
        render(&mut scene, &sample, 1.0);
        assert_eq!(coop_of(&scene).feeders.len(), 2);
        assert!(coop_of(&scene).feeders.iter().all(|feeder| !feeder.shared));
        let legend = legend_of(&scene);
        assert!(!legend.contains("share one trough"));
        assert!(legend.contains("no CPU pressure"));
    }

    #[test]
    fn queued_feeders_wrap_into_columns_and_every_target_stays_inside_the_fence() {
        let path = "/system.slice/busy.service";
        let inside = |scene: &Scene| {
            let yard = &coop_of(scene).yard;
            for c in &yard.chickens {
                for point in [c.position, c.target] {
                    assert_eq!(
                        keep_inside(point, c.radius, yard.low, yard.high),
                        point,
                        "pid {} at {point:?} is outside {:?} to {:?}",
                        c.id.pid,
                        yard.low,
                        yard.high
                    );
                }
            }
        };
        let running = |state: char, elapsed: f64| {
            let processes = (1..=12)
                .map(|pid| {
                    let mut p = process(pid, path);
                    p.state = state;
                    p.cpu = 90.0;
                    p
                })
                .collect();
            let mut sample = snapshot(processes, elapsed);
            sample.cores = 1;
            sample
        };
        // The first population is placed straight at its targets, which must not be in the hedge.
        let mut scene = Scene::new();
        render(&mut scene, &running('R', 1.0), 1.0);
        assert_eq!(coop_of(&scene).feeders.len(), 1);
        inside(&scene);
        // A population that starts walking must be able to arrive, one chicken to a spot.
        let mut scene = Scene::new();
        render(&mut scene, &running('S', 1.0), 1.0);
        let sample = running('R', 2.0);
        for frame in 0..400 {
            render(&mut scene, &sample, 2.0 + frame as f32 * 0.05);
        }
        inside(&scene);
        let chickens = &coop_of(&scene).yard.chickens;
        assert!(
            chickens
                .iter()
                .all(|c| c.role == Role::Feeding && c.arrived)
        );
        for (a, b) in chickens
            .iter()
            .flat_map(|a| chickens.iter().map(move |b| (a, b)))
        {
            if a.id < b.id {
                let apart = (a.target[0] - b.target[0]).hypot(a.target[1] - b.target[1]);
                assert!(
                    apart > 0.2,
                    "pids {} and {} share a spot",
                    a.id.pid,
                    b.id.pid
                );
            }
        }
    }
}
