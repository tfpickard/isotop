//! The Necropolis view: exited processes as headstones in a moonlit cemetery. Each row is one
//! UTC minute of death, the newest in front. A stone's height is the lifetime CPU time of the
//! process it remembers and its width the most memory it held; its stone is tinted by kind and
//! its epitaph says how it died. Zombies stand in open pits in front of the newest row, a unit
//! whose processes are all gone gets a mausoleum, a ghost rises when a new process takes a
//! buried pid, the fog thickens with memory pressure and the moon shows its real phase.
//!
//! Deaths come from `Snapshot::life` (B3's fork, exec and exit events), taken in by `record` on
//! every sample whichever view is shown. Everything a grave shows is fixed when it is dug, and
//! its stone is built once in its own coordinates; a frame only moves, fogs and culls it.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::f32::consts::PI;

use crate::geometry::{dot, normalize};
use crate::layout::Seats;
use crate::model::{Identity, Kind, Process, Snapshot, bounded};
use crate::palette::{STATE_ZOMBIE, mix, stone_color, tint};
use crate::platform::{Cause, ExitRecord, LifeKind, LifeSource, Source};
use crate::render::{Camera, Card, Color, Frame, Item, NONE, Point, SCALE, Stage, Thing};
use crate::simulation::{Rng, smoothstep};
use crate::status::missing_note;
use crate::views::ViewModule;

/// The kind of thing a headstone or mass grave is, for picking and the inspector.
pub const GRAVE: &str = "grave";
/// The kind of thing a mausoleum is.
pub const MAUSOLEUM: &str = "mausoleum";

/// World units between the rows of consecutive minutes.
const ROW_SPACING: f32 = 3.1;
/// The length of a line of graves, filled from its left end.
const ROW_LENGTH: f32 = 26.0;
/// Each row holds two lines of graves, the second this far behind the first; graves beyond both
/// are counted on the row's label instead.
const LINE_OFFSET: f32 = 1.5;
/// The space between neighbouring stones.
const STONE_GAP: f32 = 0.18;
/// The left end of every line; graves fill towards smaller y, which is rightwards on screen.
const ROW_LEFT: f32 = ROW_LENGTH / 2.0;
/// Where the open pits of zombies are, in front of the newest row.
const PIT_X: f32 = 3.7;
/// The lot left of the rows where mausoleums stand, one 5-unit column at a time.
const LOT_Y: f32 = ROW_LEFT + 1.2;
const LOT_WIDTH: f32 = 4.6;
/// The front fence and the left fence.
const FENCE_X: f32 = 5.6;
const FENCE_Y: f32 = LOT_Y + LOT_WIDTH + 1.4;
/// The right edge of the plot, past the corner field of unmarked markers.
const PLOT_RIGHT: f32 = -ROW_LEFT - 6.0;
/// The fewest rows the plot is drawn for, so an empty cemetery is framed like a settled one.
const LEAST_ROWS: usize = 8;

/// Seconds the rows take to ease back one spacing when a new minute opens a row.
const ROW_EASE: f32 = 1.2;
/// Seconds a death waits before it is buried: long enough to know whether it belongs to a mass
/// grave and whether an OOM kill claims it.
const BURIAL_DELAY: f64 = 1.0;
/// More than this many deaths from one parent within `MASS_WINDOW` seconds share a mass grave.
const MASS_THRESHOLD: usize = 8;
const MASS_WINDOW: f64 = 1.0;
/// Above this many graves the oldest whole rows are dropped; the two oldest rows sink into the
/// fog as the count nears it, starting at `SINK_FROM` of it.
const CAP: usize = 600;
const SINK_FROM: f32 = 0.75;
/// How deep the oldest row sinks once the cemetery is full.
const SINK_DEPTH: f32 = 1.5;
/// The most unmarked markers drawn; all of them are counted.
const MARKERS_DRAWN: usize = 120;
/// Seconds a vanished zombie takes to sink into its pit.
const ZOMBIE_SINK: f32 = 0.5;
/// Seconds a ghost rises, then fades.
const GHOST_RISE: f32 = 4.0;
const GHOST_FADE: f32 = 2.0;
/// The most ghosts at once.
const GHOSTS: usize = 4;
/// Seconds an identity seen in a sample, or a fork, is remembered after it was last seen.
const REMEMBER: f64 = 120.0;
/// The fog's density at no memory pressure, and how much the pressure adds.
const FOG_BASE: f32 = 0.4;
const FOG_PRESSURE: f32 = 1.2;
/// The most a face is blended into the fog by distance alone.
const FOG_MOST: f32 = 0.6;
/// The most fog wisps; there are always four.
const WISPS: usize = 16;

/// Separates the shape draws of graves from other uses of the identity hash.
const SALT: u64 = 0x6e65_6372_6f21;

/// The fog colour, a cold violet grey a little above the sky.
const FOG: Color = [66, 66, 98];
const GRASS: Color = [38, 50, 50];
const GRAVEL: Color = [70, 70, 80];
const SOIL: Color = [36, 31, 33];
const PIT: Color = [14, 12, 16];
const IRON: Color = [24, 24, 32];
const MARBLE: Color = [118, 138, 134];
const MOON: Color = [236, 232, 206];
const EARTHSHINE: Color = [30, 32, 48];
const GHOST: Color = [170, 205, 255];
const OAK: Color = [22, 22, 30];

/// Moonlight, from high on the right behind the viewer's shoulder.
const LIGHT: Point = [0.55, -0.35, 0.76];

/// New moon of 2000-01-06 18:14 UTC, in seconds since the Unix epoch.
const NEW_MOON: f64 = 947_182_440.0;
/// The synodic month in days.
pub const SYNODIC_DAYS: f64 = 29.530_588_853;

/// The moon's age in days since the last new moon at `wall` seconds since the Unix epoch.
pub fn moon_age(wall: f64) -> f64 {
    ((wall - NEW_MOON) / 86400.0).rem_euclid(SYNODIC_DAYS)
}

/// The lit fraction of the moon's disc at `age` days.
pub fn moon_lit(age: f64) -> f64 {
    (1.0 - (std::f64::consts::TAU * age / SYNODIC_DAYS).cos()) / 2.0
}

/// The name of the phase at `age` days.
fn moon_phase(age: f64) -> &'static str {
    let lit = moon_lit(age);
    let waxing = age < SYNODIC_DAYS / 2.0;
    if lit < 0.01 {
        "new moon"
    } else if lit > 0.99 {
        "full moon"
    } else if (lit - 0.5).abs() < 0.02 {
        if waxing {
            "first quarter"
        } else {
            "last quarter"
        }
    } else {
        match (waxing, lit < 0.5) {
            (true, true) => "waxing crescent",
            (true, false) => "waxing gibbous",
            (false, true) => "waning crescent",
            (false, false) => "waning gibbous",
        }
    }
}

/// A lifetime as the epitaph says it: milliseconds under a second, tenths of a second under a
/// minute, then minutes and seconds, then hours and minutes.
pub fn lifetime_text(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    if seconds < 1.0 {
        format!("{:.0} ms", seconds * 1000.0)
    } else if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else if seconds < 3600.0 {
        let whole = seconds as u64;
        format!("{} m {} s", whole / 60, whole % 60)
    } else {
        let whole = seconds as u64;
        format!("{} h {} m", whole / 3600, whole % 3600 / 60)
    }
}

/// How a process died, by the first rule that matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Death {
    /// SIGKILL while its cgroup's OOM kill count rose in the same or the next sample.
    Oom,
    /// A signal killed it and it dumped core.
    Core,
    /// Another signal killed it.
    Signal(i32),
    /// It forked and never called exec, then exited 0 (AFORK).
    ForkedNeverExecd,
    /// It exited with a nonzero status.
    Exited(i32),
    /// It exited 0.
    Peaceful,
    /// Nothing said why: sample diffing or kqueue saw it go.
    Unknown,
}

impl Death {
    /// The cause the exit event gives, before an OOM kill can claim it: rules 2 to 7.
    pub fn of(cause: Cause, record: Option<&ExitRecord>) -> Death {
        match cause {
            Cause::Signal { core: true, .. } => Death::Core,
            Cause::Signal { number, .. } => Death::Signal(number),
            Cause::Code(0) if record.is_some_and(|record| record.forked_never_execd) => {
                Death::ForkedNeverExecd
            }
            Cause::Code(0) => Death::Peaceful,
            Cause::Code(code) => Death::Exited(code),
            Cause::Unknown => Death::Unknown,
        }
    }

    /// The epitaph's words for it.
    pub fn text(self) -> String {
        match self {
            Death::Oom => "taken by the OOM killer".into(),
            Death::Core => "dumped core".into(),
            Death::Signal(number) => match Cause::signal_name(number) {
                Some(name) => format!("killed by {name}"),
                None => format!("killed by signal {number}"),
            },
            Death::ForkedNeverExecd => "forked, never exec'd".into(),
            Death::Exited(code) => format!("exited {code}"),
            Death::Peaceful => "exited peacefully".into(),
            Death::Unknown => "died between samples".into(),
        }
    }
}

/// The height of a stone for `cpu_ms` milliseconds of lifetime CPU.
pub fn stone_height(cpu_ms: f64) -> f32 {
    (0.55 + 0.36 * cpu_ms.max(3.0).log10() as f32).min(3.2)
}

/// The width of a stone for a peak of `peak_mib` MiB.
pub fn stone_width(peak_mib: f64) -> f32 {
    (0.62 + 0.15 * peak_mib.max(1.0).log2() as f32).min(2.4)
}

/// `name` without a trailing `-pid`, so a name that already carries its pid is not given it
/// twice in `name-pid`.
fn bare_name(name: &str, pid: u32) -> &str {
    name.strip_suffix(&format!("-{pid}")).unwrap_or(name)
}

/// What a process looked like the last time a sample listed it.
#[derive(Clone, Debug)]
struct Seen {
    name: String,
    kind: Kind,
    cgroup: String,
    parent: u32,
    /// The most memory any sample saw it hold, in bytes.
    peak: u64,
    /// Its CPU seconds in the last sample.
    cpu_time: f32,
    started: f64,
    /// `Snapshot::elapsed` of the last sample that listed it.
    last: f64,
}

/// A death waiting for its burial.
#[derive(Clone, Debug)]
struct Pending {
    id: Identity,
    /// When it died, in `Snapshot::elapsed` seconds, and on the wall clock.
    at: f64,
    wall: f64,
    cause: Cause,
    record: Option<ExitRecord>,
    /// The sample that brought it.
    sample: u64,
}

/// The shape of a stone, drawn from the identity it remembers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Round,
    Slab,
    Gothic,
    Cross,
    Obelisk,
    Mass,
}

/// A triangle of a stone or building in its own coordinates, already shaded.
#[derive(Clone, Copy, Debug)]
struct Facet {
    points: [Point; 3],
    normal: Point,
    color: Color,
}

/// A many-in-one grave: more than `MASS_THRESHOLD` deaths from one parent within a second.
#[derive(Clone, Debug)]
struct Mass {
    count: usize,
    /// The most common name among them.
    comm: String,
    /// The seconds between the first and the last of them.
    spread: f64,
}

/// One grave: a headstone or a mass grave.
#[derive(Clone, Debug)]
struct Grave {
    serial: u64,
    /// The process it remembers; for a mass grave, the first of them.
    id: Identity,
    name: String,
    parent: u32,
    parent_name: Option<String>,
    kind: Kind,
    cgroup: String,
    /// When it died, in `Snapshot::elapsed` seconds and on the wall clock.
    died: f64,
    wall: f64,
    lifetime: Option<f64>,
    death: Death,
    width: f32,
    mass: Option<Mass>,
    /// Which of its row's lines it stands in and where along it; None when the row was full.
    slot: Option<(usize, f32)>,
    /// It left its row for a mausoleum.
    entombed: bool,
    facets: Vec<Facet>,
}

impl Grave {
    /// The inspector card's title: `name-pid`, or for a mass grave `comm x N (parent-pid)`.
    fn title(&self) -> String {
        match &self.mass {
            Some(mass) => format!("{} x {} ({})", mass.comm, mass.count, self.parent_label()),
            None => format!("{}-{}", bare_name(&self.name, self.id.pid), self.id.pid),
        }
    }

    fn parent_label(&self) -> String {
        match &self.parent_name {
            Some(name) => format!("{}-{}", bare_name(name, self.parent), self.parent),
            None => format!("pid {}", self.parent),
        }
    }

    /// The epitaph: what the inspector shows below the title.
    fn epitaph(&self) -> Vec<String> {
        let first = match &self.mass {
            Some(mass) => format!(
                "{} deaths within {:.1} s   {}",
                mass.count,
                mass.spread,
                self.death.text()
            ),
            None => format!(
                "lived {}   {}",
                self.lifetime.map_or_else(|| "?".into(), lifetime_text),
                self.death.text()
            ),
        };
        let cgroup = if self.cgroup.is_empty() {
            "no cgroup"
        } else {
            &self.cgroup
        };
        vec![
            first,
            format!(
                "{} | {cgroup} | parent {}",
                format!("{:?}", self.kind).to_lowercase(),
                self.parent_label()
            ),
        ]
    }
}

