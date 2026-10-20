//! The Plumbing module view: the pipes, sockets and loopback connections between processes as the
//! plumbing of a boiler room. A process with at least one drawn link is a tank (size from memory,
//! band from kind) with a pressure gauge that reads its attributed outflow; a pipe or FIFO is a
//! copper run with a glass window, a Unix socket a brass tube overhead holding the capsules that
//! wait unread, loopback TCP a green hose; and a pipe nobody reads any more bursts.
//!
//! Every flow is either measured or marked as not measured. What a process wrote and where it
//! can have gone is `flow`; the rest of this file keeps what the view remembers between samples
//! (who is a tank, which pipes lost their reader and when, how far the water has travelled) and
//! `paint` draws a frame from it. The layout is `floor` (bays by cgroup, seats for tanks) and
//! `route` (copper runs found by A* round the tanks), both stable.

mod card;
mod draw;
mod floor;
mod flow;
mod paint;
mod path;
mod room;
mod route;

use std::collections::{BTreeMap, HashSet};

use crate::model::{Identity, Process, Snapshot};
use crate::palette::kind_color;
use crate::platform::{PipeId, Source};
use crate::render::{Card, Color, Stage, Thing};
use crate::status::{missing_note, sources_note};
use crate::views::ViewModule;
use card::Subject;
use floor::Floor;
use room::Room;
use route::Router;

/// How long a tank stays after its last link closes, in seconds.
pub const LINGER: f32 = 10.0;
/// The most tanks drawn, ranked by link count and then traffic.
pub const MOST_TANKS: usize = 256;

/// A tank as a frame stands it: a process with a place.
#[derive(Clone, Debug)]
struct Tank {
    id: Identity,
    /// Its index in the frame's process slice, which clicking it picks.
    index: usize,
    at: [f32; 2],
    radius: f32,
    height: f32,
    color: Color,
    /// The raw name and memory, for its label.
    name: String,
    memory: u64,
}

/// A pipe whose reader has gone, as the view saw it go.
#[derive(Clone, Copy, Debug)]
struct Break {
    /// When the view saw it, in seconds of the scene's clock.
    since: f32,
    /// The process that held the read end when it was last seen.
    reader: Identity,
}

/// What a frame drew that a card can be built for, and the figures the legend counts.
#[derive(Default)]
struct Drawn {
    things: BTreeMap<u64, Subject>,
    tanks: usize,
    pipes: usize,
    tubes: usize,
    hoses: usize,
    bursts: usize,
    manifolds: usize,
    blocked: usize,
    dry: usize,
}

/// The Plumbing view.
#[derive(Default)]
pub struct Plumbing {
    floor: Floor,
    router: Router,
    /// `Snapshot::elapsed` of the sample last taken in.
    sample: Option<(f64, u64)>,
    /// The pipework of that sample.
    room: Room,
    /// When each process last had a link, for the tank that stays after its links close.
    linked: BTreeMap<Identity, f32>,
    /// The reader of every pipe last seen with one, by writer: a pipe that loses it has burst.
    readers: BTreeMap<(Identity, PipeId), Identity>,
    /// The pipes seen to lose their last reader while a writer remained.
    breaks: BTreeMap<(Identity, PipeId), Break>,
    /// How far the dashes of each run have travelled, forward and back, in world units.
    phases: BTreeMap<(Identity, Identity), (f32, f32)>,
    /// How far the beads of each hose have travelled, from its lower end and from its higher.
    beads: BTreeMap<(Identity, Identity), (f32, f32)>,
    drawn: Drawn,
}

/// A stable key for a thing of the room, apart from every other: a pair of processes and what
/// kind of thing joins them.
fn thing_key(a: Identity, b: Identity, tag: u64) -> u64 {
    crate::simulation::hash_identity(a, crate::simulation::hash_identity(b, tag))
}

impl Plumbing {
    /// Takes in the sample `snapshot` if it is a new one: the pipework it holds, and which pipes
    /// lost their last reader since the last sample. A pipe that has no reader and never had one
    /// the view saw is not burst, only unread; and a reader that is merely not visible
    /// (another user's process, a pass that has not reached it) is told from one that closed its
    /// end by having been seen to hold it.
    fn take_in(&mut self, time: f32, snapshot: &Snapshot, shown: &HashSet<Identity>) {
        // The shown set is part of the key: clearing a focus while paused shows other processes of
        // the same sample.
        let key = (
            snapshot.elapsed,
            crate::simulation::identities_key(shown.iter().copied()),
        );
        if self.sample == Some(key) {
            return;
        }
        self.sample = Some(key);
        self.room = Room::read(snapshot, shown);
        let severed: HashSet<(Identity, PipeId)> = self.room.severed.iter().copied().collect();
        for &key in &severed {
            if let Some(&reader) = self.readers.get(&key) {
                self.breaks.entry(key).or_insert(Break {
                    since: time,
                    reader,
                });
            }
        }
        self.breaks.retain(|key, _| severed.contains(key));
        for run in &self.room.pipes {
            for &id in &run.pipes {
                self.readers.insert((run.writer, id), run.reader);
                self.breaks.remove(&(run.writer, id));
            }
        }
        self.readers
            .retain(|(writer, _), reader| shown.contains(writer) && shown.contains(reader));
    }

