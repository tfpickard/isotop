//! The boiler room `DemoOptions::plumbing` synthesizes over a finished demo snapshot: sixteen of
//! the base processes renamed and resized into the cast below, every link, pipe, queue, wait
//! channel and descriptor count the plumbing view draws, and nothing else. Everything is a pure
//! function of the time and the process's index, so a call at any time, in any order, gives the
//! same snapshot, and no random stream is drawn.
//!
//! The cast, by what the view shows of it:
//! - a four-stage shell pipeline in `session-3.scope`: `cat` into `grep` into `sort` into `uniq`.
//!   `cat` and `sort` have one outlet each, so their flow is exact (`sort`'s is exactly 0);
//!   `grep` also holds a terminal, which makes its flow `split unknown`; `uniq` writes only to
//!   its terminal. `uniq` is waiting in `pipe_read`, so its inlet runs dry;
//! - `browser`, blocked in `pipe_write`, and the `pipewire` it writes to, which B8 did not read
//!   (its flow is unmeasured) in `user.slice`; the `compiler` of `user.slice` writes three pipes
//!   (to `language-server`, whose descriptors are unreadable, to `journald` and to `postgres`),
//!   so its flow is `split unknown` over three branches;
//! - Unix sockets in `system.slice`: `systemd` and `journald` (a stream with 12 KiB queued),
//!   `dbus` to `journald` (a datagram waiting), `nginx` to `worker` (a stream with 28 KiB
//!   queued), and a FIFO that `systemd` and `dbus` both opened for reading and writing;
//! - loopback TCP from `worker` to `postgres` in `docker`, 1.4 MB/s one way and 0.22 MB/s back;
//! - `redis`, in `docker`, writing 9 MB/s into a pipe whose reader, `gzip`, drops its end every
//!   20 s from the tenth second to the eighteenth, without any process exiting.
//!
//! A demo of more processes than the default 192 strings the rest into shell pipelines of four
//! (`EXTRA_FROM`, `CHAIN`), each process writing into a pipe the next reads, so that a run of
//! `--processes 1000` has the hundreds of tanks and runs that the view caps. Only the first
//! `MEASURED_CHAINS` of them have the descriptors a pass of the I/O detail would have read; the
//! others are not among the busiest and read as unmeasured.
//!
//! Each of these processes' descriptors, files and `unattributed` bytes are what a pass of
//! `io_detail::IoScan` would have read, set by hand; `gzip`'s write-only archive takes all its
//! write calls, so what is left unattributed is 0.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::{Hose, PipeHold, Severed};
use crate::io_detail::{FileFlow, Flow, IoDetail, Unattributed};
use crate::model::{Identity, Measured, Pipe, Queue, Snapshot, SyscallIo};
use crate::platform::{AccessMode, PipeId, Source, Wait};

const SYSTEMD: usize = 1;
const JOURNALD: usize = 2;
const NGINX: usize = 3;
const DBUS: usize = 4;
const LANGUAGE_SERVER: usize = 16;
const BROWSER: usize = 17;
const PIPEWIRE: usize = 18;
const COMPILER: usize = 19;
const POSTGRES: usize = 33;
const WORKER: usize = 34;
const REDIS: usize = 35;
const CAT: usize = 81;
const GREP: usize = 82;
const SORT: usize = 83;
const UNIQ: usize = 84;
const GZIP: usize = 85;

/// The processes of the boiler room: index in the demo, name and resident memory in MiB.
const CAST: [(usize, &str, u64); 16] = [
    (SYSTEMD, "systemd", 14),
    (JOURNALD, "journald", 62),
    (NGINX, "nginx", 44),
    (DBUS, "dbus-daemon", 9),
    (LANGUAGE_SERVER, "language-server", 910),
    (BROWSER, "browser", 2150),
    (PIPEWIRE, "pipewire", 38),
    (COMPILER, "compiler", 640),
    (POSTGRES, "postgres", 1230),
    (WORKER, "worker", 150),
    (REDIS, "redis", 310),
    (CAT, "cat", 2),
    (GREP, "grep", 3),
    (SORT, "sort", 96),
    (UNIQ, "uniq", 2),
    (GZIP, "gzip", 3),
];

