//! The per-sample decisions: each chicken's role, target, speed, noise and peers.

use std::collections::{HashMap, HashSet, VecDeque};
use std::f32::consts::{PI, TAU};

use crate::layout::spin;
use crate::model::{Blocker, Identity, Measured, Process, Snapshot, bounded, bytes};
use crate::palette::core_name;
use crate::platform::Source;
use crate::simulation::Rng;
use crate::simulation::hash::unit_from as fraction;

use super::layout::{
    FEEDER_ROW, Feeder, Flock, NEST_GAP, NEST_HALF, PAD_HEIGHT, SLOT, body_radius, flock_key,
    nest_seats, seat_offset,
};
use super::sim::{Chicken, Role, keep_inside, noise, peer_weight, speed};
use super::{Coop, DUST_RATE, Drawn, plural, puffs};

/// Roosting hysteresis in percent CPU: roost below ENTER, leave above LEAVE; a newcomer roosts
/// below NEWCOMER.
const ROOST_ENTER: f32 = 0.3;
const ROOST_LEAVE: f32 = 0.8;
const ROOST_NEWCOMER: f32 = 0.5;
const MAX_CHICKS: u32 = 12;
const MAX_EYES: usize = 6;
/// Queue columns per feeder: straight back, one slot to the right and one to the left, which
/// keeps a queue inside its feeder's lane (feeders are at least 3.6 long with 1.2 between).
const QUEUE_COLUMNS: usize = 3;

/// Coefficient of variation of the recorded CPU samples; 0 when too few or too idle to judge.
fn variation(samples: Option<&VecDeque<f32>>) -> f32 {
    let Some(samples) = samples.filter(|s| s.len() >= 3) else {
        return 0.0;
    };
    let count = samples.len() as f32;
    let mean = samples.iter().sum::<f32>() / count;
    if mean < 0.5 {
        return 0.0;
    }
    let spread = samples.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / count;
    spread.sqrt() / mean
}

fn rate(value: f32) -> String {
    bytes(value.max(0.0) as u64)
}

/// What one decision pass reads besides the yard: the drawn processes as measured, the sample
/// and the flock each key belongs to.
struct Pass<'a> {
    drawn: Drawn<'a>,
    snapshot: &'a Snapshot,
    /// Whether the sample is new, as opposed to the same sample drawn again for a changed set.
    fresh: bool,
    /// The index of each drawn flock by key.
    flock_of: HashMap<String, usize>,
    /// Every sampled process by pid, for lock holders.
    by_pid: HashMap<u32, &'a Process>,
}

impl Pass<'_> {
    /// The flock the process belongs to.
    fn flock(&self, process: &Process) -> usize {
        self.flock_of[flock_key(process).as_str()]
    }
}

/// The chickens of one decision pass, in drawing order.
struct Roll {
    chickens: Vec<Chicken>,
    /// Whether each hatched in this pass.
    newcomers: Vec<bool>,
    /// Whether each came back from hiding, its role decided for an older sample.
    returning: Vec<bool>,
    /// Whether the yard held no chicken, hidden or drawn, before this pass.
    first: bool,
}

/// The chickens queueing or eating at one feeder: priority, identity and index in the roll.
type Line = Vec<(i32, Identity, usize)>;

/// The strongest pull on each chicken: weight, the chicken it comes from and the measured
/// traffic, when there is one.
type Strongest = Vec<Option<(f32, usize, Option<f32>)>>;

impl Coop {
    /// Lays out houses, perches and feeders and decides each chicken's role, target, speed, noise
    /// and peers from the measured sample. Runs once per new sample or when the drawn set changes.
    pub(super) fn decide(
        &mut self,
        processes: &[&Process],
        order: &[usize],
        snapshot: &Snapshot,
        fresh: bool,
    ) {
        let drawn = Drawn::new(processes, order, snapshot);
        let flock_of = self.arrange(&drawn, snapshot);
        let pass = Pass {
            by_pid: snapshot
                .processes
                .iter()
                .map(|process| (process.id.pid, process))
                .collect(),
            drawn,
            snapshot,
            fresh,
            flock_of,
        };
        let mut roll = self.call_roll(&pass);
        self.note_missing(snapshot);
        let nest_of = self.judge(&pass, &mut roll);
        let eating = self.roost_and_gather(&pass, &mut roll.chickens);
        self.queue_feeders(&pass, &mut roll.chickens, eating);
        self.seat_nests(&mut roll.chickens, &nest_of);
        self.describe(&pass, &mut roll.chickens, &nest_of);
        if roll.first {
            self.place_first(&mut roll.chickens);
        }
        pull_peers(&pass, &mut roll.chickens);
        self.count_unreadable(&pass);
        self.yard.chickens = roll.chickens;
        self.raise_eyes(snapshot);
    }