    /// The processes that are tanks this frame, placed: those with a link, the selected one and
    /// those whose last link closed less than `LINGER` seconds ago, at most `MOST_TANKS`.
    fn stand(&mut self, stage: &Stage, processes: &[&Process]) -> Vec<Tank> {
        let index: BTreeMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(at, process)| (process.id, at))
            .collect();
        let now = stage.time;
        for &id in self.room.links.keys() {
            self.linked.insert(id, now);
        }
        self.linked
            .retain(|id, at| index.contains_key(id) && now - *at <= LINGER);
        let mut ranked: Vec<(u32, f32, Identity)> = self
            .linked
            .keys()
            .map(|id| {
                let links = self.room.links.get(id).copied().unwrap_or_default();
                // A flow that was not measured ranks below every measured one, zero included.
                let traffic = self
                    .room
                    .outflows
                    .get(id)
                    .and_then(|outflow| outflow.rate())
                    .unwrap_or(f32::NEG_INFINITY);
                (links, traffic, *id)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)).then(a.2.cmp(&b.2)));
        ranked.truncate(MOST_TANKS);
        let mut chosen: Vec<Identity> = ranked.into_iter().map(|(_, _, id)| id).collect();
        if let Some(selected) = stage.selected
            && index.contains_key(&selected)
            && !chosen.contains(&selected)
        {
            if chosen.len() == MOST_TANKS {
                chosen.pop();
            }
            chosen.push(selected);
        }
        chosen.sort_unstable();
        let members: Vec<(Identity, String)> = chosen
            .iter()
            .map(|id| (*id, bay_key(processes[index[id]])))
            .collect();
        self.floor.arrange(&members);
        chosen
            .iter()
            .filter_map(|id| {
                let process = processes[index[id]];
                let at = self.floor.position(*id)?;
                let radius = draw::tank::radius_of(process.memory);
                Some(Tank {
                    id: *id,
                    index: index[id],
                    at,
                    radius,
                    height: draw::tank::height_of(radius),
                    color: kind_color(process),
                    name: process.name.clone(),
                    memory: process.memory,
                })
            })
            .collect()
    }
}

/// The key of the bay a process stands in: its cgroup, or its group where it has none.
fn bay_key(process: &Process) -> String {
    if process.cgroup.is_empty() {
        process.group.clone()
    } else {
        process.cgroup.clone()
    }
}

/// The title of a bay: the group of any process in it, the name its cgroup has.
fn bay_title(process: &Process) -> String {
    if process.group.is_empty() {
        "no cgroup".to_owned()
    } else {
        process.group.clone()
    }
}

impl ViewModule for Plumbing {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let shown: HashSet<Identity> = processes.iter().map(|process| process.id).collect();
        self.take_in(stage.time, snapshot, &shown);
        let tanks = self.stand(stage, processes);
        self.paint(stage, processes, &tanks);
        tanks.len()
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let drawn = &self.drawn;
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        let mut text = format!(
            " {} | {}, {}, {}",
            plural(drawn.tanks, "tank", "tanks"),
            plural(drawn.pipes, "pipe run", "pipe runs"),
            plural(drawn.tubes, "unix socket", "unix sockets"),
            plural(drawn.hoses, "loopback TCP", "loopback TCP"),
        );
        if drawn.bursts > 0 {
            text += &format!(" | {}", plural(drawn.bursts, "burst", "bursts"));
        }
        if drawn.blocked + drawn.dry > 0 {
            text += &format!(" | {} blocked, {} dry", drawn.blocked, drawn.dry);
        }
        text += " | K = 1 capsule per 4 KiB";
        for (source, consequence) in [
            (Source::Pipes, "no pipes"),
            (Source::WaitChannel, "no red or dry gauges"),
            (Source::IoDetail, "water unmeasured"),
            (Source::UnattributedIo, "water and gauges unmeasured"),
            (Source::SocketTraffic, "hoses unmeasured"),
        ] {
            if let Some(note) = missing_note(snapshot, source, consequence) {
                text += &note;
            }
        }
        if let Some(note) = sources_note(snapshot) {
            text += &note;
        }
        text += " | tank = process (size = memory, band = kind) | copper = pipe or FIFO \
                | water speed = writer's outflow | brass tube = unix socket \
                | green hose = loopback TCP | gauge = outflow, red = blocked";
        text
    }

    fn card(&self, thing: Thing, snapshot: &Snapshot) -> Option<Card> {
        let subject = self.drawn.things.get(&thing.key)?;
        Some(card::card(subject, snapshot))
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests;
