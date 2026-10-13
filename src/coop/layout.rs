//! Where things stand: flock keys, house discs, perches, nests, feeders and the fence, all
//! stable under changes of CPU, memory and population.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::model::{CoreKind, Cpu, Kind, Measured, Process, Snapshot};
use crate::platform::DeletedFile;

use super::Coop;

/// Brooding hens sit on straw pads PAD_HEIGHT high beside the nest box, clear of its eggs, and
/// hens blocked on a lock queue after them; seats are NEST_GAP apart, and close up to at least
/// MIN_SEAT apart when more hens come than the flock's reserved space holds.
pub(super) const PAD_HEIGHT: f32 = 0.12;
pub(super) const NEST_GAP: f32 = 0.1;
const MIN_SEAT: f32 = 0.15;
/// Perches: bars of six seats, four bars to a ladder, stepping up and back.
pub(super) const SEAT: f32 = 1.2;
pub(super) const BAR_SEATS: usize = 6;
pub(super) const LADDER_BARS: usize = 4;
pub(super) const RUNG_STEP: f32 = 0.9;
const RUNG_RISE: f32 = 0.6;
const LADDER_WIDTH: f32 = SEAT * BAR_SEATS as f32 + 1.0;
const LADDER_DEPTH: f32 = RUNG_STEP * LADDER_BARS as f32 + 1.0;
/// Ladder cell rows sit this far in front of the disc centre; the henhouse takes the row behind.
const LADDER_Y: f32 = 2.3;
pub(super) const HOUSE_Y: f32 = LADDER_Y - LADDER_DEPTH;
pub(super) const HOUSE_HALF: [f32; 2] = [2.0, 1.3];
pub(super) const NEST_HALF: [f32; 2] = [0.6, 0.8];
/// Feeders: slot width, depth of one row of troughs with room for a queue, and the trough.
pub(super) const SLOT: f32 = 1.6;
pub(super) const FEEDER_ROW: f32 = 6.0;
pub(super) const TROUGH_DEPTH: f32 = 0.7;
/// Distance of the hedge outside the fence.
pub(super) const HEDGE: f32 = 2.2;
/// Grass kept between the houses' reserved discs and the fence.
pub(super) const YARD_MARGIN: f32 = 2.5;
/// The narrowest yard, wide enough for a row of feeders.
pub(super) const MIN_YARD: f32 = 24.0;

pub(super) struct Feeder {
    pub(super) cpu: u32,
    pub(super) kind: CoreKind,
    pub(super) busy: f32,
    pub(super) center: [f32; 2],
    pub(super) slots: usize,
    /// The one trough every running chicken shares when the OS cannot say which CPU a process
    /// last ran on. Its `cpu` is meaningless and its `busy` is the mean over the CPUs.
    pub(super) shared: bool,
}

impl Feeder {
    pub(super) fn length(&self) -> f32 {
        self.slots as f32 * SLOT + 0.4
    }

    /// Where eating chickens stand, behind the trough as seen from the yard.
    pub(super) fn eating_line(&self) -> f32 {
        self.center[1] - TROUGH_DEPTH * 0.5 - 0.7
    }

    pub(super) fn slot(&self, slot: usize) -> [f32; 2] {
        [
            self.center[0] + (slot as f32 - (self.slots as f32 - 1.0) * 0.5) * SLOT,
            self.eating_line(),
        ]
    }
}

pub(super) struct Flock {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) members: usize,
    pub(super) center: [f32; 2],
    pub(super) span: usize,
    /// The radius of the disc reserved around `center`, which the nest's seats stay inside.
    pub(super) reserved: f32,
    pub(super) nest: Nest,
}

/// What a flock's nest shows, from the members whose descriptor tables were read.
#[derive(Clone, Debug, Default)]
pub(super) struct Nest {
    /// Open regular files summed over members, each counting a file once however many of its
    /// descriptors refer to it.
    pub(super) files: u64,
    /// Members whose tables could not be read, and those not read yet; they add nothing.
    pub(super) unreadable: usize,
    pub(super) pending: usize,
    /// Distinct files deleted while still open, across members: an inherited descriptor on a
    /// rotated log is one file however many processes hold it.
    pub(super) deleted: BTreeSet<DeletedFile>,
    /// Whether some member's table was only partly examined: `files` is then estimated and
    /// `deleted` a lower bound.
    pub(super) partial: bool,
}

