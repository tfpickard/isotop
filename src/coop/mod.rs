//! Coop: the machine as a fenced chicken yard. Every process is a chicken in a Vicsek flock, one
//! flock per cgroup around its henhouse; idle processes roost, running ones peck at the feeder of
//! the CPU they ran on, and lock holders brood by the nest. Eggs, rotten eggs, chicks, dust and
//! foxes stand for open files, deleted files still open, threads, reads and OOM kills.

mod draw;
#[cfg(test)]
mod fixtures;
mod layout;
mod roles;
mod sim;

use std::collections::{HashMap, HashSet, VecDeque};

use crate::layout::{Discs, Seats};
use crate::model::{Identity, Process, Snapshot};
use crate::platform::GROUP_LABEL;
use crate::render::Stage;
use crate::views::ViewModule;

use self::layout::{Feeder, Flock, HEDGE};
use self::sim::{Chicken, Yard, YardClock};

/// Samples of CPU kept per process for the coefficient of variation.
const HISTORY: usize = 30;
/// Reads faster than this raise dust, one puff per doubling, at most MAX_PUFFS.
const DUST_RATE: f32 = 65536.0;
const MAX_PUFFS: usize = 5;
const FOX_SECONDS: f32 = 3.0;
const MAX_FOXES: u64 = 3;
/// Fox calls held for a view that is not shown; older ones are dropped as stale.
const MAX_PENDING_CALLS: usize = 2 * MAX_FOXES as usize;
/// How long a member that left its cgroup stays a suspect for an OOM kill the counter reports
/// late: the units are read every 2 s on a background thread, so the count can lag the process
/// list by a couple of samples.
const DEPARTED_SECONDS: f64 = 4.0;

/// An OOM kill recorded but not yet sent into the yard.
struct Call {
    flock: String,
    /// The sample time of the kill, so a call outlived by its fox is dropped.
    at: f64,
    victim: Option<Identity>,
    /// Where the victim was last drawn and its radius, once the yard has been checked.
    seen: Option<([f32; 2], f32)>,
    delay: f32,
}

/// A member that has left its cgroup, kept briefly so a late OOM-kill count can name it.
struct Departed {
    cgroup: String,
    id: Identity,
    memory: u64,
    /// The sample it was first missing from.
    gone: f64,
    /// Where it was last drawn and its radius, once the yard has been checked.
    seen: Option<([f32; 2], f32)>,
}

struct Fox {
    start: f32,
    entry: [f32; 2],
    prey: [f32; 2],
    exit: [f32; 2],
    /// Radius of the departed member it carries off.
    ghost: Option<f32>,
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The processes a decision pass covers: those drawn, in identity order, each read from the
/// measured sample where that has an entry for it.
struct Drawn<'a> {
    processes: &'a [&'a Process],
    order: &'a [usize],
    measured: HashMap<Identity, &'a Process>,
}

impl<'a> Drawn<'a> {
    fn new(processes: &'a [&'a Process], order: &'a [usize], snapshot: &'a Snapshot) -> Self {
        Self {
            processes,
            order,
            measured: snapshot.processes.iter().map(|p| (p.id, p)).collect(),
        }
    }

    /// The process at `index` of the slice the view was handed, as measured.
    fn process(&self, index: usize) -> &'a Process {
        self.measured
            .get(&self.processes[index].id)
            .copied()
            .unwrap_or(self.processes[index])
    }

    /// The `position`th drawn process, as measured.
    fn at(&self, position: usize) -> &'a Process {
        self.process(self.order[position])
    }

    /// The drawn processes in order, as measured.
    fn iter(&self) -> impl Iterator<Item = &'a Process> + '_ {
        self.order.iter().map(|&index| self.process(index))
    }

    fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether the measured sample has `id`, drawn or not.
    fn sampled(&self, id: Identity) -> bool {
        self.measured.contains_key(&id)
    }
}