/// A row of graves: every grave that died in one UTC minute.
#[derive(Clone, Debug)]
struct Row {
    /// Minutes since the Unix epoch.
    minute: i64,
    graves: Vec<u64>,
    /// When the row opened, on the animation clock.
    opened: f32,
    /// Graves that found no room in its two lines.
    overflow: usize,
}

/// A unit whose last process is gone, standing at the left end of its extinction minute's row.
#[derive(Clone, Debug)]
struct Mausoleum {
    serial: u64,
    unit: String,
    minute: i64,
    graves: usize,
    /// The lot column it stands in, counted leftwards.
    column: usize,
    /// When its last grave died, on the wall clock.
    wall: f64,
}

/// A small marker for a process that was born and died between two samples.
#[derive(Clone, Copy, Debug)]
struct Marker {
    minute: i64,
    kind: Option<Kind>,
}

/// A ghost over the grave whose pid a new process took.
#[derive(Clone, Debug)]
struct Ghost {
    grave: u64,
    raised: f32,
    pid: u32,
    resting: String,
}

/// A zombie that was reaped, sinking into its pit.
#[derive(Clone, Copy, Debug)]
struct Sinking {
    pit: usize,
    since: f32,
}

/// The cemetery and everything it remembers.
#[derive(Default)]
pub struct Necropolis {
    graves: BTreeMap<u64, Grave>,
    rows: VecDeque<Row>,
    mausoleums: Vec<Mausoleum>,
    markers: VecDeque<Marker>,
    ghosts: Vec<Ghost>,
    sinking: Vec<Sinking>,
    pending: Vec<Pending>,
    seen: HashMap<Identity, Seen>,
    /// Forked children by identity: their parent and when the fork was seen.
    forks: HashMap<Identity, (Identity, f64)>,
    /// Identities whose death is pending or buried, and when it was taken in.
    mourned: HashMap<Identity, f64>,
    /// The zombies of the last sample, and their pits.
    zombies: Vec<Identity>,
    pits: Seats,
    /// Each cgroup's OOM kill count in the last sample.
    oom_counts: HashMap<String, u64>,
    /// OOM kills not yet matched to a victim: cgroup, the sample they rose in, how many.
    oom_credits: Vec<(String, u64, u64)>,
    /// The wall-clock times of the deaths of the last minute.
    recent: VecDeque<f64>,
    /// The mechanisms that delivered the deaths taken in, for a demo or live run alike.
    delivered: Vec<LifeSource>,
    samples: u64,
    last: Option<f64>,
    serials: u64,
    /// What the legend reads, as of the last record.
    wall: f64,
}

impl Necropolis {
    fn serial(&mut self) -> u64 {
        self.serials += 1;
        self.serials
    }

    /// The graves standing in rows, mass graves counting once.
    fn standing(&self) -> usize {
        self.rows
            .iter()
            .flat_map(|row| &row.graves)
            .filter(|serial| {
                self.graves
                    .get(serial)
                    .is_some_and(|grave| !grave.entombed && grave.slot.is_some())
            })
            .count()
    }

    /// Seats the zombies of `snapshot` in pits, keeping each in its pit for life.
    fn seat_zombies(&mut self, snapshot: &Snapshot) {
        self.pits.assign(
            snapshot
                .processes
                .iter()
                .filter(|process| process.state == 'Z')
                .map(|process| (process.id, "pit")),
        );
    }

    /// Takes in what `snapshot` saw since the sample before.
    fn take_in(&mut self, snapshot: &Snapshot, time: f32) {
        self.samples += 1;
        self.wall = snapshot.wall;
        let sample = self.samples;
        let elapsed = snapshot.elapsed;
        let wall_of = |at: f64| snapshot.wall + (at - elapsed);
        // OOM kills that rose since the sample before, each waiting for a victim.
        for (cgroup, unit) in &snapshot.units {
            let Some(count) = unit.oom_kills else {
                continue;
            };
            if let Some(&before) = self.oom_counts.get(cgroup)
                && count > before
            {
                self.oom_credits
                    .push((cgroup.clone(), sample, count - before));
            }
            self.oom_counts.insert(cgroup.clone(), count);
        }
        self.oom_credits.retain(|&(_, rose, _)| rose + 1 >= sample);
        self.oom_credits.sort();
        let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        // Processes that take the pid of a buried one raise a ghost over its grave.
        for process in &snapshot.processes {
            if !self.seen.contains_key(&process.id) && self.last.is_some() {
                self.raise_ghost(process.id, time);
            }
            let seen = self.seen.entry(process.id).or_insert_with(|| Seen {
                name: process.name.clone(),
                kind: process.kind,
                cgroup: process.cgroup.clone(),
                parent: process.parent,
                peak: 0,
                cpu_time: 0.0,
                started: process.started,
                last: elapsed,
            });
            seen.name.clone_from(&process.name);
            seen.kind = process.kind;
            seen.cgroup.clone_from(&process.cgroup);
            seen.parent = process.parent;
            seen.peak = seen.peak.max(process.memory);
            seen.cpu_time = process.cpu_time;
            seen.last = elapsed;
        }
        for event in &snapshot.life {
            if !self.delivered.contains(&event.source) {
                self.delivered.push(event.source);
                self.delivered
                    .sort_by_key(|source| LifeSource::ALL.iter().position(|s| s == source));
            }
            match &event.kind {
                LifeKind::Fork { parent, child } => {
                    self.forks.insert(*child, (*parent, event.at));
                }
                LifeKind::Exec { .. } => {}
                LifeKind::Exit { id, cause, record } => {
                    if self.mourned.contains_key(id) {
                        continue;
                    }
                    self.mourned.insert(*id, elapsed);
                    self.pending.push(Pending {
                        id: *id,
                        at: event.at,
                        wall: wall_of(event.at),
                        cause: *cause,
                        record: record.clone(),
                        sample,
                    });
                }
            }
        }
        // A zombie that is gone was reaped: it sinks into its pit and is buried, with its exit
        // event if one came, and as having died between samples if none did.
        let zombies: Vec<Identity> = snapshot
            .processes
            .iter()
            .filter(|process| process.state == 'Z')
            .map(|process| process.id)
            .collect();
        for id in std::mem::take(&mut self.zombies) {
            if alive.contains(&id) {
                continue;
            }
            if let Some(pit) = self.pits.seat(id) {
                self.sinking.push(Sinking { pit, since: time });
            }
            if let std::collections::hash_map::Entry::Vacant(entry) = self.mourned.entry(id) {
                entry.insert(elapsed);
                self.pending.push(Pending {
                    id,
                    at: elapsed,
                    wall: snapshot.wall,
                    cause: Cause::Unknown,
                    record: None,
                    sample,
                });
            }
        }
        self.zombies = zombies;
        self.seat_zombies(snapshot);
        self.sinking
            .retain(|sinking| time - sinking.since < ZOMBIE_SINK && time >= sinking.since);
        self.bury(snapshot, &alive, time);
        self.entomb(snapshot);
        self.cap();
        self.recent.retain(|&wall| wall > snapshot.wall - 60.0);
        self.ghosts
            .retain(|ghost| time - ghost.raised < GHOST_RISE + GHOST_FADE && time >= ghost.raised);
        self.seen.retain(|_, seen| seen.last > elapsed - REMEMBER);
        self.forks.retain(|_, &mut (_, at)| at > elapsed - REMEMBER);
        self.mourned.retain(|_, &mut at| at > elapsed - REMEMBER);
    }

    /// Raises a ghost over the newest standing grave whose pid `id` took.
    fn raise_ghost(&mut self, id: Identity, time: f32) {
        let Some(grave) = self.graves.values().rev().find(|grave| {
            grave.mass.is_none()
                && grave.id.pid == id.pid
                && grave.id != id
                && !grave.entombed
                && grave.slot.is_some()
        }) else {
            return;
        };
        let ghost = Ghost {
            grave: grave.serial,
            raised: time,
            pid: id.pid,
            resting: grave.title(),
        };
        self.ghosts.retain(|old| old.grave != ghost.grave);
        if self.ghosts.len() >= GHOSTS {
            self.ghosts.remove(0);
        }
        self.ghosts.push(ghost);
    }

    /// The parent pid of a pending death: the process it was last seen as, or its fork.
    fn parent_of(&self, id: Identity) -> Option<u32> {
        self.seen
            .get(&id)
            .map(|seen| seen.parent)
            .or_else(|| self.forks.get(&id).map(|(parent, _)| parent.pid))
    }

