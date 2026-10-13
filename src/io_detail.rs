//! Where the busiest processes' I/O goes (`Source::IoDetail`): which open regular files they read
//! and write, and how many pipe, socket, terminal and anonymous descriptors they hold besides.
//! What a platform reads of one process (`DescriptorReader`), how a pass walks the processes
//! within its caps and turns two readings into rates (`IoScan`), the processes it walks
//! (`select`), and the readings the demo synthesizes (`demo_detail`). The background thread runs a
//! pass only while a shown view demands `Source::IoDetail`.
//!
//! **Method.** An open file description keeps a position that every `read`, `write`, `readv` and
//! `writev` on it advances by the bytes they moved. Two readings of a descriptor's position, one
//! pass apart, therefore bound what was read or written through it in between: the growth of the
//! position, routed by the descriptor's access mode (`attribute`).
//!
//! **What this cannot see, and says so.** A call that names its own offset (`pread`, `pwrite`,
//! `preadv`, `pwritev`, `sendfile` given an offset, io_uring, AIO) moves no position, and
//! memory-mapped I/O moves none; pipes, sockets and terminals have none to move. That I/O is not
//! assigned to a file. It stays in `IoDetail::unattributed`, the part of the process's own read
//! and write byte counters that no file position explains. A file's rate is therefore a lower
//! bound on what the process did with it. A descriptor is not credited at all when its position
//! says nothing about bytes: one opened `O_APPEND`, which always sits at the end of the file
//! whoever appended, and a position past the end of the file (a seek, or a file such as those of
//! `/proc` whose offset counts records).
//!
//! **Never more than was done.** Positions move on `lseek` too. Where the platform counts the
//! bytes of read and write calls (Linux: `rchar` and `wchar` of `/proc/<pid>/io`), the claims of
//! all a process's files are held to the growth of those counters across the same interval, scaled
//! down in proportion where they exceed it. Where it does not (macOS), nothing bounds a claim but
//! the file's size, `Source::UnattributedIo` is missing, and `IoDetail::unattributed` is None.

mod attribute;
mod demo;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::model::{Identity, Kind, Measured, Process, top_n};
use crate::platform::{self, AccessMode};

pub use attribute::{Limit, attribute, unmeasured};
pub use demo::demo_detail;

/// How many processes a pass reads: the busiest by `platform::io_rank`.
pub const IO_PROCESSES: usize = 24;

/// Regular files one process records at most. A process with more is cut short and `partial`.
pub const FILES_PER_PROCESS: usize = 256;

/// Descriptors one pass examines at most, over all processes; the same bound as an open-file scan
/// of one process. A process the pass runs out of budget in is cut short and `partial`, and every
/// process after it is not read.
pub const DESCRIPTORS_PER_PASS: usize = 4096;

/// What one pass may read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub per_pass: usize,
    pub per_process: usize,
}

impl Caps {
    pub const DEFAULT: Caps = Caps {
        per_pass: DESCRIPTORS_PER_PASS,
        per_process: FILES_PER_PROCESS,
    };
}

/// Where one process's I/O goes, as of the last pass that read it.
///
/// The rates are in bytes per second over the interval between that pass and the one before. A
/// file's rate is a lower bound on what the process did with it, since a position cannot see all
/// I/O (the module documentation says which); `unattributed` holds what the positions cannot
/// explain.
///
/// Text from the system is stored raw: `FileFlow::path` is untrusted, so whoever shows it makes
/// it a `terminal::SafeText`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IoDetail {
    /// The regular files the process holds open, one entry for a file held through several
    /// descriptors, sorted by (device, inode). At most `FILES_PER_PROCESS` of them.
    pub files: Vec<FileFlow>,
    /// The descriptors it holds that are pipes, sockets, terminals (`/dev/pts/*`, `/dev/tty*`,
    /// `/dev/console`) and anonymous kernel objects (`anon_inode:*` on Linux: epoll, eventfd,
    /// signalfd; kqueues and shared memory on macOS), and every other one that is not a regular
    /// file: directories, devices, named pipes, memfds and descriptors that could not be read.
    /// Descriptors, not distinct objects.
    pub pipes: u32,
    pub sockets: u32,
    pub ttys: u32,
    pub anonymous: u32,
    pub other: u32,
    /// What its read and write calls moved that no file position explains; None where the
    /// platform cannot say (`Source::UnattributedIo` is then missing), and on a first reading.
    pub unattributed: Option<Unattributed>,
    /// A cap cut the reading short: the process holds more regular files than `files` lists, or
    /// the pass ran out of descriptors to examine, or a descriptor could not be read. `files` is
    /// then a subset and the counts are lower bounds.
    pub partial: bool,
}