/// The persistent coop view.
#[derive(Default)]
pub struct Coop {
    recorded: Option<f64>,
    /// Seconds between the last two recorded samples.
    interval: f64,
    history: HashMap<Identity, VecDeque<f32>>,
    kills: HashMap<String, u64>,
    /// Members of each cgroup in the last recorded sample with their memory, for fox victims.
    members: HashMap<String, Vec<(Identity, u64)>>,
    departed: Vec<Departed>,
    calls: Vec<Call>,
    foxes: Vec<Fox>,
    seats: Seats,
    discs: Discs,
    flocks: Vec<Flock>,
    feeders: Vec<Feeder>,
    /// The northern edge of the feeder band, just south of the houses.
    band_top: f32,
    /// The bounds of every reserved house disc seen so far, and the deepest feeder band. The
    /// fence is derived from them and so only ever moves outward.
    extent: Option<([f32; 2], [f32; 2])>,
    band: f32,
    yard: Yard,
    /// Chickens of processes still sampled but not drawn now (hidden by focus or the process
    /// limit), kept as they were so they come back where they left.
    hidden: HashMap<Identity, Chicken>,
    /// The sample the roles were last decided for, and the processes they were decided over.
    decided: Option<f64>,
    drawn: Vec<Identity>,
    clock: YardClock,
    phi: Option<f32>,
    pressure_missing: bool,
    /// The OS has no cgroups, so flocks are its process groups (`GROUP_LABEL`) and no OOM kill
    /// brings a fox.
    cgroups_missing: bool,
    /// The OS reports no last CPU, so running chickens queue at one shared trough.
    trough_shared: bool,
    /// The OS has no file lock table, so no hen broods or queues at a nest.
    locks_missing: bool,
    unreadable: usize,
    /// Drawn processes whose descriptor tables or lock state could not be read (not merely not
    /// read yet), and locks that name no process.
    no_files: usize,
    no_locks: usize,
    unattributed_locks: u32,
}

impl Coop {
    /// Takes in a new sample: CPU history, OOM kills and cgroup membership. Older or
    /// repeated samples are ignored, so the scene's record hook and `draw` may both call it.
    fn ingest(&mut self, snapshot: &Snapshot) {
        if self
            .recorded
            .is_some_and(|elapsed| snapshot.elapsed <= elapsed)
        {
            return;
        }
        if let Some(previous) = self.recorded {
            self.interval = snapshot.elapsed - previous;
        }
        self.recorded = Some(snapshot.elapsed);
        let present: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        self.history.retain(|id, _| present.contains(id));
        for process in &snapshot.processes {
            let samples = self.history.entry(process.id).or_default();
            samples.push_back(process.cpu);
            if samples.len() > HISTORY {
                samples.pop_front();
            }
        }
        // Members that left since the last sample stay suspects for a few seconds, because the
        // kill count comes from the slower background sampler and can rise after they are gone.
        let mut left: Vec<Departed> = self
            .members
            .iter()
            .flat_map(|(cgroup, members)| {
                members
                    .iter()
                    .filter(|(id, _)| !present.contains(id))
                    .map(|&(id, memory)| Departed {
                        cgroup: cgroup.clone(),
                        id,
                        memory,
                        gone: snapshot.elapsed,
                        seen: None,
                    })
            })
            .collect();
        left.sort_by_key(|gone| gone.id);
        self.departed.extend(left);
        // Two and a half sampling intervals cover a kill counted up to two samples late when
        // --sample-ms is longer than the default.
        let window = DEPARTED_SECONDS.max(2.5 * self.interval);
        self.departed
            .retain(|gone| snapshot.elapsed - gone.gone <= window);
        let mut paths: Vec<&String> = snapshot.units.keys().collect();
        paths.sort();
        for path in paths {
            // An unreadable count says nothing: the last known one stays the one to compare to.
            let Some(kills) = snapshot.units[path].oom_kills else {
                continue;
            };
            let seen = *self.kills.entry(path.clone()).or_insert(kills);
            if kills > seen {
                // Every kill claims a suspect, but only the first MAX_FOXES send a fox. The loop
                // stops once the suspects run out, since the count can be very large.
                let suspects = self.departed.iter().filter(|g| g.cgroup == *path).count();
                let claims = (kills - seen).min(MAX_FOXES.max(suspects as u64));
                for k in 0..claims as usize {
                    // The largest departed member that no earlier kill has claimed.
                    let victim = self
                        .departed
                        .iter()
                        .enumerate()
                        .filter(|(_, gone)| gone.cgroup == *path)
                        .max_by(|(_, a), (_, b)| a.memory.cmp(&b.memory).then(b.id.cmp(&a.id)))
                        .map(|(index, _)| index)
                        .map(|index| self.departed.remove(index));
                    if k as u64 >= MAX_FOXES {
                        continue;
                    }
                    self.calls.push(Call {
                        flock: path.clone(),
                        at: snapshot.elapsed,
                        victim: victim.as_ref().map(|gone| gone.id),
                        seen: victim.and_then(|gone| gone.seen),
                        delay: k as f32 * 0.8,
                    });
                }
            }
            self.kills.insert(path.clone(), kills);
        }
        self.expire_calls(snapshot.elapsed);
        self.kills
            .retain(|path, _| snapshot.units.contains_key(path));
        self.members.clear();
        for process in snapshot.processes.iter().filter(|p| !p.cgroup.is_empty()) {
            self.members
                .entry(process.cgroup.clone())
                .or_default()
                .push((process.id, process.memory));
        }
    }