    /// Buries the deaths that have waited long enough: in a mass grave, as an unmarked marker
    /// when no sample ever listed the process, or under a headstone.
    fn bury(&mut self, snapshot: &Snapshot, alive: &HashSet<Identity>, time: f32) {
        let elapsed = snapshot.elapsed;
        let mut pending = std::mem::take(&mut self.pending);
        pending.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.id.cmp(&b.id)));
        let samples = self.samples;
        let ready = |death: &Pending| {
            elapsed - death.at >= BURIAL_DELAY && death.sample < samples
                || death.at < elapsed - REMEMBER
        };
        let mut taken = vec![false; pending.len()];
        let mut singles = Vec::new();
        for index in 0..pending.len() {
            if taken[index] || !ready(&pending[index]) || alive.contains(&pending[index].id) {
                continue;
            }
            let first = &pending[index];
            let parent = self.parent_of(first.id);
            let members: Vec<usize> = match parent {
                Some(parent) => (index..pending.len())
                    .filter(|&other| {
                        !taken[other]
                            && !alive.contains(&pending[other].id)
                            && pending[other].at - first.at <= MASS_WINDOW
                            && self.parent_of(pending[other].id) == Some(parent)
                    })
                    .collect(),
                None => Vec::new(),
            };
            if members.len() > MASS_THRESHOLD {
                let deaths: Vec<Pending> = members.iter().map(|&m| pending[m].clone()).collect();
                for &member in &members {
                    taken[member] = true;
                }
                self.dig_mass(&deaths, snapshot, time);
                continue;
            }
            taken[index] = true;
            singles.push(pending[index].clone());
        }
        // Each OOM kill that rose claims one SIGKILL of its cgroup, the largest first.
        let mut killed: Vec<(u64, Identity, String, u64)> = singles
            .iter()
            .filter(|death| {
                death.cause
                    == Cause::Signal {
                        number: 9,
                        core: false,
                    }
            })
            .filter_map(|death| {
                let seen = self.seen.get(&death.id)?;
                let peak = death
                    .record
                    .as_ref()
                    .map_or(seen.peak, |record| record.hiwater_rss.max(seen.peak));
                Some((peak, death.id, seen.cgroup.clone(), death.sample))
            })
            .collect();
        killed.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut oom = HashSet::new();
        for (_, id, cgroup, sample) in killed {
            if let Some(credit) = self.oom_credits.iter_mut().find(|(unit, rose, left)| {
                *unit == cgroup && *left > 0 && (*rose == sample || *rose == sample + 1)
            }) {
                credit.2 -= 1;
                oom.insert(id);
            }
        }
        for death in singles {
            let killed = oom.contains(&death.id);
            self.dig(death, killed, snapshot, time);
        }
        self.pending = pending
            .into_iter()
            .zip(taken)
            .filter_map(|(death, taken)| (!taken).then_some(death))
            .collect();
    }

    /// The name of the process with pid `pid`, if a sample listed one.
    fn name_of(&self, snapshot: &Snapshot, pid: u32) -> Option<String> {
        snapshot
            .processes
            .iter()
            .find(|process| process.id.pid == pid)
            .map(|process| process.name.clone())
            .or_else(|| {
                let mut named: Vec<(&Identity, &Seen)> =
                    self.seen.iter().filter(|(id, _)| id.pid == pid).collect();
                named.sort_by_key(|(id, _)| **id);
                named.last().map(|(_, seen)| seen.name.clone())
            })
    }

    /// Digs a headstone for `death`, taken by the OOM killer if `oom`, or sets an unmarked marker
    /// when no sample listed it.
    fn dig(&mut self, death: Pending, oom: bool, snapshot: &Snapshot, time: f32) {
        self.recent.push_back(death.wall);
        let minute = (death.wall / 60.0).floor() as i64;
        let Some(seen) = self.seen.get(&death.id).cloned() else {
            let kind = self
                .forks
                .get(&death.id)
                .and_then(|(parent, _)| self.seen.get(parent))
                .map(|seen| seen.kind);
            self.markers.push_back(Marker { minute, kind });
            return;
        };
        let record = death.record.as_ref();
        let cpu_ms = record.map_or(f64::from(seen.cpu_time) * 1000.0, |record| {
            record.cpu as f64 / 1000.0
        });
        let peak = record
            .map(|record| record.hiwater_rss)
            .filter(|&peak| peak > 0)
            .unwrap_or(seen.peak);
        let lifetime = record.map(|record| record.etime as f64 / 1e6).or_else(|| {
            (seen.started > 0.0 && death.wall > seen.started).then_some(death.wall - seen.started)
        });
        let width = stone_width(peak as f64 / 1048576.0);
        let height = stone_height(cpu_ms);
        let mut rng = Rng::for_identity(death.id, SALT);
        let shape = match rng.unit() {
            _ if height > 2.55 && rng.unit() < 0.6 => Shape::Obelisk,
            draw if draw < 0.55 => Shape::Round,
            draw if draw < 0.75 => Shape::Slab,
            draw if draw < 0.93 => Shape::Gothic,
            _ => Shape::Cross,
        };
        let serial = self.serial();
        let grave = Grave {
            serial,
            id: death.id,
            name: seen.name.clone(),
            parent: seen.parent,
            parent_name: self.name_of(snapshot, seen.parent),
            kind: seen.kind,
            cgroup: seen.cgroup.clone(),
            died: death.at,
            wall: death.wall,
            lifetime,
            death: if oom {
                Death::Oom
            } else {
                Death::of(death.cause, death.record.as_ref())
            },
            width,
            mass: None,
            slot: None,
            entombed: false,
            facets: stone(shape, width, height, stone_color(seen.kind)),
        };
        self.place(grave, minute, time);
    }

    /// Digs one mass grave for `deaths`, which share a parent.
    fn dig_mass(&mut self, deaths: &[Pending], snapshot: &Snapshot, time: f32) {
        let first = &deaths[0];
        let last = &deaths[deaths.len() - 1];
        for death in deaths {
            self.recent.push_back(death.wall);
        }
        let mut names: BTreeMap<String, usize> = BTreeMap::new();
        let mut kind = None;
        let mut cgroup = String::new();
        let mut parent = 0;
        for death in deaths {
            let name = self
                .seen
                .get(&death.id)
                .map(|seen| seen.name.clone())
                .or_else(|| death.record.as_ref().map(|record| record.comm.clone()))
                .unwrap_or_else(|| "?".into());
            *names
                .entry(bare_name(&name, death.id.pid).to_owned())
                .or_default() += 1;
            if let Some(seen) = self.seen.get(&death.id) {
                kind.get_or_insert(seen.kind);
                if cgroup.is_empty() {
                    cgroup.clone_from(&seen.cgroup);
                }
            }
            parent = self.parent_of(death.id).unwrap_or(parent);
        }
        let comm = names
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(name, _)| name.clone())
            .unwrap_or_default();
        let kind = kind.unwrap_or_default();
        let count = deaths.len();
        let causes: Vec<Death> = deaths
            .iter()
            .map(|death| Death::of(death.cause, death.record.as_ref()))
            .collect();
        let death = if causes.iter().all(|cause| *cause == causes[0]) {
            causes[0]
        } else {
            Death::Unknown
        };
        let width = (2.4 + 0.1 * count as f32).min(3.6);
        let serial = self.serial();
        let grave = Grave {
            serial,
            id: first.id,
            name: comm.clone(),
            parent,
            parent_name: self.name_of(snapshot, parent),
            kind,
            cgroup,
            died: last.at,
            wall: last.wall,
            lifetime: None,
            death,
            width,
            mass: Some(Mass {
                count,
                comm,
                spread: last.at - first.at,
            }),
            slot: None,
            entombed: false,
            facets: stone(Shape::Mass, width, 0.42, tint(stone_color(kind), 0.92)),
        };
        self.place(grave, (last.wall / 60.0).floor() as i64, time);
    }

    /// Puts `grave` in the row of `minute`: a new row when the minute is newer than every row,
    /// the open (newest) row beside its family, or the end of a closed row.
    fn place(&mut self, grave: Grave, minute: i64, time: f32) {
        let serial = grave.serial;
        let (pid, parent) = (grave.id.pid, grave.parent);
        let mass = grave.mass.is_some();
        self.graves.insert(serial, grave);
        if self.rows.back().is_none_or(|row| minute > row.minute) {
            self.rows.push_back(Row {
                minute,
                graves: Vec::new(),
                opened: time,
                overflow: 0,
            });
        }
        let newest = self.rows.len() - 1;
        let index = self
            .rows
            .iter()
            .rposition(|row| row.minute <= minute)
            .unwrap_or(0);
        if index == newest {
            // A parent or child already buried in the open row takes this grave beside it.
            let graves = &self.graves;
            let row = &self.rows[newest];
            // Its parent, or a child of its; then the family is those and their siblings.
            let related = |other: &Grave| {
                other.mass.is_none() && (other.id.pid == parent || (!mass && other.parent == pid))
            };
            let kin = |other: &Grave| related(other) || (parent != 0 && other.parent == parent);
            let find = |test: &dyn Fn(&Grave) -> bool| {
                row.graves
                    .iter()
                    .rposition(|serial| graves.get(serial).is_some_and(test))
            };
            let at = match find(&related) {
                Some(_) => find(&kin).map_or(row.graves.len(), |index| index + 1),
                None => row.graves.len(),
            };
            self.rows[newest].graves.insert(at, serial);
            self.pack(newest);
        } else {
            self.rows[index].graves.push(serial);
            self.pack_last(index);
        }
    }

    /// Lays every grave of row `index` out again, in order: the open row only.
    fn pack(&mut self, index: usize) {
        let mut cursor = (0, ROW_LEFT);
        let mut overflow = 0;
        for serial in self.rows[index].graves.clone() {
            if let Some(grave) = self.graves.get_mut(&serial) {
                grave.slot = next_slot(&mut cursor, grave.width);
                overflow += usize::from(grave.slot.is_none());
            }
        }
        self.rows[index].overflow = overflow;
    }

    /// Lays out only the last grave of row `index`, after the others, which stay where they are.
    fn pack_last(&mut self, index: usize) {
        let row = &self.rows[index];
        let mut cursor = (0, ROW_LEFT);
        for serial in &row.graves[..row.graves.len() - 1] {
            if let Some(grave) = self.graves.get(serial) {
                next_slot(&mut cursor, grave.width);
            }
        }
        let last = row.graves[row.graves.len() - 1];
        if let Some(grave) = self.graves.get_mut(&last) {
            grave.slot = next_slot(&mut cursor, grave.width);
            if grave.slot.is_none() {
                self.rows[index].overflow += 1;
            }
        }
    }

    /// Moves the graves of every unit that has no process left, once it has three standing, into
    /// a mausoleum at the left end of the row of its last death. On Linux a unit is a cgroup
    /// that ends in `.service`; where there are no cgroups, any group.
    fn entomb(&mut self, snapshot: &Snapshot) {
        let without_cgroups = snapshot.missing.contains(Source::Cgroups);
        let live: HashSet<&str> = snapshot
            .processes
            .iter()
            .map(|process| process.cgroup.as_str())
            .collect();
        let mut units: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        for row in &self.rows {
            for serial in &row.graves {
                let Some(grave) = self.graves.get(serial) else {
                    continue;
                };
                let leaf = grave.cgroup.rsplit('/').next().unwrap_or_default();
                let unit = if without_cgroups {
                    !grave.cgroup.is_empty()
                } else {
                    leaf.ends_with(".service")
                };
                if unit && !grave.entombed && grave.slot.is_some() {
                    units.entry(grave.cgroup.clone()).or_default().push(*serial);
                }
            }
        }
        for (cgroup, serials) in units {
            let existing = self.mausoleums.iter().position(|m| m.unit == cgroup);
            if live.contains(cgroup.as_str()) || (serials.len() < 3 && existing.is_none()) {
                continue;
            }
            let mut wall = 0.0_f64;
            for serial in &serials {
                if let Some(grave) = self.graves.get_mut(serial) {
                    grave.entombed = true;
                    wall = wall.max(grave.wall);
                }
            }
            if let Some(index) = existing {
                let mausoleum = &mut self.mausoleums[index];
                mausoleum.graves += serials.len();
                mausoleum.wall = mausoleum.wall.max(wall);
                continue;
            }
            let minute = serials
                .iter()
                .filter_map(|serial| self.graves.get(serial))
                .map(|grave| (grave.wall / 60.0).floor() as i64)
                .max()
                .unwrap_or_default();
            let row = self
                .rows
                .iter()
                .rposition(|row| row.minute <= minute)
                .unwrap_or(0);
            let minute = self.rows[row].minute;
            let near = |other: &Mausoleum| {
                self.rows
                    .iter()
                    .position(|row| row.minute == other.minute)
                    .is_some_and(|other| other.abs_diff(row) <= 1)
            };
            let column = (0..)
                .find(|column| {
                    !self
                        .mausoleums
                        .iter()
                        .any(|other| other.column == *column && near(other))
                })
                .unwrap_or(0);
            let serial = self.serial();
            self.mausoleums.push(Mausoleum {
                serial,
                unit: cgroup,
                minute,
                graves: serials.len(),
                column,
                wall,
            });
        }
    }

    /// Drops the oldest whole rows while more than `CAP` graves stand, with their mausoleums and
    /// the markers of their minutes.
    fn cap(&mut self) {
        while self.standing() > CAP && self.rows.len() > 1 {
            if let Some(row) = self.rows.pop_front() {
                for serial in row.graves {
                    self.graves.remove(&serial);
                }
                self.mausoleums.retain(|m| m.minute != row.minute);
            }
        }
        let oldest = self.rows.front().map_or(i64::MAX, |row| row.minute);
        while self
            .markers
            .front()
            .is_some_and(|marker| marker.minute < oldest)
        {
            self.markers.pop_front();
        }
        let kept: HashSet<u64> = self.graves.keys().copied().collect();
        self.ghosts.retain(|ghost| kept.contains(&ghost.grave));
    }

    /// Where row `index` (0 the oldest) stands at `time`: the newest at 0 and each older one a
    /// spacing behind, easing back one spacing when a new row opens.
    fn row_x(&self, index: usize, time: f32) -> f32 {
        let from_front = (self.rows.len() - 1 - index) as f32;
        let ease = self
            .rows
            .back()
            .map_or(1.0, |row| smoothstep((time - row.opened) / ROW_EASE));
        -ROW_SPACING * (from_front - (1.0 - ease))
    }

    /// How far the two oldest rows have sunk into the fog: the oldest by `SINK_DEPTH` and fully
    /// fogged once `CAP` graves stand, the second by half that, from `SINK_FROM` of the cap.
    fn sinking_rows(&self) -> f32 {
        let standing = self.standing() as f32;
        ((standing / CAP as f32 - SINK_FROM) / (1.0 - SINK_FROM)).clamp(0.0, 1.0)
    }

    /// The deaths of the last minute.
    fn per_minute(&self) -> usize {
        self.recent.len()
    }

    /// The x of the plot's back edge, behind the oldest row.
    fn back_x(&self) -> f32 {
        -ROW_SPACING * (self.rows.len().max(LEAST_ROWS) as f32 - 1.0) - 2.6
    }
}

/// The next place along a row's lines for a grave `width` wide, from `cursor` (line, left
/// edge); None when both lines are full.
fn next_slot(cursor: &mut (usize, f32), width: f32) -> Option<(usize, f32)> {
    if cursor.0 < 2 && cursor.1 - width < ROW_LEFT - ROW_LENGTH {
        *cursor = (cursor.0 + 1, ROW_LEFT);
    }
    if cursor.0 >= 2 {
        return None;
    }
    let center = cursor.1 - width / 2.0;
    cursor.1 -= width + STONE_GAP;
    Some((cursor.0, center))
}

/// A face shaded by how it faces the moonlight.
fn shade(color: Color, normal: Point) -> Color {
    tint(color, 0.5 + 0.62 * dot(normal, normalize(LIGHT)).max(0.0))
}

fn push_triangle(out: &mut Vec<Facet>, points: [Point; 3], normal: Point, color: Color) {
    out.push(Facet {
        points,
        normal,
        color: shade(color, normal),
    });
}

fn push_quad(out: &mut Vec<Facet>, points: [Point; 4], normal: Point, color: Color) {
    let [a, b, c, d] = points;
    push_triangle(out, [a, b, c], normal, color);
    push_triangle(out, [a, c, d], normal, color);
}