/// One regular file a process holds open.
#[derive(Clone, Debug, PartialEq)]
pub struct FileFlow {
    pub device: u64,
    pub inode: u64,
    /// Its size in bytes at the last reading.
    pub size: u64,
    /// Where its descriptor links to, raw. A trailing ` (deleted)` is taken off when the file has
    /// no link left (`deleted`); a name that merely ends so is left alone.
    pub path: String,
    /// The file was deleted while open: its space stays in use until every descriptor closes.
    pub deleted: bool,
    /// What the process may do with it: the union over its descriptors of the file.
    pub access: AccessMode,
    pub flow: Flow,
}

/// What the positions of a file's descriptors say about its I/O over the last interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flow {
    /// Nothing is known, which is not zero: no descriptor of the file has an earlier reading of
    /// the same descriptor to compare with (a first pass, a descriptor opened since, closed and
    /// reopened or changed in its flags, or moved to another file), or the position went
    /// backwards (a rewind), or the platform's byte counters said the pass could not be trusted.
    Unmeasured,
    /// Its descriptors' positions say nothing of bytes: opened `O_APPEND`, or a position past the
    /// end of the file. Whatever the process wrote to it is in the unattributed remainder.
    Unpositioned,
    /// The positions of its descriptors advanced by these bytes per second, routed by access
    /// mode and held to the process's own byte counters where the platform has them. `read` is
    /// what read-only descriptors advanced, `written` what write-only ones did, and `either`
    /// what read-write ones did: a read-write descriptor's bytes are never split between the two.
    /// A descriptor with no earlier reading adds nothing.
    Measured {
        read: f32,
        written: f32,
        either: f32,
    },
}

/// The part of a process's read and write calls that no file position explains, in bytes per
/// second: the growth of its `rchar` and `wchar` over the interval, less what its files claimed.
///
/// It includes the I/O of pipes, sockets and terminals, which have no file position, and of calls
/// that name their own offset (`pread`, `pwrite`), memory-mapped I/O, and files whose position
/// says nothing (`Flow::Unpositioned`). Bytes the process moved through a read-write descriptor
/// (`either`) could have been reads or writes, and the split is not guessed: `read` and `write`
/// are what remains after the read-only and write-only descriptors, and `either` is how much of
/// `read + write` the read-write descriptors account for. What nothing explains is
/// `read + write - either`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unattributed {
    pub read: f32,
    pub write: f32,
    pub either: f32,
}

/// The cumulative bytes a process has passed to read and write calls: `rchar` and `wchar`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Characters {
    pub read: u64,
    pub write: u64,
}

/// One regular file as a descriptor of a process held it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFile {
    /// The descriptor number.
    pub fd: u32,
    pub device: u64,
    pub inode: u64,
    /// The file's size in bytes, read no earlier than the position, so that a position that
    /// counts bytes never exceeds it.
    pub size: u64,
    /// The link target, raw, without a ` (deleted)` suffix.
    pub path: String,
    pub deleted: bool,
    pub access: AccessMode,
    /// The descriptor was opened `O_APPEND`.
    pub appending: bool,
    /// The file offset in bytes.
    pub position: u64,
}

