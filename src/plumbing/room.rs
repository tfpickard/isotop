//! What one sample says of the pipework, before any geometry: the runs (pipes, Unix tubes, loopback
//! hoses), the pipes nobody reads, the outflow of every process that has a run and where each is
//! blocked. Pure functions of the snapshot and of which processes the frame shows.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::flow::{self, Outflow, Sockets};
use crate::model::{Hose, Identity, Process, Queue, Snapshot};
use crate::platform::{PipeId, Source, Wait};

/// The pipes and FIFOs from one process to another: one run, however many pipes it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeRun {
    pub writer: Identity,
    pub reader: Identity,
    /// The pipes of the pair, sorted: at least one.
    pub pipes: Vec<PipeId>,
    /// A FIFO opened for reading and writing at both ends: data runs both ways.
    pub two_way: bool,
    /// The bytes waiting in the pipes of the pair, where the platform tells for every one of them
    /// (macOS, anonymous pipes); None where it does not, which is not zero.
    pub buffered: Option<u64>,
}

impl PipeRun {
    /// How many of the pipes are FIFOs (named pipes).
    pub fn fifos(&self) -> usize {
        self.pipes
            .iter()
            .filter(|id| matches!(id, PipeId::Fifo { .. }))
            .count()
    }
}

/// What waits unread in one direction of a Unix tube, or that it was not measured.
pub type Lane = Option<Queue>;

/// A pair of processes joined by a Unix socket: a tube with two lanes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TubeRun {
    pub low: Identity,
    pub high: Identity,
    /// What `high` has not read of what `low` wrote, and the other way round.
    pub toward_high: Lane,
    pub toward_low: Lane,
}

/// A pair of processes joined by TCP over loopback: a hose.
#[derive(Clone, Debug, PartialEq)]
pub struct HoseRun {
    pub low: Identity,
    pub high: Identity,
    /// What `low` sent `high` and the other way round, measured exactly; None where no rate
    /// exists (the first scan, a platform without per-connection counters).
    pub toward_high: Option<Hose>,
    pub toward_low: Option<Hose>,
}

impl HoseRun {
    /// Whether any rate was measured.
    pub fn measured(&self) -> bool {
        self.toward_high.is_some() || self.toward_low.is_some()
    }
}

/// The pipework of a sample.
#[derive(Clone, Debug, Default)]
pub struct Room {
    pub pipes: Vec<PipeRun>,
    pub tubes: Vec<TubeRun>,
    pub hoses: Vec<HoseRun>,
    /// The pipes whose every reader is gone, by writer, among the shown processes.
    pub severed: Vec<(Identity, PipeId)>,
    /// The outflow of every process that has a run or a severed pipe.
    pub outflows: BTreeMap<Identity, Outflow>,
    /// Where each of them is blocked, where it was read.
    pub waits: BTreeMap<Identity, Wait>,
    /// How many runs touch each process: pipes, tubes, hoses and severed pipes.
    pub links: BTreeMap<Identity, u32>,
}

