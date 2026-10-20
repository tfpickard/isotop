//! What one sample tells the colony, with no drawing: which streams of ants run from which nest to
//! which pile or blanket and how fast, which files are piles and who holds them, and which tunnels
//! join the nests. Everything here is a function of the sample, so the colony cannot invent traffic.
//!
//! **Where the numbers come from.** A file's bytes are the growth of its descriptors' positions
//! (`io_detail::FileFlow::flow`), a lower bound on what the process did with it. The calls are the
//! process's own (`SyscallIo::reads` and `writes`), split over the files in proportion to those
//! bytes: a read-only file takes the share of the read calls its bytes are of all the bytes the
//! process read in the interval (`rchar`, which is what its files and `Unattributed::read` add up
//! to), a write-only file the same of the write calls, and what is left of the calls (the share of
//! the bytes no read-only or write-only file explains) is shared by the read-write files and what
//! nothing explains, in proportion to their bytes. The shares add up to the process's calls and
//! never to more.
//!
//! **What stays unknown.** A file with no earlier reading (`Flow::Unmeasured`) or a position that
//! says nothing (`Flow::Unpositioned`) sends no ants and is a pile all the same. A read-write file's
//! direction is not guessed (`Way::Either`), and nor is that of the unexplained bytes when
//! read-write files claimed some. What nothing explains travels the process's tunnels, weighted by
//! their traffic where every tunnel's is known and equally otherwise, goes to a blanket when the
//! process has no tunnel at all and holds a terminal, and is counted otherwise: never given to a
//! file.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::io_detail::{FileFlow, Flow, IoDetail, Unattributed};
use crate::model::{Identity, Measured, Process, Snapshot, SyscallIo};
use crate::platform::{AccessMode, Source};
use crate::simulation::hash;

/// The most piles drawn: the files with the most attributed bytes.
pub const MOST_PILES: usize = 256;

/// The most tunnels drawn: the busiest, then by identity.
pub const MOST_TUNNELS: usize = 512;

/// The most blankets: one for each corner of the clearing.
pub const MOST_BLANKETS: usize = 4;

/// Bytes per second under which a read-write file's unexplained remainder counts as none.
const NONE_LEFT: f32 = 0.5;

/// What one ant stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// `K` read and write calls. Where the platform counts them.
    Calls,
    /// `K` KiB moved, where it counts none (macOS): the bytes a position advanced.
    Kibibytes,
}

/// Which way the bytes of a stream went, which sets the colour of its crumbs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Way {
    Read,
    Write,
    /// Through a descriptor opened for both: not known, and never split.
    Either,
}

/// Where a stream of ants goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Goal {
    /// A pile: `file_key` of its device and inode.
    File(u64),
    /// The blanket of a process that holds a terminal.
    Blanket(Identity),
}

/// A stream of ants from a nest to a goal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stream {
    pub nest: Identity,
    pub goal: Goal,
    pub way: Way,
    /// Units per second: calls, or KiB moved (`Unit`). Ants per second is this over K.
    pub rate: f32,
    /// Bytes per call, the mean over the calls the stream stands for; None where the platform
    /// counts none, which is what an unsized crumb is.
    pub crumb: Option<f32>,
}

/// How a pair of nests is joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Medium {
    Pipe,
    Unix,
    /// A loopback TCP connection.
    Loopback,
    /// A socket link on a platform that does not tell the protocols apart.
    Socket,
}

/// A tunnel between two nests: a pipe, or sockets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tunnel {
    /// The two ends, ordered by identity, except that the end of a one-way pipe that writes is
    /// `from` and the end that reads `to`.
    pub from: Identity,
    pub to: Identity,
    pub medium: Medium,
    /// Data goes `from` to `to` only (a pipe); sockets and two-way FIFOs carry both ways.
    pub one_way: bool,
    /// Calls per second of what the files do not explain, routed through it; None when neither end
    /// has a reading of that.
    pub rate: Option<f32>,
    /// Bytes per second the connection carried, where it is measured (loopback TCP only).
    pub traffic: Option<f32>,
}

/// One holder of a file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Holder {
    pub id: Identity,
    pub access: AccessMode,
    pub flow: Flow,
}

/// An open regular file, as the pile it is drawn as: one for every file the nests hold, whoever
/// holds it. `path` is raw and reaches the terminal only through `SafeText`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pile {
    pub key: u64,
    pub path: String,
    pub size: u64,
    pub deleted: bool,
    /// Sorted by identity.
    pub holders: Vec<Holder>,
}

/// What a pile's holders measured of it, in bytes per second, as lower bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moved {
    pub read: f32,
    pub written: f32,
    pub either: f32,
}

impl Moved {
    pub fn total(self) -> f32 {
        self.read + self.written + self.either
    }
}

impl Pile {
    /// The sum of what its holders' positions advanced by, or None when no holder has a measured
    /// flow (which is not zero).
    pub fn moved(&self) -> Option<Moved> {
        let mut moved = None;
        for holder in &self.holders {
            if let Flow::Measured {
                read,
                written,
                either,
            } = holder.flow
            {
                let sum: &mut Moved = moved.get_or_insert_with(Moved::default);
                sum.read += read;
                sum.written += written;
                sum.either += either;
            }
        }
        moved
    }

    /// Bytes per second attributed to it: what ranks the piles. Unknown ranks as none.
    pub fn traffic(&self) -> f32 {
        self.moved().map_or(0.0, Moved::total)
    }