impl Nest {
    pub(super) fn add(&mut self, process: &Process) {
        match &process.files {
            Measured::Known(files) => {
                self.files += u64::from(files.open);
                self.deleted.extend(files.deleted.iter().copied());
                self.partial |= files.partial;
            }
            Measured::Unreadable => self.unreadable += 1,
            Measured::Pending => self.pending += 1,
        }
    }

    pub(super) fn deleted_bytes(&self) -> u64 {
        self.deleted
            .iter()
            .fold(0, |total, file| total.saturating_add(file.size))
    }
}

impl Flock {
    /// The middle of the nest box on the east side of the henhouse.
    pub(super) fn nest(&self) -> [f32; 2] {
        [
            self.center[0] + HOUSE_HALF[0] + NEST_HALF[0],
            self.center[1] + HOUSE_Y,
        ]
    }
}

/// `count` seats beside a flock's nest box for chickens of up to `radius`, each with whether it
/// is in the first column, which hugs the box. The seats form a grid east of the box, filled a
/// column at a time from the box's north end, clear of the eggs, the house and the flock's
/// ladders and inside its reserved disc, so neither brooders nor a long queue reach another
/// flock's space or the feeders. When fewer seats fit than are needed the grid closes up, down to
/// MIN_SEAT apart (the chickens then overlap), and past that the seats are reused.
pub(super) fn nest_seats(flock: &Flock, count: usize, radius: f32) -> Vec<([f32; 2], bool)> {
    if count == 0 {
        return Vec::new();
    }
    let nest = flock.nest();
    let center = flock.center;
    let ladders: Vec<[f32; 2]> = (0..ladders(flock.span))
        .map(|g| {
            let cell = ladder(g);
            [center[0] + cell[0], center[1] + cell[1]]
        })
        .collect();
    let rim = nest[0] + NEST_HALF[0] + NEST_GAP;
    let top = nest[1] + NEST_HALF[1];
    let fits = |spot: [f32; 2]| {
        (spot[0] - center[0]).hypot(spot[1] - center[1]) + radius <= flock.reserved
            && ladders.iter().all(|cell| {
                (spot[0] - cell[0]).abs() >= LADDER_WIDTH * 0.5 + radius
                    || (spot[1] - cell[1]).abs() >= LADDER_DEPTH * 0.5 + radius
            })
    };
    let mut step = (2.0 * radius + NEST_GAP).max(MIN_SEAT);
    loop {
        let columns = ((center[0] + flock.reserved - rim) / step).max(1.0) as usize;
        let rows = ((top - center[1] + flock.reserved) / step).max(1.0) as usize;
        let seats: Vec<([f32; 2], bool)> = (0..columns)
            .flat_map(|column| {
                (0..rows).map(move |row| {
                    let x = rim + radius + column as f32 * step;
                    ([x, top - step * (row as f32 + 0.5)], column == 0)
                })
            })
            .filter(|&(spot, _)| fits(spot))
            .collect();
        if seats.len() >= count || step <= MIN_SEAT {
            if seats.is_empty() {
                return vec![([rim + radius, top - radius], true); count];
            }
            return seats.iter().copied().cycle().take(count).collect();
        }
        step = (step * 0.8).max(MIN_SEAT);
    }
}

/// The flock a process belongs to: its cgroup, or its group without one; kernel threads share one.
pub(super) fn flock_key(process: &Process) -> String {
    if process.kind == Kind::Kernel {
        "kernel".into()
    } else if process.cgroup.is_empty() {
        process.group.clone()
    } else {
        process.cgroup.clone()
    }
}

pub(super) fn flock_label(process: &Process) -> String {
    if process.kind == Kind::Kernel {
        "kernel".into()
    } else {
        process.group.clone()
    }
}