/// The descriptors of a process that are not regular files, by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub pipes: u32,
    pub sockets: u32,
    pub ttys: u32,
    pub anonymous: u32,
    pub other: u32,
}

/// What one reading of a process's descriptors found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub files: Vec<OpenFile>,
    pub counts: Counts,
    /// The byte counters read just before the walk and just after it, so that the interval they
    /// bound contains the interval of the positions: the growth from the earlier reading's first
    /// counters to this reading's last ones is at least what happened between the two walks.
    /// None where the platform has no such counters or they could not be read.
    pub characters: Option<(Characters, Characters)>,
    /// How many descriptors the reading examined, which the pass budget counts.
    pub examined: usize,
    /// A cap cut the reading short, or a descriptor could not be read.
    pub partial: bool,
}

/// What a pass lets a reader spend on one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Descriptors it may examine; past them the reading is partial.
    pub descriptors: usize,
    /// Regular files it may record; past them the reading is partial.
    pub files: usize,
}

/// What reading one process came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading {
    /// It has exited, or its pid now names another process.
    Gone,
    /// The platform refused, normally for another user's process.
    Unreadable,
    Taken(Held),
}

/// Reads the descriptors of one process; each platform provides one as `platform::Descriptors`.
pub trait DescriptorReader {
    /// Whether this platform counts the bytes of read and write calls, so that a process's claims
    /// can be bounded by them and what remains can be told (`Held::characters`). When it does
    /// not, `Source::UnattributedIo` is missing.
    fn counts_bytes(&self) -> bool;

    /// Reads `id`'s descriptors within `budget`. The reading is of the process `id` names, or
    /// `Gone`: a pid that now belongs to another process is not read.
    fn read(&mut self, id: Identity, budget: Budget) -> Reading;
}

/// What the previous pass saw of a process.
struct Seen {
    held: Held,
    /// When its reading began, before the first byte counter was read.
    began: Instant,
}

/// The pass state kept on the background thread between cycles: the previous reading of each
/// process it reads.
#[derive(Default)]
pub struct IoScan {
    seen: HashMap<Identity, Seen>,
    /// The process the last pass stopped before, because its descriptor budget ran out. The next
    /// pass starts there, so a process with thousands of descriptors cannot keep every later
    /// target unread.
    resume: Option<Identity>,
    /// The latest result for each target, handed out again for a target a pass did not reach.
    last: HashMap<Identity, Measured<Arc<IoDetail>>>,
}

impl IoScan {
    /// Reads the descriptors of `targets` within `Caps::DEFAULT`, in the order given.
    pub fn pass(
        &mut self,
        targets: &[Identity],
        reader: &mut impl DescriptorReader,
    ) -> HashMap<Identity, Measured<Arc<IoDetail>>> {
        self.pass_with(targets, reader, Caps::DEFAULT, Instant::now)
    }