    /// Drops fox calls whose fox would already have left, and the oldest beyond the cap, so calls
    /// recorded while another view is shown do not replay on switching to this one.
    fn expire_calls(&mut self, sample: f64) {
        self.calls
            .retain(|call| sample - call.at <= FOX_SECONDS as f64);
        let excess = self.calls.len().saturating_sub(MAX_PENDING_CALLS);
        self.calls.drain(..excess);
    }
}

impl ViewModule for Coop {
    /// Takes in each sample whichever view is shown, so the history, the OOM counts and the
    /// cgroup members are there when the coop opens.
    fn record(&mut self, snapshot: &Snapshot, _time: f32) {
        self.ingest(snapshot);
    }

    /// The status legend, with the yard's order parameter and what could not be measured.
    fn legend(&self, _snapshot: &Snapshot) -> String {
        let phi = self
            .phi
            .map_or_else(|| "- (none foraging)".to_owned(), |phi| format!("{phi:.2}"));
        let feeder = if self.trough_shared {
            "feeders = CPU busy"
        } else {
            "feeder = CPU it ran on"
        };
        let (flock, fox) = if self.cgroups_missing {
            (GROUP_LABEL, "")
        } else {
            ("cgroup", "fox = OOM kill, ")
        };
        let locks = if self.locks_missing {
            ""
        } else {
            "brooding = holds a file lock, queue at a nest = blocked on one | "
        };
        let mut line = format!(
            " Chicken = process, size = memory | flock = {flock} | foraging speed = CPU, roost = idle | {feeder}, pecking order = priority | eggs = open files (log2), cracked = deleted but open | {locks}chicks = threads | dust = reads | {fox}eyes = memory pressure | phi {phi}"
        );
        if self.trough_shared {
            line.push_str(" | no last CPU: running chickens share one trough");
        }
        if self.pressure_missing {
            line.push_str(" | no CPU pressure: noise from CPU variation only");
        }
        if self.cgroups_missing {
            line.push_str(" | no cgroups: no OOM foxes");
        }
        if self.unreadable > 0 {
            line.push_str(&format!(" | I/O unreadable for {}", self.unreadable));
        }
        if self.no_files > 0 {
            line.push_str(&format!(" | no files for {} (permissions)", self.no_files));
        }
        if self.locks_missing {
            line.push_str(" | no file lock table: no brooding or queueing");
        } else if self.no_locks > 0 {
            line.push_str(&format!(" | file locks unreadable for {}", self.no_locks));
        }
        if self.unattributed_locks > 0 {
            line.push_str(&format!(
                " | {} without a process (OFD)",
                plural(self.unattributed_locks.into(), "file lock", "file locks")
            ));
        }
        line
    }

    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        self.ingest(snapshot);
        let mut order: Vec<usize> = (0..processes.len()).collect();
        order.sort_by_key(|&i| processes[i].id);
        let ids: Vec<Identity> = order.iter().map(|&i| processes[i].id).collect();
        // Victims are looked up before the new sample removes them from the yard, and the foxes
        // sent after it, so they run to the yard as it is now laid out.
        for call in &mut self.calls {
            if let Some(chicken) = call.victim.and_then(|id| self.yard.find(id)) {
                call.seen = Some((chicken.position, chicken.radius));
            }
        }
        for gone in &mut self.departed {
            if let Some(chicken) = self.yard.find(gone.id) {
                gone.seen = Some((chicken.position, chicken.radius));
            }
        }
        let fresh = self.decided != Some(snapshot.elapsed);
        if fresh || ids != self.drawn {
            self.decide(processes, &order, snapshot, fresh);
            self.decided = Some(snapshot.elapsed);
            self.drawn = ids;
        }
        self.summon(stage.time, snapshot.elapsed);
        let alpha = self.advance(stage.time);
        self.phi = self.yard.order();
        self.paint(stage, processes, &order, snapshot, alpha)
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Where each drawn chicken belongs, wherever it has walked to: its seat on its house's
    /// perches, on the ground. The view harness checks layout stability here.
    #[cfg(test)]
    fn anchors(&self) -> Option<HashMap<Identity, crate::render::Point>> {
        Some(
            self.yard
                .chickens
                .iter()
                .filter_map(|chicken| {
                    let center = self.flocks.get(chicken.flock)?.center;
                    let (offset, _) = layout::seat_offset(self.seats.seat(chicken.id)?);
                    Some((
                        chicken.id,
                        [center[0] + offset[0], center[1] + offset[1], 0.0],
                    ))
                })
                .collect(),
        )
    }
}