/// The families of sixteen base processes that the boiler room puts in cgroups of their own, and
/// the names of those.
const BAYS: [(usize, &str); 4] = [
    (0, "system.slice"),
    (1, "user.slice"),
    (2, "docker"),
    (5, "session-3.scope"),
];

/// The first inode of the anonymous pipes of the boiler room, and the device and inode of its FIFO.
const PIPE_INODE: u64 = 7001;
const FIFO: PipeId = PipeId::Fifo {
    device: 0x0100_0004,
    inode: 9001,
};

/// The first base process of the pipelines a demo of more processes adds, how many processes
/// make one, and how many of them have their descriptors read.
const EXTRA_FROM: usize = 192;
const CHAIN: usize = 4;
const MEASURED_CHAINS: usize = 6;

/// The writer and the reader of each pipe of the extra pipelines of a demo of `count` processes.
fn chain_pipes(count: usize) -> impl Iterator<Item = (usize, usize)> {
    (EXTRA_FROM..count)
        .filter(move |index| (index - EXTRA_FROM) % CHAIN + 1 < CHAIN && index + 1 < count)
        .map(|index| (index, index + 1))
}

/// Seconds of the cycle in which `gzip` drops its end of redis's pipe, and the phases of it when
/// it does and when it picks it up again.
const CYCLE: f64 = 20.0;
const DROPS_AT: f64 = 10.0;
const RETURNS_AT: f64 = 18.0;

/// A byte rate wandering by up to 15% round its mean, slowly and differently for each process.
fn pulse(time: f64, index: usize) -> f32 {
    (0.85 + 0.15 * (time * 0.6 + index as f64 * 1.3).sin()) as f32
}

/// How a process's descriptors and write calls look to a pass of the I/O detail.
struct Spec {
    /// What its write calls moved into pipes and sockets, the terminal and the like, apart from
    /// what it sent over TCP: the rate the view attributes to its outlets.
    outflow: f32,
    /// Its regular files.
    files: Vec<FileFlow>,
    /// Socket and terminal descriptors, apart from the connections of `remotes`.
    sockets: u32,
    ttys: u32,
}

fn file(inode: u64, path: &str, access: AccessMode, flow: Flow) -> FileFlow {
    FileFlow {
        device: 2049,
        inode,
        size: 1 << 32,
        path: path.to_owned(),
        deleted: false,
        access,
        flow,
    }
}

/// What the file moved per second, as a write-only descriptor's position says.
fn written(rate: f32) -> Flow {
    Flow::Measured {
        read: 0.0,
        written: rate,
        either: 0.0,
    }
}

fn read(rate: f32) -> Flow {
    Flow::Measured {
        read: rate,
        written: 0.0,
        either: 0.0,
    }
}