    /// Every chicken drawn so far joins the hidden ones, the drawn set is taken back out of
    /// them, and what is left waits there unchanged while its process is still sampled. A
    /// process with no chicken hatches one.
    fn call_roll(&mut self, pass: &Pass) -> Roll {
        let first = self.yard.chickens.is_empty() && self.hidden.is_empty();
        let before: HashSet<Identity> = self.yard.chickens.iter().map(|c| c.id).collect();
        self.hidden.extend(
            std::mem::take(&mut self.yard.chickens)
                .into_iter()
                .map(|chicken| (chicken.id, chicken)),
        );
        let mut roll = Roll {
            chickens: Vec::with_capacity(pass.drawn.len()),
            newcomers: Vec::with_capacity(pass.drawn.len()),
            returning: Vec::with_capacity(pass.drawn.len()),
            first,
        };
        for process in pass.drawn.iter() {
            if let Some(chicken) = self.hidden.remove(&process.id) {
                roll.returning.push(!before.contains(&process.id));
                roll.chickens.push(chicken);
                roll.newcomers.push(false);
                continue;
            }
            roll.returning.push(false);
            roll.chickens.push(self.hatch(process, pass.flock(process)));
            roll.newcomers.push(true);
        }
        self.hidden.retain(|id, _| pass.drawn.sampled(*id));
        roll
    }

    /// A newcomer hatches at its flock's gathering spot beyond the perches, close enough to its
    /// mates to align with them; the spot's direction is fixed by the flock name.
    fn hatch(&self, process: &Process, flock: usize) -> Chicken {
        let mut rng = Rng::for_identity(process.id, 0xc0_0b);
        let home = self.yard.homes[flock];
        let toward = spin(&self.flocks[flock].key);
        let spot = home.footprint + 2.5;
        let scatter = 1.0 + 0.35 * (self.flocks[flock].members as f32).sqrt();
        let (angle, distance) = (rng.unit() * TAU, scatter * rng.unit().sqrt());
        let position = [
            home.center[0] + spot * toward.cos() + distance * angle.cos(),
            home.center[1] + spot * toward.sin() + distance * angle.sin(),
        ];
        let theta = rng.unit() * TAU;
        Chicken::new(process.id, flock, position, theta, rng)
    }

    /// Records what the platform could not measure, for the legend and the notes.
    fn note_missing(&mut self, snapshot: &Snapshot) {
        self.pressure_missing = snapshot.missing.contains(Source::CpuPressure);
        self.cgroups_missing = snapshot.missing.contains(Source::Cgroups);
        self.locks_missing = snapshot.missing.contains(Source::FileLocks);
    }

    /// Gives each chicken its flock, role, lock holder, body, speed, noise and notes from its
    /// process. Returns the flock whose nest each chicken goes to: the lock holder's for a
    /// waiting one.
    fn judge(&self, pass: &Pass, roll: &mut Roll) -> Vec<usize> {
        let psi = pass.snapshot.pressure[0];
        let mut nest_of: Vec<usize> = Vec::with_capacity(pass.drawn.len());
        for (k, process) in pass.drawn.iter().enumerate() {
            let chicken = &mut roll.chickens[k];
            chicken.flock = pass.flock(process);
            if process.state == 'R' {
                chicken.quiet = 0;
            } else if pass.fresh {
                chicken.quiet = chicken.quiet.saturating_add(1);
            }
            if pass.fresh || roll.newcomers[k] || roll.returning[k] {
                let previous = (!roll.newcomers[k]).then_some(chicken.role);
                let role = next_role(process, previous, chicken.quiet);
                change_role(chicken, role, roll.newcomers[k]);
            }
            let holder = match process.blocked_on {
                Some(Blocker::Process(pid)) => pass.by_pid.get(&pid).copied(),
                Some(Blocker::Unnamed) | None => None,
            };
            chicken.holder = holder.map(|holder| holder.id);
            nest_of.push(
                holder
                    .filter(|_| chicken.role == Role::Waiting)
                    .and_then(|holder| pass.flock_of.get(flock_key(holder).as_str()).copied())
                    .unwrap_or(chicken.flock),
            );
            let cv = self.measure(chicken, process, psi);
            chicken.notes = self.measurement_notes(chicken, process, cv, psi);
        }
        nest_of
    }

