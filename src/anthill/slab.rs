//! Where things stand on the slab, with no drawing. The slab is a square: a clearing at the middle
//! for the nests, and round it a fixed pinwheel of five regions, one for each of `/usr`, `/home`,
//! `/var`, `/tmp` and everything else. A file's pile stands in the cell its second-level directory
//! was given, at a spot its path hashes to; a nest stands where the packer first found room for
//! the room it reserved. Nothing here moves for a change of CPU, memory below what a nest
//! reserved, or the population: only a clearing too small for its nests grows, in steps of 1.25.

use std::collections::{HashMap, HashSet};

use crate::layout::spiral;
use crate::model::Identity;
use crate::simulation::hash;

/// The depth of each arm of the pinwheel, in world units.
pub const ARM: f32 = 16.0;

/// Half the width of the first clearing, in world units: the slab starts 64 across.
pub const FIRST_CLEARING: f32 = 16.0;

/// How much a clearing that is too small grows by.
pub const GROWTH: f32 = 1.25;

/// How many world units across a cell of a region is at most.
const CELL: f32 = 8.0;

/// The spacing of the spiral the packer looks for room along, in world units.
const STEP: f32 = 0.8;

/// How far a pile's centre keeps from the edge of its cell.
const MARGIN: f32 = 2.0;

/// The size of a bucket of the packer's index, in world units.
const BUCKET: f32 = 8.0;

/// The five regions of the slab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Region {
    Usr,
    Home,
    Var,
    Tmp,
    Other,
}

impl Region {
    pub const ALL: [Region; 5] = [
        Region::Usr,
        Region::Home,
        Region::Var,
        Region::Tmp,
        Region::Other,
    ];

    /// What the region is called on the slab.
    pub fn name(self) -> &'static str {
        match self {
            Region::Usr => "/usr",
            Region::Home => "/home",
            Region::Var => "/var",
            Region::Tmp => "/tmp",
            Region::Other => "other",
        }
    }

    /// The region a path lies in and the key of its second-level directory, `usr/lib` for
    /// `/usr/lib/x86_64-linux-gnu/libc.so.6`. A file directly in a top-level directory is keyed by
    /// that directory. The programs of macOS (`/System`, `/Library`, `/Applications`) lie in
    /// `/usr`, its `/Users` in `/home`, and `/private/var`, `/private/tmp` in `/var` and `/tmp`;
    /// shared memory (`/dev/shm`) is in `/tmp`. The text is raw and only ever hashed or compared.
    pub fn of(path: &str) -> (Region, String) {
        let mut parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        if parts.first() == Some(&"private") && parts.len() > 1 {
            parts.remove(0);
        }
        let region = match (parts.first().copied(), parts.get(1).copied()) {
            (Some("usr" | "lib" | "lib32" | "lib64" | "libx32" | "bin" | "sbin"), _)
            | (Some("System" | "Library" | "Applications"), _) => Region::Usr,
            (Some("home" | "Users" | "root"), _) => Region::Home,
            (Some("var"), _) => Region::Var,
            (Some("tmp"), _) | (Some("dev"), Some("shm")) => Region::Tmp,
            _ => Region::Other,
        };
        let key = match parts.len() {
            0 => String::new(),
            1 | 2 => parts[0].to_owned(),
            _ => format!("{}/{}", parts[0], parts[1]),
        };
        (region, key)
    }

    /// A tint the sand takes in the region, added to each channel.
    pub fn tint(self) -> [f32; 3] {
        match self {
            Region::Usr => [4.0, 6.0, 12.0],
            Region::Home => [10.0, 6.0, 0.0],
            Region::Var => [3.0, 6.0, 7.0],
            Region::Tmp => [0.0, 7.0, 5.0],
            Region::Other => [7.0, 2.0, 9.0],
        }
    }
}

/// A rectangle on the ground, by its corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl Rect {
    pub fn center(&self) -> [f32; 2] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
        ]
    }

    pub fn width(&self) -> f32 {
        self.max[0] - self.min[0]
    }

    pub fn depth(&self) -> f32 {
        self.max[1] - self.min[1]
    }

    pub fn contains(&self, at: [f32; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&at[0]) && (self.min[1]..=self.max[1]).contains(&at[1])
    }
}