/// The top and four sides of an axis-aligned box.
fn push_box(out: &mut Vec<Facet>, low: Point, high: Point, color: Color) {
    let [x0, y0, z0] = low;
    let [x1, y1, z1] = high;
    push_quad(
        out,
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        [0.0, 0.0, 1.0],
        color,
    );
    push_quad(
        out,
        [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        [1.0, 0.0, 0.0],
        color,
    );
    push_quad(
        out,
        [[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]],
        [-1.0, 0.0, 0.0],
        color,
    );
    push_quad(
        out,
        [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
        [0.0, 1.0, 0.0],
        color,
    );
    push_quad(
        out,
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        [0.0, -1.0, 0.0],
        color,
    );
}

/// A slab of thickness `x` whose front and back follow `profile`, a closed outline in (y, z)
/// that is convex and runs anticlockwise seen from the front, with its top edges as strips.
fn push_profile(out: &mut Vec<Facet>, profile: &[(f32, f32)], half: f32, color: Color) {
    let count = profile.len() as f32;
    let center = profile.iter().fold((0.0, 0.0), |sum, p| {
        (sum.0 + p.0 / count, sum.1 + p.1 / count)
    });
    for (index, &a) in profile.iter().enumerate() {
        let b = profile[(index + 1) % profile.len()];
        push_triangle(
            out,
            [
                [half, center.0, center.1],
                [half, b.0, b.1],
                [half, a.0, a.1],
            ],
            [1.0, 0.0, 0.0],
            color,
        );
        push_triangle(
            out,
            [
                [-half, center.0, center.1],
                [-half, a.0, a.1],
                [-half, b.0, b.1],
            ],
            [-1.0, 0.0, 0.0],
            color,
        );
        // The rim between front and back, facing outwards.
        let outward = normalize([0.0, b.1 - a.1, a.0 - b.0]);
        let outward = if dot(
            outward,
            [
                0.0,
                (a.0 + b.0) / 2.0 - center.0,
                (a.1 + b.1) / 2.0 - center.1,
            ],
        ) < 0.0
        {
            outward.map(|v| -v)
        } else {
            outward
        };
        if a.1 > 0.0 || b.1 > 0.0 {
            push_quad(
                out,
                [
                    [-half, a.0, a.1],
                    [half, a.0, a.1],
                    [half, b.0, b.1],
                    [-half, b.0, b.1],
                ],
                outward,
                color,
            );
        }
    }
}

/// The engraved lines on a stone's front face, at `half` its thickness and up to `shoulder`.
fn push_engraving(out: &mut Vec<Facet>, half: f32, width: f32, shoulder: f32, color: Color) {
    let lines: &[(f32, f32)] = if shoulder > 0.7 {
        &[(0.62, 0.55), (0.5, 0.75), (0.41, 0.5)]
    } else {
        &[(0.55, 0.6)]
    };
    let x = half + 0.012;
    for &(up, wide) in lines {
        let z = shoulder * up;
        let w = width * wide * 0.5;
        push_quad(
            out,
            [[x, w, z], [x, -w, z], [x, -w, z + 0.055], [x, w, z + 0.055]],
            [1.0, 0.0, 0.0],
            tint(color, 0.45),
        );
    }
}

/// A stone of `shape`, `width` along its row and `height` tall, in its own coordinates: the base
/// centred on the origin, the row along y, the face towards +x, with its soil bed in front.
fn stone(shape: Shape, width: f32, height: f32, color: Color) -> Vec<Facet> {
    let mut out = Vec::with_capacity(48);
    let half = (0.30 + 0.05 * width.min(2.4)) / 2.0;
    let a = width / 2.0;
    let bed = if shape == Shape::Mass { 1.0 } else { half };
    push_quad(
        &mut out,
        [
            [bed + 0.06, a - 0.05, 0.012],
            [bed + 0.95, a - 0.05, 0.012],
            [bed + 0.95, -a + 0.05, 0.012],
            [bed + 0.06, -a + 0.05, 0.012],
        ],
        [0.0, 0.0, 1.0],
        SOIL,
    );
    let plinth = 0.14_f32.min(height * 0.3);
    match shape {
        Shape::Round | Shape::Gothic => {
            let gothic = shape == Shape::Gothic;
            let rise = if gothic {
                (a * 1.3).min(height * 0.45)
            } else {
                a.min(0.55).min(height * 0.4)
            };
            let shoulder = height - rise;
            let mut profile = vec![(-a, 0.0), (a, 0.0)];
            const SEGMENTS: usize = 6;
            for step in 0..=SEGMENTS {
                let s = step as f32 / SEGMENTS as f32;
                let y = a - 2.0 * a * s;
                let z = if gothic {
                    let t = 1.0 - (2.0 * s - 1.0).abs();
                    shoulder + rise * (1.0 - (1.0 - t) * (1.0 - t))
                } else {
                    let angle = PI * s;
                    shoulder + rise * angle.sin()
                };
                profile.push((y, z));
            }
            push_profile(&mut out, &profile, half, color);
            push_engraving(&mut out, half, width, shoulder, color);
        }
        Shape::Slab => {
            push_box(
                &mut out,
                [-half - 0.12, -a - 0.1, 0.0],
                [half + 0.12, a + 0.1, plinth],
                tint(color, 0.85),
            );
            push_box(&mut out, [-half, -a, plinth], [half, a, height], color);
            push_engraving(&mut out, half, width, height, color);
        }
        Shape::Cross => {
            let arm = a.min(0.45);
            push_box(
                &mut out,
                [-half - 0.12, -arm - 0.1, 0.0],
                [half + 0.12, arm + 0.1, plinth],
                tint(color, 0.85),
            );
            push_box(
                &mut out,
                [-half * 0.8, -arm * 0.32, plinth],
                [half * 0.8, arm * 0.32, height],
                color,
            );
            let bar = plinth + (height - plinth) * 0.68;
            push_box(
                &mut out,
                [-half * 0.8, -arm, bar - 0.12],
                [half * 0.8, arm, bar + 0.12],
                color,
            );
        }
        Shape::Obelisk => {
            let base = a.min(0.6);
            push_box(
                &mut out,
                [-base - 0.14, -base - 0.14, 0.0],
                [base + 0.14, base + 0.14, plinth],
                tint(color, 0.85),
            );
            let top = base * 0.62;
            let shoulder = plinth + (height - plinth) * 0.9;
            let ring = |r: f32, z: f32| [[r, r, z], [-r, r, z], [-r, -r, z], [r, -r, z]];
            let low = ring(base, plinth);
            let high = ring(top, shoulder);
            let apex = [0.0, 0.0, height];
            for side in 0..4 {
                let next = (side + 1) % 4;
                let middle = [
                    (low[side][0] + low[next][0]) / 2.0,
                    (low[side][1] + low[next][1]) / 2.0,
                    0.0,
                ];
                let lean = (base - top) / (shoulder - plinth);
                let normal = normalize([middle[0], middle[1], lean * base]);
                push_quad(
                    &mut out,
                    [low[side], low[next], high[next], high[side]],
                    normal,
                    color,
                );
                let roof = normalize([middle[0], middle[1], base * 0.8]);
                push_triangle(&mut out, [high[side], high[next], apex], roof, color);
            }
        }
        Shape::Mass => {
            push_box(
                &mut out,
                [-0.9, -a, 0.0],
                [0.9, a, height * 0.6],
                tint(color, 0.92),
            );
            push_box(
                &mut out,
                [-0.8, -a + 0.1, height * 0.6],
                [0.8, a - 0.1, height],
                color,
            );
        }
    }
    out
}

/// A mausoleum `depth` deep (along x) and `width` wide, in its own coordinates: its back corner
/// at the origin, its portico towards +x.
fn mausoleum(depth: f32, width: f32) -> Vec<Facet> {
    let mut out = Vec::with_capacity(120);
    let (x0, x1, y0, y1) = (0.0, depth, 0.0, width);
    push_box(
        &mut out,
        [x0 - 0.3, y0 - 0.3, 0.0],
        [x1 + 0.3, y1 + 0.3, 0.22],
        tint(MARBLE, 0.8),
    );
    push_box(
        &mut out,
        [x0 - 0.15, y0 - 0.15, 0.22],
        [x1 + 0.15, y1 + 0.15, 0.42],
        tint(MARBLE, 0.88),
    );
    let wall = x1 - 0.9;
    push_box(
        &mut out,
        [x0, y0 + 0.15, 0.42],
        [wall, y1 - 0.15, 2.6],
        tint(MARBLE, 0.95),
    );
    let middle = (y0 + y1) / 2.0;
    for k in 0..4 {
        let y = y0 + 0.35 + (y1 - y0 - 0.7) * k as f32 / 3.0;
        push_box(
            &mut out,
            [x1 - 0.42, y - 0.12, 0.42],
            [x1 - 0.18, y + 0.12, 2.6],
            MARBLE,
        );
    }
    push_quad(
        &mut out,
        [
            [wall + 0.01, middle + 0.45, 0.42],
            [wall + 0.01, middle - 0.45, 0.42],
            [wall + 0.01, middle - 0.45, 1.85],
            [wall + 0.01, middle + 0.45, 1.85],
        ],
        [1.0, 0.0, 0.0],
        [20, 22, 30],
    );
    push_box(
        &mut out,
        [x0 - 0.12, y0 - 0.02, 2.6],
        [x1 + 0.05, y1 + 0.02, 3.05],
        tint(MARBLE, 1.02),
    );
    let (eave, ridge) = (3.05, 3.85);
    let slope = normalize([0.0, 1.0, (y1 - middle) / (ridge - eave)]);
    push_quad(
        &mut out,
        [
            [x0 - 0.12, y1 + 0.02, eave],
            [x1 + 0.05, y1 + 0.02, eave],
            [x1 + 0.05, middle, ridge],
            [x0 - 0.12, middle, ridge],
        ],
        slope,
        tint(MARBLE, 0.9),
    );
    push_quad(
        &mut out,
        [
            [x1 + 0.05, y0 - 0.02, eave],
            [x0 - 0.12, y0 - 0.02, eave],
            [x0 - 0.12, middle, ridge],
            [x1 + 0.05, middle, ridge],
        ],
        [0.0, -slope[1], slope[2]],
        tint(MARBLE, 0.9),
    );
    push_triangle(
        &mut out,
        [
            [x1 + 0.05, y0 - 0.02, eave],
            [x1 + 0.05, middle, ridge],
            [x1 + 0.05, y1 + 0.02, eave],
        ],
        [1.0, 0.0, 0.0],
        MARBLE,
    );
    out
}

/// A zombie standing with its feet at `base`, arms reaching towards +x, swaying with `sway`.
fn zombie(base: Point, sway: f32) -> Vec<Facet> {
    let mut out = Vec::with_capacity(60);
    let [x, y, z] = base;
    let dark = tint(STATE_ZOMBIE, 0.78);
    push_box(
        &mut out,
        [x - 0.13, y - 0.24, z],
        [x + 0.13, y - 0.03, z + 0.75],
        dark,
    );
    push_box(
        &mut out,
        [x - 0.13, y + 0.03, z],
        [x + 0.13, y + 0.24, z + 0.75],
        dark,
    );
    let torso = z + 0.75;
    push_box(
        &mut out,
        [x - 0.17, y - 0.3, torso],
        [x + 0.17, y + 0.3, torso + 0.72],
        STATE_ZOMBIE,
    );
    let arms = torso + 0.48 + sway;
    for side in [-1.0_f32, 1.0] {
        let inner = y + side * 0.3;
        let outer = inner + side * 0.17;
        push_box(
            &mut out,
            [x - 0.1, inner.min(outer), arms],
            [x + 0.78, inner.max(outer), arms + 0.17],
            tint(STATE_ZOMBIE, 0.95),
        );
    }
    let head = torso + 0.72;
    push_box(
        &mut out,
        [x - 0.15, y - 0.18, head],
        [x + 0.23, y + 0.18, head + 0.38],
        tint(STATE_ZOMBIE, 1.05),
    );
    out
}

/// Draws `facets` moved by `offset`, fogged by `fog`, culling those that face away.
fn draw_facets(
    frame: &mut Frame,
    camera: &Camera,
    facets: &[Facet],
    offset: Point,
    fog: f32,
    pick: u32,
) {
    let viewer = camera.viewer();
    for facet in facets {
        if dot(facet.normal, viewer) <= 0.0 {
            continue;
        }
        let points = facet
            .points
            .map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]);
        frame.facet(camera, points, mix(facet.color, FOG, fog), pick);
    }
}

/// The fog over a point: its distance from the front of the plot along the view, as a share of
/// the plot's depth, times the density, which memory pressure sets.
/// Wisps thicken it where they drift: Gaussian patches on the ground, baked into the faces'
/// colours like the rest of the fog, so both renderers draw them alike.
struct Fog {
    near: f32,
    far: f32,
    density: f32,
    /// Each wisp's centre on the ground, its radius in world units and its strength.
    wisps: Vec<([f32; 2], f32, f32)>,
}