    /// How many holders opened it for reading, and for writing.
    pub fn readers(&self) -> usize {
        self.holders
            .iter()
            .filter(|holder| holder.access.reads())
            .count()
    }

    pub fn writers(&self) -> usize {
        self.holders
            .iter()
            .filter(|holder| holder.access.writes())
            .count()
    }

    /// How many holders have no earlier reading of the file, and how many have a position that
    /// says nothing of bytes.
    pub fn unmeasured(&self) -> usize {
        self.holders
            .iter()
            .filter(|holder| holder.flow == Flow::Unmeasured)
            .count()
    }

    pub fn unpositioned(&self) -> usize {
        self.holders
            .iter()
            .filter(|holder| holder.flow == Flow::Unpositioned)
            .count()
    }
}

/// A process that holds a terminal and whose unattributed I/O has nowhere else to go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub holder: Identity,
    /// How many terminal descriptors it holds.
    pub ttys: u32,
    /// Bytes per second that go to the blanket: read, write, and the calls they stand for.
    pub bytes: f32,
}

/// What the plan counted and could not draw, for the legend.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tally {
    /// Processes whose descriptors were read.
    pub measured: usize,
    /// Processes whose descriptors the platform refused (another user's).
    pub unreadable: usize,
    /// Processes not read yet, or not among the busiest.
    pub pending: usize,
    /// Read, but with no counts of read and write calls on a platform that has them: no ants.
    pub uncounted: usize,
    /// Readings a cap cut short.
    pub partial: usize,
    /// Bytes per second of unattributed I/O that no tunnel or blanket takes, so it is not drawn.
    pub unplaced: f32,
    /// Files beyond `MOST_PILES`, which are not piles.
    pub beyond: usize,
    /// Tunnels beyond `MOST_TUNNELS`.
    pub tunnels_beyond: usize,
}

/// Everything a sample tells the colony.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub unit: Unit,
    pub streams: Vec<Stream>,
    pub tunnels: Vec<Tunnel>,
    /// Sorted by traffic, the most first, then by key.
    pub piles: Vec<Pile>,
    pub candidates: Vec<Candidate>,
    pub tally: Tally,
}

/// A stable key for a file: its device and inode.
pub fn file_key(device: u64, inode: u64) -> u64 {
    hash::fnv1a_64_bytes(device.to_le_bytes().into_iter().chain(inode.to_le_bytes()))
}

/// What the calls of one process are worth: the part of them its files do not explain.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spare {
    /// Bytes per second no file explains, and the calls that stand for.
    bytes: f32,
    calls: f32,
    /// The part of those calls that were reads and writes, when the direction is known: None when
    /// read-write files took some of the bytes.
    split: Option<(f32, f32)>,
    sizes: Sizes,
}

/// The mean bytes per call of a process's reads, writes and both.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sizes {
    read: Option<f32>,
    write: Option<f32>,
    both: Option<f32>,
}

fn mean(bytes: f32, calls: f32) -> Option<f32> {
    (calls > 0.0).then(|| bytes / calls)
}

impl Sizes {
    fn of(io: SyscallIo) -> Self {
        Sizes {
            read: mean(io.read_chars, io.reads),
            write: mean(io.write_chars, io.writes),
            both: mean(io.read_chars + io.write_chars, io.reads + io.writes),
        }
    }

    fn of_way(self, way: Way) -> Option<f32> {
        match way {
            Way::Read => self.read,
            Way::Write => self.write,
            Way::Either => self.both,
        }
    }
}