impl Coop {
    /// Steps the yard on wall-clock time; returns how far the frame lies into the next step.
    fn advance(&mut self, time: f32) -> f32 {
        self.clock.0.advance(time, |step| self.yard.step(step))
    }

    /// Turns recorded OOM kills into fox runs: to where the victim was last drawn, or to its
    /// flock's house when it was not drawn.
    fn summon(&mut self, time: f32, sample: f64) {
        self.expire_calls(sample);
        for call in std::mem::take(&mut self.calls) {
            let home = self
                .flocks
                .iter()
                .find(|flock| flock.key == call.flock)
                .map(|flock| flock.center);
            let middle = [
                (self.yard.low[0] + self.yard.high[0]) * 0.5,
                (self.yard.low[1] + self.yard.high[1]) * 0.5,
            ];
            let prey = call
                .seen
                .map_or(home.unwrap_or(middle), |(position, _)| position);
            let (entry, exit) = self.hedge_gap(prey);
            self.foxes.push(Fox {
                start: time + call.delay,
                entry,
                prey,
                exit,
                ghost: call.seen.map(|(_, radius)| radius),
            });
        }
        self.foxes
            .retain(|fox| (-10.0..FOX_SECONDS).contains(&(time - fox.start)));
    }

    /// The hedge point nearest `point`, and one further along the same side to leave by.
    fn hedge_gap(&self, point: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let (low, high) = (self.yard.low, self.yard.high);
        let distances = [
            point[0] - low[0],
            high[0] - point[0],
            point[1] - low[1],
            high[1] - point[1],
        ];
        let side = (0..4)
            .min_by(|&a, &b| distances[a].total_cmp(&distances[b]))
            .unwrap_or(0);
        let along = |value: f32, lo: f32, hi: f32| {
            let shifted = value + 7.0;
            if shifted > hi - 1.0 {
                value - 7.0
            } else {
                shifted.max(lo + 1.0)
            }
        };
        match side {
            0 => (
                [low[0] - HEDGE, point[1]],
                [low[0] - HEDGE, along(point[1], low[1], high[1])],
            ),
            1 => (
                [high[0] + HEDGE, point[1]],
                [high[0] + HEDGE, along(point[1], low[1], high[1])],
            ),
            2 => (
                [point[0], low[1] - HEDGE],
                [along(point[0], low[0], high[0]), low[1] - HEDGE],
            ),
            _ => (
                [point[0], high[1] + HEDGE],
                [along(point[0], low[0], high[0]), high[1] + HEDGE],
            ),
        }
    }
}