impl Fog {
    /// The fog over the plot from `back` to the front for `pressure` percent of memory pressure
    /// at `time`, whose wisps measure at most 12% of the frame's width on screen.
    fn new(frame: &Frame, camera: &Camera, back: f32, pressure: f32, time: f32) -> Fog {
        let corners = [
            [FENCE_X, FENCE_Y, 0.0],
            [FENCE_X, PLOT_RIGHT, 0.0],
            [back, FENCE_Y, 0.0],
            [back, PLOT_RIGHT, 0.0],
        ];
        let depths = corners.map(|corner| frame.project(camera, corner)[2]);
        let thick = bounded(pressure, 10.0);
        let count = (4 + (thick * (WISPS - 4) as f32).round() as usize).min(WISPS);
        let radius = (frame.width as f32 * 0.12 / (camera.zoom * SCALE)).min(6.0);
        let wisps = (0..count)
            .map(|wisp| {
                let k = wisp as f32;
                let drift = (time * 0.04 + k * 1.7).sin() * 3.0;
                let center = [
                    back + (FENCE_X - back) * (k * 0.381_966 + 0.1).fract(),
                    PLOT_RIGHT + (FENCE_Y - PLOT_RIGHT) * (k * 0.618_034 + 0.3).fract() + drift,
                ];
                (center, radius, 0.12 + 0.2 * thick)
            })
            .collect();
        Fog {
            near: depths.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            far: depths.iter().copied().fold(f32::INFINITY, f32::min),
            density: FOG_BASE + FOG_PRESSURE * thick,
            wisps,
        }
    }

    fn at(&self, frame: &Frame, camera: &Camera, point: Point) -> f32 {
        let depth = frame.project(camera, point)[2];
        let distance = ((self.near - depth) / (self.near - self.far).max(1e-3)).clamp(0.0, 1.0);
        let wisps: f32 = self
            .wisps
            .iter()
            .map(|&(center, radius, strength)| {
                let (dx, dy) = (point[0] - center[0], point[1] - center[1]);
                strength * (-(dx * dx + dy * dy) / (radius * radius)).exp()
            })
            .sum();
        (distance * self.density).min(FOG_MOST) + wisps.min(0.3)
    }
}

impl Necropolis {
    /// Draws the ground, paths, fence and oaks: everything that carries no data.
    fn draw_grounds(&self, stage: &mut Stage, fog: &Fog, pits: &[f32]) {
        let (frame, camera) = (&mut *stage.frame, stage.camera);
        let back = self.back_x();
        let (front, left, right) = (FENCE_X, FENCE_Y, PLOT_RIGHT);
        let tile = 2.0;
        let flat = |frame: &mut Frame, x0: f32, x1: f32, y0: f32, y1: f32, z: f32, color: Color| {
            let middle = [(x0 + x1) / 2.0, (y0 + y1) / 2.0, z];
            let color = mix(color, FOG, fog.at(frame, camera, middle));
            frame.quad(
                camera,
                [[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]],
                color,
                NONE,
            );
        };
        // Grass in tiles, so the fog grades across it, leaving the pit strip to its own pieces.
        let mut x = back;
        while x < front {
            let x1 = (x + tile).min(front);
            let mut y = right;
            while y < left {
                let y1 = (y + tile).min(left);
                let in_strip = x1 > PIT_X - 0.9 && x < PIT_X + 0.9;
                if !in_strip {
                    flat(frame, x, x1, y, y1, 0.0, GRASS);
                }
                y = y1;
            }
            x = x1;
        }
        // The strip of pits: grass around holes.
        let (strip_back, strip_front) = (PIT_X - 0.9, PIT_X + 0.9);
        let mut cut = right;
        for &center in pits.iter().rev() {
            let (y0, y1) = (center - 0.55, center + 0.55);
            if y0 > cut {
                flat(frame, strip_back, strip_front, cut, y0, 0.0, GRASS);
            }
            flat(frame, strip_back, PIT_X - 0.6, y0, y1, 0.0, GRASS);
            flat(frame, PIT_X + 0.6, strip_front, y0, y1, 0.0, GRASS);
            cut = y1;
        }
        flat(frame, strip_back, strip_front, cut, left, 0.0, GRASS);
        // Paths: along the front of the rows and past their left ends.
        flat(frame, 1.6, 2.6, right, left, 0.006, GRAVEL);
        flat(
            frame,
            back,
            front,
            ROW_LEFT + 0.25,
            LOT_Y - 0.1,
            0.006,
            GRAVEL,
        );
        // The fence along the front and the left side, posts with spikes and two rails.
        let post = |frame: &mut Frame, x: f32, y: f32| {
            let mut facets = Vec::with_capacity(12);
            push_box(
                &mut facets,
                [x - 0.04, y - 0.04, 0.0],
                [x + 0.04, y + 0.04, 1.0],
                IRON,
            );
            push_triangle(
                &mut facets,
                [
                    [x + 0.04, y - 0.04, 1.0],
                    [x + 0.04, y + 0.04, 1.0],
                    [x, y, 1.16],
                ],
                [1.0, 0.0, 0.3],
                IRON,
            );
            push_triangle(
                &mut facets,
                [
                    [x - 0.04, y + 0.04, 1.0],
                    [x + 0.04, y + 0.04, 1.0],
                    [x, y, 1.16],
                ],
                [0.0, 1.0, 0.3],
                IRON,
            );
            let fogged = fog.at(frame, camera, [x, y, 0.5]);
            draw_facets(frame, camera, &facets, [0.0; 3], fogged, NONE);
        };
        let gate = (1.6, 2.6);
        let mut y = right + 0.3;
        while y < left {
            post(frame, front, y);
            y += 0.62;
        }
        let mut x = back + 0.3;
        while x < front {
            if !(gate.0..gate.1).contains(&x) {
                post(frame, x, left);
            }
            x += 0.62;
        }
        let pixel = (camera.zoom * SCALE * 0.05).clamp(0.6, 3.0);
        for rail in [0.22, 0.86] {
            let mut y = right;
            while y < left {
                let y1 = (y + 2.48).min(left);
                let color = mix(IRON, FOG, fog.at(frame, camera, [front, y, rail]));
                frame.hairline(camera, [front, y, rail], [front, y1, rail], color, pixel);
                y = y1;
            }
            for (x0, x1) in [(back, gate.0), (gate.1, front)] {
                let mut x = x0;
                while x < x1 {
                    let next = (x + 2.48).min(x1);
                    let color = mix(IRON, FOG, fog.at(frame, camera, [x, left, rail]));
                    frame.hairline(camera, [x, left, rail], [next, left, rail], color, pixel);
                    x = next;
                }
            }
        }
        // Two bare oaks behind the oldest rows.
        for (index, base) in [[back - 0.5, left - 4.0], [back + 2.0, right + 3.0]]
            .into_iter()
            .enumerate()
        {
            let fogged = fog.at(frame, camera, [base[0], base[1], 3.0]);
            let mut facets = Vec::with_capacity(256);
            oak(&mut facets, base, 7.5 + index as f32, index as u64);
            draw_facets(frame, camera, &facets, [0.0; 3], fogged, NONE);
        }
    }
}

/// Draws the moon in screen space at the top right, its lit part bounded by the limb and an
/// elliptical terminator, with a glow, behind everything at `depth`.
fn draw_moon(frame: &mut Frame, wall: f64, depth: f32) {
    let age = moon_age(wall);
    let lit = moon_lit(age) as f32;
    let waxing = age < SYNODIC_DAYS / 2.0;
    let (width, height) = (frame.width as f32, frame.height as f32);
    let radius = (width * 0.028).max(4.0);
    let center = [width - radius * 3.2, height * 0.04 + radius * 2.2];
    frame.items.push(Item::Glow {
        center,
        radius: radius * 3.5,
        color: MOON,
        strength: 0.12 + 0.3 * lit,
    });
    const STEPS: usize = 24;
    let point = |angle: f32| {
        [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
            depth,
        ]
    };
    for k in 0..STEPS {
        let (a, b) = (
            k as f32 / STEPS as f32 * std::f32::consts::TAU,
            (k + 1) as f32 / STEPS as f32 * std::f32::consts::TAU,
        );
        frame.items.push(Item::Triangle(
            [[center[0], center[1], depth], point(a), point(b)],
            EARTHSHINE,
            NONE,
        ));
    }
    // The terminator's half-width at each height is k times the limb's: from the limb (new)
    // through the centre (quarter) to the far limb (full).
    let k = 1.0 - 2.0 * lit;
    let lit_depth = depth + 0.001;
    for row in 0..STEPS {
        let (v0, v1) = (
            -1.0 + 2.0 * row as f32 / STEPS as f32,
            -1.0 + 2.0 * (row + 1) as f32 / STEPS as f32,
        );
        let span = |v: f32| {
            let limb = (1.0 - v * v).max(0.0).sqrt();
            if waxing {
                (k * limb, limb)
            } else {
                (-limb, -k * limb)
            }
        };
        let (a, b) = (span(v0), span(v1));
        let at = |u: f32, v: f32| [center[0] + radius * u, center[1] + radius * v, lit_depth];
        let quad = [at(a.0, v0), at(a.1, v0), at(b.1, v1), at(b.0, v1)];
        frame
            .items
            .push(Item::Triangle([quad[0], quad[1], quad[2]], MOON, NONE));
        frame
            .items
            .push(Item::Triangle([quad[0], quad[2], quad[3]], MOON, NONE));
    }
}

/// A bare oak as thin quads in the plane facing the default view, from `seed`.
fn oak(out: &mut Vec<Facet>, base: [f32; 2], height: f32, seed: u64) {
    let mut rng = Rng::new(0x6f61_6b00 ^ seed);
    let across = [
        std::f32::consts::FRAC_1_SQRT_2,
        -std::f32::consts::FRAC_1_SQRT_2,
    ];
    let normal = [
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
    ];
    let world = |u: f32, z: f32| [base[0] + across[0] * u, base[1] + across[1] * u, z];
    let mut stack = vec![((0.0_f32, 0.0_f32), 0.0_f32, height * 0.32, 0.3_f32, 6_u32)];
    while let Some(((u, z), angle, length, width, depth)) = stack.pop() {
        if depth == 0 || length < 0.15 {
            continue;
        }
        let (du, dz) = (angle.sin() * length, angle.cos() * length);
        let tip = (u + du, z + dz);
        let (nu, nz) = (angle.cos(), -angle.sin());
        let narrow = width * 0.68;
        push_quad(
            out,
            [
                world(u + nu * width, z + nz * width),
                world(u - nu * width, z - nz * width),
                world(tip.0 - nu * narrow, tip.1 - nz * narrow),
                world(tip.0 + nu * narrow, tip.1 + nz * narrow),
            ],
            normal,
            OAK,
        );
        let branches = if depth > 4 {
            2
        } else {
            1 + (rng.next_u64() % 2) as usize
        };
        for branch in 0..branches {
            let turn = if branch == 0 { 0.4 } else { -0.4 };
            stack.push((
                tip,
                angle + rng.signed() * 0.8 + turn,
                length * (0.6 + 0.22 * rng.unit()),
                narrow,
                depth - 1,
            ));
        }
    }
}

impl Necropolis {
    /// Draws the open pits, the zombies standing in them and the reaped ones sinking.
    fn draw_zombies(
        &self,
        stage: &mut Stage,
        zombies: &[(usize, &&Process)],
        fog: &Fog,
        pits: &[f32],
    ) {
        let (camera, time) = (stage.camera, stage.time);
        // Zombies in their pits, and those that were reaped sinking into them.
        for &center in pits {
            let mut facets = Vec::with_capacity(16);
            let (x0, x1, y0, y1, floor) =
                (PIT_X - 0.6, PIT_X + 0.6, center - 0.55, center + 0.55, -0.7);
            push_quad(
                &mut facets,
                [
                    [x0, y0, floor],
                    [x1, y0, floor],
                    [x1, y1, floor],
                    [x0, y1, floor],
                ],
                [0.0, 0.0, 1.0],
                PIT,
            );
            push_quad(
                &mut facets,
                [
                    [x0, y0, floor],
                    [x0, y1, floor],
                    [x0, y1, 0.0],
                    [x0, y0, 0.0],
                ],
                [1.0, 0.0, 0.0],
                tint(SOIL, 1.2),
            );
            push_quad(
                &mut facets,
                [
                    [x0, y0, floor],
                    [x1, y0, floor],
                    [x1, y0, 0.0],
                    [x0, y0, 0.0],
                ],
                [0.0, 1.0, 0.0],
                SOIL,
            );
            let mound = [x0 - 0.35, center, 0.35];
            push_triangle(
                &mut facets,
                [[x0 - 0.1, y1, 0.0], [x0 - 0.1, y0, 0.0], mound],
                normalize([0.6, 0.0, 1.0]),
                [86, 70, 58],
            );
            let fogged = fog.at(stage.frame, camera, [PIT_X, center, 0.0]);
            draw_facets(stage.frame, camera, &facets, [0.0; 3], fogged, NONE);
        }
        for &(index, process) in zombies {
            let Some(seat) = self.pits.seat(process.id) else {
                continue;
            };
            let center = pit_y(seat);
            let sway = 0.05 * (time * 1.3 + seat as f32).sin();
            let base = [PIT_X, center, -0.55];
            let fogged = fog.at(stage.frame, camera, base);
            draw_facets(
                stage.frame,
                camera,
                &zombie(base, sway),
                [0.0; 3],
                fogged,
                index as u32,
            );
            let top = [PIT_X, center, 1.4];
            stage.positions.insert(process.id, [PIT_X, center, 0.0]);
            stage.places.push((
                [top[0], top[1], top[2] + 0.7],
                format!(
                    "{}-{}",
                    crate::render::label_name(bare_name(&process.name, process.id.pid)),
                    process.id.pid
                ),
            ));
            stage.places.push((top, "awaiting wait()".into()));
        }
        for sinking in &self.sinking {
            let age = ((time - sinking.since) / ZOMBIE_SINK).clamp(0.0, 1.0);
            let center = pit_y(sinking.pit);
            let base = [PIT_X, center, -0.55 - 2.2 * age];
            let fogged = fog.at(stage.frame, camera, base);
            draw_facets(
                stage.frame,
                camera,
                &zombie(base, 0.0),
                [0.0; 3],
                fogged,
                NONE,
            );
        }
    }