    /// Sizes the chicken from its process's memory, threads, CPU and CPU history, and clears its
    /// peers. Returns the coefficient of variation of its CPU.
    fn measure(&self, chicken: &mut Chicken, process: &Process, psi: f32) -> f32 {
        chicken.radius = body_radius(process.memory as f32);
        chicken.speed = speed(process.cpu);
        chicken.chicks = process.threads.saturating_sub(1).min(MAX_CHICKS);
        let cv = variation(self.history.get(&process.id));
        chicken.noise = noise(cv, psi);
        chicken.peers.clear();
        cv
    }

    /// The inspector lines of a chicken from its measurements; the first, left empty here, is
    /// for what the chicken is doing.
    fn measurement_notes(
        &self,
        chicken: &Chicken,
        process: &Process,
        cv: f32,
        psi: f32,
    ) -> Vec<String> {
        let mut notes = Vec::new();
        notes.push(String::new());
        notes.push(format!(
            "noise +/-{:.0} deg (CPU variation CV {cv:.2}, CPU pressure {})",
            chicken.noise * 0.5 * 180.0 / PI,
            if self.pressure_missing {
                "unavailable".to_owned()
            } else {
                format!("{psi:.0}%")
            }
        ));
        match &process.files {
            Measured::Known(files) => {
                let (about, at_least) = if files.partial {
                    ("about ", "at least ")
                } else {
                    ("", "")
                };
                notes.push(format!(
                    "{about}{} (eggs in the {} nest)",
                    plural(files.open.into(), "open file", "open files"),
                    self.flocks[chicken.flock].label
                ));
                if !files.deleted.is_empty() {
                    notes.push(format!(
                        "{at_least}{} still open, {} held (rotten eggs)",
                        plural(files.deleted.len() as u64, "deleted file", "deleted files"),
                        bytes(files.deleted_bytes())
                    ));
                }
            }
            Measured::Unreadable => notes.push("open files unreadable (permissions)".into()),
            Measured::Pending => notes.push("open files not read yet".into()),
        }
        if let Some(&held) = process.locks_held.known().filter(|&&held| held > 0) {
            notes.push(format!(
                "holds {}",
                plural(held.into(), "file lock", "file locks")
            ));
        }
        match process.written {
            Some(written) => {
                let mut line = format!("{} written since it started", bytes(written));
                if let Some(rate) = process.write_rate.filter(|&r| r > 0.0) {
                    line.push_str(&format!(", writing {}/s", self::rate(rate)));
                }
                notes.push(line);
            }
            None => notes.push("I/O unreadable".into()),
        }
        if let Some(read) = process.read_rate.filter(|&r| r > DUST_RATE) {
            notes.push(format!(
                "dust bathing: {} puff{} (reading {}/s)",
                puffs(read),
                if puffs(read) == 1 { "" } else { "s" },
                rate(read)
            ));
        }
        if process.threads > 1 {
            notes.push(format!(
                "chicks {} (threads {})",
                chicken.chicks, process.threads
            ));
        }
        notes
    }

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
    fn roost_and_gather(&self, pass: &Pass, chickens: &mut [Chicken]) -> Vec<Line> {
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
    fn queue_feeders(&self, pass: &Pass, chickens: &mut [Chicken], eating: Vec<Line>) {
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

    /// Writes what each chicken not at a feeder is doing as the first line of its notes.
    fn describe(&self, pass: &Pass, chickens: &mut [Chicken], nest_of: &[usize]) {
        for (k, chicken) in chickens.iter_mut().enumerate() {
            let process = pass.drawn.at(k);
            if let Some(note) = self.role_note(pass, chicken.role, process, nest_of[k]) {
                chicken.notes[0] = note;
            }
        }
    }

    /// What a chicken in `role` is doing, or `None` for a feeding one, whose note says where it
    /// stands in line.
    fn role_note(&self, pass: &Pass, role: Role, process: &Process, nest: usize) -> Option<String> {
        Some(match role {
            Role::Feeding => return None,
            Role::Foraging => "foraging".into(),
            Role::Roosting => "roosting".into(),
            Role::Brooding => format!(
                "brooding by the nest: not running while holding {}",
                plural(
                    process.locks_held.known().copied().unwrap_or(0).into(),
                    "file lock",
                    "file locks"
                )
            ),
            Role::Waiting => self.waiting_note(pass, process, nest),
            Role::Zombie => "feet up: zombie, waiting for its parent to reap it".into(),
            Role::Frozen => "frozen: stopped (tonic immobility)".into(),
            Role::Stuck => "stuck in the mud: uninterruptible sleep (D)".into(),
        })
    }

    /// Who a waiting chicken is blocked on, and whose nest it queues at.
    fn waiting_note(&self, pass: &Pass, process: &Process, nest: usize) -> String {
        let pid = match process.blocked_on {
            Some(Blocker::Process(pid)) => Some(pid),
            Some(Blocker::Unnamed) | None => None,
        };
        match pid.map(|pid| (pid, pass.by_pid.get(&pid))) {
            None => {
                "waiting for a file lock no process can be named for (an OFD lock, or a holder \
                     in another pid namespace), at its own nest"
                    .into()
            }
            Some((pid, Some(holder))) if flock_key(holder) == self.flocks[nest].key => {
                format!(
                    "waiting for a file lock held by {} (pid {pid})",
                    holder.name
                )
            }
            Some((pid, Some(holder))) => format!(
                "waiting for a file lock held by {} (pid {pid}), at its own nest \
                 because the holder's flock is not drawn",
                holder.name
            ),
            Some((pid, None)) => format!(
                "waiting for a file lock held by pid {pid}, which was not sampled, \
                 at its own nest"
            ),
        }
    }

    /// The first population starts where it belongs rather than walking in from home.
    fn place_first(&self, chickens: &mut [Chicken]) {
        for chicken in chickens {
            if chicken.walking() {
                chicken.position = keep_inside(
                    chicken.target,
                    chicken.radius,
                    self.yard.low,
                    self.yard.high,
                );
                chicken.previous = chicken.position;
                chicken.arrived = true;
                chicken.theta = chicken.role.resting_heading();
            }
        }
    }

    /// Counts the drawn processes whose I/O, descriptor tables or lock state could not be read,
    /// and the locks that name no process, for the legend.
    fn count_unreadable(&mut self, pass: &Pass) {
        self.unreadable = pass
            .drawn
            .iter()
            .filter(|process| process.written.is_none())
            .count();
        self.no_files = pass
            .drawn
            .iter()
            .filter(|process| process.files == Measured::Unreadable)
            .count();
        self.no_locks = pass
            .drawn
            .iter()
            .filter(|process| process.locks_held == Measured::Unreadable)
            .count();
        self.unattributed_locks = pass.snapshot.unattributed_locks;
    }

    /// Fox eyes round the fence, more of them and nearer the higher the memory pressure.
    fn raise_eyes(&mut self, snapshot: &Snapshot) {
        let psi_memory = snapshot.pressure[1];
        self.yard.eyes.clear();
        if psi_memory > 0.5 {
            let pairs = (1 + (psi_memory / 10.0) as usize).min(MAX_EYES);
            let outside = 3.0 * (1.0 - bounded(psi_memory, 20.0));
            for k in 0..pairs {
                self.yard
                    .eyes
                    .push(self.perimeter(fraction(k as u32 + 17), outside + 0.4));
            }
        }
    }

    /// Seats brooding chickens beside their own nest box and queues waiting ones beside the nest
    /// of `nest_of`, brooders first and each in identity order, on the seats of `nest_seats`.
    fn seat_nests(&self, chickens: &mut [Chicken], nest_of: &[usize]) {
        let mut seated: Vec<Vec<usize>> = vec![Vec::new(); self.flocks.len()];
        for role in [Role::Brooding, Role::Waiting] {
            for (k, chicken) in chickens.iter().enumerate() {
                if chicken.role == role {
                    let flock = if role == Role::Waiting {
                        nest_of[k]
                    } else {
                        chicken.flock
                    };
                    seated[flock].push(k);
                }
            }
        }
        let (low, high) = (self.yard.low, self.yard.high);
        for (flock, members) in seated.iter().enumerate() {
            let widest = members
                .iter()
                .map(|&k| chickens[k].radius)
                .fold(0.0, f32::max);
            let rim = self.flocks[flock].nest()[0] + NEST_HALF[0] + NEST_GAP;
            let seats = nest_seats(&self.flocks[flock], members.len(), widest);
            for (&k, ([x, y], first)) in members.iter().zip(seats) {
                let chicken = &mut chickens[k];
                let x = if first { rim + chicken.radius } else { x };
                let target = keep_inside([x, y], chicken.radius, low, high);
                if target != chicken.target {
                    chicken.arrived = false;
                }
                chicken.target = target;
                chicken.perch = if chicken.role == Role::Brooding {
                    PAD_HEIGHT
                } else {
                    0.0
                };
            }
        }
    }
}

/// The role a process earns, given the role its chicken had (`None` for a newcomer) and the
/// samples in a row it has not been running.
fn next_role(process: &Process, previous: Option<Role>, quiet: u8) -> Role {
    match process.state {
        'Z' => Role::Zombie,
        'T' | 't' => Role::Frozen,
        'D' => Role::Stuck,
        // A request blocked on a lock sleeps (S), so a running process the lock
        // table, read up to 2 s ago, still shows as blocked has been granted it.
        'R' => Role::Feeding,
        _ if process.blocked_on.is_some() => Role::Waiting,
        // The feeding hold: a chicken leaves the feeder only after two samples in a
        // row without running, so R/S flicker on 1 s samples does not shuttle it.
        _ if previous == Some(Role::Feeding) && quiet < 2 => Role::Feeding,
        _ if process.locks_held.known().is_some_and(|&held| held > 0) => Role::Brooding,
        _ => {
            let threshold = match previous {
                None => ROOST_NEWCOMER,
                Some(Role::Roosting) => ROOST_LEAVE,
                Some(_) => ROOST_ENTER,
            };
            let idle = if previous == Some(Role::Roosting) {
                process.cpu <= threshold
            } else {
                process.cpu < threshold
            };
            if idle { Role::Roosting } else { Role::Foraging }
        }
    }
}

/// Gives the chicken its new role; one that changes role, or hatches, must walk to its place.
fn change_role(chicken: &mut Chicken, role: Role, newcomer: bool) {
    if role != chicken.role || newcomer {
        chicken.arrived = false;
        if matches!(role, Role::Zombie | Role::Stuck) {
            chicken.perch = 0.0;
        }
        if role == Role::Foraging {
            chicken.perch = 0.0;
        }
    }
    chicken.role = role;
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

/// The linked pairs of the sample, each ordered, sorted and without repeats.
fn linked_pairs(snapshot: &Snapshot) -> Vec<(Identity, Identity)> {
    let mut pairs: Vec<(Identity, Identity)> = snapshot
        .links
        .iter()
        .map(|&(a, b, _)| if a < b { (a, b) } else { (b, a) })
        .collect();
    pairs.sort();
    pairs.dedup();
    pairs
}

/// The pull between two foraging chickens whose processes are linked, with the measured TCP
/// traffic it comes from when that decided it; `None` when there is no pull.
fn pair_pull(
    snapshot: &Snapshot,
    (a, b): (Identity, Identity),
    (cpu_a, cpu_b): (f32, f32),
) -> Option<(f32, Option<f32>)> {
    let traffic = snapshot
        .link_traffic
        .get(&(a, b))
        .or_else(|| snapshot.link_traffic.get(&(b, a)))
        .copied();
    let unix = snapshot.unix_links.contains(&(a, b)) || snapshot.unix_links.contains(&(b, a));
    // A pair can talk over TCP and Unix sockets at once, and measured TCP traffic says
    // nothing about the Unix sockets, so the stronger of the two pulls. A pair known by
    // neither (TCP not measured yet, or a platform that does not tell the protocols
    // apart) falls back to co-activity.
    let together = peer_weight(None, cpu_a, cpu_b);
    let (weight, traffic) = match traffic {
        Some(rate) => {
            let tcp = peer_weight(Some(rate), 0.0, 0.0);
            if unix && together > tcp {
                (together, None)
            } else {
                (tcp, Some(rate))
            }
        }
        None => (together, None),
    };
    if weight <= 0.0 {
        return None;
    }
    Some((weight, traffic))
}

/// Makes the foraging chickens of linked processes peers, and notes whom each walks with.
fn pull_peers(pass: &Pass, chickens: &mut [Chicken]) {
    let index_of: HashMap<Identity, usize> = chickens
        .iter()
        .enumerate()
        .map(|(k, c)| (c.id, k))
        .collect();
    let mut strongest: Strongest = vec![None; chickens.len()];
    for (a, b) in linked_pairs(pass.snapshot) {
        let (Some(&i), Some(&j)) = (index_of.get(&a), index_of.get(&b)) else {
            continue;
        };
        if i == j || chickens[i].role != Role::Foraging || chickens[j].role != Role::Foraging {
            continue;
        }
        let cpus = (pass.drawn.at(i).cpu, pass.drawn.at(j).cpu);
        let Some((weight, traffic)) = pair_pull(pass.snapshot, (a, b), cpus) else {
            continue;
        };
        chickens[i].peers.push((j, weight));
        chickens[j].peers.push((i, weight));
        for (me, other) in [(i, j), (j, i)] {
            if strongest[me].is_none_or(|(w, _, _)| weight > w) {
                strongest[me] = Some((weight, other, traffic));
            }
        }
    }
    for (k, best) in strongest.into_iter().enumerate() {
        if let Some((_, other, traffic)) = best {
            let how = match traffic {
                Some(rate) => format!("{}/s over TCP", self::rate(rate)),
                None => "both busy".into(),
            };
            let name = &pass.drawn.at(other).name;
            chickens[k].notes.push(format!("walks with {name} ({how})"));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::coop::fixtures::{coop_of, identity, legend_of, notes, process, render, snapshot};
    use crate::coop::layout::{NEST_HALF, PAD_HEIGHT, SLOT, TROUGH_DEPTH};
    use crate::coop::sim::{Chicken, Role, keep_inside, peer_weight};
    use crate::model::{Blocker, CoreKind, Cpu, Measured, Process, Snapshot};
    use crate::platform::{Source, Sources};
    use crate::render::Scene;
    use crate::views::ViewModule;

    #[test]
    fn a_busy_unix_link_still_pulls_when_the_same_pair_has_an_idle_tcp_link() {
        let pull = |traffic: f32, unix: bool| {
            let processes: Vec<Process> = (1..=2)
                .map(|pid| {
                    let mut p = process(pid, "/system.slice/a.service");
                    p.cpu = 50.0;
                    p
                })
                .collect();
            let mut sample = snapshot(processes, 1.0);
            let pair = (identity(1), identity(2));
            sample.links = vec![(pair.0, pair.1, 2)];
            sample.link_traffic.insert(pair, traffic);
            if unix {
                sample.unix_links.insert(pair);
            }
            let mut scene = Scene::new();
            render(&mut scene, &sample, 1.0);
            let chicken = coop_of(&scene).yard.find(identity(1)).unwrap();
            assert_eq!(chicken.role, Role::Foraging);
            let walks = notes(&scene, 1)
                .into_iter()
                .find(|note| note.starts_with("walks with"));
            (chicken.peers.first().map(|&(_, weight)| weight), walks)
        };
        let busy = peer_weight(None, 50.0, 50.0);
        assert!(busy > 0.0);
        assert_eq!(pull(0.0, false), (None, None), "an idle TCP link alone");
        assert_eq!(
            pull(0.0, true),
            (Some(busy), Some("walks with hen-2 (both busy)".into()))
        );
        // Busy TCP outweighs co-activity, and the note names the term that won.
        let (weight, walks) = pull(1.0e6, true);
        assert_eq!(weight, Some(peer_weight(Some(1.0e6), 0.0, 0.0)));
        assert!(weight.unwrap() > busy);
        assert!(walks.unwrap().ends_with("/s over TCP)"));
    }

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

    /// Whether a seated chicken keeps clear of its nest's eggs and house.
    fn clear_of_house_and_eggs(chicken: &Chicken, nest: [f32; 2]) -> bool {
        chicken.target[0] - chicken.radius >= nest[0] + NEST_HALF[0]
    }

    #[test]
    fn an_idle_lock_holder_broods_beside_her_own_nest() {
        let path = "/system.slice/locks.service";
        let holders: Vec<Process> = (1..=3)
            .map(|pid| {
                let mut p = process(pid, path);
                p.cpu = 20.0;
                p.locks_held = Measured::Known(pid);
                p
            })
            .collect();
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(holders.clone(), 1.0), 1.0);
        let coop = coop_of(&scene);
        let nest = coop.flocks[0].nest();
        let brooders: Vec<&Chicken> = coop.yard.chickens.iter().collect();
        for chicken in &brooders {
            assert_eq!(chicken.role, Role::Brooding, "foraging CPU still broods");
            assert!(
                clear_of_house_and_eggs(chicken, nest),
                "{:?}",
                chicken.target
            );
            assert!(chicken.target[0] - chicken.radius < nest[0] + NEST_HALF[0] + 0.5);
            assert_eq!(chicken.perch, PAD_HEIGHT);
        }
        assert!((brooders[0].target[1] - nest[1]).abs() <= NEST_HALF[1]);
        for (a, b) in brooders
            .iter()
            .flat_map(|a| brooders.iter().map(move |b| (a, b)))
            .filter(|(a, b)| a.id < b.id)
        {
            let apart = (a.target[0] - b.target[0]).hypot(a.target[1] - b.target[1]);
            assert!(apart >= a.radius + b.radius, "brooders overlap");
        }
        assert_eq!(
            notes(&scene, 2)[0],
            "brooding by the nest: not running while holding 2 file locks"
        );
        // A large idle holder, such as a browser holding its profile's SQLite locks, keeps clear
        // of the house wall and the eggs too, inside the flock's disc.
        let mut large = holders[0].clone();
        large.memory = 600 << 20;
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![large], 1.0), 1.0);
        let flock = &coop_of(&scene).flocks[0];
        let chicken = &coop_of(&scene).yard.chickens[0];
        assert!(chicken.radius > 1.0);
        assert!(clear_of_house_and_eggs(chicken, flock.nest()));
        let [x, y] = chicken.target;
        assert!(
            (x - flock.center[0]).hypot(y - flock.center[1]) + chicken.radius <= flock.reserved
        );
    }

    #[test]
    fn a_running_lock_holder_feeds_instead() {
        let mut holder = process(1, "/system.slice/locks.service");
        holder.state = 'R';
        holder.cpu = 90.0;
        holder.locks_held = Measured::Known(1);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![holder.clone()], 1.0), 1.0);
        assert_eq!(coop_of(&scene).yard.chickens[0].role, Role::Feeding);
        assert!(notes(&scene, 1).contains(&"holds 1 file lock".into()));
        // Once it stops running and the feeding hold has passed, it broods.
        holder.state = 'S';
        for elapsed in [2.0, 3.0] {
            render(
                &mut scene,
                &snapshot(vec![holder.clone()], elapsed),
                elapsed as f32,
            );
        }
        assert_eq!(coop_of(&scene).yard.chickens[0].role, Role::Brooding);
    }