/// `part` of `whole`, 0 when there is no whole.
fn share(part: f32, whole: f32) -> f32 {
    if whole > 0.0 {
        (part / whole).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The streams of one process that run to its files, and what its files leave of its calls.
fn files_of(
    process: &Process,
    detail: &IoDetail,
    unit: Unit,
    flows: &mut Vec<Stream>,
) -> Option<Spare> {
    let measured = |file: &FileFlow| match file.flow {
        Flow::Measured {
            read,
            written,
            either,
        } => Some((read, written, either)),
        _ => None,
    };
    let key = |file: &FileFlow| Goal::File(file_key(file.device, file.inode));
    if unit == Unit::Kibibytes {
        // No counts of calls: an ant is KiB moved, the direction is the open flags', and nothing
        // is said of what the positions leave.
        for file in &detail.files {
            let Some((read, written, either)) = measured(file) else {
                continue;
            };
            for (way, bytes) in [
                (Way::Read, read),
                (Way::Write, written),
                (Way::Either, either),
            ] {
                if bytes > 0.0 {
                    flows.push(Stream {
                        nest: process.id,
                        goal: key(file),
                        way,
                        rate: bytes / 1024.0,
                        crumb: None,
                    });
                }
            }
        }
        return None;
    }
    let io = process.syscall_io?;
    let left: Unattributed = detail.unattributed?;
    let sizes = Sizes::of(io);
    let (mut read_files, mut write_files) = (0.0, 0.0);
    for (read, written, _) in detail.files.iter().filter_map(measured) {
        read_files += read;
        write_files += written;
    }
    // `rchar` and `wchar` over the interval of the reading: what the files claimed and the rest.
    let read_total = read_files + left.read;
    let write_total = write_files + left.write;
    let reads_left = io.reads * share(left.read, read_total);
    let writes_left = io.writes * share(left.write, write_total);
    let pool_bytes = left.read + left.write;
    let pool_calls = reads_left + writes_left;
    for file in &detail.files {
        let Some((read, written, either)) = measured(file) else {
            continue;
        };
        let calls = [
            (Way::Read, io.reads * share(read, read_total)),
            (Way::Write, io.writes * share(written, write_total)),
            (Way::Either, pool_calls * share(either, pool_bytes)),
        ];
        for (way, rate) in calls {
            if rate > 0.0 {
                flows.push(Stream {
                    nest: process.id,
                    goal: key(file),
                    way,
                    rate,
                    crumb: sizes.of_way(way),
                });
            }
        }
    }
    let bytes = (pool_bytes - left.either).max(0.0);
    Some(Spare {
        bytes,
        calls: pool_calls * share(bytes, pool_bytes),
        split: (left.either <= NONE_LEFT).then_some((reads_left, writes_left)),
        sizes,
    })
}

/// The share of a process's bytes that its files explain, from 0 to 1; None when the platform
/// cannot say what is left (`Unattributed`), or when the process moved no bytes.
pub fn explained(detail: &IoDetail) -> Option<f32> {
    let left = detail.unattributed?;
    let files: f32 = detail
        .files
        .iter()
        .filter_map(|file| match file.flow {
            Flow::Measured {
                read,
                written,
                either,
            } => Some(read + written + either),
            _ => None,
        })
        .sum();
    let unexplained = (left.read + left.write - left.either).max(0.0);
    (files + unexplained > 0.0).then(|| files / (files + unexplained))
}

/// A pair of processes joined by a medium, whichever end is which.
/// Orders two rates busiest first, a rate nobody measured after every one that was measured: unknown
/// is not idle, so it is not ranked as if it were zero.
pub fn busier_first(a: Option<f32>, b: Option<f32>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn pair(a: Identity, b: Identity) -> (Identity, Identity) {
    if a <= b { (a, b) } else { (b, a) }
}

/// A tunnel one process has, to a partner that may not be drawn, with the traffic of its
/// connection where that is measured.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tie {
    partner: Identity,
    medium: Medium,
    traffic: Option<f32>,
}

/// Every tunnel each process has, drawn or not.
type Ties = HashMap<Identity, Vec<Tie>>;

/// Every tunnel between two drawn nests, and every tunnel each process has, whether its other end
/// is drawn or not.
fn tunnels_of(snapshot: &Snapshot, drawn: &HashSet<Identity>) -> (Vec<Tunnel>, Ties) {
    let mut found: HashMap<(Identity, Identity, Medium), Tunnel> = HashMap::new();
    // Every tunnel seen, drawn or not: a hidden peer sharing several pipes is one tie, as a
    // drawn one is one tunnel.
    let mut seen: HashSet<(Identity, Identity, Medium)> = HashSet::new();
    let mut every = Ties::new();
    let mut note =
        |from: Identity, to: Identity, medium: Medium, one_way: bool, traffic: Option<f32>| {
            let (low, high) = pair(from, to);
            if from == to || !seen.insert((low, high, medium)) {
                return;
            }
            for (end, partner) in [(from, to), (to, from)] {
                every.entry(end).or_default().push(Tie {
                    partner,
                    medium,
                    traffic,
                });
            }
            if drawn.contains(&from) && drawn.contains(&to) {
                found.insert(
                    (low, high, medium),
                    Tunnel {
                        from,
                        to,
                        medium,
                        one_way,
                        rate: None,
                        traffic,
                    },
                );
            }
        };
    for pipe in &snapshot.pipes {
        note(pipe.writer, pipe.reader, Medium::Pipe, !pipe.two_way, None);
    }
    for &(a, b, _) in &snapshot.links {
        let (low, high) = pair(a, b);
        let traffic = snapshot
            .link_traffic
            .get(&(a, b))
            .or_else(|| snapshot.link_traffic.get(&(b, a)))
            .copied();
        let unix = snapshot.unix_links.contains(&(a, b)) || snapshot.unix_links.contains(&(b, a));
        if unix {
            note(low, high, Medium::Unix, false, None);
        }
        if traffic.is_some() {
            note(low, high, Medium::Loopback, false, traffic);
        }
        if !unix && traffic.is_none() {
            note(low, high, Medium::Socket, false, None);
        }
    }
    let mut tunnels: Vec<Tunnel> = found.into_values().collect();
    tunnels.sort_by_key(|tunnel| (pair(tunnel.from, tunnel.to), tunnel.medium, tunnel.one_way));
    (tunnels, every)
}

/// The share of a process's unexplained calls that its tunnel to `partner` by `medium` takes: by
/// traffic when every tunnel the process has is measured, equally otherwise.
fn tunnel_share(ties: &[Tie], partner: Identity, medium: Medium) -> f32 {
    let traffic: Option<Vec<f32>> = ties.iter().map(|tie| tie.traffic).collect();
    let mine = ties
        .iter()
        .position(|tie| tie.partner == partner && tie.medium == medium);
    match (traffic, mine) {
        (_, None) => 0.0,
        (Some(traffic), Some(index)) => {
            let total: f32 = traffic.iter().sum();
            if total > 0.0 {
                traffic[index] / total
            } else {
                1.0 / traffic.len() as f32
            }
        }
        (None, Some(_)) => 1.0 / ties.len() as f32,
    }
}

/// What reading each process's descriptors gave, before the files are ranked and what they do
/// not explain is routed.
struct Gathered {
    streams: Vec<Stream>,
    files: HashMap<u64, Pile>,
    /// Each process's unexplained remainder, and how many terminals it holds.
    spares: Vec<(Identity, Spare, u32)>,
    tally: Tally,
}

/// Reads the descriptors of `ordered` into streams to files, piles with their holders, and what the
/// files leave of each process's calls.
fn gather(ordered: &[&Process], unit: Unit) -> Gathered {
    let mut gathered = Gathered {
        streams: Vec::new(),
        files: HashMap::new(),
        spares: Vec::new(),
        tally: Tally::default(),
    };
    let tally = &mut gathered.tally;
    for process in ordered {
        let detail = match &process.io_detail {
            Measured::Known(detail) => detail,
            Measured::Unreadable => {
                tally.unreadable += 1;
                continue;
            }
            Measured::Pending => {
                tally.pending += 1;
                continue;
            }
        };
        tally.measured += 1;
        tally.partial += usize::from(detail.partial);
        for file in &detail.files {
            let key = file_key(file.device, file.inode);
            let pile = gathered.files.entry(key).or_insert_with(|| Pile {
                key,
                path: file.path.clone(),
                size: file.size,
                deleted: file.deleted,
                holders: Vec::new(),
            });
            pile.size = pile.size.max(file.size);
            pile.deleted |= file.deleted;
            pile.holders.push(Holder {
                id: process.id,
                access: file.access,
                flow: file.flow,
            });
        }
        if unit == Unit::Calls && process.syscall_io.is_none() {
            tally.uncounted += 1;
        }
        if let Some(spare) = files_of(process, detail, unit, &mut gathered.streams) {
            gathered.spares.push((process.id, spare, detail.ttys));
        }
    }
    gathered
}

/// The files ranked by what their holders' positions advanced by, the most first and ties by key,
/// at most `MOST_PILES` of them.
fn rank(files: HashMap<u64, Pile>, tally: &mut Tally) -> Vec<Pile> {
    let mut piles: Vec<Pile> = files.into_values().collect();
    piles.sort_by(|a, b| b.traffic().total_cmp(&a.traffic()).then(a.key.cmp(&b.key)));
    if piles.len() > MOST_PILES {
        tally.beyond = piles.len() - MOST_PILES;
        piles.truncate(MOST_PILES);
    }
    piles
}

/// What the files do not explain of each process's calls goes to its tunnels, or to a blanket when
/// it has none and holds a terminal, or nowhere and is counted. Gives each tunnel the calls routed
/// through it, and returns the processes that got a blanket.
fn route(
    spares: Vec<(Identity, Spare, u32)>,
    ties: &Ties,
    tunnels: &mut [Tunnel],
    streams: &mut Vec<Stream>,
    tally: &mut Tally,
) -> Vec<Candidate> {
    let mut routed: HashMap<(Identity, Identity, Medium), f32> = HashMap::new();
    let mut candidates = Vec::new();
    for (id, spare, ttys) in spares {
        let own = ties.get(&id).map(Vec::as_slice).unwrap_or_default();
        if own.is_empty() {
            if ttys > 0 && spare.bytes > NONE_LEFT {
                let way = |way, rate| {
                    (rate > 0.0).then(|| Stream {
                        nest: id,
                        goal: Goal::Blanket(id),
                        way,
                        rate,
                        crumb: spare.sizes.of_way(way),
                    })
                };
                match spare.split {
                    Some((reads, writes)) => {
                        streams.extend(way(Way::Read, reads));
                        streams.extend(way(Way::Write, writes));
                    }
                    None => streams.extend(way(Way::Either, spare.calls)),
                }
                candidates.push(Candidate {
                    holder: id,
                    ttys,
                    bytes: spare.bytes,
                });
            } else {
                tally.unplaced += spare.bytes;
            }
            continue;
        }
        let mut placed = 0.0;
        for tunnel in tunnels.iter().filter(|t| t.from == id || t.to == id) {
            let partner = if tunnel.from == id {
                tunnel.to
            } else {
                tunnel.from
            };
            let weight = tunnel_share(own, partner, tunnel.medium);
            placed += weight;
            let (low, high) = pair(tunnel.from, tunnel.to);
            *routed.entry((low, high, tunnel.medium)).or_default() += spare.calls * weight;
        }
        tally.unplaced += spare.bytes * (1.0 - placed).max(0.0);
    }
    for tunnel in tunnels.iter_mut() {
        let (low, high) = pair(tunnel.from, tunnel.to);
        tunnel.rate = routed.get(&(low, high, tunnel.medium)).copied();
    }
    candidates.sort_by(|a, b| b.bytes.total_cmp(&a.bytes).then(a.holder.cmp(&b.holder)));
    candidates
}

impl Plan {
    /// The plan for the drawn `processes` of `snapshot`.
    pub fn new(processes: &[&Process], snapshot: &Snapshot) -> Plan {
        let unit = if snapshot.missing.contains(Source::SyscallIo) {
            Unit::Kibibytes
        } else {
            Unit::Calls
        };
        let mut ordered: Vec<&Process> = processes.to_vec();
        ordered.sort_by_key(|process| process.id);
        let drawn: HashSet<Identity> = ordered.iter().map(|process| process.id).collect();
        let (mut tunnels, ties) = tunnels_of(snapshot, &drawn);
        let Gathered {
            mut streams,
            files,
            spares,
            mut tally,
        } = gather(&ordered, unit);
        let piles = rank(files, &mut tally);
        let kept: HashSet<u64> = piles.iter().map(|pile| pile.key).collect();
        streams.retain(|stream| match stream.goal {
            Goal::File(key) => kept.contains(&key),
            Goal::Blanket(_) => true,
        });
        let candidates = route(spares, &ties, &mut tunnels, &mut streams, &mut tally);
        if tunnels.len() > MOST_TUNNELS {
            tally.tunnels_beyond = tunnels.len() - MOST_TUNNELS;
            tunnels.sort_by(|a, b| {
                busier_first(a.rate, b.rate)
                    .then_with(|| pair(a.from, a.to).cmp(&pair(b.from, b.to)))
            });
            tunnels.truncate(MOST_TUNNELS);
        }
        Plan {
            unit,
            streams,
            tunnels,
            piles,
            candidates,
            tally,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthill::fixtures::*;
    use crate::model::Pipe;
    use crate::platform::Sources;

    fn plan_of(snapshot: &Snapshot) -> Plan {
        Plan::new(&drawn(snapshot), snapshot)
    }

    fn key_of(path: &str) -> u64 {
        let file = file(path, AccessMode::Read, Flow::Unmeasured);
        file_key(file.device, file.inode)
    }

    fn streams_to(plan: &Plan, path: &str) -> Vec<Stream> {
        plan.streams
            .iter()
            .filter(|stream| stream.goal == Goal::File(key_of(path)))
            .copied()
            .collect()
    }

    fn pipe(writer: u32, reader: u32) -> Pipe {
        Pipe {
            id: crate::platform::PipeId::Anonymous(u64::from(writer) << 32 | u64::from(reader)),
            reader: id(reader),
            writer: id(writer),
            two_way: false,
            buffered: None,
            fifo: false,
        }
    }

    #[test]
    fn read_ants_go_to_the_files_whose_position_advanced() {
        let busy = holding(
            counting(process(1, 100), 100.0, 4096.0, 0.0, 0.0),
            vec![
                file("/usr/lib/a", AccessMode::Read, moved(409_600.0, 0.0)),
                file("/var/log/b", AccessMode::Read, Flow::Unmeasured),
                file("/usr/lib/c", AccessMode::Read, Flow::Unpositioned),
            ],
            Some(left(0.0, 0.0, 0.0)),
            0,
        );
        let snapshot = snapshot(vec![busy], 5.0);
        let plan = plan_of(&snapshot);
        assert_eq!(plan.unit, Unit::Calls);
        // One stream, to the one file whose position advanced; the others are piles all the same.
        assert_eq!(plan.streams.len(), 1, "{:?}", plan.streams);
        let stream = plan.streams[0];
        assert_eq!(stream.goal, Goal::File(key_of("/usr/lib/a")));
        assert_eq!((stream.way, stream.nest), (Way::Read, id(1)));
        assert!((stream.rate - 100.0).abs() < 1e-3, "{}", stream.rate);
        assert_eq!(plan.piles.len(), 3);
        assert_eq!(
            plan.piles[0].path, "/usr/lib/a",
            "the pile with the traffic ranks first"
        );
        assert_eq!(plan.piles[0].traffic(), 409_600.0);
        assert!(plan.piles[1..].iter().all(|pile| pile.moved().is_none()));
        assert!(streams_to(&plan, "/var/log/b").is_empty());
        assert!(streams_to(&plan, "/usr/lib/c").is_empty());
    }

    #[test]
    fn calls_are_split_over_the_files_in_proportion_to_their_bytes_and_never_exceed_the_calls() {
        // 60 reads a second of 10,000 bytes: 600,000 bytes, of which two files hold 400,000 and
        // 200,000 are in the remainder; one write a second of 1,000 bytes in a file.
        let process = holding(
            counting(process(1, 100), 60.0, 10_000.0, 1.0, 1000.0),
            vec![
                file("/usr/lib/a", AccessMode::Read, moved(300_000.0, 0.0)),
                file("/usr/lib/b", AccessMode::Read, moved(100_000.0, 0.0)),
                file("/tmp/w", AccessMode::Write, moved(0.0, 1000.0)),
            ],
            Some(left(200_000.0, 0.0, 0.0)),
            0,
        );
        let snapshot = snapshot(vec![process], 5.0);
        let plan = plan_of(&snapshot);
        let rate = |path| streams_to(&plan, path)[0].rate;
        assert!((rate("/usr/lib/a") - 30.0).abs() < 1e-3);
        assert!((rate("/usr/lib/b") - 10.0).abs() < 1e-3);
        assert!(
            (rate("/tmp/w") - 1.0).abs() < 1e-3,
            "the one write call it made"
        );
        let reads: f32 = plan
            .streams
            .iter()
            .filter(|stream| stream.way == Way::Read)
            .map(|stream| stream.rate)
            .sum();
        assert!(reads <= 60.0, "{reads} of the 60 reads");
        // The remaining 20 reads of the 60 are the unexplained part: no tunnel, no terminal, so
        // they are counted, not drawn.
        assert!(
            (plan.tally.unplaced - 200_000.0).abs() < 1.0,
            "{}",
            plan.tally.unplaced
        );
    }

    #[test]
    fn crumb_size_follows_bytes_per_syscall_of_the_direction_the_bytes_went() {
        let process = holding(
            counting(process(1, 100), 100.0, 4096.0, 10.0, 65_536.0),
            vec![
                file("/usr/lib/r", AccessMode::Read, moved(100_000.0, 0.0)),
                file("/tmp/w", AccessMode::Write, moved(0.0, 100_000.0)),
                file(
                    "/home/x/rw",
                    AccessMode::Both,
                    Flow::Measured {
                        read: 0.0,
                        written: 0.0,
                        either: 100_000.0,
                    },
                ),
            ],
            Some(left(400_000.0, 600_000.0, 100_000.0)),
            0,
        );
        let snapshot = snapshot(vec![process], 5.0);
        let plan = plan_of(&snapshot);
        assert_eq!(streams_to(&plan, "/usr/lib/r")[0].crumb, Some(4096.0));
        assert_eq!(streams_to(&plan, "/tmp/w")[0].crumb, Some(65_536.0));
        // A read-write file's calls could have been either: the mean of all of the process's.
        let both = streams_to(&plan, "/home/x/rw")[0];
        assert_eq!(both.way, Way::Either);
        let mean = (100.0 * 4096.0 + 10.0 * 65_536.0) / 110.0;
        assert!((both.crumb.unwrap() - mean).abs() < 1.0, "{:?}", both.crumb);
    }

    #[test]
    fn a_read_write_file_sends_ants_that_carry_both_ways_and_its_direction_is_never_split() {
        let process = holding(
            counting(process(1, 100), 50.0, 2000.0, 0.0, 0.0),
            vec![file(
                "/home/x/db",
                AccessMode::Both,
                Flow::Measured {
                    read: 0.0,
                    written: 0.0,
                    either: 100_000.0,
                },
            )],
            Some(left(100_000.0, 0.0, 100_000.0)),
            0,
        );
        let snapshot = snapshot(vec![process], 5.0);
        let plan = plan_of(&snapshot);
        let streams = streams_to(&plan, "/home/x/db");
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].way, Way::Either);
        assert!((streams[0].rate - 50.0).abs() < 1e-3, "{}", streams[0].rate);
        // Nothing is left unexplained, so nothing is unplaced.
        assert_eq!(plan.tally.unplaced, 0.0);
    }

    #[test]
    fn a_file_two_processes_hold_is_one_pile_with_both_holders() {
        let shared = |pid, flow| {
            holding(
                counting(process(pid, 10), 10.0, 1000.0, 0.0, 0.0),
                vec![file("/usr/lib/shared.so", AccessMode::Read, flow)],
                Some(left(0.0, 0.0, 0.0)),
                0,
            )
        };
        let snapshot = snapshot(
            vec![shared(1, moved(10_000.0, 0.0)), shared(2, Flow::Unmeasured)],
            5.0,
        );
        let plan = plan_of(&snapshot);
        assert_eq!(plan.piles.len(), 1);
        let pile = &plan.piles[0];
        let holders: Vec<Identity> = pile.holders.iter().map(|holder| holder.id).collect();
        assert_eq!(holders, [id(1), id(2)]);
        assert_eq!((pile.readers(), pile.writers()), (2, 0));
        assert_eq!((pile.unmeasured(), pile.unpositioned()), (1, 0));
        assert_eq!(
            pile.moved().unwrap().read,
            10_000.0,
            "only the measured holder counts"
        );
    }

    #[test]
    fn unattributed_bytes_travel_tunnels_or_the_blanket_and_never_a_file() {
        let quiet = |pid| {
            holding(
                counting(process(pid, 10), 10.0, 1000.0, 0.0, 0.0),
                vec![],
                Some(left(10_000.0, 0.0, 0.0)),
                0,
            )
        };
        // 1 and 2 share a pipe; 3 holds a terminal and has no tunnel; 4 has neither; 5 has a
        // pipe to 9, which is not drawn.
        let terminal = holding(
            counting(process(3, 10), 0.0, 0.0, 4.0, 1000.0),
            vec![],
            Some(left(0.0, 4000.0, 0.0)),
            1,
        );
        let mut snapshot = snapshot(vec![quiet(1), quiet(2), terminal, quiet(4), quiet(5)], 5.0);
        snapshot.pipes = vec![pipe(1, 2), pipe(5, 9)];
        let plan = plan_of(&snapshot);
        // The pipe from 1 to 2 carries the calls of both ends' unexplained bytes: 10 + 10.
        assert_eq!(plan.tunnels.len(), 1);
        let tunnel = plan.tunnels[0];
        assert_eq!(
            (tunnel.from, tunnel.to, tunnel.one_way),
            (id(1), id(2), true)
        );
        assert!(
            (tunnel.rate.unwrap() - 20.0).abs() < 1e-3,
            "{:?}",
            tunnel.rate
        );
        // The terminal's holder sends write ants to its blanket: 4 writes a second.
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].goal, Goal::Blanket(id(3)));
        assert_eq!(plan.streams[0].way, Way::Write);
        assert!((plan.streams[0].rate - 4.0).abs() < 1e-3);
        assert_eq!(plan.candidates.len(), 1);
        // 4 has nowhere to put 10,000 bytes a second, and 5's pipe leads out of sight: both are
        // counted and not drawn.
        assert!(
            (plan.tally.unplaced - 20_000.0).abs() < 1.0,
            "{}",
            plan.tally.unplaced
        );
        assert!(plan.piles.is_empty(), "nothing was handed to a file");
    }

    #[test]
    fn a_process_with_a_tunnel_does_not_send_its_unexplained_bytes_to_a_blanket() {
        let both = |pid| {
            holding(
                counting(process(pid, 10), 10.0, 1000.0, 0.0, 0.0),
                vec![],
                Some(left(10_000.0, 0.0, 0.0)),
                1,
            )
        };
        let mut snapshot = snapshot(vec![both(1), both(2)], 5.0);
        snapshot.unix_links.insert((id(1), id(2)));
        snapshot.links.push((id(1), id(2), 1));
        let plan = plan_of(&snapshot);
        assert!(plan.streams.is_empty(), "{:?}", plan.streams);
        assert!(plan.candidates.is_empty());
        assert_eq!(plan.tunnels.len(), 1);
        assert_eq!(plan.tunnels[0].medium, Medium::Unix);
        assert!(!plan.tunnels[0].one_way);
    }

    #[test]
    fn several_pipes_to_a_peer_that_is_not_drawn_are_one_tie() {
        let both = |pid| {
            holding(
                counting(process(pid, 10), 10.0, 1000.0, 0.0, 0.0),
                vec![],
                Some(left(10_000.0, 0.0, 0.0)),
                1,
            )
        };
        let mut snapshot = snapshot(vec![both(1), both(2)], 5.0);
        snapshot.pipes = vec![pipe(1, 2), pipe(1, 2), pipe(1, 2)];
        // Only process 1 is drawn: its partner is hidden, and the pipes between them are one tie.
        let drawn: HashSet<Identity> = [id(1)].into_iter().collect();
        let (tunnels, ties) = tunnels_of(&snapshot, &drawn);
        assert!(tunnels.is_empty(), "no tunnel to a hidden nest");
        assert_eq!(ties[&id(1)].len(), 1, "{:?}", ties[&id(1)]);
        // Drawn, the same pipes are one tunnel and one tie, as before.
        let drawn: HashSet<Identity> = [id(1), id(2)].into_iter().collect();
        let (tunnels, ties) = tunnels_of(&snapshot, &drawn);
        assert_eq!((tunnels.len(), ties[&id(1)].len()), (1, 1));
    }

    #[test]
    fn a_tunnels_share_follows_measured_traffic_only_when_every_tunnel_is_measured() {
        let hub = |pid| {
            holding(
                counting(process(pid, 10), 10.0, 1000.0, 0.0, 0.0),
                vec![],
                Some(left(10_000.0, 0.0, 0.0)),
                0,
            )
        };
        let spoke = |pid| process(pid, 10);
        // 1 has two loopback connections, measured at 3,000 and 1,000 bytes a second.
        let mut measured = snapshot(vec![hub(1), spoke(2), spoke(3)], 5.0);
        measured.links = vec![(id(1), id(2), 1), (id(1), id(3), 1)];
        measured.link_traffic = [((id(1), id(2)), 3000.0), ((id(1), id(3)), 1000.0)].into();
        let rate_between = |plan: &Plan, a, b| {
            plan.tunnels
                .iter()
                .find(|t| (t.from, t.to) == (id(a), id(b)) || (t.from, t.to) == (id(b), id(a)))
                .and_then(|tunnel| tunnel.rate)
                .unwrap()
        };
        let plan = plan_of(&measured);
        assert!((rate_between(&plan, 1, 2) - 7.5).abs() < 1e-3);
        assert!((rate_between(&plan, 1, 3) - 2.5).abs() < 1e-3);
        // Add a Unix socket that has no measured traffic: the shares are equal.
        let mut mixed = measured.clone();
        mixed.processes.push(spoke(4));
        mixed.links.push((id(1), id(4), 1));
        mixed.unix_links.insert((id(1), id(4)));
        let plan = plan_of(&mixed);
        for (a, b) in [(1, 2), (1, 3), (1, 4)] {
            assert!(
                (rate_between(&plan, a, b) - 10.0 / 3.0).abs() < 1e-3,
                "{a} {b}"
            );
        }
    }

    #[test]
    fn a_tunnel_between_nests_that_were_not_read_has_an_unknown_rate_not_a_zero_one() {
        let mut snapshot = snapshot(vec![process(1, 10), process(2, 10)], 5.0);
        snapshot.links.push((id(1), id(2), 1));
        snapshot.link_traffic.insert((id(1), id(2)), 0.0);
        let plan = plan_of(&snapshot);
        assert_eq!(plan.tunnels.len(), 1);
        assert_eq!(plan.tunnels[0].rate, None);
        assert_eq!(plan.tunnels[0].traffic, Some(0.0), "measured, and idle");
        assert_eq!(plan.tunnels[0].medium, Medium::Loopback);
    }

    #[test]
    fn missing_syscall_counts_switch_ants_to_bytes_and_say_so() {
        let process = holding(
            process(1, 100),
            vec![
                file("/usr/lib/r", AccessMode::Read, moved(10.0 * 1024.0, 0.0)),
                file("/tmp/w", AccessMode::Write, moved(0.0, 4096.0)),
                file(
                    "/home/x/rw",
                    AccessMode::Both,
                    Flow::Measured {
                        read: 0.0,
                        written: 0.0,
                        either: 2048.0,
                    },
                ),
            ],
            None,
            1,
        );
        let mut snapshot = snapshot(vec![process], 5.0);
        snapshot.missing = Sources::of(&[Source::SyscallIo, Source::UnattributedIo]);
        let plan = plan_of(&snapshot);
        assert_eq!(plan.unit, Unit::Kibibytes);
        assert_eq!(plan.streams.len(), 3);
        let by = |path| streams_to(&plan, path)[0];
        // An ant is KiB moved, the direction is the open flags', and the crumbs are unsized.
        assert_eq!(
            (by("/usr/lib/r").way, by("/usr/lib/r").rate),
            (Way::Read, 10.0)
        );
        assert_eq!((by("/tmp/w").way, by("/tmp/w").rate), (Way::Write, 4.0));
        assert_eq!(
            (by("/home/x/rw").way, by("/home/x/rw").rate),
            (Way::Either, 2.0)
        );
        assert!(plan.streams.iter().all(|stream| stream.crumb.is_none()));
        assert!(
            plan.candidates.is_empty(),
            "what is left is not known, so nothing goes to a blanket"
        );
        assert_eq!(
            plan.tally.uncounted, 0,
            "there are no counts to be missing here"
        );
    }

    #[test]
    fn a_process_without_syscall_counts_where_the_platform_has_them_sends_no_ants_and_is_counted() {
        let process = holding(
            process(1, 100),
            vec![file("/usr/lib/r", AccessMode::Read, moved(1000.0, 0.0))],
            Some(left(0.0, 0.0, 0.0)),
            0,
        );
        let snapshot = snapshot(vec![process], 5.0);
        let plan = plan_of(&snapshot);
        assert!(plan.streams.is_empty());
        assert_eq!(plan.tally.uncounted, 1);
        assert_eq!(plan.piles.len(), 1, "the file is still a pile");
    }

    #[test]
    fn unreadable_and_not_yet_read_processes_are_counted_apart() {
        let mut denied = process(1, 10);
        denied.io_detail = Measured::Unreadable;
        let snapshot = snapshot(vec![denied, process(2, 10)], 5.0);
        let plan = plan_of(&snapshot);
        assert_eq!(
            (
                plan.tally.unreadable,
                plan.tally.pending,
                plan.tally.measured
            ),
            (1, 1, 0)
        );
    }

    #[test]
    fn at_most_two_hundred_and_fifty_six_piles_and_the_ones_with_most_bytes_win() {
        let files: Vec<FileFlow> = (0..300)
            .map(|k| {
                file(
                    &format!("/tmp/f{k:03}"),
                    AccessMode::Read,
                    moved(100.0 + k as f32, 0.0),
                )
            })
            .collect();
        let process = holding(
            counting(process(1, 10), 10.0, 100.0, 0.0, 0.0),
            files,
            Some(left(0.0, 0.0, 0.0)),
            0,
        );
        let snapshot = snapshot(vec![process], 5.0);
        let plan = plan_of(&snapshot);
        assert_eq!(plan.piles.len(), MOST_PILES);
        assert_eq!(plan.tally.beyond, 44);
        assert_eq!(plan.piles[0].path, "/tmp/f299");
        assert!(
            plan.piles
                .iter()
                .all(|pile| pile.path.as_str() >= "/tmp/f044")
        );
        // Streams to the files that did not make it are gone with them.
        let kept: HashSet<u64> = plan.piles.iter().map(|pile| pile.key).collect();
        assert!(plan.streams.iter().all(|stream| matches!(
            stream.goal,
            Goal::File(key) if kept.contains(&key)
        )));
    }

    #[test]
    fn the_share_of_bytes_the_files_explain_is_what_their_positions_hold_over_all_of_them() {
        let detail = |files: Vec<FileFlow>, left| {
            let process = holding(process(1, 1), files, left, 0);
            match process.io_detail {
                Measured::Known(detail) => detail,
                _ => unreachable!(),
            }
        };
        let half = detail(
            vec![file("/a", AccessMode::Read, moved(500.0, 0.0))],
            Some(left(500.0, 0.0, 0.0)),
        );
        assert_eq!(explained(&half), Some(0.5));
        assert_eq!(
            explained(&detail(vec![], None)),
            None,
            "not known is not zero"
        );
        assert_eq!(
            explained(&detail(vec![], Some(left(0.0, 0.0, 0.0)))),
            None,
            "nothing moved"
        );
    }

    #[test]
    fn the_plan_is_the_same_whatever_order_the_processes_come_in() {
        let make = |pid| {
            holding(
                counting(process(pid, 10), 10.0, 1000.0, 5.0, 500.0),
                vec![
                    file("/usr/lib/a", AccessMode::Read, moved(10_000.0, 0.0)),
                    file("/tmp/b", AccessMode::Write, moved(0.0, 2500.0)),
                ],
                Some(left(0.0, 0.0, 0.0)),
                0,
            )
        };
        let forward = snapshot(vec![make(1), make(2), make(3)], 5.0);
        let backward = snapshot(vec![make(3), make(2), make(1)], 5.0);
        assert_eq!(plan_of(&forward), plan_of(&backward));
    }

    #[test]
    fn a_rate_nobody_measured_ranks_after_every_measured_one() {
        let mut rates = [None, Some(0.0), Some(5.0), None, Some(1.0)];
        rates.sort_by(|a, b| busier_first(*a, *b));
        assert_eq!(rates, [Some(5.0), Some(1.0), Some(0.0), None, None]);
    }
}