/// Body radius from memory: volume proportional to memory, from 0.3 at about 16 MiB and below
/// (0.3 / 0.12 = 2.5, and 2.5 cubed is 15.6) to 1.2 at 1000 MiB and above, so small processes
/// stay visible and large ones keep their spread.
pub(super) fn body_radius(memory: f32) -> f32 {
    (0.12 * (memory / 1_048_576.0).max(0.0).cbrt()).clamp(0.3, 1.2)
}

/// Ladder centres around a house in the order they fill, nearest first and the house's front
/// before its back. The cell grid is fixed, so a seat never moves while its house stays put.
pub(super) fn ladder(index: usize) -> [f32; 2] {
    static CELLS: OnceLock<Vec<[f32; 2]>> = OnceLock::new();
    let cells = CELLS.get_or_init(|| {
        let mut cells: Vec<(f32, i32, i32)> = Vec::new();
        for row in -6..=8_i32 {
            for column in -6..=6_i32 {
                if (column, row) == (0, -1) {
                    continue;
                }
                let center = [
                    column as f32 * LADDER_WIDTH,
                    LADDER_Y + row as f32 * LADDER_DEPTH,
                ];
                let behind = if row < 0 { 2.0 } else { 0.0 };
                cells.push((center[0].hypot(center[1]) + behind, -row, -column));
            }
        }
        cells.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.1, a.2).cmp(&(b.1, b.2))));
        cells
            .into_iter()
            .map(|(_, row, column)| {
                [
                    -column as f32 * LADDER_WIDTH,
                    LADDER_Y - row as f32 * LADDER_DEPTH,
                ]
            })
            .collect()
    });
    match cells.get(index) {
        Some(&cell) => cell,
        None => [
            0.0,
            LADDER_Y + (9 + index - cells.len()) as f32 * LADDER_DEPTH,
        ],
    }
}

/// A seat's ground point relative to its house's disc centre, and its perch height.
pub(super) fn seat_offset(seat: usize) -> ([f32; 2], f32) {
    let cell = ladder(seat / (BAR_SEATS * LADDER_BARS));
    let bar = seat % (BAR_SEATS * LADDER_BARS) / BAR_SEATS;
    let place = seat % BAR_SEATS;
    (
        [
            cell[0] + (place as f32 - (BAR_SEATS as f32 - 1.0) * 0.5) * SEAT,
            cell[1] + RUNG_STEP * (1.5 - bar as f32),
        ],
        bar_height(bar),
    )
}

pub(super) fn bar_height(bar: usize) -> f32 {
    RUNG_RISE * (bar as f32 + 1.0)
}

/// Radius of the disc a house and its perches for `span` seats need.
/// Perch ladders for a flock of `span` seats.
fn ladders(span: usize) -> usize {
    span.div_ceil(BAR_SEATS * LADDER_BARS).max(1)
}

pub(super) fn house_radius(span: usize) -> f32 {
    let ladders = ladders(span);
    let house = (HOUSE_HALF[0] + 2.0 * NEST_HALF[0]).hypot(-HOUSE_Y + HOUSE_HALF[1]);
    (0..ladders)
        .map(|g| {
            let cell = ladder(g);
            (cell[0].abs() + LADDER_WIDTH * 0.5).hypot(cell[1].abs() + LADDER_DEPTH * 0.4)
        })
        .fold(house, f32::max)
        + 0.5
}

/// The distance from home beyond which a forager is drawn back: two and a half house radii.
pub(super) fn home_range(radius: f32) -> f32 {
    (2.5 * radius).max(6.0)
}

