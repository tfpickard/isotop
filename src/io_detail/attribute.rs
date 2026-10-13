//! The attribution maths of `IoScan`, pure: two readings of a process's descriptors and the growth
//! of its byte counters between them in, the bytes each file may be credited with out.
//!
//! A claim is the growth of a descriptor's position, and a descriptor is only compared with
//! itself: the same descriptor number on the same file, opened the same way. Anything else is a
//! new descriptor and has no earlier reading. A claim is never made up: a position that went
//! backwards, one that says nothing about bytes, and a first reading all claim nothing, and
//! `Flow` says which. Where the platform counts the bytes of the process's own read and write
//! calls, the claims are held to them.

use std::collections::{BTreeMap, HashMap};

use super::{FileFlow, Flow, Held, OpenFile, Unattributed};
use crate::platform::AccessMode;

/// The growth of a process's byte counters across the interval its claims are made over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cover {
    /// `rchar`: bytes passed to read-like calls.
    pub read: u64,
    /// `wchar`: bytes passed to write-like calls.
    pub write: u64,
}

/// What bounds the claims of one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// The platform has no byte counters (macOS). Claims are whole, held only to the file sizes
    /// (a position past the end of a file claims nothing), and nothing is said about what remains.
    Unbounded,
    /// The process's own counters grew by this much over the interval: its read-only
    /// descriptors together claim at most `read`, its write-only ones at most `write`, and its
    /// read-write ones at most what the two leave. A claim over its bound is scaled down, all
    /// the descriptors of that kind in proportion.
    To(Cover),
    /// The platform counts, but the counters could not be read across this interval or went
    /// backwards, so nothing can vouch for a claim and none is made.
    Withheld,
}

/// What the positions of one process's files made of an interval.
#[derive(Clone, Debug, PartialEq)]
pub struct Attributed {
    /// One entry per file, sorted by (device, inode).
    pub files: Vec<FileFlow>,
    /// What the claims leave of the byte counters; None unless the claims were `Limit::To`.
    pub unattributed: Option<Unattributed>,
}

/// A descriptor as it is told from another: where it points, how it was opened, which number it
/// is. If any part changes between two readings, it is a different descriptor. (A descriptor
/// closed and opened again on the same number, onto the same file and the same way, cannot be told
/// from the one that continued; its position then restarts, which a lower position shows as a
/// rewind, and a higher one makes a claim no larger than the bytes moved since it was opened.)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    fd: u32,
    device: u64,
    inode: u64,
    access: AccessMode,
    appending: bool,
}

impl Key {
    fn of(file: &OpenFile) -> Key {
        Key {
            fd: file.fd,
            device: file.device,
            inode: file.inode,
            access: file.access,
            appending: file.appending,
        }
    }
}

/// What one descriptor's two readings came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// Its position advanced by this many bytes.
    Bytes(u64),
    /// Nothing can be said: no earlier reading of this descriptor, or the position went backwards.
    Unmeasured,
    /// Its position says nothing about bytes.
    Unpositioned,
}

/// The descriptors of `files` that are not copies of one another. Two descriptors on one open
/// file description (`dup`, `dup2`, `cmd >log 2>&1`, a descriptor inherited across `fork`) share
/// a position, so a file's bytes would be counted once for each. They show as descriptors of the
/// same file, opened the same way, at the same position, and only the lowest number is kept. Two
/// separate opens that happen to sit at the same position are merged by mistake, which only
/// lowers a claim.
fn distinct(files: &[OpenFile]) -> Vec<&OpenFile> {
    let mut kept: Vec<&OpenFile> = Vec::with_capacity(files.len());
    let mut ordered: Vec<&OpenFile> = files.iter().collect();
    ordered.sort_by_key(|file| file.fd);
    for file in ordered {
        let copy = kept.iter().any(|other| {
            (
                other.device,
                other.inode,
                other.access,
                other.appending,
                other.position,
            ) == (
                file.device,
                file.inode,
                file.access,
                file.appending,
                file.position,
            )
        });
        if !copy {
            kept.push(file);
        }
    }
    kept
}