impl Room {
    /// The pipework between the processes in `shown`, from `snapshot`.
    pub fn read(snapshot: &Snapshot, shown: &HashSet<Identity>) -> Room {
        let mut room = Room::default();
        let mut pairs: BTreeMap<(Identity, Identity), PipeRun> = BTreeMap::new();
        for pipe in &snapshot.pipes {
            if !shown.contains(&pipe.reader) || !shown.contains(&pipe.writer) {
                continue;
            }
            let run = pairs
                .entry((pipe.writer, pipe.reader))
                .or_insert_with(|| PipeRun {
                    writer: pipe.writer,
                    reader: pipe.reader,
                    pipes: Vec::new(),
                    two_way: false,
                    buffered: Some(0),
                });
            run.pipes.push(pipe.id);
            run.two_way |= pipe.two_way;
            run.buffered = run.buffered.zip(pipe.buffered).map(|(sum, now)| sum + now);
        }
        room.pipes = pairs
            .into_values()
            .map(|mut run| {
                run.pipes.sort_unstable();
                run.pipes.dedup();
                run
            })
            .collect();
        room.severed = snapshot
            .severed
            .iter()
            .filter(|cut| shown.contains(&cut.writer))
            .map(|cut| (cut.writer, cut.id))
            .collect();

        let unix = |a: Identity, b: Identity| {
            snapshot.unix_links.contains(&(a, b)) || snapshot.unix_links.contains(&(b, a))
        };
        let traffic = |a: Identity, b: Identity| {
            snapshot.link_traffic.contains_key(&(a, b))
                || snapshot.link_traffic.contains_key(&(b, a))
        };
        let mut seen = BTreeSet::new();
        for &(a, b, _) in &snapshot.links {
            let (low, high) = (a.min(b), a.max(b));
            if low == high || !shown.contains(&low) || !shown.contains(&high) {
                continue;
            }
            if !seen.insert((low, high)) {
                continue;
            }
            if unix(low, high) {
                room.tubes.push(TubeRun {
                    low,
                    high,
                    toward_high: snapshot.unix_queues.get(&(high, low)).copied(),
                    toward_low: snapshot.unix_queues.get(&(low, high)).copied(),
                });
            }
            // A link that is not Unix is TCP, and over loopback it has rates once it was seen
            // twice; before that the hose is drawn, unmeasured.
            if !unix(low, high) || traffic(low, high) {
                room.hoses.push(HoseRun {
                    low,
                    high,
                    toward_high: snapshot.loopback_sent.get(&(low, high)).copied(),
                    toward_low: snapshot.loopback_sent.get(&(high, low)).copied(),
                });
            }
        }
        room.tubes.sort_by_key(|tube| (tube.low, tube.high));
        room.hoses.sort_by_key(|hose| (hose.low, hose.high));

        // Who is on which run.
        let mut touched: BTreeMap<Identity, u32> = BTreeMap::new();
        let mut touch = |id: Identity| *touched.entry(id).or_insert(0) += 1;
        for run in &room.pipes {
            touch(run.writer);
            touch(run.reader);
        }
        for tube in &room.tubes {
            touch(tube.low);
            touch(tube.high);
        }
        for hose in &room.hoses {
            touch(hose.low);
            touch(hose.high);
        }
        for &(writer, _) in &room.severed {
            touch(writer);
        }
        room.links = touched;

        let by_id: BTreeMap<Identity, &Process> = snapshot
            .processes
            .iter()
            .filter(|process| room.links.contains_key(&process.id))
            .map(|process| (process.id, process))
            .collect();
        for (&id, &process) in &by_id {
            room.outflows.insert(id, room.outflow_of(snapshot, process));
            if let Some(&wait) = snapshot.waits.get(&id) {
                room.waits.insert(id, wait);
            }
        }
        room
    }