/// The descriptors and flows of the cast member `index` at pulse `p`, or None for one that B8 is
/// not given: `pipewire` is not among the busiest, `language-server` is another user's.
fn spec(index: usize, p: f32) -> Option<Spec> {
    if index >= EXTRA_FROM {
        return extra_spec(index, p);
    }
    let (outflow, files, sockets, ttys) = match index {
        CAT => (
            48.0e6 * p,
            vec![file(
                11,
                "/var/log/nginx/access.log",
                AccessMode::Read,
                read(48.0e6 * p),
            )],
            0,
            0,
        ),
        GREP => (2.2e6 * p, Vec::new(), 0, 1),
        SORT => (0.0, Vec::new(), 0, 0),
        UNIQ => (0.0, Vec::new(), 0, 1),
        BROWSER => (0.0, Vec::new(), 0, 0),
        COMPILER => (
            3.1e6 * p,
            vec![file(
                12,
                "/home/demo/src/project/src/main.rs",
                AccessMode::Read,
                read(0.4e6 * p),
            )],
            0,
            0,
        ),
        JOURNALD => (
            1.2e6 * p,
            vec![file(
                13,
                "/var/log/journal/system.journal",
                AccessMode::Write,
                written(0.5e6 * p),
            )],
            3,
            0,
        ),
        POSTGRES => (
            6.0e6 * p,
            vec![
                file(
                    14,
                    "/var/lib/postgresql/wal/000000010000",
                    AccessMode::Write,
                    written(5.0e6 * p),
                ),
                file(
                    15,
                    "/var/lib/postgresql/base/16384",
                    AccessMode::Both,
                    Flow::Measured {
                        read: 0.0,
                        written: 0.0,
                        either: 0.6e6 * p,
                    },
                ),
            ],
            2,
            0,
        ),
        WORKER => (5.0e6 * p, Vec::new(), 2, 0),
        NGINX => (4.5e6 * p, Vec::new(), 1, 0),
        SYSTEMD => (0.05e6 * p, Vec::new(), 1, 0),
        DBUS => (0.02e6 * p, Vec::new(), 1, 0),
        REDIS => (9.0e6 * p, Vec::new(), 0, 0),
        GZIP => (
            0.0,
            vec![file(
                16,
                "/var/backups/archive.gz",
                AccessMode::Write,
                written(3.0e6 * p),
            )],
            0,
            0,
        ),
        _ => return None,
    };
    Some(Spec {
        outflow,
        files,
        sockets,
        ttys,
    })
}

/// The descriptors of a process of an extra pipeline: every one but the last writes its pipe,
/// the last writes to its terminal. None for the pipelines that are not among the busiest.
fn extra_spec(index: usize, p: f32) -> Option<Spec> {
    let (chain, member) = ((index - EXTRA_FROM) / CHAIN, (index - EXTRA_FROM) % CHAIN);
    if chain >= MEASURED_CHAINS {
        return None;
    }
    let last = member + 1 == CHAIN;
    Some(Spec {
        outflow: if last { 0.3e6 * p } else { 1.5e6 * p },
        files: Vec::new(),
        sockets: 0,
        ttys: u32::from(last),
    })
}

/// What `sender` sent over loopback TCP, per second, at pulse `p`.
fn sent_over_tcp(sender: usize, p: f32) -> f32 {
    match sender {
        WORKER => 1.4e6 * p,
        POSTGRES => 0.22e6 * p,
        _ => 0.0,
    }
}

/// Where the cast member `index` is blocked.
fn waiting(index: usize) -> Wait {
    match index {
        BROWSER => Wait::PipeWrite,
        UNIQ => Wait::PipeRead,
        LANGUAGE_SERVER => Wait::Unreadable,
        _ => Wait::Other,
    }
}

/// Puts the boiler room into `snapshot`, a finished demo snapshot of `count` base processes at
/// `time`. Cast members whose index is beyond `count` are left out with whatever joins them.
pub(crate) fn apply(snapshot: &mut Snapshot, time: f64, count: usize) {
    let base = count.min(snapshot.processes.len());
    cast_and_bays(snapshot, base);
    let ids: Vec<Identity> = snapshot.processes.iter().take(base).map(|p| p.id).collect();
    let (pipes, severed) = pipes_at(&ids, time);
    let pipe_holds = holds(&pipes, &severed);
    let sockets = sockets_at(&ids, time);
    let waits = describe(snapshot, &ids, &pipe_holds, time);
    snapshot.links = sockets.links;
    snapshot.link_traffic = sockets.link_traffic;
    snapshot.unix_links = sockets.unix_links;
    snapshot.unix_queues = sockets.unix_queues;
    snapshot.pipes = pipes;
    snapshot.pipes_partial = false;
    snapshot.pipe_holds = pipe_holds;
    snapshot.severed = severed;
    snapshot.waits = waits;
    snapshot.loopback_sent = sockets.loopback_sent;
    snapshot.sources.insert(Source::Pipes);
    snapshot.sources.insert(Source::IoDetail);
    snapshot.sources.insert(Source::WaitChannel);
}