fn outcome(before: Option<&OpenFile>, now: &OpenFile) -> Outcome {
    // Opened to append, the position is the end of the file whoever appended last; past the end
    // it counts something else (a seek, or the records of a file of /proc).
    if now.appending || now.position > now.size {
        return Outcome::Unpositioned;
    }
    let Some(before) = before else {
        return Outcome::Unmeasured;
    };
    if before.position > before.size {
        return Outcome::Unmeasured;
    }
    // A rewind, or a descriptor reopened at its start: how far it got meanwhile is unknown.
    match now.position.checked_sub(before.position) {
        Some(bytes) => Outcome::Bytes(bytes.min(MOST_PER_DESCRIPTOR)),
        None => Outcome::Unmeasured,
    }
}

/// The most one descriptor can claim. File offsets are signed 63-bit numbers, and the cap keeps
/// the arithmetic of `scaled` clear of overflow for any number of descriptors and any bound.
const MOST_PER_DESCRIPTOR: u64 = 1 << 62;

/// What `bytes` of a kind of claim become when the claims of that kind total `total` and may
/// total at most `allowed`: itself, or its proportional share of `allowed`, rounded down so that
/// the shares never sum to more than `allowed`. `bytes` is at most `MOST_PER_DESCRIPTOR` and
/// `allowed` below `total`, which is below 2^71 for the 256 files a process keeps, so the product
/// fits.
fn scaled(bytes: u64, allowed: u128, total: u128) -> u64 {
    if total <= allowed {
        bytes
    } else {
        (u128::from(bytes) * allowed / total) as u64
    }
}

/// A file as its descriptors held it, with what each of them came to.
struct Entry {
    path: String,
    size: u64,
    deleted: bool,
    access: AccessMode,
    outcomes: Vec<(AccessMode, Outcome)>,
}

/// Bytes by the access mode of the descriptors they were claimed through.
#[derive(Clone, Copy, Default)]
struct ByMode {
    read: u128,
    write: u128,
    either: u128,
}

impl ByMode {
    fn slot(&mut self, access: AccessMode) -> &mut u128 {
        match access {
            AccessMode::Read => &mut self.read,
            AccessMode::Write => &mut self.write,
            AccessMode::Both => &mut self.either,
        }
    }
}

/// What the positions of the files in `later` made of the interval since `earlier`, `seconds`
/// long, under `limit`. The files in the two readings are told apart by descriptor (`Key`).
pub fn attribute(earlier: &Held, later: &Held, seconds: f64, limit: Limit) -> Attributed {
    let before: HashMap<Key, &OpenFile> = distinct(&earlier.files)
        .into_iter()
        .map(|file| (Key::of(file), file))
        .collect();
    let mut entries: BTreeMap<(u64, u64), Entry> = BTreeMap::new();
    for file in distinct(&later.files) {
        let mut outcome = outcome(before.get(&Key::of(file)).copied(), file);
        if limit == Limit::Withheld && matches!(outcome, Outcome::Bytes(_)) {
            outcome = Outcome::Unmeasured;
        }
        let entry = entries
            .entry((file.device, file.inode))
            .or_insert_with(|| Entry {
                path: file.path.clone(),
                size: file.size,
                deleted: file.deleted,
                access: file.access,
                outcomes: Vec::new(),
            });
        entry.access = entry.access.union(file.access);
        entry.outcomes.push((file.access, outcome));
    }

    // What the descriptors advanced, by access mode.
    let mut advanced = ByMode::default();
    for entry in entries.values() {
        for &(access, outcome) in &entry.outcomes {
            if let Outcome::Bytes(bytes) = outcome {
                *advanced.slot(access) += u128::from(bytes);
            }
        }
    }
    // Read-only and write-only claims first, each held to its own counter, then the read-write
    // ones, held to what the two counters leave together.
    let (mut allowed_read, mut allowed_write) = match limit {
        Limit::To(cover) => (u128::from(cover.read), u128::from(cover.write)),
        Limit::Unbounded | Limit::Withheld => (u128::MAX, u128::MAX),
    };
    let mut claimed = ByMode::default();
    for entry in entries.values_mut() {
        for (access, outcome) in &mut entry.outcomes {
            if let Outcome::Bytes(bytes) = outcome
                && *access != AccessMode::Both
            {
                let allowed = if *access == AccessMode::Read {
                    allowed_read
                } else {
                    allowed_write
                };
                *bytes = scaled(*bytes, allowed, *advanced.slot(*access));
                *claimed.slot(*access) += u128::from(*bytes);
            }
        }
    }
    allowed_read = allowed_read.saturating_sub(claimed.read);
    allowed_write = allowed_write.saturating_sub(claimed.write);
    let allowed_either = allowed_read.saturating_add(allowed_write);
    for entry in entries.values_mut() {
        for (access, outcome) in &mut entry.outcomes {
            if let Outcome::Bytes(bytes) = outcome
                && *access == AccessMode::Both
            {
                *bytes = scaled(*bytes, allowed_either, advanced.either);
                claimed.either += u128::from(*bytes);
            }
        }
    }

    let rate = |bytes: u128| (bytes as f64 / seconds) as f32;
    let files = entries
        .into_iter()
        .map(|((device, inode), entry)| {
            let mut sums = ByMode::default();
            let (mut measured, mut unmeasured) = (false, false);
            for &(access, outcome) in &entry.outcomes {
                match outcome {
                    Outcome::Bytes(bytes) => {
                        measured = true;
                        *sums.slot(access) += u128::from(bytes);
                    }
                    Outcome::Unmeasured => unmeasured = true,
                    Outcome::Unpositioned => {}
                }
            }
            let flow = if measured {
                Flow::Measured {
                    read: rate(sums.read),
                    written: rate(sums.write),
                    either: rate(sums.either),
                }
            } else if unmeasured {
                Flow::Unmeasured
            } else {
                Flow::Unpositioned
            };
            FileFlow {
                device,
                inode,
                size: entry.size,
                path: entry.path,
                deleted: entry.deleted,
                access: entry.access,
                flow,
            }
        })
        .collect();
    let unattributed = match limit {
        Limit::To(cover) => Some(Unattributed {
            read: rate(u128::from(cover.read).saturating_sub(claimed.read)),
            write: rate(u128::from(cover.write).saturating_sub(claimed.write)),
            either: rate(claimed.either),
        }),
        Limit::Unbounded | Limit::Withheld => None,
    };
    Attributed {
        files,
        unattributed,
    }
}