/// Feeders in rows below the houses, between `left` and `right`, the first row's troughs just
/// south of `top`, sorted by kind then id. With `shared`, one more trough follows in a row of its
/// own, centred in the yard and as wide as the CPUs are many (3 to 8 slots). Returns the feeders
/// and the depth of the band they fill.
pub(super) fn place_feeders(
    snapshot: &Snapshot,
    left: f32,
    right: f32,
    top: f32,
    shared: bool,
) -> (Vec<Feeder>, f32) {
    let mut cpus: Vec<Cpu> = snapshot.cpus.clone();
    if cpus.is_empty() {
        cpus = (0..snapshot.cores.max(1) as u32)
            .map(|id| Cpu {
                id,
                kind: CoreKind::Unknown,
                busy: None,
                mhz: None,
                wait: None,
            })
            .collect();
    }
    cpus.sort_by_key(|cpu| (cpu.kind, cpu.id));
    let mut feeders = Vec::with_capacity(cpus.len());
    let (mut cursor, mut row) = (left + 1.0, 0);
    for cpu in &cpus {
        let slots = if cpu.kind == CoreKind::Performance {
            3
        } else {
            2
        };
        let length = slots as f32 * SLOT + 0.4;
        if cursor + length > right - 1.0 && cursor > left + 1.0 {
            cursor = left + 1.0;
            row += 1;
        }
        feeders.push(Feeder {
            cpu: cpu.id,
            kind: cpu.kind,
            busy: cpu.busy.unwrap_or(0.0).clamp(0.0, 1.0),
            center: [cursor + length * 0.5, top - 1.2 - row as f32 * FEEDER_ROW],
            slots,
            shared: false,
        });
        cursor += length + 1.2;
    }
    if shared {
        row += 1;
        let slots = cpus.len().clamp(3, 8);
        feeders.push(Feeder {
            cpu: u32::MAX,
            kind: CoreKind::Unknown,
            busy: cpus
                .iter()
                .map(|cpu| cpu.busy.unwrap_or(0.0).clamp(0.0, 1.0))
                .sum::<f32>()
                / cpus.len() as f32,
            center: [(left + right) * 0.5, top - 1.2 - row as f32 * FEEDER_ROW],
            slots,
            shared: true,
        });
    }
    (feeders, (row + 1) as f32 * FEEDER_ROW)
}