    /// A pass with its own caps and clock. `targets` come in the order the caller wants them
    /// read (the frame thread decides it, so the pass never sorts), and the pass stops when
    /// `caps.per_pass` descriptors are examined: the process it stops in is cut short, and every
    /// process after it is not read; the next pass starts at the first of those (`resume`), and
    /// meanwhile such a process keeps its latest result, or has none if it never had one. A
    /// process that is gone is absent from the result and forgotten.
    ///
    /// What the pass remembers of a process it did not reach is kept, so the next reading
    /// compares with the last one that exists over the longer interval between them. A process
    /// that is no longer a target is forgotten.
    pub fn pass_with(
        &mut self,
        targets: &[Identity],
        reader: &mut impl DescriptorReader,
        caps: Caps,
        mut clock: impl FnMut() -> Instant,
    ) -> HashMap<Identity, Measured<Arc<IoDetail>>> {
        let counted = reader.counts_bytes();
        let mut remaining = caps.per_pass;
        let mut found = HashMap::with_capacity(targets.len());
        let start = self
            .resume
            .and_then(|resume| targets.iter().position(|&id| id == resume))
            .unwrap_or(0);
        self.resume = None;
        let order = targets[start..].iter().chain(&targets[..start]);
        for &id in order {
            if remaining == 0 {
                self.resume = Some(id);
                break;
            }
            let began = clock();
            let budget = Budget {
                descriptors: remaining,
                files: caps.per_process,
            };
            match reader.read(id, budget) {
                Reading::Gone => {
                    self.seen.remove(&id);
                    self.last.remove(&id);
                }
                Reading::Unreadable => {
                    self.seen.remove(&id);
                    found.insert(id, Measured::Unreadable);
                }
                Reading::Taken(held) => {
                    remaining = remaining.saturating_sub(held.examined);
                    let ended = clock();
                    let earlier = self.seen.get(&id);
                    let detail = detail(earlier, &held, ended, counted);
                    self.seen.insert(id, Seen { held, began });
                    found.insert(id, Measured::Known(Arc::new(detail)));
                }
            }
        }
        self.seen.retain(|id, _| targets.contains(id));
        self.last.retain(|id, _| targets.contains(id));
        for &id in targets {
            match found.get(&id) {
                Some(result) => {
                    self.last.insert(id, result.clone());
                }
                None => {
                    if let Some(result) = self.last.get(&id) {
                        found.insert(id, result.clone());
                    }
                }
            }
        }
        found
    }

    /// Forgets every reading, for a cycle that read none: the next pass starts again at nothing
    /// measured rather than averaging over the gap.
    pub fn forget(&mut self) {
        self.seen.clear();
        self.last.clear();
        self.resume = None;
    }

    /// Only tests ask: whether any reading is remembered.
    #[cfg(test)]
    pub fn remembers_nothing(&self) -> bool {
        self.seen.is_empty()
    }
}

/// The detail of the reading `held`, which ended at `ended`, against the `earlier` reading of the
/// same process. `counted`: the platform counts the bytes of read and write calls.
fn detail(earlier: Option<&Seen>, held: &Held, ended: Instant, counted: bool) -> IoDetail {
    let interval = earlier.and_then(|seen| {
        let seconds = ended.checked_duration_since(seen.began)?.as_secs_f64();
        (seconds > 0.0).then_some((seen, seconds))
    });
    let attributed = match interval {
        Some((seen, seconds)) => {
            let limit = match (counted, cover(&seen.held, held)) {
                (false, _) => Limit::Unbounded,
                (true, Some(cover)) => Limit::To(cover),
                // The platform counts, but not across this interval: nothing to hold claims to.
                (true, None) => Limit::Withheld,
            };
            attribute(&seen.held, held, seconds, limit)
        }
        None => unmeasured(held),
    };
    IoDetail {
        files: attributed.files,
        pipes: held.counts.pipes,
        sockets: held.counts.sockets,
        ttys: held.counts.ttys,
        anonymous: held.counts.anonymous,
        other: held.counts.other,
        unattributed: attributed.unattributed,
        partial: held.partial,
    }
}

/// The growth of the byte counters across the interval of the two readings: from the counters
/// read before the earlier walk to those read after the later one, which contains the time
/// between the two walks. None when either reading has no counters or one went backwards.
fn cover(earlier: &Held, later: &Held) -> Option<attribute::Cover> {
    let (before, _) = earlier.characters?;
    let (_, after) = later.characters?;
    Some(attribute::Cover {
        read: after.read.checked_sub(before.read)?,
        write: after.write.checked_sub(before.write)?,
    })
}

/// How busy `process` is by its syscalls, for choosing which get an `IoDetail` on Linux: calls to
/// read and write per second. None until the process has a reading to take a rate from.
pub fn by_syscall_rate(process: &Process) -> Option<f64> {
    let io = process.syscall_io?;
    Some(f64::from(io.reads) + f64::from(io.writes))
}