    /// Draws the graves row by row and returns where each stands, for the ghosts.
    fn draw_graves(
        &self,
        stage: &mut Stage,
        snapshot: &Snapshot,
        fog: &Fog,
    ) -> HashMap<u64, Point> {
        let (camera, time) = (stage.camera, stage.time);
        // The graves, row by row from the back, the two oldest sinking as the cemetery fills.
        let sunk = self.sinking_rows();
        let highlighted = stage.thing;
        let mut anchors: HashMap<u64, Point> = HashMap::new();
        for (index, row) in self.rows.iter().enumerate() {
            let x = self.row_x(index, time);
            let (sink, extra) = match index {
                0 => (SINK_DEPTH * sunk, sunk),
                1 => (SINK_DEPTH * sunk * 0.5, sunk * 0.5),
                _ => (0.0, 0.0),
            };
            let mut shown = 0;
            for serial in &row.graves {
                let Some(grave) = self.graves.get(serial) else {
                    continue;
                };
                let Some((line, y)) = grave.slot else {
                    continue;
                };
                if grave.entombed || grave.died > snapshot.elapsed {
                    continue;
                }
                shown += 1;
                let offset = [x - LINE_OFFSET * line as f32, y, -sink];
                let fogged = (fog.at(stage.frame, camera, offset) + extra).min(1.0);
                let pick = stage.frame.thing_pick(Thing {
                    kind: GRAVE,
                    key: grave.serial,
                });
                let lit =
                    highlighted.is_some_and(|thing| thing.kind == GRAVE && thing.key == *serial);
                let fogged = if lit { fogged * 0.3 } else { fogged };
                draw_facets(stage.frame, camera, &grave.facets, offset, fogged, pick);
                anchors.insert(*serial, offset);
                if let Some(mass) = &grave.mass {
                    stage.places.push((
                        [offset[0] + 0.6, offset[1], offset[2] + 0.8],
                        format!(
                            "{} x {}  mass grave ({})",
                            crate::render::label_name(&mass.comm),
                            mass.count,
                            crate::render::label_name(&grave.parent_label())
                        ),
                    ));
                }
            }
            let label = civil_minute(row.minute);
            let newest = index + 1 == self.rows.len();
            let more = if row.overflow > 0 {
                format!(" +{}", row.overflow)
            } else {
                String::new()
            };
            if shown > 0 || newest {
                stage.places.push((
                    [x + 0.4, LOT_Y - 0.2, -sink],
                    format!("{label}{}{more}", if newest { " UTC" } else { "" }),
                ));
            }
        }
        anchors
    }

    /// Draws each mausoleum at the left end of its row, two rows deep.
    fn draw_mausoleums(&self, stage: &mut Stage, fog: &Fog) {
        let (camera, time) = (stage.camera, stage.time);
        // Mausoleums at the left end of their rows, two rows deep.
        for mausoleum in &self.mausoleums {
            let Some(index) = self
                .rows
                .iter()
                .position(|row| row.minute == mausoleum.minute)
            else {
                continue;
            };
            let x = self.row_x(index, time);
            let low_y = LOT_Y + 0.4 + mausoleum.column as f32 * LOT_WIDTH;
            let origin = [x - ROW_SPACING - 0.9, low_y, 0.0];
            let pick = stage.frame.thing_pick(Thing {
                kind: MAUSOLEUM,
                key: mausoleum.serial,
            });
            let fogged = fog.at(stage.frame, camera, origin);
            draw_facets(
                stage.frame,
                camera,
                &mausoleum_facets(),
                origin,
                fogged,
                pick,
            );
            stage.places.push((
                [origin[0] + 2.0, origin[1] + 1.8, 4.4],
                format!(
                    "{} (unit stopped, {} graves)",
                    crate::render::label_name(unit_name(&mausoleum.unit)),
                    mausoleum.graves
                ),
            ));
        }
    }

    /// Draws the corner field of unmarked markers.
    fn draw_markers(&self, stage: &mut Stage, fog: &Fog) {
        let camera = stage.camera;
        // The corner field of processes that lived and died between samples.
        if !self.markers.is_empty() {
            let shown = self.markers.len().min(MARKERS_DRAWN);
            for (index, marker) in self.markers.iter().rev().take(shown).enumerate() {
                let (column, line) = (index % 6, index / 6);
                let x = 1.0 - line as f32 * 0.6;
                let y = -ROW_LEFT - 1.2 - column as f32 * 0.6;
                let color = marker.kind.map_or([96, 98, 108], stone_color);
                let mut facets = Vec::with_capacity(10);
                push_box(
                    &mut facets,
                    [x - 0.1, y - 0.16, 0.0],
                    [x + 0.1, y + 0.16, 0.28],
                    color,
                );
                let fogged = fog.at(stage.frame, camera, [x, y, 0.0]);
                draw_facets(stage.frame, camera, &facets, [0.0; 3], fogged, NONE);
            }
            stage.places.push((
                [1.6, -ROW_LEFT - 2.7, 0.6],
                format!("{} died between samples", self.markers.len()),
            ));
        }
    }

    /// Draws the ghosts over the graves at `anchors`.
    fn draw_ghosts(&self, stage: &mut Stage, anchors: &HashMap<u64, Point>) {
        let (camera, time) = (stage.camera, stage.time);
        // Ghosts: stacked glows rising over a grave whose pid was taken, then fading.
        let pixels = (camera.zoom * SCALE).max(1.0);
        for ghost in &self.ghosts {
            let Some(&anchor) = anchors.get(&ghost.grave) else {
                continue;
            };
            let age = time - ghost.raised;
            if !(0.0..GHOST_RISE + GHOST_FADE).contains(&age) {
                continue;
            }
            let rise = (age / GHOST_RISE).min(1.0);
            let fade = 1.0 - ((age - GHOST_RISE) / GHOST_FADE).clamp(0.0, 1.0);
            let lift = 0.8 + 1.6 * smoothstep(rise);
            for (step, radius) in [
                (0.0, 0.55),
                (0.45, 0.5),
                (0.9, 0.42),
                (1.3, 0.36),
                (1.65, 0.3),
            ] {
                stage.frame.glow(
                    camera,
                    [anchor[0], anchor[1], anchor[2] + lift + step],
                    radius * pixels,
                    GHOST,
                    0.55 * fade * smoothstep(age / 0.6),
                );
            }
            let top = [anchor[0], anchor[1], anchor[2] + lift + 2.4];
            stage.places.push((
                [top[0], top[1], top[2] + 0.5],
                format!("pid {} reused", ghost.pid),
            ));
            stage.places.push((
                top,
                format!("{} rests below", crate::render::label_name(&ghost.resting)),
            ));
        }
    }
}

/// Where the pit of seat `seat` is along the strip in front of the newest row.
fn pit_y(seat: usize) -> f32 {
    ROW_LEFT - 0.8 - seat as f32 * 2.2
}

/// Draws the stars and the moon at `depth`, behind everything.
fn draw_sky(frame: &mut Frame, wall: f64, depth: f32) {
    let (width, height) = (frame.width, frame.height);
    for index in 0..160_u32 {
        let hash = crate::simulation::hash::knuth(index.wrapping_mul(2_654_435_761) ^ 0x6e63);
        let x = (hash % width) as f32;
        let y = (hash.rotate_left(11) % (height * 2 / 5).max(1)) as f32;
        let bright = 40 + (hash >> 24) as u8 % 60;
        frame
            .items
            .push(Item::Star([x, y], [bright, bright, bright + 20]));
    }

    draw_moon(frame, wall, depth);
}

impl ViewModule for Necropolis {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        if self.last.is_none_or(|last| snapshot.elapsed >= last)
            && processes
                .iter()
                .any(|p| p.state == 'Z' && self.pits.seat(p.id).is_none())
        {
            self.seat_zombies(snapshot);
        }
        let camera = stage.camera;
        let back = self.back_x();
        let corners = [
            [FENCE_X, FENCE_Y, 0.0],
            [FENCE_X, PLOT_RIGHT, 0.0],
            [back, FENCE_Y, 0.0],
            [back, PLOT_RIGHT, 0.0],
        ];
        let fog = Fog::new(stage.frame, camera, back, snapshot.pressure[1], stage.time);
        stage.bounds.extend(corners);
        stage.bounds.extend(corners.map(|[x, y, _]| [x, y, 2.5]));
        draw_sky(stage.frame, snapshot.wall, fog.far - 1.0);
        let zombies: Vec<(usize, &&Process)> = processes
            .iter()
            .enumerate()
            .filter(|(_, p)| p.state == 'Z')
            .collect();
        let mut pits: Vec<f32> = zombies
            .iter()
            .filter_map(|(_, p)| self.pits.seat(p.id))
            .chain(self.sinking.iter().map(|sinking| sinking.pit))
            .map(pit_y)
            .collect();
        pits.sort_by(f32::total_cmp);
        pits.dedup();
        self.draw_grounds(stage, &fog, &pits);
        self.draw_zombies(stage, &zombies, &fog, &pits);
        let anchors = self.draw_graves(stage, snapshot, &fog);
        self.draw_mausoleums(stage, &fog);
        self.draw_markers(stage, &fog);
        self.draw_ghosts(stage, &anchors);
        processes.len()
    }

    fn record(&mut self, snapshot: &Snapshot, time: f32) {
        if self.last.is_some_and(|last| snapshot.elapsed <= last) {
            return;
        }
        self.take_in(snapshot, time);
        self.last = Some(snapshot.elapsed);
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let mut sources: Vec<&str> = LifeSource::ALL
            .iter()
            .filter(|source| snapshot.sources.contains(source.source()))
            .map(|source| source.name())
            .collect();
        if sources.is_empty() {
            sources = self.delivered.iter().map(|source| source.name()).collect();
        }
        let sources = if sources.is_empty() {
            "none yet".to_owned()
        } else {
            sources.join(" + ")
        };
        let wall = if snapshot.wall > 0.0 {
            snapshot.wall
        } else {
            self.wall
        };
        let age = moon_age(wall);
        let mausoleums = self.mausoleums.len();
        let notes: String = [
            missing_note(
                snapshot,
                Source::ExitRecords,
                "causes from the exit status, sizes as observed",
            ),
            missing_note(snapshot, Source::ForkEvents, "no unmarked markers"),
            missing_note(snapshot, Source::MemoryPressure, "fog at its base density"),
        ]
        .into_iter()
        .flatten()
        .collect();
        format!(
            " row = minute of death, newest in front | height = lifetime CPU (log) | width = peak RSS | tint = kind | pink = zombie | fog = memory pressure {:.0}% | deaths: {sources} | moon: {} {:.0}% lit | {} graves | {mausoleums} mausoleum{} | {} deaths/min{notes}",
            snapshot.pressure[1],
            moon_phase(age),
            moon_lit(age) * 100.0,
            self.standing(),
            if mausoleums == 1 { "" } else { "s" },
            self.per_minute(),
        )
    }

    fn card(&self, thing: Thing, _snapshot: &Snapshot) -> Option<Card> {
        match thing.kind {
            GRAVE => {
                let grave = self.graves.get(&thing.key)?;
                if grave.entombed {
                    return None;
                }
                let (line, y) = grave.slot?;
                let index = self
                    .rows
                    .iter()
                    .position(|row| row.graves.contains(&thing.key))?;
                let x = self.row_x(index, f32::MAX);
                Some(Card {
                    title: grave.title(),
                    lines: grave.epitaph(),
                    at: [x - LINE_OFFSET * line as f32, y, 1.0],
                })
            }
            MAUSOLEUM => {
                let mausoleum = self.mausoleums.iter().find(|m| m.serial == thing.key)?;
                let index = self
                    .rows
                    .iter()
                    .position(|row| row.minute == mausoleum.minute)?;
                let x = self.row_x(index, f32::MAX);
                Some(Card {
                    title: unit_name(&mausoleum.unit).to_owned(),
                    lines: vec![
                        format!("unit stopped, {} graves", mausoleum.graves),
                        format!("last death {} UTC", civil_second(mausoleum.wall)),
                    ],
                    at: [x - ROW_SPACING, LOT_Y + 2.0, 3.0],
                })
            }
            _ => None,
        }
    }
}