/// Puts the base processes into the cgroups of the bays and names, sizes and wakes the cast.
fn cast_and_bays(snapshot: &mut Snapshot, base: usize) {
    for (index, process) in snapshot.processes.iter_mut().enumerate().take(base) {
        if let Some(&(_, name)) = BAYS.iter().find(|(family, _)| *family == index / 16) {
            name.clone_into(&mut process.group);
            process.cgroup = format!("/{name}");
        }
        if let Some(&(_, name, mebibytes)) = CAST.iter().find(|(member, ..)| *member == index) {
            name.clone_into(&mut process.name);
            process.command = format!("{name} --demo");
            process.memory = mebibytes << 20;
            // The base workload makes some of these zombies, which hold no descriptors.
            process.state = if process.cpu > 30.0 { 'R' } else { 'S' };
        }
    }
}

/// The pipes and FIFOs of the boiler room at `time`, writer first, and the pipes whose reader has
/// let go. `ids` are the base processes by index.
fn pipes_at(ids: &[Identity], time: f64) -> (Vec<Pipe>, Vec<Severed>) {
    let id = |index: usize| ids.get(index).copied();
    let phase = time.rem_euclid(CYCLE);
    let cut_now = (DROPS_AT..RETURNS_AT).contains(&phase);
    let mut pipes: Vec<Pipe> = Vec::new();
    let mut severed: Vec<Severed> = Vec::new();
    let mut pipe_of = |inode: u64, writer: usize, reader: usize, cut: bool| {
        let (Some(writer), Some(reader)) = (id(writer), id(reader)) else {
            return;
        };
        let id = PipeId::Anonymous(PIPE_INODE + inode);
        if cut {
            severed.push(Severed { writer, id });
        } else {
            pipes.push(Pipe {
                id,
                reader,
                writer,
                two_way: false,
                buffered: None,
                fifo: false,
            });
        }
    };
    pipe_of(0, BROWSER, PIPEWIRE, false);
    pipe_of(1, COMPILER, LANGUAGE_SERVER, false);
    pipe_of(2, COMPILER, JOURNALD, false);
    pipe_of(3, COMPILER, POSTGRES, false);
    pipe_of(4, CAT, GREP, false);
    pipe_of(5, GREP, SORT, false);
    pipe_of(6, SORT, UNIQ, false);
    // The cycle's pipe: its reader lets go for eight seconds of every twenty.
    pipe_of(7, REDIS, GZIP, cut_now);
    for (writer, reader) in chain_pipes(ids.len()) {
        pipe_of(100 + writer as u64, writer, reader, false);
    }
    if let (Some(a), Some(b)) = (id(SYSTEMD), id(DBUS)) {
        pipes.push(Pipe {
            id: FIFO,
            reader: a.min(b),
            writer: a.max(b),
            two_way: true,
            buffered: None,
            fifo: true,
        });
    }
    pipes.sort_by_key(|pipe| (pipe.reader, pipe.writer, pipe.fifo));
    severed.sort();
    (pipes, severed)
}

/// What each process holds of `pipes` and of the severed pipes it still writes.
fn holds(pipes: &[Pipe], severed: &[Severed]) -> HashMap<Identity, PipeHold> {
    let mut holds: HashMap<Identity, PipeHold> = HashMap::new();
    for pipe in pipes {
        holds.entry(pipe.reader).or_default().reads += 1;
        holds.entry(pipe.writer).or_default().writes += 1;
        if pipe.two_way {
            holds.entry(pipe.reader).or_default().writes += 1;
            holds.entry(pipe.writer).or_default().reads += 1;
        }
    }
    for cut in severed {
        holds.entry(cut.writer).or_default().writes += 1;
    }
    holds
}

/// The socket side of the boiler room: Unix streams with queues, a datagram and the loopback
/// connection, in the shapes the snapshot keeps them.
struct Sockets {
    links: Vec<(Identity, Identity, u32)>,
    unix_links: HashSet<(Identity, Identity)>,
    unix_queues: HashMap<(Identity, Identity), Queue>,
    link_traffic: HashMap<(Identity, Identity), f32>,
    loopback_sent: HashMap<(Identity, Identity), Hose>,
}