/// What the files of the reading `held` make of an interval that has nothing to compare them
/// with: a first reading. A file whose position says nothing about bytes is told so even now; the
/// others have no flow yet.
pub fn unmeasured(held: &Held) -> Attributed {
    attribute(&Held::default(), held, 1.0, Limit::Withheld)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A regular file held through descriptor `fd`, on inode `inode`, in a file of a terabyte.
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

    fn held(files: impl IntoIterator<Item = OpenFile>) -> Held {
        Held {
            files: files.into_iter().collect(),
            ..Held::default()
        }
    }

    /// One second between the readings, so a rate is the bytes themselves (exactly, below 2^24).
    fn over_a_second(earlier: &Held, later: &Held, limit: Limit) -> Attributed {
        attribute(earlier, later, 1.0, limit)
    }

    fn flow_of(attributed: &Attributed, inode: u64) -> Flow {
        attributed
            .files
            .iter()
            .find(|file| file.inode == inode)
            .unwrap_or_else(|| panic!("inode {inode} is listed"))
            .flow
    }

    fn measured(read: f32, written: f32, either: f32) -> Flow {
        Flow::Measured {
            read,
            written,
            either,
        }
    }

    const PLENTY: Cover = Cover {
        read: 10_000_000,
        write: 10_000_000,
    };

    #[test]
    fn advancing_positions_attribute_bytes_by_access_mode() {
        let earlier = held([
            file(3, 10, AccessMode::Read, 1000),
            file(4, 11, AccessMode::Write, 0),
            file(5, 12, AccessMode::Both, 500),
        ]);
        let later = held([
            file(3, 10, AccessMode::Read, 5096),
            file(4, 11, AccessMode::Write, 8192),
            file(5, 12, AccessMode::Both, 1500),
        ]);
        let attributed = attribute(&earlier, &later, 2.0, Limit::To(PLENTY));
        // Over two seconds: 4096 read, 8192 written, and 1000 either way, which is never split.
        assert_eq!(flow_of(&attributed, 10), measured(2048.0, 0.0, 0.0));
        assert_eq!(flow_of(&attributed, 11), measured(0.0, 4096.0, 0.0));
        assert_eq!(flow_of(&attributed, 12), measured(0.0, 0.0, 500.0));
        let files: Vec<(u64, AccessMode)> = attributed
            .files
            .iter()
            .map(|file| (file.inode, file.access))
            .collect();
        assert_eq!(
            files,
            [
                (10, AccessMode::Read),
                (11, AccessMode::Write),
                (12, AccessMode::Both)
            ]
        );
    }

    #[test]
    fn one_file_held_through_several_descriptors_is_one_entry_that_sums_them() {
        // The file is read through fd 3 and written through fd 4, which are two open files.
        let earlier = held([
            file(3, 10, AccessMode::Read, 0),
            file(4, 10, AccessMode::Write, 100),
        ]);
        let later = held([
            file(3, 10, AccessMode::Read, 700),
            file(4, 10, AccessMode::Write, 400),
        ]);
        let attributed = over_a_second(&earlier, &later, Limit::To(PLENTY));
        assert_eq!(attributed.files.len(), 1);
        assert_eq!(attributed.files[0].access, AccessMode::Both);
        assert_eq!(flow_of(&attributed, 10), measured(700.0, 300.0, 0.0));
    }

    #[test]
    fn descriptors_that_share_an_open_file_description_count_once() {
        // `cmd >log 2>&1`: fds 1 and 2 are one description, so one position, seen twice.
        let earlier = held([
            file(1, 10, AccessMode::Write, 1000),
            file(2, 10, AccessMode::Write, 1000),
        ]);
        let later = held([
            file(1, 10, AccessMode::Write, 1600),
            file(2, 10, AccessMode::Write, 1600),
        ]);
        let attributed = over_a_second(&earlier, &later, Limit::To(PLENTY));
        assert_eq!(flow_of(&attributed, 10), measured(0.0, 600.0, 0.0));
        let unattributed = attributed.unattributed.unwrap();
        assert_eq!(unattributed.write, 10_000_000.0 - 600.0);
    }

    #[test]
    fn rewound_or_reused_descriptors_attribute_nothing() {
        let earlier = held([
            file(3, 10, AccessMode::Read, 5000),
            file(4, 11, AccessMode::Read, 100),
            file(5, 12, AccessMode::Read, 100),
            file(6, 13, AccessMode::Read, 100),
            file(7, 14, AccessMode::Read, 100),
        ]);
        let mut other_device = file(7, 14, AccessMode::Read, 9000);
        other_device.device = 8;
        let later = held([
            // Rewound: it may have been read again and again, or reopened; either way unknown.
            file(3, 10, AccessMode::Read, 100),
            // Descriptor 4 now holds another file.
            file(4, 99, AccessMode::Read, 9000),
            // Descriptor 5 was reopened for reading and writing.
            file(5, 12, AccessMode::Both, 9000),
            // Descriptor 6 is gone and 8 is new.
            file(8, 13, AccessMode::Read, 9000),
            other_device,
        ]);
        let attributed = over_a_second(&earlier, &later, Limit::To(PLENTY));
        assert_eq!(attributed.files.len(), 5);
        for file in &attributed.files {
            assert_eq!(file.flow, Flow::Unmeasured, "{}", file.path);
        }
        // Nothing was claimed, so everything the process moved is left over.
        let unattributed = attributed.unattributed.unwrap();
        assert_eq!(
            (unattributed.read, unattributed.write, unattributed.either),
            (10_000_000.0, 10_000_000.0, 0.0)
        );
    }

    #[test]
    fn a_position_that_says_nothing_about_bytes_is_not_credited() {
        let mut appended = file(3, 10, AccessMode::Write, 4000);
        appended.appending = true;
        appended.size = 4000;
        let mut grown = appended.clone();
        grown.position = 9000;
        grown.size = 9000;
        // Past the end of the file: a seek, or the records of a file of /proc.
        let mut sought = file(4, 11, AccessMode::Read, 0);
        sought.size = 100;
        let mut jumped = sought.clone();
        jumped.position = 1 << 30;
        let attributed = over_a_second(
            &held([appended, sought]),
            &held([grown, jumped]),
            Limit::To(PLENTY),
        );
        assert_eq!(flow_of(&attributed, 10), Flow::Unpositioned);
        assert_eq!(flow_of(&attributed, 11), Flow::Unpositioned);
        // The appended bytes are still in what the process wrote.
        assert_eq!(attributed.unattributed.unwrap().write, 10_000_000.0);
    }

    #[test]
    fn a_first_reading_has_no_flow_and_not_a_zero_one() {
        let mut appended = file(5, 12, AccessMode::Write, 700);
        appended.appending = true;
        let first = unmeasured(&held([
            file(3, 10, AccessMode::Read, 4096),
            file(4, 11, AccessMode::Both, 0),
            appended,
        ]));
        assert_eq!(flow_of(&first, 10), Flow::Unmeasured);
        assert_eq!(flow_of(&first, 11), Flow::Unmeasured);
        assert_eq!(
            flow_of(&first, 12),
            Flow::Unpositioned,
            "known without history"
        );
        assert_eq!(first.unattributed, None);
    }

    #[test]
    fn attribution_never_exceeds_the_read_and_write_totals() {
        // An lseek moved the positions of two read-only files by far more than the process read:
        // its counters grew by 2000, so the 3000 and 1000 claimed shrink in proportion.
        let earlier = held([
            file(3, 10, AccessMode::Read, 0),
            file(4, 11, AccessMode::Read, 0),
        ]);
        let later = held([
            file(3, 10, AccessMode::Read, 3000),
            file(4, 11, AccessMode::Read, 1000),
        ]);
        let cover = Cover {
            read: 2000,
            write: 0,
        };
        let attributed = over_a_second(&earlier, &later, Limit::To(cover));
        assert_eq!(flow_of(&attributed, 10), measured(1500.0, 0.0, 0.0));
        assert_eq!(flow_of(&attributed, 11), measured(500.0, 0.0, 0.0));
        let left = attributed.unattributed.unwrap();
        assert_eq!((left.read, left.write, left.either), (0.0, 0.0, 0.0));

        // Writes are held to the write counter alone, whatever the read one has to spare.
        let earlier = held([
            file(3, 10, AccessMode::Write, 0),
            file(4, 11, AccessMode::Read, 0),
        ]);
        let later = held([
            file(3, 10, AccessMode::Write, 3000),
            file(4, 11, AccessMode::Read, 100),
        ]);
        let cover = Cover {
            read: 5000,
            write: 1000,
        };
        let attributed = over_a_second(&earlier, &later, Limit::To(cover));
        assert_eq!(flow_of(&attributed, 10), measured(0.0, 1000.0, 0.0));
        assert_eq!(flow_of(&attributed, 11), measured(100.0, 0.0, 0.0));

        // Read-write descriptors take what the two counters leave together: 400 and 1000.
        let earlier = held([
            file(3, 10, AccessMode::Read, 0),
            file(4, 11, AccessMode::Both, 0),
        ]);
        let later = held([
            file(3, 10, AccessMode::Read, 600),
            file(4, 11, AccessMode::Both, 5000),
        ]);
        let cover = Cover {
            read: 1000,
            write: 1000,
        };
        let attributed = over_a_second(&earlier, &later, Limit::To(cover));
        assert_eq!(flow_of(&attributed, 11), measured(0.0, 0.0, 1400.0));
        let left = attributed.unattributed.unwrap();
        assert_eq!(
            (left.read, left.write, left.either),
            (400.0, 1000.0, 1400.0)
        );
        assert_eq!(
            left.read + left.write - left.either,
            0.0,
            "everything the counters saw is explained"
        );
    }

    #[test]
    fn claims_never_sum_to_more_than_the_counters_for_any_movement() {
        // A deterministic spread of position movements against counters of the same orders.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };
        for _ in 0..500 {
            let accesses = [AccessMode::Read, AccessMode::Write, AccessMode::Both];
            let count = 1 + next(6) as u32;
            let (mut before, mut after) = (Vec::new(), Vec::new());
            for index in 0..count {
                let access = accesses[next(3) as usize];
                let start = next(1 << 20);
                before.push(file(3 + index, 10 + u64::from(index), access, start));
                after.push(file(
                    3 + index,
                    10 + u64::from(index),
                    access,
                    start + next(1 << 22),
                ));
            }
            let cover = Cover {
                read: next(1 << 21),
                write: next(1 << 21),
            };
            let attributed = over_a_second(&held(before), &held(after), Limit::To(cover));
            let (mut read, mut write, mut either) = (0.0_f64, 0.0_f64, 0.0_f64);
            for file in &attributed.files {
                if let Flow::Measured {
                    read: r,
                    written: w,
                    either: e,
                } = file.flow
                {
                    read += f64::from(r);
                    write += f64::from(w);
                    either += f64::from(e);
                }
            }
            assert!(read <= cover.read as f64, "{read} of {}", cover.read);
            assert!(write <= cover.write as f64, "{write} of {}", cover.write);
            assert!(
                read + write + either <= (cover.read + cover.write) as f64,
                "{} of {}",
                read + write + either,
                cover.read + cover.write
            );
            let left = attributed.unattributed.unwrap();
            assert!(left.read >= 0.0 && left.write >= 0.0);
            assert!(
                f64::from(left.read) + f64::from(left.write) - f64::from(left.either) >= 0.0,
                "{left:?}"
            );
        }
    }

    #[test]
    fn positions_and_counters_at_the_limits_of_the_integer_range_neither_wrap_nor_overflow() {
        let far = |fd: u32, access: AccessMode, position: u64| {
            let mut far = file(fd, 10 + u64::from(fd), access, position);
            far.size = u64::MAX;
            far
        };
        let modes = [AccessMode::Read, AccessMode::Write, AccessMode::Both];
        let earlier = held((0..9).map(|n| far(3 + n, modes[n as usize % 3], 0)));
        let later = held((0..9).map(|n| far(3 + n, modes[n as usize % 3], u64::MAX - 1)));
        for cover in [
            Cover {
                read: u64::MAX,
                write: u64::MAX,
            },
            Cover { read: 5, write: 7 },
        ] {
            let attributed = over_a_second(&earlier, &later, Limit::To(cover));
            let claimed: f64 = attributed
                .files
                .iter()
                .map(|file| match file.flow {
                    Flow::Measured {
                        read,
                        written,
                        either,
                    } => f64::from(read) + f64::from(written) + f64::from(either),
                    _ => 0.0,
                })
                .sum();
            // f32 rates round, so the huge case is only held to the bound with a hair to spare.
            let bound = cover.read as f64 + cover.write as f64;
            assert!(claimed <= bound * (1.0 + 1e-6), "{claimed} of {bound}");
            let left = attributed.unattributed.unwrap();
            assert!(left.read >= 0.0 && left.write >= 0.0 && left.either >= 0.0);
        }
        // No bound at all: each descriptor claims at most what an offset can hold.
        let attributed = over_a_second(&earlier, &later, Limit::Unbounded);
        assert!(
            attributed
                .files
                .iter()
                .all(|file| matches!(file.flow, Flow::Measured { .. }))
        );
        // A position that is the largest number and one that wrapped to a small one: a rewind.
        let wrapped = held([far(3, AccessMode::Read, 5)]);
        let top = held([far(3, AccessMode::Read, u64::MAX)]);
        let attributed = over_a_second(&top, &wrapped, Limit::Unbounded);
        assert_eq!(attributed.files[0].flow, Flow::Unmeasured);
    }

    #[test]
    fn unattributed_bytes_are_the_remainder() {
        // The process read 10 000 and wrote 4 000 bytes; its files explain 3 000 and 1 000. A
        // positioned read or write explains nothing of the rest: that is pipes, sockets,
        // `pread`, `pwrite` and mappings. The file that `pread` reads sits still.
        let earlier = held([
            file(3, 10, AccessMode::Read, 0),
            file(4, 11, AccessMode::Write, 0),
            file(5, 12, AccessMode::Read, 77),
        ]);
        let later = held([
            file(3, 10, AccessMode::Read, 3000),
            file(4, 11, AccessMode::Write, 1000),
            file(5, 12, AccessMode::Read, 77),
        ]);
        let cover = Cover {
            read: 10_000,
            write: 4000,
        };
        let attributed = over_a_second(&earlier, &later, Limit::To(cover));
        let left = attributed.unattributed.expect("the counters are known");
        assert_eq!((left.read, left.write, left.either), (7000.0, 3000.0, 0.0));
        // The file read by `pread` is known to have moved no position: no bytes claimed, which is
        // a lower bound and not a statement that nothing was read.
        assert_eq!(flow_of(&attributed, 12), measured(0.0, 0.0, 0.0));
    }

    #[test]
    fn without_byte_counters_claims_are_whole_and_nothing_is_said_of_the_rest() {
        let earlier = held([file(3, 10, AccessMode::Read, 0)]);
        let later = held([file(3, 10, AccessMode::Read, 5_000_000)]);
        let attributed = over_a_second(&earlier, &later, Limit::Unbounded);
        assert_eq!(flow_of(&attributed, 10), measured(5_000_000.0, 0.0, 0.0));
        assert_eq!(attributed.unattributed, None);
    }

    #[test]
    fn claims_are_withheld_when_the_counters_cannot_vouch_for_them() {
        let earlier = held([file(3, 10, AccessMode::Read, 0)]);
        let later = held([file(3, 10, AccessMode::Read, 5000)]);
        let attributed = over_a_second(&earlier, &later, Limit::Withheld);
        assert_eq!(flow_of(&attributed, 10), Flow::Unmeasured);
        assert_eq!(attributed.unattributed, None);
    }
}