/// The slab: a clearing of half-width `clearing` at the origin with the pinwheel round it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slab {
    clearing: f32,
}

impl Default for Slab {
    fn default() -> Self {
        Slab {
            clearing: FIRST_CLEARING,
        }
    }
}

impl Slab {
    /// Half the width of the clearing.
    #[cfg(test)]
    pub fn clearing(&self) -> f32 {
        self.clearing
    }

    /// Half the width of the slab.
    pub fn half(&self) -> f32 {
        self.clearing + ARM
    }

    /// The width of the slab.
    pub fn size(&self) -> f32 {
        2.0 * self.half()
    }

    /// The clearing itself.
    pub fn middle(&self) -> Rect {
        let c = self.clearing;
        Rect {
            min: [-c, -c],
            max: [c, c],
        }
    }

    /// A region's rectangle. Each of `/usr`, `/var` and `/tmp` is an arm that reaches a corner of
    /// the slab, `/home` the arm between the clearing's two corners on the west, and `other` the
    /// square the pinwheel leaves in the south-west corner.
    pub fn rect(&self, region: Region) -> Rect {
        let (c, h) = (self.clearing, self.half());
        let (min, max) = match region {
            Region::Usr => ([-h, -h], [c, -c]),
            Region::Var => ([c, -h], [h, c]),
            Region::Tmp => ([-c, c], [h, h]),
            Region::Home => ([-h, -c], [-c, c]),
            Region::Other => ([-h, c], [-c, h]),
        };
        Rect { min, max }
    }

    /// The slab's corners on the ground, clockwise from the north-west one.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let h = self.half();
        [[-h, -h], [h, -h], [h, h], [-h, h]]
    }

    /// Grows the clearing, in steps of `GROWTH`, until it holds `reach` (how far the farthest
    /// reserved nest extends from the middle along an axis). Whether it grew.
    pub fn grow_to_hold(&mut self, reach: f32) -> bool {
        let before = self.clearing;
        while self.clearing < reach {
            self.clearing *= GROWTH;
        }
        self.clearing != before
    }

    /// Where blanket `index` lies: the corners of the clearing, which the round heap of nests
    /// leaves free.
    pub fn blanket(&self, index: usize) -> [f32; 2] {
        let at = self.clearing - 3.6;
        let sign = |bit: usize| if index & bit == 0 { -1.0 } else { 1.0 };
        [sign(1) * at, sign(2) * at]
    }

    /// The cells a region is cut into: columns and rows, each at most `CELL` across.
    fn cells(&self, region: Region) -> (usize, usize) {
        let rect = self.rect(region);
        let count = |length: f32| ((length / CELL).floor() as usize).max(1);
        (count(rect.width()), count(rect.depth()))
    }
}

/// The cell each second-level directory was given. A directory keeps its cell for as long as the
/// view lives; one that finds its cell taken by another takes the next free one, and when every
/// cell is taken shares its own.
#[derive(Default)]
pub struct Parcels {
    seated: HashMap<(Region, String), usize>,
    taken: HashMap<Region, HashSet<usize>>,
}

impl Parcels {
    /// Where the pile of the file at `path` stands: in its directory's cell, at a spot the whole
    /// path hashes to.
    pub fn pile(&mut self, slab: &Slab, path: &str) -> [f32; 2] {
        let (region, directory) = Region::of(path);
        let (columns, rows) = slab.cells(region);
        let capacity = columns * rows;
        let seat = match self.seated.get(&(region, directory.clone())) {
            Some(&seat) => seat,
            None => {
                let taken = self.taken.entry(region).or_default();
                let wanted = (hash::fnv1a_64(&directory) % capacity as u64) as usize;
                let seat = (0..capacity)
                    .map(|offset| (wanted + offset) % capacity)
                    .find(|seat| !taken.contains(seat))
                    .unwrap_or(wanted);
                taken.insert(seat);
                self.seated.insert((region, directory), seat);
                seat
            }
        };
        let rect = slab.rect(region);
        let (width, depth) = (rect.width() / columns as f32, rect.depth() / rows as f32);
        let (column, row) = ((seat % capacity) % columns, (seat % capacity) / columns);
        let spot = hash::fnv1a_64(path);
        let unit = |bits: u64| (bits & 0xffff) as f32 / 65535.0;
        let room = |length: f32| (length - 2.0 * MARGIN).max(0.0);
        [
            rect.min[0]
                + column as f32 * width
                + MARGIN.min(width * 0.5)
                + unit(spot >> 8) * room(width),
            rect.min[1]
                + row as f32 * depth
                + MARGIN.min(depth * 0.5)
                + unit(spot >> 28) * room(depth),
        ]
    }
}