fn puffs(read: f32) -> usize {
    (1 + (read / DUST_RATE).log2().max(0.0) as usize).min(MAX_PUFFS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coop::fixtures::{coop_of, identity, process, render, snapshot};
    use crate::coop::layout::body_radius;
    use crate::model::{Unit, demo};
    use crate::render::{Camera, RenderInput, Scene};
    use crate::views::view;

    #[test]
    fn an_oom_kill_sends_a_fox_for_the_departed_member() {
        let path = "/system.slice/greedy.service";
        let unit = |kills| {
            HashMap::from([(
                path.to_string(),
                Unit {
                    oom_kills: Some(kills),
                    ..Unit::default()
                },
            )])
        };
        let mut small = process(1, path);
        small.memory = 50 << 20;
        let mut large = process(2, path);
        large.memory = 900 << 20;
        let mut bystander = process(3, path);
        bystander.memory = 100 << 20;
        let mut before = snapshot(vec![small.clone(), large.clone(), bystander.clone()], 1.0);
        before.units = unit(2);
        let mut scene = Scene::new();
        render(&mut scene, &before, 1.0);
        render(&mut scene, &before, 1.5);
        let victim = coop_of(&scene).yard.find(identity(2)).unwrap().position;
        assert!(
            coop_of(&scene).foxes.is_empty(),
            "the first sight is the baseline"
        );
        let mut after = snapshot(vec![small, bystander], 2.0);
        after.units = unit(3);
        render(&mut scene, &after, 2.0);
        assert_eq!(coop_of(&scene).foxes.len(), 1);
        let fox = &coop_of(&scene).foxes[0];
        assert_eq!(fox.prey, victim);
        assert!(
            fox.ghost
                .is_some_and(|r| (r - body_radius((900 << 20) as f32)).abs() < 1e-5)
        );
        let mut again = after.clone();
        again.elapsed = 3.0;
        again.units = unit(4);
        render(&mut scene, &again, 3.0);
        assert!(
            coop_of(&scene).foxes.iter().any(|fox| fox.ghost.is_none()),
            "a kill with nobody gone sends a fox that leaves with nothing"
        );
    }

    fn units(path: &str, kills: u64) -> HashMap<String, Unit> {
        HashMap::from([(
            path.to_string(),
            Unit {
                oom_kills: Some(kills),
                ..Unit::default()
            },
        )])
    }

    /// Three members of one cgroup rendered at samples 1 and 1.5, ready to lose the large one.
    fn greedy_flock(scene: &mut Scene, path: &str) -> ([f32; 2], Vec<Process>) {
        let mut small = process(1, path);
        small.memory = 50 << 20;
        let mut large = process(2, path);
        large.memory = 900 << 20;
        let mut bystander = process(3, path);
        bystander.memory = 100 << 20;
        let mut before = snapshot(vec![small.clone(), large, bystander.clone()], 1.0);
        before.units = units(path, 2);
        render(scene, &before, 1.0);
        render(scene, &before, 1.5);
        let victim = coop_of(scene).yard.find(identity(2)).unwrap().position;
        (victim, vec![small, bystander])
    }

    #[test]
    fn a_kill_counted_one_sample_after_the_member_vanished_still_names_it() {
        let path = "/system.slice/greedy.service";
        let mut scene = Scene::new();
        let (victim, survivors) = greedy_flock(&mut scene, path);
        let mut gone = snapshot(survivors.clone(), 2.0);
        gone.units = units(path, 2);
        render(&mut scene, &gone, 2.0);
        assert!(
            coop_of(&scene).foxes.is_empty(),
            "the counter has not moved yet"
        );
        let mut counted = snapshot(survivors, 3.0);
        counted.units = units(path, 3);
        render(&mut scene, &counted, 3.0);
        assert_eq!(coop_of(&scene).foxes.len(), 1);
        let fox = &coop_of(&scene).foxes[0];
        assert_eq!(fox.prey, victim);
        assert!(
            fox.ghost
                .is_some_and(|r| (r - body_radius((900 << 20) as f32)).abs() < 1e-5)
        );
    }

    #[test]
    fn a_kill_counted_two_samples_after_the_member_vanished_still_names_it() {
        let path = "/system.slice/greedy.service";
        let mut scene = Scene::new();
        let (victim, survivors) = greedy_flock(&mut scene, path);
        for elapsed in [2.0, 3.0] {
            let mut gone = snapshot(survivors.clone(), elapsed);
            gone.units = units(path, 2);
            render(&mut scene, &gone, elapsed as f32);
        }
        let mut counted = snapshot(survivors.clone(), 4.0);
        counted.units = units(path, 3);
        render(&mut scene, &counted, 4.0);
        assert_eq!(coop_of(&scene).foxes.len(), 1);
        assert_eq!(coop_of(&scene).foxes[0].prey, victim);
        assert!(coop_of(&scene).foxes[0].ghost.is_some());
        // The victim is claimed once: a second kill finds nobody left to take.
        let mut again = snapshot(survivors, 5.0);
        again.units = units(path, 4);
        render(&mut scene, &again, 5.0);
        assert!(coop_of(&scene).foxes.iter().any(|fox| fox.ghost.is_none()));
    }

    #[test]
    fn a_member_that_left_long_before_the_kill_is_not_blamed_for_it() {
        let path = "/system.slice/greedy.service";
        let mut scene = Scene::new();
        let (_, survivors) = greedy_flock(&mut scene, path);
        for elapsed in [2.0, 4.0, 6.0] {
            let mut gone = snapshot(survivors.clone(), elapsed);
            gone.units = units(path, 2);
            render(&mut scene, &gone, elapsed as f32);
        }
        let mut counted = snapshot(survivors, 7.0);
        counted.units = units(path, 3);
        render(&mut scene, &counted, 7.0);
        assert_eq!(coop_of(&scene).foxes.len(), 1);
        assert!(coop_of(&scene).foxes[0].ghost.is_none());
    }

    #[test]
    fn kills_recorded_while_another_view_is_shown_do_not_replay_on_switching_to_coop() {
        let path = "/system.slice/greedy.service";
        let mut scene = Scene::new();
        let processes = vec![process(1, path), process(2, path)];
        for k in 0..40 {
            let mut sample = snapshot(processes.clone(), 60.0 * k as f64);
            sample.units = units(path, k);
            scene.record(&sample, 60.0 * k as f32);
        }
        assert!(
            coop_of(&scene).calls.len() <= 1,
            "an hour of kills leaves only the newest pending, not {}",
            coop_of(&scene).calls.len()
        );
        let mut now = snapshot(processes, 60.0 * 40.0);
        now.units = units(path, 39);
        render(&mut scene, &now, 2400.0);
        assert!(coop_of(&scene).foxes.is_empty());
        assert!(coop_of(&scene).calls.is_empty());
    }

    #[test]
    fn kills_beyond_the_fox_cap_still_claim_their_victims() {
        let path = "/system.slice/greedy.service";
        let mut coop = Coop::default();
        let members: Vec<Process> = (1..=6)
            .map(|pid| {
                let mut member = process(pid, path);
                member.memory = (pid as u64 * 100) << 20;
                member
            })
            .collect();
        let mut before = snapshot(members.clone(), 1.0);
        before.units = units(path, 0);
        coop.ingest(&before);
        let mut burst = snapshot(members[..1].to_vec(), 2.0);
        burst.units = units(path, 5);
        coop.ingest(&burst);
        let named: Vec<Option<u32>> = coop
            .calls
            .iter()
            .map(|call| call.victim.map(|id| id.pid))
            .collect();
        assert_eq!(named, vec![Some(6), Some(5), Some(4)]);
        coop.calls.clear();
        let mut later = snapshot(members[..1].to_vec(), 3.0);
        later.units = units(path, 6);
        coop.ingest(&later);
        assert_eq!(coop.calls.len(), 1);
        assert_eq!(
            coop.calls[0].victim, None,
            "the five killed together are all accounted for by the burst"
        );
    }

    #[test]
    fn pending_fox_calls_are_capped() {
        let mut coop = Coop::default();
        let paths: Vec<String> = (0..10)
            .map(|k| format!("/system.slice/unit{k}.service"))
            .collect();
        let at = |kills| {
            let mut sample = snapshot(Vec::new(), 1.0 + kills as f64 * 0.1);
            for path in &paths {
                sample.units.insert(
                    path.clone(),
                    Unit {
                        oom_kills: Some(kills),
                        ..Unit::default()
                    },
                );
            }
            sample
        };
        coop.ingest(&at(0));
        coop.ingest(&at(5));
        assert_eq!(coop.calls.len(), MAX_PENDING_CALLS);
    }

    #[test]
    fn coop_renders_the_same_frame_twice_from_the_same_inputs() {
        let draw = || {
            let mut scene = Scene::new();
            let mut pixels = Vec::new();
            for k in 0..6 {
                let sample = demo(20.0 + k as f64, 160);
                scene.record(&sample, 20.0 + k as f32);
                let mut frame = scene.render(&RenderInput::small(
                    &sample,
                    view("coop"),
                    &Camera::default(),
                    20.0 + k as f32 * 0.7,
                ));
                frame.rasterize();
                pixels = frame.pixels.clone();
                scene.spare = frame.release();
            }
            pixels
        };
        assert!(draw() == draw());
    }
}