fn sockets_at(ids: &[Identity], time: f64) -> Sockets {
    let id = |index: usize| ids.get(index).copied();
    let mut sockets = Sockets {
        links: Vec::new(),
        unix_links: HashSet::new(),
        unix_queues: HashMap::new(),
        link_traffic: HashMap::new(),
        loopback_sent: HashMap::new(),
    };
    let ordered = |a: Identity, b: Identity| (a.min(b), a.max(b));
    let mut unix = |writer: usize, reader: usize, queue: Queue| {
        let (Some(writer), Some(reader)) = (id(writer), id(reader)) else {
            return;
        };
        let pair = ordered(writer, reader);
        sockets.links.push((pair.0, pair.1, 1_u32));
        sockets.unix_links.insert(pair);
        sockets.unix_queues.insert((reader, writer), queue);
        sockets.unix_queues.entry((writer, reader)).or_default();
    };
    let stream = |bytes: f64, index: usize| Queue {
        unread: (bytes * f64::from(pulse(time, index))) as u64,
        datagram_waiting: false,
    };
    unix(SYSTEMD, JOURNALD, stream(12_288.0, SYSTEMD));
    unix(NGINX, WORKER, stream(28_672.0, NGINX));
    unix(
        DBUS,
        JOURNALD,
        Queue {
            unread: 0,
            datagram_waiting: true,
        },
    );
    if let (Some(worker), Some(postgres)) = (id(WORKER), id(POSTGRES)) {
        let pair = ordered(worker, postgres);
        sockets.links.push((pair.0, pair.1, 1));
        let out = sent_over_tcp(WORKER, pulse(time, WORKER));
        let back = sent_over_tcp(POSTGRES, pulse(time, POSTGRES));
        sockets.link_traffic.insert(pair, out + back);
        for (from, to, rate) in [(worker, postgres, out), (postgres, worker, back)] {
            sockets.loopback_sent.insert(
                (from, to),
                Hose {
                    rate,
                    connections: 1,
                },
            );
        }
    }
    sockets.links.sort();
    sockets
}

/// Gives each cast member the descriptors, files and write calls a pass of the I/O detail would
/// have read, and returns where each process that holds a pipe is blocked.
fn describe(
    snapshot: &mut Snapshot,
    ids: &[Identity],
    pipe_holds: &HashMap<Identity, PipeHold>,
    time: f64,
) -> HashMap<Identity, Wait> {
    let mut waits = HashMap::new();
    let members = CAST
        .iter()
        .map(|&(index, ..)| index)
        .chain(EXTRA_FROM..ids.len());
    for index in members {
        let Some(&who) = ids.get(index) else {
            continue;
        };
        let p = pulse(time, index);
        if pipe_holds.contains_key(&who) {
            waits.insert(who, waiting(index));
        }
        let hold = pipe_holds.get(&who).copied().unwrap_or_default();
        let remote_up = snapshot_remote_up(&snapshot.remotes, who);
        let remotes = snapshot.remotes.iter().filter(|r| r.id == who).count() as u32;
        let process = &mut snapshot.processes[index];
        process.io_detail = match spec(index, p) {
            None if index == LANGUAGE_SERVER => Measured::Unreadable,
            None => Measured::Pending,
            Some(spec) => {
                let tcp = sent_over_tcp(index, p) + remote_up;
                let (mut file_writes, mut read_bytes, mut either) = (0.0, 0.0, 0.0);
                for file in &spec.files {
                    if let Flow::Measured {
                        read,
                        written,
                        either: both,
                    } = file.flow
                    {
                        file_writes += written;
                        read_bytes += read;
                        either += both;
                    }
                }
                process.syscall_io = Some(SyscallIo {
                    read_chars: read_bytes + either + 0.1e6,
                    write_chars: spec.outflow + tcp + file_writes,
                    reads: 200.0,
                    writes: 200.0,
                });
                Measured::Known(Arc::new(IoDetail {
                    files: spec.files,
                    pipes: hold.reads + hold.writes,
                    sockets: spec.sockets + remotes,
                    ttys: spec.ttys,
                    anonymous: 4,
                    other: 1,
                    unattributed: Some(Unattributed {
                        read: 0.1e6,
                        write: spec.outflow + tcp,
                        either,
                    }),
                    partial: false,
                }))
            }
        };
    }
    waits
}