/// The room reserved for each nest. A nest keeps the place it was first given until it outgrows
/// the room it reserved, and a newcomer takes the first place along a golden-angle spiral from the
/// middle where its room fits. A spatial index keeps that cheap with a thousand nests.
#[derive(Default)]
pub struct Packer {
    reserved: HashMap<Identity, ([f32; 2], f32)>,
    buckets: HashMap<(i32, i32), Vec<Identity>>,
    widest: f32,
}

fn bucket(at: [f32; 2]) -> (i32, i32) {
    (
        (at[0] / BUCKET).floor() as i32,
        (at[1] / BUCKET).floor() as i32,
    )
}

impl Packer {
    /// Where `id` stands and the radius it reserved, if it has a place.
    pub fn get(&self, id: Identity) -> Option<([f32; 2], f32)> {
        self.reserved.get(&id).copied()
    }

    /// Forgets the nest of `id`, freeing its room.
    pub fn remove(&mut self, id: Identity) {
        if let Some((at, _)) = self.reserved.remove(&id)
            && let Some(inside) = self.buckets.get_mut(&bucket(at))
        {
            inside.retain(|other| *other != id);
        }
    }

    /// Forgets every nest `keep` does not name.
    pub fn retain(&mut self, keep: impl Fn(Identity) -> bool) {
        let gone: Vec<Identity> = self
            .reserved
            .keys()
            .copied()
            .filter(|&id| !keep(id))
            .collect();
        for id in gone {
            self.remove(id);
        }
    }

    /// Whether a disc of `radius` at `at` overlaps no reserved room.
    fn vacant(&self, at: [f32; 2], radius: f32) -> bool {
        let reach = ((radius + self.widest) / BUCKET).ceil() as i32;
        let (column, row) = bucket(at);
        (column - reach..=column + reach).all(|x| {
            (row - reach..=row + reach).all(|y| {
                self.buckets.get(&(x, y)).is_none_or(|inside| {
                    inside.iter().all(|other| {
                        let (there, room) = self.reserved[other];
                        (there[0] - at[0]).hypot(there[1] - at[1]) >= room + radius
                    })
                })
            })
        })
    }

    /// Reserves `radius` for `id` at the first place along the spiral where it fits, starting
    /// `resume` world units from the middle (less a step), and moves `resume` to how far out that
    /// was. A batch of newcomers, biggest first, passes the same `resume`, so that placing a
    /// thousand costs one walk along the spiral rather than a thousand.
    pub fn place(&mut self, id: Identity, radius: f32, resume: &mut f32) -> [f32; 2] {
        self.remove(id);
        let first = ((*resume - STEP).max(0.0) / STEP).powi(2) as u32;
        let (at, distance) = (first..)
            .map(|k| {
                let (distance, angle) = spiral(STEP, 0.0, k as f32, 0.0);
                ([distance * angle.cos(), distance * angle.sin()], distance)
            })
            .find(|&(at, _)| self.vacant(at, radius))
            .expect("the spiral eventually clears every reserved room");
        *resume = distance;
        self.widest = self.widest.max(radius);
        self.reserved.insert(id, (at, radius));
        self.buckets.entry(bucket(at)).or_default().push(id);
        at
    }

