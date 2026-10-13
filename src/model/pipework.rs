//! What the plumbing view reads of pipes and sockets beyond the fields every view shares: what
//! each process holds of the pipes the scan listed (`PipeHold`), the pipes nobody reads any more
//! (`Severed`), and what one process sent another over loopback TCP, per direction (`Hose`). The
//! wait channels (`Snapshot::waits`) are `platform::Wait`s. Everything here is read on the
//! background thread, with the pipe scan (`Source::Pipes`) or the socket scan, and joined to a
//! sample by `Identity` like them; `demo` synthesizes it for `DemoOptions::plumbing`.

pub(super) mod demo;

use std::collections::{BTreeMap, HashMap};

use super::Identity;
use crate::platform::{AccessMode, PipeEnd, PipeId};

/// The most writers of one pipe that `roles` lists as severed, the lowest identities, as
/// `pair_pipes` keeps to the same bound for the writers it pairs.
const SEVERED_FAN_OUT: usize = super::PIPE_FAN_OUT;

/// How many distinct pipes and FIFOs a process holds an end of, by what its descriptors may do.
/// A FIFO opened for reading and writing, and a pipe whose two ends the process both holds, count
/// in both. Several descriptors of one pipe count once.
///
/// Present for a process the pipe scan reached that holds at least one pipe; a process that is
/// absent holds none, or was not reached (`Snapshot::pipes_partial`) or could not be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipeHold {
    pub writes: u32,
    pub reads: u32,
}

/// A pipe or FIFO that `writer` may write and that no process the scan read holds a read end of:
/// every reader has closed it or exited, so the next write fails with `EPIPE` (and `SIGPIPE`,
/// unless the writer ignores it). A reader that belongs to a process the scan could not read, or
/// had not reached, looks the same, so a view that says the reader is *gone* waits to see the pipe
/// with a reader first (`Pipe::id`), then without one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Severed {
    pub writer: Identity,
    pub id: PipeId,
}

/// What `sender` sent `receiver` over loopback TCP, summed over their connections: the exact
/// byte rate of the sockets it was measured on, and how many connections that is. The rate is
/// what `receiver`'s ends received from `sender` since the scan that last saw them, so the same
/// connections give the rate in each direction; it is absent where nothing could be measured
/// (the first scan, a platform without per-connection counters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hose {
    pub rate: f32,
    pub connections: u32,
}

/// What each process holds of the pipes in `held`, and which pipes no reader holds, from the
/// descriptors each process holds (as `pair_pipes` takes them). `holds` has an entry for every
/// process that holds at least one pipe end; `severed` is sorted.
pub(super) fn roles<'a>(
    held: impl IntoIterator<Item = (Identity, &'a [PipeEnd])>,
) -> (HashMap<Identity, PipeHold>, Vec<Severed>) {
    let mut holders: BTreeMap<PipeId, BTreeMap<Identity, AccessMode>> = BTreeMap::new();
    for (holder, ends) in held {
        for end in ends {
            holders
                .entry(end.id)
                .or_default()
                .entry(holder)
                .and_modify(|access| *access = access.union(end.access))
                .or_insert(end.access);
        }
    }
    let mut holds: HashMap<Identity, PipeHold> = HashMap::new();
    let mut severed = Vec::new();
    for (&id, holders) in &holders {
        let read = holders.values().any(|access| access.reads());
        let mut writers = 0;
        for (&holder, access) in holders {
            let hold = holds.entry(holder).or_default();
            hold.reads += u32::from(access.reads());
            if access.writes() {
                hold.writes += 1;
                writers += 1;
                if !read && writers <= SEVERED_FAN_OUT {
                    severed.push(Severed { writer: holder, id });
                }
            }
        }
    }
    severed.sort();
    (holds, severed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(pid: u32) -> Identity {
        Identity {
            pid,
            start: u64::from(pid),
        }
    }

    fn end(id: u64, access: AccessMode) -> PipeEnd {
        PipeEnd {
            id: PipeId::Anonymous(id),
            access,
            buffered: None,
        }
    }

    #[test]
    fn a_process_holds_each_pipe_once_whatever_the_descriptors_it_has_of_it() {
        let both = [
            end(7, AccessMode::Read),
            end(7, AccessMode::Write),
            end(8, AccessMode::Write),
            end(8, AccessMode::Write),
        ];
        let (holds, severed) = roles([(who(1), &both[..])]);
        assert_eq!(
            holds[&who(1)],
            PipeHold {
                writes: 2,
                reads: 1
            },
            "pipe 7 is read and written through two descriptors, pipe 8 only written, twice"
        );
        assert_eq!(
            severed,
            [Severed {
                writer: who(1),
                id: PipeId::Anonymous(8)
            }],
            "pipe 7's reader is the process itself; pipe 8 has none"
        );
    }

    #[test]
    fn a_pipe_with_a_reader_is_not_severed_and_one_without_is_listed_for_each_writer() {
        let writer = [end(5, AccessMode::Write), end(6, AccessMode::Write)];
        let shell = [end(6, AccessMode::Write)];
        let reader = [end(5, AccessMode::Read)];
        let (holds, severed) = roles([
            (who(2), &writer[..]),
            (who(3), &shell[..]),
            (who(4), &reader[..]),
        ]);
        assert_eq!(
            severed,
            [
                Severed {
                    writer: who(2),
                    id: PipeId::Anonymous(6)
                },
                Severed {
                    writer: who(3),
                    id: PipeId::Anonymous(6)
                },
            ]
        );
        assert_eq!(
            holds[&who(4)],
            PipeHold {
                writes: 0,
                reads: 1
            }
        );
        assert!(
            !holds.contains_key(&who(9)),
            "a process with none is absent"
        );
    }

    #[test]
    fn a_fifo_and_a_pipe_with_one_number_are_two_pipes() {
        let fifo = PipeEnd {
            id: PipeId::Fifo {
                device: 1,
                inode: 5,
            },
            access: AccessMode::Write,
            buffered: None,
        };
        let mixed = [fifo, end(5, AccessMode::Write)];
        let (holds, severed) = roles([(who(1), &mixed[..])]);
        assert_eq!(holds[&who(1)].writes, 2);
        assert_eq!(severed.len(), 2);
    }

    #[test]
    fn severed_writers_of_one_pipe_are_capped_at_the_lowest_identities() {
        let ends = [end(1, AccessMode::Write)];
        let many = SEVERED_FAN_OUT as u32 + 10;
        let (_, severed) = roles((1..=many).map(|pid| (who(pid), &ends[..])));
        assert_eq!(severed.len(), SEVERED_FAN_OUT);
        assert_eq!(severed.last().unwrap().writer, who(SEVERED_FAN_OUT as u32));
    }
}