impl Coop {
    /// A point `outside` units beyond the fence at fraction `t` of the way round it.
    pub(super) fn perimeter(&self, t: f32, outside: f32) -> [f32; 2] {
        let (low, high) = (self.yard.low, self.yard.high);
        let (width, height) = (high[0] - low[0], high[1] - low[1]);
        let mut d = t.rem_euclid(1.0) * 2.0 * (width + height);
        if d < width {
            return [low[0] + d, high[1] + outside];
        }
        d -= width;
        if d < height {
            return [high[0] + outside, high[1] - d];
        }
        d -= height;
        if d < width {
            return [high[0] - d, low[1] - outside];
        }
        d -= width;
        [low[0] - outside, low[1] + d]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    use crate::coop::draw::{CLUTCH, clutch};
    use crate::coop::fixtures::{
        coop_of, deleted_file, holding, legend_of, nest_line, notes, process, render, snapshot,
    };
    use crate::model::{Identity, Measured, Process, Snapshot, demo};
    use crate::platform::DeletedFile;
    use crate::render::{Camera, RenderInput, Scene};
    use crate::views::view;

    #[test]
    fn clutch_size_grows_with_open_files_and_is_capped() {
        // log2(46 341) is just over 15.5, so 46 340 files already fill the clutch.
        assert_eq!(
            [0, 1, 2, 7, 140, 46_339, 46_340, 46_341].map(clutch),
            [0, 1, 2, 3, 7, 15, 16, 16]
        );
        assert!((0..100_000).all(|files| clutch(files) <= clutch(files + 1)));
        assert_eq!(clutch(u64::MAX), CLUTCH);
        let path = "/system.slice/files.service";
        let a = holding(1, path, 60, &[]);
        let b = holding(2, path, 80, &[]);
        let mut hidden = process(3, path);
        hidden.files = Measured::Unreadable;
        let mut fresh = process(4, path);
        fresh.files = Measured::Pending;
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![a, b, hidden, fresh], 1.0), 1.0);
        assert_eq!(coop_of(&scene).flocks[0].nest.files, 140);
        assert_eq!(clutch(coop_of(&scene).flocks[0].nest.files), 7);
        assert!(nest_line(&scene, 1).ends_with(
            "nest: 7 eggs for at least 140 open files (1 unreadable, 1 not read yet of 4 members)"
        ));
        assert!(
            notes(&scene, 1).contains(&"60 open files (eggs in the files.service nest)".into())
        );
        assert!(notes(&scene, 3).contains(&"open files unreadable (permissions)".into()));
        assert!(notes(&scene, 4).contains(&"open files not read yet".into()));
        // Only the table that could not be read counts against permissions.
        assert!(legend_of(&scene).contains("no files for 1 (permissions)"));
    }

    #[test]
    fn a_flock_with_no_readable_table_says_so_instead_of_zero_files() {
        let path = "/system.slice/root.service";
        let mut hidden = process(1, path);
        hidden.files = Measured::Unreadable;
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![hidden.clone()], 1.0), 1.0);
        assert_eq!(
            nest_line(&scene, 1),
            "root.service nest: no eggs, open files unreadable (permissions)"
        );
        hidden.files = Measured::Pending;
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![hidden], 1.0), 1.0);
        assert_eq!(
            nest_line(&scene, 1),
            "root.service nest: no eggs, open files not read yet"
        );
        assert!(!legend_of(&scene).contains("no files"));
    }

    #[test]
    fn an_estimated_table_is_shown_as_approximate() {
        let path = "/system.slice/proxy.service";
        let mut proxy = holding(1, path, 95_034, &[deleted_file(5, 1 << 20)]);
        if let Measured::Known(files) = &mut proxy.files {
            files.partial = true;
        }
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(vec![proxy], 1.0), 1.0);
        let notes = notes(&scene, 1);
        assert!(notes.contains(&"about 95034 open files (eggs in the proxy.service nest)".into()));
        assert!(
            notes
                .contains(&"at least 1 deleted file still open, 1.0 MiB held (rotten eggs)".into())
        );
        assert!(
            nest_line(&scene, 1)
                .contains("16 eggs for about 95034 open files, 1 rotten for at least 1 deleted")
        );
    }

    #[test]
    fn rotten_eggs_equal_the_deleted_count() {
        let path = "/system.slice/logger.service";
        let log = deleted_file(1, 1_288_490_189);
        let rotated = holding(1, path, 9, &[log, deleted_file(2, 1 << 20)]);
        let cache = holding(2, path, 4, &[deleted_file(3, 1 << 20)]);
        let mut scene = Scene::new();
        let mut sample = snapshot(vec![rotated, cache], 1.0);
        render(&mut scene, &sample, 1.0);
        assert_eq!(coop_of(&scene).flocks[0].nest.deleted.len(), 3);
        assert!(
            notes(&scene, 1)
                .contains(&"2 deleted files still open, 1.2 GiB held (rotten eggs)".into())
        );
        assert!(nest_line(&scene, 2).contains(", 3 rotten for 3 deleted files still open"));
        // Beyond the cap the nest shows ROTTEN and the inspector keeps the exact count.
        let many: Vec<DeletedFile> = (10..21).map(|inode| deleted_file(inode, 1)).collect();
        sample.processes[0] = holding(1, path, 11, &many);
        sample.elapsed = 2.0;
        render(&mut scene, &sample, 2.0);
        assert_eq!(coop_of(&scene).flocks[0].nest.deleted.len(), 12);
        assert!(nest_line(&scene, 2).contains(", 8 rotten for 12 deleted files still open"));
    }

    #[test]
    fn a_deleted_file_several_members_hold_is_one_rotten_egg() {
        // A master and its workers inherit one descriptor on a log that was rotated by removal.
        let path = "/system.slice/nginx.service";
        let log = deleted_file(77, 1 << 30);
        let workers: Vec<Process> = (1..=5).map(|pid| holding(pid, path, 3, &[log])).collect();
        let mut scene = Scene::new();
        render(&mut scene, &snapshot(workers, 1.0), 1.0);
        let nest = &coop_of(&scene).flocks[0].nest;
        assert_eq!(nest.deleted.len(), 1);
        assert_eq!(nest.deleted_bytes(), 1 << 30);
        assert!(nest_line(&scene, 1).contains(", 1 rotten for 1 deleted file still open (1.0 GiB"));
    }

    #[test]
    fn the_yard_and_feeders_stay_put_when_the_outermost_flock_leaves() {
        let cgroups: Vec<String> = (0..6)
            .map(|k| format!("/system.slice/f{k}.service"))
            .collect();
        let sample = |count: usize, elapsed: f64| {
            let processes = cgroups[..count]
                .iter()
                .enumerate()
                .flat_map(|(k, cgroup)| {
                    (0..30).map(move |n| process(1000 * (k as u32 + 1) + n, cgroup))
                })
                .collect();
            snapshot(processes, elapsed)
        };
        let mut scene = Scene::new();
        render(&mut scene, &sample(6, 1.0), 1.0);
        let layout = |scene: &Scene| {
            (
                coop_of(scene).yard.low,
                coop_of(scene).yard.high,
                coop_of(scene).band_top,
                coop_of(scene)
                    .feeders
                    .iter()
                    .map(|f| f.center)
                    .collect::<Vec<_>>(),
                coop_of(scene).yard.eyes.clone(),
            )
        };
        let before = layout(&scene);
        // Drop whichever flock stands furthest south, the one that sets the yard's southern edge.
        let outermost = coop_of(&scene)
            .flocks
            .iter()
            .min_by(|a, b| a.center[1].total_cmp(&b.center[1]))
            .map(|flock| flock.key.clone())
            .unwrap();
        let mut smaller = sample(6, 2.0);
        smaller.processes.retain(|p| p.cgroup != outermost);
        render(&mut scene, &smaller, 2.0);
        assert_eq!(coop_of(&scene).flocks.len(), 5);
        assert_eq!(layout(&scene), before);
        // A flock arriving beyond the fence grows it, and the fence does not shrink back.
        let mut scene = Scene::new();
        render(&mut scene, &sample(2, 1.0), 1.0);
        let small = layout(&scene);
        render(&mut scene, &sample(6, 2.0), 2.0);
        let grown = layout(&scene);
        assert!(grown.0[0] <= small.0[0] && grown.0[1] <= small.0[1]);
        assert!(grown.1[0] >= small.1[0] && grown.1[1] >= small.1[1]);
        assert!(grown.0 != small.0 || grown.1 != small.1);
        render(&mut scene, &sample(2, 3.0), 3.0);
        assert_eq!(layout(&scene), grown);
    }

    #[test]
    fn houses_and_perches_stay_put_when_resources_or_population_change() {
        let mut sample = demo(30.0, 128);
        let mut scene = Scene::new();
        render(&mut scene, &sample, 30.0);
        let houses = |scene: &Scene| -> BTreeMap<String, [f32; 2]> {
            coop_of(scene)
                .flocks
                .iter()
                .map(|f| (f.key.clone(), f.center))
                .collect()
        };
        let seats = |scene: &Scene| -> HashMap<Identity, [f32; 2]> {
            coop_of(scene)
                .yard
                .chickens
                .iter()
                .map(|c| {
                    let flock = &coop_of(scene).flocks[c.flock];
                    let (offset, _) = seat_offset(coop_of(scene).seats.seat(c.id).unwrap());
                    (
                        c.id,
                        [flock.center[0] + offset[0], flock.center[1] + offset[1]],
                    )
                })
                .collect()
        };
        let (before_houses, before_seats) = (houses(&scene), seats(&scene));
        for (k, p) in sample.processes.iter_mut().enumerate() {
            p.cpu = (k % 7) as f32 * 13.0;
            p.memory = (k as u64 % 11 + 1) * (90 << 20);
        }
        let removed = sample.processes.remove(37).id;
        sample.elapsed = 31.0;
        render(&mut scene, &sample, 31.0);
        assert_eq!(houses(&scene), before_houses);
        let after_seats = seats(&scene);
        for (id, seat) in &after_seats {
            assert_eq!(before_seats[id], *seat);
        }
        assert!(!after_seats.contains_key(&removed));
    }

    fn draw_some(
        scene: &mut Scene,
        snapshot: &Snapshot,
        time: f32,
        limit: usize,
        selected: Option<Identity>,
        focus: Option<Identity>,
    ) {
        let frame = scene.render(&RenderInput {
            width: 160,
            height: 90,
            selected,
            limit,
            focus,
            ..RenderInput::small(snapshot, view("coop"), &Camera::default(), time)
        });
        scene.spare = frame.release();
    }

    type Places = (BTreeMap<String, [f32; 2]>, BTreeMap<Identity, [f32; 2]>);

    /// House centres by flock and chicken positions by process, of what is drawn.
    fn places(scene: &Scene) -> Places {
        let houses = coop_of(scene)
            .flocks
            .iter()
            .map(|flock| (flock.key.clone(), flock.center))
            .collect();
        let chickens = coop_of(scene)
            .yard
            .chickens
            .iter()
            .map(|chicken| (chicken.id, chicken.position))
            .collect();
        (houses, chickens)
    }

    /// The demo yard after five seconds of walking on one sample, so every chicken has left the
    /// spot it hatched at; returns the scene and the time of its last frame.
    fn settled_demo(sample: &Snapshot) -> (Scene, f32) {
        let mut scene = Scene::new();
        let mut time = 30.0;
        for _ in 0..20 {
            draw_some(&mut scene, sample, time, 4096, None, None);
            time += 0.25;
        }
        (scene, time - 0.25)
    }

    /// Asserts that what `scene` draws now stands where it stood in `before`.
    fn in_place(scene: &Scene, before: &Places) {
        let (houses, chickens) = places(scene);
        for (key, center) in &houses {
            assert_eq!(before.0[key], *center, "house of {key}");
        }
        for (id, position) in &chickens {
            assert_eq!(before.1[id], *position, "chicken {id:?}");
        }
    }

    #[test]
    fn focusing_and_clearing_focus_keeps_every_house_and_chicken_in_place() {
        let sample = demo(30.0, 160);
        let (mut scene, time) = settled_demo(&sample);
        let before = places(&scene);
        let fence = (coop_of(&scene).yard.low, coop_of(&scene).yard.high);
        // One family of sixteen, all in one flock. The frames repeat the last time, so the yard
        // does not step and anything that moves was moved by the change of focus.
        let root = sample.processes[16].id;
        draw_some(&mut scene, &sample, time, 4096, None, Some(root));
        let (houses, chickens) = places(&scene);
        assert_eq!((houses.len(), chickens.len()), (1, 16));
        in_place(&scene, &before);
        draw_some(&mut scene, &sample, time, 4096, None, None);
        assert_eq!(places(&scene), before);
        assert_eq!((coop_of(&scene).yard.low, coop_of(&scene).yard.high), fence);
    }

    #[test]
    fn processes_beyond_the_limit_keep_their_flocks_place() {
        let sample = demo(30.0, 160);
        let (mut scene, time) = settled_demo(&sample);
        let before = places(&scene);
        let fence = (coop_of(&scene).yard.low, coop_of(&scene).yard.high);
        draw_some(&mut scene, &sample, time, 80, None, None);
        assert!(places(&scene).0.len() < before.0.len());
        in_place(&scene, &before);
        // Selecting a process beyond the limit swaps it in, alone in its flock: its house is
        // where its whole flock had it, sized for every member.
        let selected = sample.processes[150].id;
        let key = flock_key(&sample.processes[150]);
        let span = coop_of(&scene).seats.span(&key);
        draw_some(&mut scene, &sample, time, 80, Some(selected), None);
        assert!(places(&scene).0.contains_key(&key));
        assert!(places(&scene).1.contains_key(&selected));
        assert_eq!(coop_of(&scene).seats.span(&key), span);
        in_place(&scene, &before);
        draw_some(&mut scene, &sample, time, 4096, None, None);
        assert_eq!(places(&scene), before);
        assert_eq!((coop_of(&scene).yard.low, coop_of(&scene).yard.high), fence);
    }
}