    /// The outflow of `process`, from its descriptors and the connections drawn for it.
    fn outflow_of(&self, snapshot: &Snapshot, process: &Process) -> Outflow {
        let id = process.id;
        let mut sockets = Sockets {
            unix_pairs: self
                .tubes
                .iter()
                .filter(|tube| tube.low == id || tube.high == id)
                .count() as u32,
            ..Sockets::default()
        };
        for hose in self
            .hoses
            .iter()
            .filter(|hose| hose.low == id || hose.high == id)
        {
            let other = if hose.low == id { hose.high } else { hose.low };
            if !hose.measured() {
                sockets.unmeasured_connection = true;
                continue;
            }
            // What `id` sent is the rate of the ends its peer owns; the ends `id` owns are the
            // connections of the pair, counted from what the peer sent it.
            if let Some(sent) = snapshot.loopback_sent.get(&(id, other)) {
                sockets.loopback_sent += sent.rate;
            }
            if let Some(owned) = snapshot.loopback_sent.get(&(other, id)) {
                sockets.loopback_connections += owned.connections;
            }
        }
        let remotes: Vec<_> = snapshot
            .remotes
            .iter()
            .filter(|remote| remote.id == id)
            .collect();
        for remote in &remotes {
            match remote.up {
                Some(up) => {
                    sockets.remote_connections += 1;
                    sockets.remote_sent += up;
                }
                None => sockets.unmeasured_connection = true,
            }
        }
        if snapshot
            .outside
            .get(&id)
            .is_some_and(|&outside| outside > remotes.len() as u32)
        {
            sockets.unmeasured_connection = true;
        }
        let hold = snapshot.pipe_holds.get(&id).copied();
        let pipes_known =
            snapshot.sources.contains(Source::Pipes) && (hold.is_some() || !snapshot.pipes_partial);
        flow::outflow(&process.io_detail, hold, pipes_known, sockets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pipe, testing};

    fn who(pid: u32) -> Identity {
        Identity {
            pid,
            start: u64::from(pid),
        }
    }

    fn pipe(id: u64, writer: u32, reader: u32) -> Pipe {
        Pipe {
            id: PipeId::Anonymous(id),
            reader: who(reader),
            writer: who(writer),
            two_way: false,
            buffered: None,
            fifo: false,
        }
    }

    fn shown(pids: &[u32]) -> HashSet<Identity> {
        pids.iter().map(|&pid| who(pid)).collect()
    }

    fn snapshot(pids: &[u32]) -> Snapshot {
        Snapshot {
            processes: pids
                .iter()
                .map(|&pid| Process {
                    id: who(pid),
                    ..testing::process(pid, 1)
                })
                .collect(),
            sources: Source::Pipes.into(),
            ..Snapshot::default()
        }
    }

    #[test]
    fn pipes_join_writer_to_reader_and_an_rdwr_fifo_runs_both_ways() {
        let mut sample = snapshot(&[1, 2, 3, 4]);
        let fifo = PipeId::Fifo {
            device: 5,
            inode: 9,
        };
        sample.pipes = vec![
            pipe(10, 1, 2),
            pipe(11, 1, 2),
            pipe(12, 2, 3),
            Pipe {
                id: fifo,
                reader: who(3),
                writer: who(4),
                two_way: true,
                buffered: None,
                fifo: true,
            },
            // The reader of this one is not drawn: no run.
            pipe(13, 3, 99),
        ];
        let room = Room::read(&sample, &shown(&[1, 2, 3, 4]));
        assert_eq!(room.pipes.len(), 3, "one run for each writer and reader");
        let first = &room.pipes[0];
        assert_eq!((first.writer, first.reader), (who(1), who(2)));
        assert_eq!(first.pipes.len(), 2, "the inspector gives the pipe count");
        assert!(!first.two_way);
        let two_way = room.pipes.iter().find(|run| run.two_way).unwrap();
        assert_eq!((two_way.writer, two_way.reader), (who(4), who(3)));
        assert_eq!(two_way.fifos(), 1);
        assert_eq!(room.links[&who(2)], 2, "a reader and a writer");
        assert_eq!(room.links.get(&who(99)), None);
    }

    #[test]
    fn the_bytes_waiting_in_a_run_are_known_only_when_every_pipe_reports_them() {
        let mut sample = snapshot(&[1, 2, 3]);
        let counted = |id: u64, writer: u32, reader: u32, buffered: u64| Pipe {
            buffered: Some(buffered),
            ..pipe(id, writer, reader)
        };
        sample.pipes = vec![
            counted(10, 1, 2, 300),
            counted(11, 1, 2, 212),
            counted(12, 2, 3, 7),
            pipe(13, 2, 3),
        ];
        let room = Room::read(&sample, &shown(&[1, 2, 3]));
        assert_eq!(room.pipes[0].buffered, Some(512));
        assert_eq!(
            room.pipes[1].buffered, None,
            "one pipe that does not say is not zero"
        );
    }

    #[test]
    fn unix_links_become_tubes_with_a_lane_each_way_and_other_links_hoses() {
        let mut sample = snapshot(&[1, 2, 3]);
        sample.links = vec![
            (who(1), who(2), 1),
            (who(3), who(2), 1),
            (who(1), who(3), 2),
        ];
        sample.unix_links = HashSet::from([(who(1), who(2))]);
        // 2 reads 100 bytes 1 wrote; nothing is known of the other lane.
        sample.unix_queues = std::collections::HashMap::from([(
            (who(2), who(1)),
            Queue {
                unread: 100,
                datagram_waiting: false,
            },
        )]);
        sample.link_traffic.insert((who(1), who(3)), 50.0);
        sample.loopback_sent.insert(
            (who(1), who(3)),
            Hose {
                rate: 30.0,
                connections: 2,
            },
        );
        sample.loopback_sent.insert(
            (who(3), who(1)),
            Hose {
                rate: 20.0,
                connections: 2,
            },
        );
        let room = Room::read(&sample, &shown(&[1, 2, 3]));
        assert_eq!(room.tubes.len(), 1);
        let tube = &room.tubes[0];
        assert_eq!((tube.low, tube.high), (who(1), who(2)));
        assert_eq!(tube.toward_high.map(|queue| queue.unread), Some(100));
        assert_eq!(
            tube.toward_low, None,
            "not measured, which is not an empty tube"
        );
        assert_eq!(room.hoses.len(), 2);
        let measured = &room.hoses[0];
        assert_eq!((measured.low, measured.high), (who(1), who(3)));
        assert_eq!(measured.toward_high.map(|hose| hose.rate), Some(30.0));
        assert_eq!(measured.toward_low.map(|hose| hose.rate), Some(20.0));
        let unmeasured = &room.hoses[1];
        assert_eq!((unmeasured.low, unmeasured.high), (who(2), who(3)));
        assert!(
            !unmeasured.measured(),
            "TCP seen once: a hose with no rates"
        );
    }

    #[test]
    fn a_pair_joined_by_a_unix_socket_and_tcp_has_a_tube_and_a_hose() {
        let mut sample = snapshot(&[1, 2]);
        sample.links = vec![(who(1), who(2), 3)];
        sample.unix_links = HashSet::from([(who(1), who(2))]);
        sample.link_traffic.insert((who(1), who(2)), 10.0);
        let room = Room::read(&sample, &shown(&[1, 2]));
        assert_eq!((room.tubes.len(), room.hoses.len()), (1, 1));
    }

    #[test]
    fn processes_outside_the_frame_have_no_runs() {
        let mut sample = snapshot(&[1, 2]);
        sample.pipes = vec![pipe(1, 1, 2)];
        sample.links = vec![(who(1), who(2), 1)];
        let room = Room::read(&sample, &shown(&[1]));
        assert!(room.pipes.is_empty() && room.hoses.is_empty() && room.links.is_empty());
    }

    #[test]
    fn severed_pipes_belong_to_their_writers_and_count_as_a_link() {
        let mut sample = snapshot(&[1, 2]);
        sample.severed = vec![crate::model::Severed {
            writer: who(1),
            id: PipeId::Anonymous(5),
        }];
        let room = Room::read(&sample, &shown(&[1, 2]));
        assert_eq!(room.severed, [(who(1), PipeId::Anonymous(5))]);
        assert_eq!(room.links.get(&who(1)), Some(&1));
        assert_eq!(room.links.get(&who(2)), None);
    }

    #[test]
    fn the_wait_channel_is_kept_for_the_processes_with_runs() {
        let mut sample = snapshot(&[1, 2, 3]);
        sample.pipes = vec![pipe(1, 1, 2)];
        sample.waits.insert(who(1), Wait::PipeWrite);
        sample.waits.insert(who(3), Wait::PipeRead);
        let room = Room::read(&sample, &shown(&[1, 2, 3]));
        assert_eq!(room.waits.get(&who(1)), Some(&Wait::PipeWrite));
        assert_eq!(
            room.waits.get(&who(2)),
            None,
            "not read, which is not 'Other'"
        );
        assert_eq!(room.waits.get(&who(3)), None, "no run, no tank");
    }

    #[test]
    fn an_unmeasured_connection_makes_the_outflow_unmeasured() {
        let mut sample = snapshot(&[1, 2]);
        sample.pipes = vec![pipe(1, 1, 2)];
        sample.outside.insert(who(1), 2);
        let room = Room::read(&sample, &shown(&[1, 2]));
        // The process has no I/O detail at all, which comes first; with one it would be the
        // connections nothing has measured.
        assert!(matches!(
            room.outflows[&who(1)],
            Outflow::Unmeasured(flow::Why::NotRead)
        ));
    }
}