/// What `who` sends to machines outside, in bytes per second, from the connections the demo has
/// measured.
fn snapshot_remote_up(remotes: &[crate::model::Remote], who: Identity) -> f32 {
    remotes
        .iter()
        .filter(|remote| remote.id == who)
        .filter_map(|remote| remote.up)
        .sum()
}

/// The processes of `snapshot` that belong to the boiler room, by index.
#[cfg(test)]
pub(crate) fn cast(snapshot: &Snapshot) -> Vec<&crate::model::Process> {
    CAST.iter()
        .filter_map(|&(index, ..)| snapshot.processes.get(index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DemoOptions, Process, demo_with};

    const PLUMBING: DemoOptions = DemoOptions {
        plumbing: true,
        ..DemoOptions::NONE
    };

    fn room(time: f64) -> Snapshot {
        demo_with(time, 192, PLUMBING)
    }

    fn named<'a>(snapshot: &'a Snapshot, name: &str) -> &'a Process {
        snapshot
            .processes
            .iter()
            .find(|process| process.name == name)
            .unwrap_or_else(|| panic!("no process is called {name}"))
    }

    #[test]
    fn the_boiler_room_is_a_pure_function_of_the_time() {
        let first = format!("{:?}", room(12.0).pipes);
        for time in [3.0, 40.0, 12.0, -7.0, 12.0] {
            let _ = room(time);
        }
        assert_eq!(first, format!("{:?}", room(12.0).pipes));
        let (a, b) = (room(12.25), room(12.25));
        assert_eq!(
            format!("{:?} {:?}", a.unix_queues.len(), a.loopback_sent.len()),
            format!("{:?} {:?}", b.unix_queues.len(), b.loopback_sent.len())
        );
        assert_eq!(
            named(&a, "cat")
                .syscall_io
                .map(|io| io.write_chars.to_bits()),
            named(&b, "cat")
                .syscall_io
                .map(|io| io.write_chars.to_bits())
        );
    }

    #[test]
    fn the_cast_is_sixteen_processes_in_four_bays() {
        let snapshot = room(12.0);
        let cast = cast(&snapshot);
        assert_eq!(cast.len(), 16);
        let mut bays: Vec<&str> = cast.iter().map(|p| p.group.as_str()).collect();
        bays.sort_unstable();
        bays.dedup();
        assert_eq!(
            bays,
            ["docker", "session-3.scope", "system.slice", "user.slice"]
        );
        assert!(
            cast.iter().all(|p| p.state != 'Z'),
            "zombies hold no descriptors"
        );
        assert_eq!(named(&snapshot, "browser").memory, 2150 << 20);
    }

    #[test]
    fn the_holds_follow_from_the_pipes() {
        let snapshot = room(12.0);
        let hold = |name: &str| snapshot.pipe_holds[&named(&snapshot, name).id];
        let compiler = hold("compiler");
        assert_eq!((compiler.writes, compiler.reads), (3, 0));
        let grep = hold("grep");
        assert_eq!((grep.writes, grep.reads), (1, 1));
        assert_eq!(hold("redis").writes, 1, "even with its reader gone");
        let systemd = hold("systemd");
        assert_eq!(
            (systemd.writes, systemd.reads),
            (1, 1),
            "a FIFO opened for reading and writing counts both ways"
        );
        assert!(
            !snapshot
                .pipe_holds
                .contains_key(&named(&snapshot, "nginx").id),
            "a process with no pipe has no hold"
        );
    }

    #[test]
    fn gzip_drops_redis_pipe_from_the_tenth_second_to_the_eighteenth_of_every_twenty() {
        for (time, severed) in [
            (0.0, false),
            (9.9, false),
            (10.0, true),
            (12.0, true),
            (17.9, true),
            (18.0, false),
            (29.0, false),
            (30.0, true),
            (-9.0, true),
        ] {
            let snapshot = room(time);
            let (redis, gzip) = (named(&snapshot, "redis").id, named(&snapshot, "gzip").id);
            let joined = snapshot
                .pipes
                .iter()
                .any(|pipe| pipe.writer == redis && pipe.reader == gzip);
            let cut = snapshot.severed.iter().any(|cut| cut.writer == redis);
            assert_eq!((joined, cut), (!severed, severed), "at {time} s");
            assert_eq!(snapshot.pipe_holds.contains_key(&gzip), !severed);
            assert_eq!(
                snapshot.processes.len(),
                room(0.0).processes.len(),
                "no process is removed"
            );
        }
    }

    #[test]
    fn what_the_view_reads_is_marked_as_read() {
        let snapshot = room(12.0);
        for source in [Source::Pipes, Source::IoDetail, Source::WaitChannel] {
            assert!(snapshot.sources.contains(source), "{}", source.name());
        }
        assert_eq!(
            snapshot.waits[&named(&snapshot, "browser").id],
            Wait::PipeWrite
        );
        assert_eq!(snapshot.waits[&named(&snapshot, "uniq").id], Wait::PipeRead);
        assert_eq!(
            named(&snapshot, "pipewire").io_detail,
            Measured::Pending,
            "B8 did not read it"
        );
        assert_eq!(
            named(&snapshot, "language-server").io_detail,
            Measured::Unreadable
        );
        // Loopback TCP is measured in both directions, and the pair's rate is their sum.
        let (worker, postgres) = (
            named(&snapshot, "worker").id,
            named(&snapshot, "postgres").id,
        );
        let out = snapshot.loopback_sent[&(worker, postgres)];
        let back = snapshot.loopback_sent[&(postgres, worker)];
        assert!(out.rate > back.rate && back.rate > 0.0);
        let pair = (worker.min(postgres), worker.max(postgres));
        assert!((snapshot.link_traffic[&pair] - (out.rate + back.rate)).abs() < 1.0);
    }

    #[test]
    fn a_bigger_demo_strings_the_rest_into_pipelines_of_four() {
        let small = demo_with(12.0, 192, PLUMBING);
        let big = demo_with(12.0, 1000, PLUMBING);
        // The first 192 processes are the same boiler room; only pipes are added.
        assert_eq!(
            format!("{:?}", small.pipes),
            format!(
                "{:?}",
                big.pipes
                    .iter()
                    .filter(|pipe| small.processes.iter().any(|p| p.id == pipe.writer))
                    .cloned()
                    .collect::<Vec<_>>()
            )
        );
        let extra = big.pipes.len() - small.pipes.len();
        assert_eq!(extra, (1000 - EXTRA_FROM) / CHAIN * (CHAIN - 1));
        // Every process of a pipeline holds a pipe end; the first six pipelines are measured and
        // the rest are not among the busiest.
        let (first, later) = (
            &big.processes[EXTRA_FROM + 1],
            &big.processes[EXTRA_FROM + 100],
        );
        assert!(matches!(first.io_detail, Measured::Known(_)));
        assert_eq!(later.io_detail, Measured::Pending);
        assert!(big.pipe_holds.contains_key(&later.id));
        assert_eq!(big.waits[&later.id], Wait::Other);
    }

    #[test]
    fn no_other_option_or_view_sees_any_of_it() {
        let plain = demo_with(12.0, 192, DemoOptions::NONE);
        assert!(plain.pipes.is_empty() && plain.waits.is_empty());
        assert!(plain.pipe_holds.is_empty() && plain.severed.is_empty());
        assert!(plain.loopback_sent.is_empty());
        assert!(!plain.sources.contains(Source::Pipes));
        assert_ne!(plain.processes[CAT].name, "cat");
    }
}