    #[test]
    fn a_blocked_waiter_walks_to_the_holders_nest_in_another_flock() {
        let mut holder = process(1, "/system.slice/holder.service");
        holder.locks_held = Measured::Known(1);
        let mut waiter = process(2, "/system.slice/waiter.service");
        waiter.cpu = 50.0;
        waiter.blocked_on = Some(Blocker::Process(1));
        let mut scene = Scene::new();
        let sample = snapshot(vec![holder.clone(), waiter.clone()], 1.0);
        render(&mut scene, &sample, 1.0);
        let coop = coop_of(&scene);
        let home = coop
            .flocks
            .iter()
            .find(|flock| flock.key == "/system.slice/holder.service")
            .unwrap();
        let nest = home.nest();
        let chicken = coop.yard.find(identity(2)).unwrap();
        assert_eq!(chicken.role, Role::Waiting);
        assert_ne!(coop.flocks[chicken.flock].key, home.key);
        assert_eq!(chicken.holder, Some(identity(1)));
        assert!(chicken.target[0] > nest[0] + NEST_HALF[0]);
        assert!((chicken.target[0] - nest[0]).hypot(chicken.target[1] - nest[1]) < 3.0);
        assert_eq!(
            notes(&scene, 2)[0],
            "waiting for a file lock held by hen-1 (pid 1)"
        );
        // A blocked request sleeps, so a running process still listed as blocked by a lock
        // table read before it was granted the lock feeds.
        waiter.state = 'R';
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![holder, waiter], 1.0), 1.0);
        let chicken = coop_of(&scene).yard.find(identity(2)).unwrap();
        assert_eq!(chicken.role, Role::Feeding, "running outranks a stale wait");
        // A holder that is not sampled leaves it at its own nest.
        let mut orphan = sample.processes[1].clone();
        orphan.blocked_on = Some(Blocker::Process(77));
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![orphan.clone()], 1.0), 1.0);
        let chicken = &coop_of(&scene).yard.chickens[0];
        assert_eq!(chicken.role, Role::Waiting);
        assert!(chicken.target[0] > coop_of(&scene).flocks[0].nest()[0]);
        assert!(notes(&scene, 2)[0].contains("pid 77, which was not sampled"));
        // So does a lock no process can be named for, such as an OFD lock.
        orphan.blocked_on = Some(Blocker::Unnamed);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![orphan], 1.0), 1.0);
        let chicken = &coop_of(&scene).yard.chickens[0];
        assert_eq!(chicken.role, Role::Waiting);
        assert_eq!(chicken.holder, None);
        assert!(notes(&scene, 2)[0].contains("no process can be named for (an OFD lock"));
    }

    #[test]
    fn a_long_lock_queue_stays_inside_the_holders_reserved_disc() {
        let path = "/system.slice/cron.service";
        let mut holder = process(1, path);
        holder.locks_held = Measured::Known(1);
        for (count, memory) in [
            (12, 20 << 20),
            (60, 20 << 20),
            (120, 20 << 20),
            (16, 900 << 20),
        ] {
            let mut processes = vec![holder.clone()];
            processes.extend((2..2 + count).map(|pid| {
                let mut waiter = process(pid, "/system.slice/flock.service");
                waiter.memory = memory;
                waiter.blocked_on = Some(Blocker::Process(1));
                waiter
            }));
            let mut scene = Scene::new();
            render(&mut scene, &snapshot(processes, 1.0), 1.0);
            let coop = coop_of(&scene);
            let flock = coop.flocks.iter().find(|flock| flock.key == path).unwrap();
            let nest = flock.nest();
            let queue: Vec<&Chicken> = coop
                .yard
                .chickens
                .iter()
                .filter(|chicken| chicken.role == Role::Waiting)
                .collect();
            assert_eq!(queue.len(), count as usize);
            for chicken in &queue {
                let [x, y] = chicken.target;
                let from_center = (x - flock.center[0]).hypot(y - flock.center[1]);
                assert!(from_center <= flock.reserved, "{count} waiters: {x}, {y}");
                assert!(clear_of_house_and_eggs(chicken, nest));
                assert!(y > coop.band_top, "in the feeder band");
            }
            for (i, a) in queue.iter().enumerate() {
                for b in &queue[i + 1..] {
                    let apart = (a.target[0] - b.target[0]).hypot(a.target[1] - b.target[1]);
                    assert!(apart > 0.1, "{count} waiters coincide");
                    if count == 12 {
                        assert!(
                            apart >= a.radius + b.radius,
                            "a short queue does not overlap"
                        );
                    }
                }
            }
        }
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
    fn without_a_lock_table_no_hen_broods_or_queues_and_the_legend_says_so() {
        // macOS: every process's locks are unreadable and none is blocked on one.
        let path = "/session/app";
        let processes: Vec<Process> = (1..=4)
            .map(|pid| {
                let mut p = process(pid, path);
                p.cpu = 0.0;
                p.locks_held = Measured::Unreadable;
                p
            })
            .collect();
        let mut sample = snapshot(processes, 1.0);
        sample.missing = Source::FileLocks.into();
        let mut scene = Scene::new();
        render(&mut scene, &sample, 1.0);
        assert!(
            coop_of(&scene)
                .yard
                .chickens
                .iter()
                .all(|chicken| !matches!(chicken.role, Role::Brooding | Role::Waiting))
        );
        let legend = legend_of(&scene);
        assert!(
            legend.contains("cracked = deleted but open | chicks = threads"),
            "{legend}"
        );
        assert!(
            legend.contains("no file lock table: no brooding or queueing"),
            "{legend}"
        );
        assert!(
            !legend.contains("brooding =") && !legend.contains("file locks unreadable"),
            "{legend}"
        );
        // Where locks exist but could not be read, the legend counts the processes instead.
        sample.missing = Sources::default();
        sample.elapsed = 2.0;
        render(&mut scene, &sample, 2.0);
        let legend = legend_of(&scene);
        assert!(legend.contains("file locks unreadable for 4"), "{legend}");
        assert!(legend.contains("brooding = holds a file lock"), "{legend}");
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
