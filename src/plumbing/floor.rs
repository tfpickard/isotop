//! Where the tanks stand: one bay on the floor for each cgroup, and a seat for each tank in its
//! bay, with no drawing. A tank keeps its seat for as long as it is a tank, and a bay keeps its
//! place until its seats outgrow what it reserved, so nothing moves when memory, CPU or the
//! population of other bays changes.
//!
//! The floor is a grid of cells 2 units on a side (`CELL`). A bay's centre sits on a cell centre,
//! its seats follow a square spiral out from the centre at a pitch of 3 units (`PITCH`: a pipe
//! then passes between the widest two tanks), and the bay is reserved a whole `n` by `n` block of
//! seats at a time, so a tank joining a bay moves nothing until the block is full.

use std::collections::BTreeMap;

use crate::layout::{Discs, Seats};
use crate::model::Identity;

/// The floor's grid cell, in world units.
pub const CELL: f32 = 2.0;
/// The distance between two seats of a bay, in world units.
pub const PITCH: f32 = 3.0;
/// What a bay's disc keeps clear beyond the seats it holds, in world units: with the 15% that
/// `Discs` reserves it leaves a corridor for pipes between two bays and more than the snap to the
/// grid can move either.
const PAD: f32 = 1.5;

/// The cell of seat `seat` on a square spiral out from the origin: right, up, left, down, in legs
/// of 1, 1, 2, 2, 3, 3 cells.
pub fn seat_cell(seat: usize) -> (i32, i32) {
    let (mut x, mut y) = (0_i32, 0_i32);
    let (mut dx, mut dy) = (1_i32, 0_i32);
    let (mut leg, mut left, mut turns) = (1_i32, 1_i32, 0_u32);
    for _ in 0..seat {
        x += dx;
        y += dy;
        left -= 1;
        if left == 0 {
            (dx, dy) = (-dy, dx);
            turns += 1;
            if turns.is_multiple_of(2) {
                leg += 1;
            }
            left = leg;
        }
    }
    (x, y)
}

/// The side, in seats, of the square block a bay with `span` seats reserves: at least 2, so a
/// bay that starts with one tank has room for a second.
fn side(span: usize) -> usize {
    ((span.max(1) as f32).sqrt().ceil() as usize).max(2)
}

/// The middle of the first `side * side` seats of a bay, in seats from seat 0, which is where the
/// block of seats a bay reserves is centred.
fn block_center(side: usize) -> [f32; 2] {
    let (mut low, mut high) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
    for seat in 0..side * side {
        let (i, j) = seat_cell(seat);
        low = (low.0.min(i), low.1.min(j));
        high = (high.0.max(i), high.1.max(j));
    }
    [
        (low.0 + high.0) as f32 * 0.5 * PITCH,
        (low.1 + high.1) as f32 * 0.5 * PITCH,
    ]
}

/// How far from the middle of its block the seats of a bay whose block is `side` seats on a side
/// reach, in world units, counting half a seat of floor beyond the outermost.
fn reach(side: usize) -> f32 {
    let center = block_center(side);
    (0..side * side)
        .map(|seat| {
            let (i, j) = seat_cell(seat);
            (i as f32 * PITCH - center[0]).hypot(j as f32 * PITCH - center[1])
        })
        .fold(0.0, f32::max)
        + PITCH * 0.5 * std::f32::consts::SQRT_2
}

/// A world coordinate moved to the nearest cell centre.
fn snap(value: f32) -> f32 {
    ((value - CELL * 0.5) / CELL).round() * CELL + CELL * 0.5
}

/// A bay as drawn: its cgroup, where its seats start, the disc it reserves and the floor it paints.
#[derive(Clone, Debug, PartialEq)]
pub struct Bay {
    pub key: String,
    /// Where seat 0 is, on a cell centre: every seat is a whole number of seats from here.
    pub origin: [f32; 2],
    /// The middle of the block of seats the bay reserves, which its disc is centred on.
    pub center: [f32; 2],
    /// How far the seats of the block the bay reserves reach from `center`; the disc it takes
    /// on the floor is `PAD` larger.
    pub reach: f32,
    /// The rectangle of floor the bay's seats in use cover, on the grid: low corner, high corner.
    pub low: [f32; 2],
    pub high: [f32; 2],
}

/// What a bay was given when it was last placed, which it keeps until it is placed again.
#[derive(Clone, Copy, Debug)]
struct Placed {
    /// Where `Discs` put the disc, to tell a bay it left alone from one it moved.
    disc: [f32; 2],
    origin: [f32; 2],
    side: usize,
}