/// The processes to read, busiest first by `rank`: at most `IO_PROCESSES`, ties to the smaller
/// `Identity`. A process with no rank, no I/O at all, no descriptors of its own (kernel threads)
/// or no longer running (zombies) is never chosen, so a quiet machine reads few.
pub fn select(processes: &[Process], rank: impl Fn(&Process) -> Option<f64>) -> Vec<Identity> {
    let key = |process: &Process| match rank(process) {
        Some(busy) if busy > 0.0 && process.kind != Kind::Kernel && process.state != 'Z' => busy,
        _ => f64::NAN,
    };
    top_n(processes, IO_PROCESSES, key)
        .into_iter()
        .filter(|process| !key(process).is_nan())
        .map(|process| process.id)
        .collect()
}

/// The processes the background thread reads while `Source::IoDetail` is demanded, busiest first
/// by this platform's measure.
pub fn targets(processes: &[Process]) -> Vec<Identity> {
    select(processes, platform::io_rank)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing;
    use crate::terminal::SafeText;
    use std::collections::VecDeque;
    use std::time::Duration;

    fn id(pid: u32) -> Identity {
        Identity { pid, start: 1 }
    }

    fn file(fd: u32, inode: u64, access: AccessMode, position: u64) -> OpenFile {
        OpenFile {
            fd,
            device: 7,
            inode,
            size: 1 << 40,
            path: format!("/data/file-{inode}"),
            deleted: false,
            access,
            appending: false,
            position,
        }
    }

    /// A reading of `files` that bracketed the walk with the byte counters `read` and `write`.
    fn held(files: Vec<OpenFile>, read: u64, write: u64) -> Held {
        let counters = Characters { read, write };
        Held {
            examined: files.len(),
            files,
            characters: Some((counters, counters)),
            ..Held::default()
        }
    }

    /// Answers each process with the readings it was given, in turn, and records what it was asked.
    struct Fake {
        readings: HashMap<u32, VecDeque<Reading>>,
        asked: Vec<(u32, Budget)>,
        counted: bool,
    }

    impl Fake {
        fn new(readings: impl IntoIterator<Item = (u32, Vec<Reading>)>) -> Self {
            Fake {
                readings: readings
                    .into_iter()
                    .map(|(pid, all)| (pid, all.into()))
                    .collect(),
                asked: Vec::new(),
                counted: true,
            }
        }
    }

    impl DescriptorReader for Fake {
        fn counts_bytes(&self) -> bool {
            self.counted
        }

        fn read(&mut self, id: Identity, budget: Budget) -> Reading {
            self.asked.push((id.pid, budget));
            self.readings
                .get_mut(&id.pid)
                .and_then(VecDeque::pop_front)
                .unwrap_or(Reading::Gone)
        }
    }

    /// A clock that moves one second per call from a fixed start.
    fn seconds() -> impl FnMut() -> Instant {
        let mut now = Instant::now();
        move || {
            now += Duration::from_secs(1);
            now
        }
    }

    fn detail_of(found: &HashMap<Identity, Measured<Arc<IoDetail>>>, pid: u32) -> Arc<IoDetail> {
        Arc::clone(found[&id(pid)].known().expect("the process was read"))
    }

    #[test]
    fn a_pass_attributes_the_growth_of_a_position_over_the_interval_between_passes() {
        let mut scan = IoScan::default();
        let targets = [id(5)];
        let mut fake = Fake::new([(
            5,
            vec![
                Reading::Taken(held(vec![file(3, 10, AccessMode::Read, 1000)], 500, 50)),
                Reading::Taken(held(vec![file(3, 10, AccessMode::Read, 7000)], 9500, 450)),
            ],
        )]);
        let mut clock = seconds();
        // The first pass has nothing to compare with.
        let first = scan.pass_with(&targets, &mut fake, Caps::DEFAULT, &mut clock);
        let first = detail_of(&first, 5);
        assert_eq!(first.files[0].flow, Flow::Unmeasured);
        assert_eq!(first.unattributed, None);
        // The clock read 1 s and 2 s around the first reading, and 3 s and 4 s around the second:
        // the interval from the start of the first to the end of the second is three seconds, and
        // the counters bracket it, from the first reading's first to the second's last.
        let second = scan.pass_with(&targets, &mut fake, Caps::DEFAULT, &mut clock);
        let second = detail_of(&second, 5);
        assert_eq!(
            second.files[0].flow,
            Flow::Measured {
                read: 2000.0,
                written: 0.0,
                either: 0.0
            }
        );
        let left = second.unattributed.unwrap();
        // 9000 read and 400 written in three seconds, of which 6000 are in the file.
        assert_eq!(
            (left.read, left.write, left.either),
            (1000.0, (400.0_f64 / 3.0) as f32, 0.0)
        );
    }

    #[test]
    fn a_pass_without_counters_the_platform_has_withholds_rather_than_guesses() {
        let reading = |position, characters| {
            let mut taken = held(vec![file(3, 10, AccessMode::Read, position)], 0, 0);
            taken.characters = characters;
            Reading::Taken(taken)
        };
        let mut fake = Fake::new([(
            5,
            vec![reading(0, None), reading(4096, None), reading(8192, None)],
        )]);
        let mut scan = IoScan::default();
        let mut clock = seconds();
        scan.pass_with(&[id(5)], &mut fake, Caps::DEFAULT, &mut clock);
        // The platform counts bytes, but this reading has none: nothing can vouch for a claim.
        let second = scan.pass_with(&[id(5)], &mut fake, Caps::DEFAULT, &mut clock);
        let second = detail_of(&second, 5);
        assert_eq!(second.files[0].flow, Flow::Unmeasured);
        assert_eq!(second.unattributed, None);
        // A platform that counts none (macOS) claims whole and says nothing of the rest.
        fake.counted = false;
        let third = scan.pass_with(&[id(5)], &mut fake, Caps::DEFAULT, &mut clock);
        let third = detail_of(&third, 5);
        assert!(matches!(third.files[0].flow, Flow::Measured { read, .. } if read > 0.0));
        assert_eq!(third.unattributed, None);
    }

    #[test]
    fn counters_that_went_backwards_vouch_for_nothing() {
        let mut fake = Fake::new([(
            5,
            vec![
                Reading::Taken(held(vec![file(3, 10, AccessMode::Read, 0)], 9000, 0)),
                Reading::Taken(held(vec![file(3, 10, AccessMode::Read, 500)], 100, 0)),
            ],
        )]);
        let mut scan = IoScan::default();
        let mut clock = seconds();
        scan.pass_with(&[id(5)], &mut fake, Caps::DEFAULT, &mut clock);
        let second = scan.pass_with(&[id(5)], &mut fake, Caps::DEFAULT, &mut clock);
        let second = detail_of(&second, 5);
        assert_eq!(second.files[0].flow, Flow::Unmeasured);
        assert_eq!(second.unattributed, None);
    }

    #[test]
    fn caps_mark_the_detail_partial() {
        // Each process lists 4 descriptors; the pass may examine 10 and each process 2 files.
        let reading = |examined, partial| {
            let mut taken = held(vec![file(3, 10, AccessMode::Read, 0)], 0, 0);
            taken.examined = examined;
            taken.partial = partial;
            Reading::Taken(taken)
        };
        let mut fake = Fake::new([
            (1, vec![reading(4, false)]),
            (2, vec![reading(4, false)]),
            (3, vec![reading(2, true)]),
            (4, vec![reading(4, false)]),
        ]);
        let caps = Caps {
            per_pass: 10,
            per_process: 2,
        };
        let targets = [id(1), id(2), id(3), id(4)];
        let found = IoScan::default().pass_with(&targets, &mut fake, caps, Instant::now);
        let asked: Vec<(u32, usize, usize)> = fake
            .asked
            .iter()
            .map(|(pid, budget)| (*pid, budget.descriptors, budget.files))
            .collect();
        // Each process gets what the pass has left, and the files cap of its own; a process the
        // budget ran out in is cut short (its reader said so), and the one after is not read.
        assert_eq!(asked, [(1, 10, 2), (2, 6, 2), (3, 2, 2)]);
        assert!(!detail_of(&found, 1).partial && !detail_of(&found, 2).partial);
        assert!(detail_of(&found, 3).partial);
        assert!(
            !found.contains_key(&id(4)),
            "not reached, so Pending with the caller"
        );
    }

    #[test]
    fn a_process_the_pass_did_not_reach_keeps_its_earlier_reading_for_the_next() {
        let at = |position| {
            let mut taken = held(vec![file(3, 10, AccessMode::Read, position)], 0, 0);
            taken.characters = Some((
                Characters {
                    read: position,
                    write: 0,
                },
                Characters {
                    read: position,
                    write: 0,
                },
            ));
            taken.examined = 6;
            Reading::Taken(taken)
        };
        let mut fake = Fake::new([
            (1, vec![at(0), at(100), at(200)]),
            (2, vec![at(0), at(1000)]),
        ]);
        let mut scan = IoScan::default();
        let mut clock = seconds();
        let both = [id(1), id(2)];
        let first = scan.pass_with(&both, &mut fake, Caps::DEFAULT, &mut clock);
        // Room for one process only: the second is not reached.
        let short = Caps {
            per_pass: 6,
            per_process: FILES_PER_PROCESS,
        };
        let second = scan.pass_with(&both, &mut fake, short, &mut clock);
        assert!(second.contains_key(&id(1)));
        // Not reached, it is handed out with the result it had, not as unread.
        assert_eq!(second[&id(2)], first[&id(2)]);
        // Reached again, it compares with the reading from two passes ago, over the longer time.
        let third = scan.pass_with(&[id(2)], &mut fake, Caps::DEFAULT, &mut clock);
        let flow = detail_of(&third, 2).files[0].flow;
        assert!(
            matches!(flow, Flow::Measured { read, .. } if read > 0.0),
            "{flow:?}"
        );
        // Process 1 stopped being a target, so what was kept of it is gone: it starts afresh.
        let fourth = scan.pass_with(&both, &mut fake, Caps::DEFAULT, &mut clock);
        assert_eq!(detail_of(&fourth, 1).files[0].flow, Flow::Unmeasured);
    }

    #[test]
    fn a_process_that_spends_the_whole_budget_does_not_starve_the_ones_after_it() {
        // Three processes of ten descriptors each and room for ten: one is read a pass.
        let reading = || {
            let mut taken = held(vec![file(3, 10, AccessMode::Read, 0)], 0, 0);
            taken.examined = 10;
            Reading::Taken(taken)
        };
        let mut fake = Fake::new([
            (1, vec![reading(), reading()]),
            (2, vec![reading(), reading()]),
            (3, vec![reading(), reading()]),
        ]);
        let caps = Caps {
            per_pass: 10,
            per_process: FILES_PER_PROCESS,
        };
        let targets = [id(1), id(2), id(3)];
        let mut scan = IoScan::default();
        let mut clock = seconds();
        scan.pass_with(&targets, &mut fake, caps, &mut clock);
        scan.pass_with(&targets, &mut fake, caps, &mut clock);
        let third = scan.pass_with(&targets, &mut fake, caps, &mut clock);
        let asked: Vec<u32> = fake.asked.iter().map(|(pid, _)| *pid).collect();
        assert_eq!(asked, [1, 2, 3], "each pass resumes where the last one stopped");
        for pid in [1, 2, 3] {
            assert!(third.contains_key(&id(pid)), "{pid} has a result");
        }
    }

    #[test]
    fn unreadable_and_exited_processes_are_told_apart_and_forgotten() {
        let taken =
            |position| Reading::Taken(held(vec![file(3, 10, AccessMode::Read, position)], 0, 0));
        let mut fake = Fake::new([
            (1, vec![taken(0), Reading::Unreadable, taken(10)]),
            (2, vec![taken(0), Reading::Gone, taken(10)]),
        ]);
        let mut scan = IoScan::default();
        let mut clock = seconds();
        let both = [id(1), id(2)];
        scan.pass_with(&both, &mut fake, Caps::DEFAULT, &mut clock);
        let second = scan.pass_with(&both, &mut fake, Caps::DEFAULT, &mut clock);
        assert_eq!(second[&id(1)], Measured::Unreadable);
        assert!(!second.contains_key(&id(2)), "gone");
        // Neither left anything behind to compare with.
        let third = scan.pass_with(&both, &mut fake, Caps::DEFAULT, &mut clock);
        for pid in [1, 2] {
            assert_eq!(detail_of(&third, pid).files[0].flow, Flow::Unmeasured);
        }
        scan.forget();
        assert!(scan.remembers_nothing());
    }

    #[test]
    fn select_picks_the_busiest_and_never_a_process_with_nothing_to_rank() {
        let rate = |pid: u32, calls: Option<f32>| Process {
            io_rate: calls,
            ..testing::process(pid, 1)
        };
        let mut processes: Vec<Process> = (1..=40).map(|pid| rate(pid, Some(pid as f32))).collect();
        processes[3] = rate(4, None);
        processes[4] = rate(5, Some(0.0));
        processes[5] = Process {
            kind: Kind::Kernel,
            ..rate(6, Some(1.0e9))
        };
        processes[6] = Process {
            state: 'Z',
            ..rate(7, Some(1.0e9))
        };
        // Pids 16 and 17 tie at the 24th place: the smaller identity wins it.
        processes[15] = rate(16, Some(17.0));
        let chosen = select(&processes, |p| p.io_rate.map(f64::from));
        let pids: Vec<u32> = chosen.iter().map(|id| id.pid).collect();
        // 40 down to 18 is 23 processes; the 24th is 16. Pids 4 and 5 (no rate, no I/O), 6
        // (a kernel thread) and 7 (a zombie) are never chosen, however busy they read.
        let expected: Vec<u32> = (18..=40).rev().chain([16]).collect();
        assert_eq!(pids, expected);
        assert_eq!(pids.len(), IO_PROCESSES);
        let few: Vec<Process> = (1..=3).map(|pid| rate(pid, Some(1.0))).collect();
        assert_eq!(select(&few, |p| p.io_rate.map(f64::from)).len(), 3);
        assert!(select(&[], |_| Some(1.0)).is_empty());
    }

    #[test]
    fn paths_are_kept_raw_and_cleaned_for_display() {
        let hostile = "/tmp/\u{1b}]0;pwned\u{7}\u{9b}31m\u{85}\u{202e}\r\nname";
        let mut open = file(3, 10, AccessMode::Read, 0);
        open.path = hostile.to_owned();
        let mut fake = Fake::new([(5, vec![Reading::Taken(held(vec![open], 0, 0))])]);
        let found = IoScan::default().pass(&[id(5)], &mut fake);
        let detail = detail_of(&found, 5);
        assert_eq!(
            detail.files[0].path, hostile,
            "stored as the system gave it"
        );
        // The escape, bell, C1 controls, bidirectional override and line breaks each become `?`.
        let shown = SafeText::new(&detail.files[0].path);
        assert_eq!(shown.as_str(), "/tmp/?]0;pwned??31m????name");
        assert!(
            shown
                .as_str()
                .chars()
                .all(|c| c.is_ascii_graphic() || c == ' ')
        );
    }
}