    /// How far the reserved rooms reach from the middle along an axis.
    pub fn reach(&self) -> f32 {
        self.reserved
            .values()
            .map(|&(at, radius)| at[0].abs().max(at[1].abs()) + radius)
            .fold(0.0, f32::max)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.reserved.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(pid: u32) -> Identity {
        Identity {
            pid,
            start: u64::from(pid),
        }
    }

    #[test]
    fn paths_lie_in_the_region_of_their_top_directory_and_are_keyed_by_the_second() {
        let of = |path| Region::of(path);
        assert_eq!(
            of("/usr/lib/x86_64-linux-gnu/libc.so.6"),
            (Region::Usr, "usr/lib".into())
        );
        assert_eq!(
            of("/home/tom/.bash_history"),
            (Region::Home, "home/tom".into())
        );
        assert_eq!(
            of("/var/log/journal/a.journal"),
            (Region::Var, "var/log".into())
        );
        assert_eq!(of("/tmp/cc1-4f2a.s"), (Region::Tmp, "tmp".into()));
        assert_eq!(of("/dev/shm/pulse-shm-1"), (Region::Tmp, "dev/shm".into()));
        assert_eq!(of("/etc/passwd"), (Region::Other, "etc".into()));
        assert_eq!(of("/opt/app/bin/run"), (Region::Other, "opt/app".into()));
        // macOS keeps its volumes under /private and its programs outside /usr.
        assert_eq!(
            of("/private/var/log/system.log"),
            (Region::Var, "var/log".into())
        );
        assert_eq!(
            of("/Users/tom/notes.txt"),
            (Region::Home, "Users/tom".into())
        );
        assert_eq!(
            of("/Applications/Safari.app/x"),
            (Region::Usr, "Applications/Safari.app".into())
        );
        assert_eq!(of(""), (Region::Other, String::new()));
        assert_eq!(of("relative/name").0, Region::Other);
    }

    #[test]
    fn the_regions_and_the_clearing_tile_the_slab() {
        let slab = Slab::default();
        assert_eq!(slab.size(), 64.0);
        let rects: Vec<Rect> = Region::ALL
            .iter()
            .map(|&region| slab.rect(region))
            .collect();
        let area = |rect: &Rect| rect.width() * rect.depth();
        let total: f32 = rects.iter().map(area).sum::<f32>() + area(&slab.middle());
        assert!((total - 64.0 * 64.0).abs() < 0.01, "{total}");
        for (index, a) in rects.iter().enumerate() {
            for b in rects.iter().skip(index + 1).chain([&slab.middle()]) {
                let across = a.min[0] < b.max[0] - 1e-3 && b.min[0] < a.max[0] - 1e-3;
                let down = a.min[1] < b.max[1] - 1e-3 && b.min[1] < a.max[1] - 1e-3;
                assert!(!(across && down), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn a_pile_stands_inside_its_directorys_region_and_does_not_move() {
        let slab = Slab::default();
        let mut parcels = Parcels::default();
        for path in [
            "/usr/lib/libc.so.6",
            "/home/tom/notes",
            "/var/log/syslog",
            "/tmp/x",
            "/etc/hosts",
        ] {
            let (region, _) = Region::of(path);
            let at = parcels.pile(&slab, path);
            assert!(slab.rect(region).contains(at), "{path} at {at:?}");
            assert_eq!(
                parcels.pile(&slab, path),
                at,
                "asking again changes nothing"
            );
        }
        // Other directories coming and going take other cells and leave the first where it was.
        let first = parcels.pile(&slab, "/usr/lib/libc.so.6");
        for directory in ["bin", "share", "include", "libexec", "local", "src"] {
            parcels.pile(&slab, &format!("/usr/{directory}/f"));
        }
        assert_eq!(parcels.pile(&slab, "/usr/lib/libc.so.6"), first);
    }

    #[test]
    fn directories_take_different_cells_until_the_region_is_full() {
        let slab = Slab::default();
        let (columns, rows) = slab.cells(Region::Other);
        let capacity = columns * rows;
        let mut parcels = Parcels::default();
        let cells: HashSet<(i32, i32)> = (0..capacity)
            .map(|k| {
                let at = parcels.pile(&slab, &format!("/d{k}/sub/file"));
                let rect = slab.rect(Region::Other);
                (
                    ((at[0] - rect.min[0]) / (rect.width() / columns as f32)) as i32,
                    ((at[1] - rect.min[1]) / (rect.depth() / rows as f32)) as i32,
                )
            })
            .collect();
        assert_eq!(
            cells.len(),
            capacity,
            "every directory has a cell of its own"
        );
        // One more shares: it still lands inside the region.
        let at = parcels.pile(&slab, "/one-more/sub/file");
        assert!(slab.rect(Region::Other).contains(at));
    }

    #[test]
    fn the_clearing_grows_in_steps_of_one_and_a_quarter_only_when_its_nests_overflow() {
        let mut slab = Slab::default();
        assert!(!slab.grow_to_hold(16.0));
        assert_eq!(slab.size(), 64.0);
        assert!(slab.grow_to_hold(16.1));
        assert_eq!(slab.clearing(), 20.0);
        assert!(slab.grow_to_hold(30.0));
        assert!(
            (slab.clearing() - 31.25).abs() < 1e-4,
            "{}",
            slab.clearing()
        );
        assert!(!slab.grow_to_hold(10.0), "it never shrinks");
    }

    #[test]
    fn the_blankets_lie_in_the_corners_the_round_heap_of_nests_leaves_free() {
        let slab = Slab::default();
        let corners: Vec<[f32; 2]> = (0..4).map(|k| slab.blanket(k)).collect();
        for corner in &corners {
            assert!(slab.middle().contains(*corner));
            assert!(corner[0].hypot(corner[1]) > slab.clearing(), "{corner:?}");
        }
        let distinct: HashSet<(i32, i32)> =
            corners.iter().map(|c| (c[0] as i32, c[1] as i32)).collect();
        assert_eq!(distinct.len(), 4);
    }

    #[test]
    fn nests_keep_the_room_they_reserved_and_never_overlap_it() {
        let mut packer = Packer::default();
        let mut resume = 0.0;
        let radii: Vec<f32> = (0..60).map(|k| 1.0 + (k % 7) as f32 * 0.5).collect();
        for (k, &radius) in radii.iter().enumerate() {
            packer.place(id(k as u32), radius, &mut resume);
        }
        let all: Vec<([f32; 2], f32)> = (0..60)
            .map(|k| packer.get(id(k)).expect("placed"))
            .collect();
        for (a, (at, room)) in all.iter().enumerate() {
            for (b, (there, other)) in all.iter().enumerate().skip(a + 1) {
                let apart = (at[0] - there[0]).hypot(at[1] - there[1]);
                assert!(apart >= room + other - 1e-3, "{a} and {b} overlap");
            }
        }
        let before = packer.get(id(5));
        packer.place(id(500), 2.0, &mut 0.0);
        packer.remove(id(7));
        assert_eq!(packer.get(id(5)), before, "others stay where they were");
        assert!(packer.get(id(7)).is_none());
        assert!(packer.reach() > 0.0 && packer.len() == 60);
    }

    #[test]
    fn placing_the_same_nests_in_the_same_order_gives_the_same_places() {
        let run = || {
            let mut packer = Packer::default();
            let mut resume = 0.0;
            for k in 0..40 {
                packer.place(id(k), 1.0 + (k % 5) as f32, &mut resume);
            }
            (0..40)
                .map(|k| packer.get(id(k)).unwrap().0)
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_thousand_nests_are_placed_in_one_walk_along_the_spiral() {
        let mut packer = Packer::default();
        let mut resume = 0.0;
        let started = std::time::Instant::now();
        for k in 0..1000 {
            packer.place(id(k), 3.0 + (k % 3) as f32 * 0.5, &mut resume);
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(packer.len(), 1000);
    }

    #[test]
    fn retaining_frees_the_rooms_of_nests_that_left() {
        let mut packer = Packer::default();
        let mut resume = 0.0;
        let first = packer.place(id(1), 2.0, &mut resume);
        packer.place(id(2), 2.0, &mut resume);
        packer.retain(|nest| nest == id(2));
        assert!(packer.get(id(1)).is_none());
        // The middle is free again, so the next nest takes the first place on the spiral.
        assert_eq!(packer.place(id(3), 2.0, &mut 0.0), first);
    }
}