/// The mausoleum's geometry, in its own coordinates.
fn mausoleum_facets() -> Vec<Facet> {
    mausoleum(ROW_SPACING + 1.6, LOT_WIDTH - 0.8)
}

/// The unit a cgroup path names: its last part.
fn unit_name(cgroup: &str) -> &str {
    cgroup.rsplit('/').next().unwrap_or(cgroup)
}

/// `HH:MM` of a minute since the Unix epoch, in UTC.
fn civil_minute(minute: i64) -> String {
    let of_day = minute.rem_euclid(1440);
    format!("{:02}:{:02}", of_day / 60, of_day % 60)
}

/// `HH:MM:SS` of a wall-clock time, in UTC.
fn civil_second(wall: f64) -> String {
    let second = (wall.floor() as i64).rem_euclid(86400);
    format!(
        "{:02}:{:02}:{:02}",
        second / 3600,
        second % 3600 / 60,
        second % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DEMO_EPOCH, Unit, testing::process};
    use crate::platform::LifeEvent;

    /// A sample at `elapsed` seconds (on the demo's wall clock) of `processes`, with `life`.
    fn sample(elapsed: f64, processes: Vec<Process>, life: Vec<LifeEvent>) -> Snapshot {
        Snapshot {
            processes,
            elapsed,
            wall: DEMO_EPOCH + elapsed,
            life,
            ..Snapshot::default()
        }
    }

    fn exit(at: f64, id: Identity, cause: Cause, record: Option<ExitRecord>) -> LifeEvent {
        LifeEvent {
            at,
            kind: LifeKind::Exit { id, cause, record },
            source: LifeSource::Connector,
            estimated: false,
        }
    }

    /// A process called `name` in a scope, which never gets a mausoleum.
    fn named(pid: u32, parent: u32, name: &str) -> Process {
        Process {
            name: name.into(),
            cgroup: "/user.slice/session.scope".into(),
            ..process(pid, parent)
        }
    }

    /// Records `snapshots` in order, each at its own elapsed time, then one more empty sample
    /// `BURIAL_DELAY` later so every death in them is buried.
    fn buried(snapshots: Vec<Snapshot>) -> Necropolis {
        let mut cemetery = Necropolis::default();
        let mut last = 0.0;
        let mut keep = Vec::new();
        for snapshot in snapshots {
            last = snapshot.elapsed;
            keep = snapshot.processes.clone();
            cemetery.record(&snapshot, snapshot.elapsed as f32);
        }
        for step in 1..=2 {
            let at = last + step as f64 * BURIAL_DELAY;
            cemetery.record(&sample(at, keep.clone(), Vec::new()), at as f32);
        }
        cemetery
    }

    fn graves_named(cemetery: &Necropolis) -> Vec<(String, i64, (usize, f32))> {
        let mut out = Vec::new();
        for row in &cemetery.rows {
            for serial in &row.graves {
                let grave = &cemetery.graves[serial];
                out.push((grave.title(), row.minute, grave.slot.unwrap_or((9, 0.0))));
            }
        }
        out
    }

    #[test]
    fn exit_codes_and_flags_map_to_the_right_cause_of_death() {
        let afork = ExitRecord {
            forked_never_execd: true,
            ..ExitRecord::default()
        };
        let cases = [
            (Cause::of_status(0), None, "exited peacefully"),
            (Cause::of_status(3 << 8), None, "exited 3"),
            (Cause::of_status(15), None, "killed by SIGTERM"),
            (Cause::of_status(9), None, "killed by SIGKILL"),
            (Cause::of_status(11 | 0x80), None, "dumped core"),
            (Cause::of_status(6 | 0x80), None, "dumped core"),
            (Cause::of_status(40), None, "killed by signal 40"),
            (Cause::of_status(0), Some(&afork), "forked, never exec'd"),
            (Cause::of_status(1 << 8), Some(&afork), "exited 1"),
            (Cause::Unknown, None, "died between samples"),
        ];
        for (cause, record, text) in cases {
            assert_eq!(Death::of(cause, record).text(), text, "{cause:?}");
        }
        assert_eq!(Death::Oom.text(), "taken by the OOM killer");
    }

    #[test]
    fn a_sigkill_during_an_oom_kill_is_taken_by_the_oom_killer() {
        let units = |kills: u64| {
            HashMap::from([(
                "/a.scope".to_string(),
                Unit {
                    oom_kills: Some(kills),
                    ..Unit::default()
                },
            )])
        };
        let in_scope = |pid: u32, memory: u64| Process {
            cgroup: "/a.scope".into(),
            memory,
            ..process(pid, 1)
        };
        let small = in_scope(10, 1 << 20);
        let large = in_scope(11, 1 << 30);
        let other = in_scope(12, 1 << 28);
        let kill = Cause::Signal {
            number: 9,
            core: false,
        };
        let mut first = sample(
            1.0,
            vec![small.clone(), large.clone(), other.clone()],
            vec![],
        );
        first.units = units(3);
        // Two killed together; one OOM kill rises in the next sample, and claims the larger.
        let mut second = sample(
            2.0,
            vec![other.clone()],
            vec![
                exit(1.5, small.id, kill, None),
                exit(1.6, large.id, kill, None),
            ],
        );
        second.units = units(3);
        let mut third = sample(3.0, vec![other.clone()], vec![]);
        third.units = units(4);
        // A later SIGKILL with no OOM kill rising is just a kill.
        let mut fourth = sample(6.0, vec![], vec![exit(5.5, other.id, kill, None)]);
        fourth.units = units(4);
        let cemetery = buried(vec![first, second, third, fourth]);
        let cause = |id: Identity| {
            cemetery
                .graves
                .values()
                .find(|grave| grave.id == id)
                .map(|grave| grave.death.text())
        };
        assert_eq!(cause(large.id).as_deref(), Some("taken by the OOM killer"));
        assert_eq!(cause(small.id).as_deref(), Some("killed by SIGKILL"));
        assert_eq!(cause(other.id).as_deref(), Some("killed by SIGKILL"));
    }

    #[test]
    fn graves_take_the_row_of_their_minute_with_the_newest_in_front() {
        // Demo time 0 is 04:12:00: deaths at 50 s and 70 s fall in 04:12 and 04:13.
        let (a, b, c) = (
            named(10, 1, "early"),
            named(11, 1, "late"),
            named(12, 1, "later"),
        );
        let cemetery = buried(vec![
            sample(49.0, vec![a.clone(), b.clone(), c.clone()], vec![]),
            sample(
                51.0,
                vec![b.clone(), c.clone()],
                vec![exit(50.0, a.id, Cause::Code(0), None)],
            ),
            sample(
                71.0,
                vec![c.clone()],
                vec![exit(70.0, b.id, Cause::Code(0), None)],
            ),
            sample(72.0, vec![], vec![exit(71.5, c.id, Cause::Code(0), None)]),
        ]);
        let graves = graves_named(&cemetery);
        assert_eq!(
            graves
                .iter()
                .map(|g| (g.0.as_str(), g.1))
                .collect::<Vec<_>>(),
            [
                ("early-10", 29_860_092),
                ("late-11", 29_860_093),
                ("later-12", 29_860_093)
            ]
        );
        assert_eq!(cemetery.rows.len(), 2);
        assert_eq!(civil_minute(cemetery.rows[1].minute), "04:13");
        let time = 1000.0;
        assert_eq!(
            cemetery.row_x(1, time),
            0.0,
            "the newest row stands in front"
        );
        assert_eq!(cemetery.row_x(0, time), -ROW_SPACING);
        assert!(
            graves[1].2.1 > graves[2].2.1,
            "a row fills from its left end, which is +y"
        );
    }

    #[test]
    fn children_are_buried_beside_a_parent_dying_in_the_same_minute() {
        let parent = named(20, 1, "make");
        let stranger = named(30, 1, "cron");
        let child = named(21, 20, "cc1");
        let cemetery = buried(vec![
            sample(
                1.0,
                vec![parent.clone(), stranger.clone(), child.clone()],
                vec![],
            ),
            sample(
                3.0,
                vec![stranger.clone(), child.clone()],
                vec![exit(2.0, parent.id, Cause::Code(0), None)],
            ),
            sample(
                6.0,
                vec![child.clone()],
                vec![exit(5.0, stranger.id, Cause::Code(0), None)],
            ),
            sample(9.0, vec![], vec![exit(8.0, child.id, Cause::Code(0), None)]),
        ]);
        let titles: Vec<String> = graves_named(&cemetery).into_iter().map(|g| g.0).collect();
        assert_eq!(titles, ["make-20", "cc1-21", "cron-30"]);
    }

    #[test]
    fn many_deaths_from_one_parent_within_a_second_share_a_mass_grave() {
        let parent = named(40, 1, "make");
        let children: Vec<Process> = (0..12).map(|k| named(100 + k, 40, "cc1")).collect();
        let mut all = vec![parent.clone()];
        all.extend(children.iter().cloned());
        let exits: Vec<LifeEvent> = children
            .iter()
            .enumerate()
            .map(|(k, child)| exit(2.0 + k as f64 * 0.05, child.id, Cause::Code(0), None))
            .collect();
        let few: Vec<Process> = (0..8).map(|k| named(200 + k, 40, "ld")).collect();
        let mut later = vec![parent.clone()];
        later.extend(few.iter().cloned());
        let few_exits: Vec<LifeEvent> = few
            .iter()
            .map(|child| exit(10.5, child.id, Cause::Code(0), None))
            .collect();
        let cemetery = buried(vec![
            sample(1.0, all, vec![]),
            sample(3.0, vec![parent.clone()], exits),
            sample(10.0, later, vec![]),
            sample(11.0, vec![parent.clone()], few_exits),
        ]);
        let titles: Vec<String> = graves_named(&cemetery).into_iter().map(|g| g.0).collect();
        assert_eq!(
            titles.len(),
            9,
            "one mass grave and eight headstones: {titles:?}"
        );
        assert_eq!(titles[0], "cc1 x 12 (make-40)");
        assert_eq!(cemetery.standing(), 9, "a mass grave counts as one grave");
    }

    #[test]
    fn a_ghost_rises_when_a_new_process_reuses_a_buried_pid() {
        let old = named(50, 1, "sshd");
        let reborn = Process {
            id: Identity {
                pid: 50,
                start: 999,
            },
            ..named(50, 1, "nginx")
        };
        let mut cemetery = buried(vec![
            sample(1.0, vec![old.clone()], vec![]),
            sample(2.0, vec![], vec![exit(1.5, old.id, Cause::Code(0), None)]),
        ]);
        assert!(cemetery.ghosts.is_empty());
        cemetery.record(&sample(10.0, vec![reborn.clone()], vec![]), 10.0);
        assert_eq!(cemetery.ghosts.len(), 1);
        assert_eq!(cemetery.ghosts[0].resting, "sshd-50");
        assert_eq!(cemetery.ghosts[0].pid, 50);
        // Rising for four seconds, fading for two, then gone.
        cemetery.record(&sample(15.0, vec![reborn.clone()], vec![]), 15.0);
        assert_eq!(cemetery.ghosts.len(), 1);
        cemetery.record(&sample(16.5, vec![reborn], vec![]), 16.5);
        assert!(cemetery.ghosts.is_empty());
    }

    #[test]
    fn a_zombie_stands_in_an_open_grave_until_reaped_then_gets_a_headstone() {
        let living = named(60, 1, "worker");
        let undead = Process {
            state: 'Z',
            ..living.clone()
        };
        let mut cemetery = Necropolis::default();
        cemetery.record(&sample(1.0, vec![living.clone()], vec![]), 1.0);
        // The exit comes while the zombie is still listed: it waits in its pit.
        cemetery.record(
            &sample(
                2.0,
                vec![undead.clone()],
                vec![exit(1.5, living.id, Cause::Code(2), None)],
            ),
            2.0,
        );
        cemetery.record(&sample(4.0, vec![undead.clone()], vec![]), 4.0);
        assert!(cemetery.graves.is_empty(), "a zombie has no headstone yet");
        assert_eq!(cemetery.pits.seat(living.id), Some(0));
        cemetery.record(&sample(5.0, vec![], vec![]), 5.0);
        assert_eq!(
            cemetery.sinking.len(),
            1,
            "a reaped zombie sinks into its pit"
        );
        cemetery.record(&sample(6.0, vec![], vec![]), 6.0);
        cemetery.record(&sample(7.0, vec![], vec![]), 7.0);
        let grave = cemetery.graves.values().next().expect("buried");
        assert_eq!(
            (grave.title(), grave.death.text()),
            ("worker-60".into(), "exited 2".into())
        );
        assert!(cemetery.sinking.is_empty());
    }

    #[test]
    fn the_cemetery_caps_at_six_hundred_graves_and_the_oldest_rows_sink_into_fog() {
        let mut cemetery = Necropolis::default();
        let mut pid = 1000;
        let mut first_minute = None;
        for minute in 0..25 {
            let processes: Vec<Process> = (0..30).map(|k| named(pid + k, 1, "job")).collect();
            let start = minute as f64 * 60.0 + 1.0;
            cemetery.record(&sample(start, processes.clone(), vec![]), start as f32);
            let exits = processes
                .iter()
                .enumerate()
                .map(|(k, p)| exit(start + 1.0 + k as f64 * 0.5, p.id, Cause::Code(0), None))
                .collect();
            cemetery.record(&sample(start + 30.0, vec![], exits), start as f32 + 30.0);
            cemetery.record(&sample(start + 32.0, vec![], vec![]), start as f32 + 32.0);
            pid += 30;
            first_minute.get_or_insert(cemetery.rows[0].minute);
            assert!(cemetery.standing() <= CAP, "{} graves", cemetery.standing());
        }
        assert!(
            cemetery.standing() > CAP - 30,
            "only whole rows are dropped: {} in {:?}",
            cemetery.standing(),
            cemetery
                .rows
                .iter()
                .map(|r| (r.graves.len(), r.overflow))
                .collect::<Vec<_>>()
        );
        assert!(cemetery.rows[0].minute > first_minute.unwrap_or_default());
        assert!(
            cemetery
                .rows
                .iter()
                .all(|row| row.graves.len() == 30 && row.overflow == 0)
        );
        let expected = cemetery.standing() as f32 / CAP as f32 * 4.0 - 3.0;
        assert!((cemetery.sinking_rows() - expected).abs() < 1e-5);
        assert!(
            cemetery.sinking_rows() > 0.8,
            "the oldest rows sink as the cap nears"
        );
        let fresh = buried(vec![sample(1.0, vec![named(5, 1, "one")], vec![])]);
        assert_eq!(fresh.sinking_rows(), 0.0);
    }

    #[test]
    fn a_service_whose_last_process_exits_gets_a_mausoleum() {
        let in_unit = |pid: u32, cgroup: &str| Process {
            cgroup: cgroup.into(),
            ..named(pid, 1, "backup")
        };
        let unit = "/system.slice/backup.service";
        let processes: Vec<Process> = (0..4).map(|k| in_unit(70 + k, unit)).collect();
        let scope: Vec<Process> = (0..4)
            .map(|k| in_unit(80 + k, "/app.slice/a.scope"))
            .collect();
        let mut all = processes.clone();
        all.extend(scope.iter().cloned());
        let exits: Vec<LifeEvent> = processes
            .iter()
            .chain(&scope)
            .enumerate()
            .map(|(k, p)| exit(2.0 + k as f64 * 0.2, p.id, Cause::Code(0), None))
            .collect();
        let survivor = in_unit(90, "/system.slice/other.service");
        let lingering = in_unit(91, "/system.slice/other.service");
        let cemetery = buried(vec![
            sample(
                1.0,
                {
                    let mut both = all.clone();
                    both.extend([survivor.clone(), lingering.clone()]);
                    both
                },
                vec![],
            ),
            sample(4.0, vec![lingering.clone()], {
                let mut with = exits;
                with.push(exit(2.5, survivor.id, Cause::Code(0), None));
                with
            }),
        ]);
        assert_eq!(
            cemetery.mausoleums.len(),
            1,
            "only the stopped .service unit"
        );
        let mausoleum = &cemetery.mausoleums[0];
        assert_eq!((mausoleum.unit.as_str(), mausoleum.graves), (unit, 4));
        let entombed = cemetery
            .graves
            .values()
            .filter(|grave| grave.entombed)
            .count();
        assert_eq!(entombed, 4);
        assert_eq!(
            cemetery.standing(),
            5,
            "the scope's graves and the other unit's stay"
        );
        let card = cemetery
            .card(
                Thing {
                    kind: MAUSOLEUM,
                    key: mausoleum.serial,
                },
                &Snapshot::default(),
            )
            .expect("a card");
        assert_eq!(card.title, "backup.service");
        assert_eq!(card.lines[0], "unit stopped, 4 graves");
    }

    #[test]
    fn the_moon_phase_matches_known_new_and_full_moons() {
        // New moon 2024-01-11 11:57 UTC, full moon 2024-01-25 17:54 UTC, first quarter
        // 2024-01-18 03:53 UTC, new moon 2026-10-10 15:50 UTC.
        let new = moon_age(1_704_974_220.0);
        assert!(!(0.5..SYNODIC_DAYS - 0.5).contains(&new), "{new}");
        assert!(moon_lit(new) < 0.01);
        let full = moon_age(1_706_205_240.0);
        assert!((full - SYNODIC_DAYS / 2.0).abs() < 0.5, "{full}");
        assert!(moon_lit(full) > 0.99);
        assert_eq!(moon_phase(full), "full moon");
        let quarter = moon_age(1_705_549_980.0);
        assert!(
            (moon_lit(quarter) - 0.5).abs() < 0.12,
            "{}",
            moon_lit(quarter)
        );
        assert_eq!(moon_phase(quarter + 1.0), "waxing gibbous");
        let demo = moon_age(DEMO_EPOCH);
        assert!(
            (28.0..28.6).contains(&demo),
            "the demo's night is the eve of a new moon: {demo}"
        );
        assert_eq!(moon_phase(demo), "waning crescent");
    }

    #[test]
    fn graves_keep_their_places_when_new_deaths_arrive() {
        // Closed rows are frozen; in the open row only graves after an inserted one move.
        let first: Vec<Process> = (0..10).map(|k| named(300 + k, 1, "early")).collect();
        let second: Vec<Process> = (0..10).map(|k| named(400 + k, 1, "late")).collect();
        let mut all = first.clone();
        all.extend(second.iter().cloned());
        let exits = |processes: &[Process], at: f64| -> Vec<LifeEvent> {
            processes
                .iter()
                .enumerate()
                .map(|(k, p)| exit(at + k as f64 * 0.3, p.id, Cause::Code(0), None))
                .collect()
        };
        let mut cemetery = buried(vec![
            sample(1.0, all.clone(), vec![]),
            sample(10.0, second.clone(), exits(&first, 2.0)),
        ]);
        let before = graves_named(&cemetery);
        for (k, minute) in [(0, 70.0), (1, 130.0)] {
            let late = &second[k * 5..k * 5 + 5];
            let remaining: Vec<Process> = second[(k + 1) * 5..].to_vec();
            cemetery.record(
                &sample(minute, remaining.clone(), exits(late, minute - 2.0)),
                minute as f32,
            );
            cemetery.record(
                &sample(minute + 2.0, remaining.clone(), vec![]),
                minute as f32 + 2.0,
            );
            cemetery.record(
                &sample(minute + 4.0, remaining, vec![]),
                minute as f32 + 4.0,
            );
        }
        let after = graves_named(&cemetery);
        assert!(after.len() > before.len());
        assert_eq!(
            &after[..before.len()],
            &before[..],
            "earlier graves stay where they were"
        );
    }

    #[test]
    fn epitaphs_are_sanitized() {
        let hostile = named(77, 1, "evil\u{1b}]0;pwned\u{7}name");
        let cemetery = buried(vec![
            sample(1.0, vec![hostile.clone()], vec![]),
            sample(
                2.0,
                vec![],
                vec![exit(1.5, hostile.id, Cause::Code(0), None)],
            ),
        ]);
        let serial = *cemetery.graves.keys().next().expect("a grave");
        let card = cemetery
            .card(
                Thing {
                    kind: GRAVE,
                    key: serial,
                },
                &Snapshot::default(),
            )
            .expect("a card");
        let popup = crate::terminal::Popup {
            column: 0,
            row: 0,
            width: 60,
            lines: std::iter::once(card.title.clone())
                .chain(card.lines.clone())
                .map(|line| crate::terminal::SafeText::new(&line))
                .collect(),
        };
        let mut output = Vec::new();
        crate::terminal::draw_popup(&mut output, &popup).expect("drawn");
        let output = String::from_utf8(output).expect("text");
        assert!(output.contains("evil?]0;pwned?name-77"), "{output:?}");
        assert!(!output.contains("\u{1b}]0;") && !output.contains('\u{7}'));
    }

    #[test]
    fn the_demo_settles_ten_minutes_of_every_kind_of_death_by_twelve_seconds() {
        let row = crate::views::view("necropolis").info();
        let mut cemetery = Necropolis::default();
        let mut ghosts = 0;
        for back in (1..=600).rev() {
            let at = 12.0 - f64::from(back);
            cemetery.record(&crate::demo_snapshot(row, at - 1.0, at, 192), at as f32);
            ghosts += cemetery
                .ghosts
                .iter()
                .filter(|ghost| ghost.raised == at as f32)
                .count();
        }
        let causes: HashSet<String> = cemetery
            .graves
            .values()
            .map(|grave| match grave.death {
                Death::Signal(_) => "signal".into(),
                Death::Exited(_) => "exited".into(),
                other => other.text(),
            })
            .collect();
        println!(
            "overflow {} rows {} standing {} mausoleums {} markers {} ghosts {ghosts} per minute {} causes {causes:?}",
            cemetery.rows.iter().map(|row| row.overflow).sum::<usize>(),
            cemetery.rows.len(),
            cemetery.standing(),
            cemetery.mausoleums.len(),
            cemetery.markers.len(),
            cemetery.per_minute()
        );
        let snapshot = crate::demo_snapshot(row, 11.0, 12.0, 192);
        let legend = cemetery.legend(&snapshot);
        assert!(
            legend.starts_with(" row = minute of death, newest in front | height = lifetime CPU (log) | width = peak RSS | tint = kind | pink = zombie | fog = memory pressure 18% | deaths: taskstats + proc connector + sample diff | moon: waning crescent 2% lit | "),
            "{legend}"
        );
        assert!(legend.ends_with(" mausoleum | 35 deaths/min"), "{legend}");
        assert!(cemetery.rows.len() >= 8);
        assert!(cemetery.standing() >= 300);
        assert!(!cemetery.mausoleums.is_empty());
        assert!(cemetery.graves.values().any(|grave| grave.mass.is_some()));
        assert!(!cemetery.markers.is_empty());
        assert!(ghosts >= 5, "a pid is reused every 90 s");
        assert_eq!(causes.len(), 7, "{causes:?}");
        assert!((25..=60).contains(&cemetery.per_minute()));
    }

    #[test]
    fn lifetimes_are_formatted_as_human_durations() {
        assert_eq!(lifetime_text(0.25), "250 ms");
        assert_eq!(lifetime_text(2.3), "2.3 s");
        assert_eq!(lifetime_text(1310.0), "21 m 50 s");
        assert_eq!(lifetime_text(3.0 * 3600.0 + 12.0 * 60.0 + 5.0), "3 h 12 m");
    }

    #[test]
    fn stones_grow_with_cpu_and_memory_up_to_their_caps() {
        assert_eq!(stone_height(1.0), stone_height(3.0));
        assert!((stone_height(1000.0) - 1.63).abs() < 1e-4);
        assert_eq!(stone_height(1e12), 3.2);
        assert!((stone_width(64.0) - 1.52).abs() < 1e-4);
        assert_eq!(stone_width(0.0), 0.62);
        assert_eq!(stone_width(1e9), 2.4);
        for shape in [
            Shape::Round,
            Shape::Slab,
            Shape::Gothic,
            Shape::Cross,
            Shape::Obelisk,
        ] {
            assert!(stone(shape, 2.4, 3.2, [128; 3]).len() <= 48, "{shape:?}");
        }
    }

    #[test]
    fn necropolis_renders_the_same_frame_twice_from_the_same_inputs() {
        use crate::render::{RenderInput, Scene};
        let view = crate::views::view("necropolis");
        let draw = || {
            let mut scene = Scene::new();
            let row = view.info();
            let mut snapshot = Snapshot::default();
            for second in 0..240 {
                let at = f64::from(second);
                snapshot = crate::demo_snapshot(row, at - 1.0, at, 64);
                scene.record(&snapshot, at as f32);
            }
            let camera = Camera::default();
            let mut fitted = camera.clone();
            scene.render(&RenderInput::small(&snapshot, view, &camera, 240.0));
            scene.fit(&mut fitted, 320, 180);
            let mut frame = scene.render(&RenderInput::small(&snapshot, view, &fitted, 240.0));
            frame.rasterize();
            (frame.pixels, frame.picks, scene.legend(view, &snapshot))
        };
        let (pixels, picks, legend) = draw();
        assert_eq!(draw(), (pixels.clone(), picks, legend.clone()));
        assert!(legend.contains(" graves | "), "{legend}");
        assert!(pixels.chunks(3).any(|p| p != pixels[..3].as_ref()));
    }
}