/// The seats and the bays.
#[derive(Default)]
pub struct Floor {
    discs: Discs,
    seats: Seats,
    /// The bay each tank was last seated in.
    bay_of: BTreeMap<Identity, String>,
    placed: BTreeMap<String, Placed>,
    bays: BTreeMap<String, Bay>,
    /// The smallest and largest corner of the room round every bay so far, which only grows.
    room: Option<([f32; 2], [f32; 2])>,
}

impl Floor {
    /// Seats `members` (a tank and the key of its bay) and places their bays. A tank missing from
    /// `members` loses its seat, which goes to the next newcomer of its bay. A bay reserves a
    /// square block of seats and never a smaller one than it has had, so tanks leaving it move
    /// nothing; it is placed again only when its tanks outgrow what `Discs` reserved for it.
    pub fn arrange(&mut self, members: &[(Identity, String)]) {
        self.seats
            .assign(members.iter().map(|(tank, bay)| (*tank, bay.as_str())));
        self.bay_of = members.iter().cloned().collect();
        let mut sides: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, bay) in members {
            sides.entry(bay).or_insert_with(|| {
                let kept = self.placed.get(bay).map_or(0, |placed| placed.side);
                side(self.seats.span(bay)).max(kept)
            });
        }
        let discs: Vec<(String, f32)> = sides
            .iter()
            .map(|(&bay, &side)| (bay.to_owned(), reach(side) + PAD))
            .collect();
        let centers = self.discs.arrange(&discs);
        let mut placed = BTreeMap::new();
        let mut bays = BTreeMap::new();
        for (&key, &side) in &sides {
            let disc = centers[key];
            let block = block_center(side);
            let origin = match self.placed.get(key) {
                Some(before) if before.disc == disc => before.origin,
                _ => [0, 1].map(|k| snap(disc[k] - block[k])),
            };
            placed.insert(key.to_owned(), Placed { disc, origin, side });
            let (mut low, mut high) = ([0, 0], [0, 0]);
            for seat in 0..self.seats.span(key) {
                let (i, j) = seat_cell(seat);
                low = [low[0].min(i), low[1].min(j)];
                high = [high[0].max(i), high[1].max(j)];
            }
            let edge = |cell: i32, outward: f32, at: f32| {
                at + cell as f32 * PITCH + outward * (PITCH * 0.5 + 0.5)
            };
            bays.insert(
                key.to_owned(),
                Bay {
                    key: key.to_owned(),
                    origin,
                    center: [origin[0] + block[0], origin[1] + block[1]],
                    reach: reach(side),
                    low: [0, 1].map(|k| (edge(low[k], -1.0, origin[k]) / CELL).floor() * CELL),
                    high: [0, 1].map(|k| (edge(high[k], 1.0, origin[k]) / CELL).ceil() * CELL),
                },
            );
        }
        self.placed = placed;
        self.bays = bays;
        for bay in self.bays.values() {
            let (low, high) = self.room.get_or_insert((bay.low, bay.high));
            for k in 0..2 {
                low[k] = low[k].min(bay.low[k]);
                high[k] = high[k].max(bay.high[k]);
            }
        }
    }

    /// Where `tank` stands, if it has a seat.
    pub fn position(&self, tank: Identity) -> Option<[f32; 2]> {
        let seat = self.seats.seat(tank)?;
        let bay = self.bays.get(self.bay_of.get(&tank)?)?;
        let (i, j) = seat_cell(seat);
        Some([
            bay.origin[0] + i as f32 * PITCH,
            bay.origin[1] + j as f32 * PITCH,
        ])
    }

    /// The bays, by key.
    pub fn bays(&self) -> impl Iterator<Item = &Bay> {
        self.bays.values()
    }

    /// The room that holds every bay seen so far, with a margin of one cell: low corner, high
    /// corner. None before the first tank.
    pub fn room(&self) -> Option<([f32; 2], [f32; 2])> {
        self.room
            .map(|(low, high)| (low.map(|v| v - CELL), high.map(|v| v + CELL)))
    }
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

    fn members(layout: &[(u32, &str)]) -> Vec<(Identity, String)> {
        layout
            .iter()
            .map(|&(pid, bay)| (who(pid), bay.to_owned()))
            .collect()
    }

    #[test]
    fn the_spiral_visits_each_cell_once_in_square_rings() {
        let cells: Vec<(i32, i32)> = (0..25).map(seat_cell).collect();
        assert_eq!(&cells[..5], [(0, 0), (1, 0), (1, 1), (0, 1), (-1, 1)]);
        let mut unique = cells.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 25);
        // The first n*n seats fit in a square of side n, for every n.
        for n in 2..6_i32 {
            let block = &cells[..(n * n) as usize];
            let span = |axis: fn(&(i32, i32)) -> i32| {
                block.iter().map(axis).max().unwrap() - block.iter().map(axis).min().unwrap() + 1
            };
            assert!(span(|c| c.0) <= n && span(|c| c.1) <= n, "n = {n}");
        }
    }

    #[test]
    fn a_tank_keeps_its_seat_while_others_leave_and_join() {
        let mut floor = Floor::default();
        floor.arrange(&members(&[(1, "a"), (2, "a"), (3, "a"), (4, "b")]));
        let before: Vec<_> = [1, 2, 3, 4]
            .into_iter()
            .map(|pid| floor.position(who(pid)).unwrap())
            .collect();
        // 2 leaves and 5 and 6 join bay a; 4 stays alone in b.
        floor.arrange(&members(&[
            (1, "a"),
            (3, "a"),
            (5, "a"),
            (6, "a"),
            (4, "b"),
        ]));
        for (pid, was) in [(1, before[0]), (3, before[2]), (4, before[3])] {
            assert_eq!(floor.position(who(pid)), Some(was), "pid {pid}");
        }
        assert_eq!(
            floor.position(who(5)),
            Some(before[1]),
            "the freed seat goes to the next newcomer"
        );
        assert_eq!(floor.position(who(2)), None);
    }

    #[test]
    fn a_bay_keeps_its_place_until_its_block_of_seats_is_full() {
        let mut floor = Floor::default();
        floor.arrange(&members(&[(1, "a"), (50, "b"), (60, "c")]));
        let first = floor.bays().map(|bay| bay.center).collect::<Vec<_>>();
        // Up to four tanks fit the 2 by 2 block the bay reserved.
        floor.arrange(&members(&[
            (1, "a"),
            (2, "a"),
            (3, "a"),
            (4, "a"),
            (50, "b"),
            (60, "c"),
        ]));
        let second = floor.bays().map(|bay| bay.center).collect::<Vec<_>>();
        assert_eq!(first, second, "four tanks fill the block and move nothing");
        // A fifth outgrows it: the bay is placed again, and the others stay where they are.
        let before_b = floor.bays().find(|bay| bay.key == "b").unwrap().center;
        floor.arrange(&members(&[
            (1, "a"),
            (2, "a"),
            (3, "a"),
            (4, "a"),
            (5, "a"),
            (50, "b"),
            (60, "c"),
        ]));
        assert_eq!(
            floor.bays().find(|bay| bay.key == "b").unwrap().center,
            before_b
        );
    }

    #[test]
    fn bays_stand_on_the_grid_and_clear_of_one_another() {
        let mut floor = Floor::default();
        let layout: Vec<(u32, String)> = (0..40)
            .map(|pid| (pid, format!("bay-{}", pid % 7)))
            .collect();
        let borrowed: Vec<(u32, &str)> = layout.iter().map(|(p, b)| (*p, b.as_str())).collect();
        floor.arrange(&members(&borrowed));
        let bays: Vec<&Bay> = floor.bays().collect();
        for bay in &bays {
            for k in 0..2 {
                assert_eq!(
                    (bay.origin[k] - CELL * 0.5).rem_euclid(CELL),
                    0.0,
                    "a bay's first seat is on a cell centre"
                );
                assert_eq!(bay.low[k].rem_euclid(CELL), 0.0);
                assert_eq!(bay.high[k].rem_euclid(CELL), 0.0);
            }
        }
        for (i, a) in bays.iter().enumerate() {
            for b in &bays[i + 1..] {
                let apart = (a.center[0] - b.center[0]).hypot(a.center[1] - b.center[1]);
                assert!(
                    apart + 2.0 * CELL * 0.5 * 2.0_f32.sqrt() >= a.reach + b.reach - 1e-3,
                    "{} and {} overlap",
                    a.key,
                    b.key
                );
            }
        }
        // Every tank is inside the floor of its bay.
        for (pid, bay) in &layout {
            let at = floor.position(who(*pid)).unwrap();
            let bay = floor
                .bays()
                .find(|candidate| &candidate.key == bay)
                .unwrap();
            assert!(
                (0..2).all(|k| at[k] > bay.low[k] && at[k] < bay.high[k]),
                "{pid} at {at:?} outside {bay:?}"
            );
        }
    }

    #[test]
    fn the_room_only_grows() {
        let mut floor = Floor::default();
        assert_eq!(floor.room(), None);
        floor.arrange(&members(&[(1, "a"), (2, "b")]));
        let (low, high) = floor.room().unwrap();
        floor.arrange(&members(&[(1, "a")]));
        assert_eq!(
            floor.room(),
            Some((low, high)),
            "a bay that left leaves its floor"
        );
        floor.arrange(&members(&(1..30).map(|pid| (pid, "a")).collect::<Vec<_>>()));
        let (bigger_low, bigger_high) = floor.room().unwrap();
        assert!((0..2).all(|k| bigger_low[k] <= low[k] && bigger_high[k] >= high[k]));
    }
}
