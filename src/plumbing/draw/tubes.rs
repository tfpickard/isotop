//! Brass pneumatic tubes overhead, for Unix sockets, with the capsules that wait in them, and
//! green hoses on the floor, for loopback TCP, with the beads that run along them.
//!
//! A tube is a skeleton (brass bands round a pale spine) with a lane for each direction; the
//! capsules of a lane wait still at its receiving end, one for every 4 KiB the receiver has not
//! read (a stream) or one while a datagram waits, at most twelve, and they do not travel: nothing
//! measures how fast a queue is fed. A hose carries beads at the exact rate each way.

use super::{BRASS, CAPSULE, DULL, GLASS, HOSE, HOSE_R, Ink, TUBE_R, add, highlighted, scale};
use crate::model::Queue;
use crate::palette::tint;
use crate::plumbing::path;
use crate::render::Point;
use crate::render::strokes::Dash;

/// The unread bytes one capsule stands for.
pub const CAPSULE_BYTES: u64 = 4096;
/// The most capsules a lane draws; the rest are counted in its label.
pub const MOST_CAPSULES: usize = 12;

/// How many capsules a lane holds, and how many of them are drawn: None when the queue was not
/// measured, which is not an empty lane. A started 4 KiB counts as one capsule, so a lane with
/// any unread byte is never drawn empty; a datagram waiting adds one.
pub fn capsules(queue: Option<Queue>) -> Option<(usize, usize)> {
    let queue = queue?;
    let total = queue.unread.div_ceil(CAPSULE_BYTES) as usize + usize::from(queue.datagram_waiting);
    Some((total.min(MOST_CAPSULES), total))
}

/// The distances along a lane where its `count` capsules wait, for a lane `length` units long:
/// from the receiving end back, `spacing` apart, never closer than `margin` to either end. The
/// receiving end is the end if `at_end`, else the start.
pub fn slots(length: f32, count: usize, at_end: bool) -> Vec<f32> {
    const MARGIN: f32 = 0.45;
    let spacing = (0.52_f32).min(((length - 2.0 * MARGIN) / MOST_CAPSULES as f32).max(0.18));
    (0..count)
        .map(|k| {
            let back = MARGIN + k as f32 * spacing;
            if at_end { length - back } else { back }
        })
        .filter(|distance| (0.0..=length).contains(distance))
        .collect()
}

/// A tube to draw.
#[derive(Clone, Copy, Debug)]
pub struct Tube<'a> {
    /// From the top of one tank up its riser, along the overhead run and down the other riser:
    /// at least four points, the first two and the last two being the risers.
    pub points: &'a [Point],
    /// What waits unread for the end of the run (the high tank) and for its start, or None where
    /// it was not measured.
    pub toward_end: Option<Queue>,
    pub toward_start: Option<Queue>,
    pub pick: u32,
    pub hot: bool,
}

/// The sideways offset of the two lanes from the middle of the tube.
const LANE: f32 = 0.11;

/// Draws `tube`.
pub fn tube(ink: &mut Ink, tube: &Tube) {
    let points = tube.points;
    if points.len() < 4 {
        return;
    }
    let brass = highlighted(BRASS, tube.hot);
    let pick = tube.pick;
    let last = points.len() - 1;
    let run = &points[1..last];
    for riser in [&points[..2], &points[last - 1..]] {
        ink.pipe(riser, TUBE_R * 0.65, brass, pick);
    }
    ink.pipe(run, TUBE_R * 0.42, tint(GLASS, 0.85), pick);
    let length = path::length(run);
    let bands = (length / 1.6).floor().max(1.0) as usize;
    for k in 0..=bands {
        let at = (length * k as f32 / bands as f32).clamp(0.1, (length - 0.1).max(0.1));
        if let Some((point, direction)) = path::at(run, at) {
            let (before, after) = (
                add(point, scale(direction, -0.07)),
                add(point, scale(direction, 0.07)),
            );
            ink.prism(
                &[before, after],
                &[TUBE_R * 1.25, TUBE_R * 1.25],
                brass,
                pick,
                8,
                true,
            );
        }
    }
    for corner in &run[1..run.len() - 1] {
        ink.knob(*corner, TUBE_R * 1.3, brass, pick);
    }
    for end in [points[0], points[last]] {
        ink.prism(
            &[add(end, [0.0, 0.0, -0.1]), add(end, [0.0, 0.0, 0.12])],
            &[TUBE_R * 1.4, TUBE_R * 1.4],
            brass,
            pick,
            8,
            true,
        );
    }
    for (queue, at_end, side) in [
        (tube.toward_end, true, 1.0),
        (tube.toward_start, false, -1.0),
    ] {
        let Some((drawn, _)) = capsules(queue) else {
            continue;
        };
        for distance in slots(length, drawn, at_end) {
            let Some((point, direction)) = path::at(run, distance) else {
                continue;
            };
            let across = [-direction[1], direction[0], 0.0];
            let centre = add(point, scale(across, LANE * side));
            let (a, b) = (
                add(centre, scale(direction, -0.2)),
                add(centre, scale(direction, 0.2)),
            );
            ink.prism(
                &[a, b],
                &[TUBE_R * 0.62, TUBE_R * 0.62],
                highlighted(CAPSULE, tube.hot),
                pick,
                8,
                true,
            );
            for end in [a, b] {
                ink.knob(end, TUBE_R * 0.62, tint(BRASS, 0.9), pick);
            }
        }
    }
}

/// What the beads of a hose do in one direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Beads {
    /// No rate was measured: no beads, and the rubber is grey.
    Unmeasured,
    /// A rate of 0: no beads, in the colour of rubber that carries something.
    Idle,
    /// Beads that have travelled this far along the hose in this direction, in world units.
    Moving(f32),
}

/// A hose to draw.
#[derive(Clone, Copy, Debug)]
pub struct Hose<'a> {
    /// The path at `HOSE_Z`, rounded, from one tank to the other.
    pub points: &'a [Point],
    /// What flows from the first tank to the second, and back.
    pub forward: Beads,
    pub backward: Beads,
    pub pick: u32,
    pub hot: bool,
}

const BEAD_PERIOD: f32 = 0.56;
const BEAD_ON: f32 = 0.14;

/// Draws `hose`.
pub fn hose(ink: &mut Ink, hose: &Hose) {
    let points = hose.points;
    if points.len() < 2 {
        return;
    }
    let measured = hose.forward != Beads::Unmeasured || hose.backward != Beads::Unmeasured;
    let rubber = if measured { HOSE } else { DULL };
    ink.pipe(points, HOSE_R, highlighted(rubber, hose.hot), hose.pick);
    let toward = ink.toward;
    let lifted = |points: &[Point], up: f32| -> Vec<Point> {
        points
            .iter()
            .map(|&p| add(add(p, scale(toward, HOSE_R * 0.9)), [0.0, 0.0, up]))
            .collect()
    };
    // Faint ribbing, so it reads as a garden hose.
    ink.band(
        &lifted(points, 0.0),
        0.08,
        tint(rubber, 0.62),
        Some(Dash {
            on: 0.07,
            off: 0.2,
            phase: 0.0,
        }),
    );
    if let Beads::Moving(travelled) = hose.forward {
        ink.band(
            &lifted(points, 0.05),
            0.09,
            [214, 252, 190],
            Some(Dash {
                on: BEAD_ON,
                off: BEAD_PERIOD - BEAD_ON,
                phase: -travelled,
            }),
        );
    }
    if let Beads::Moving(travelled) = hose.backward {
        let reversed: Vec<Point> = points.iter().rev().copied().collect();
        ink.band(
            &lifted(&reversed, -0.04),
            0.08,
            [160, 232, 150],
            Some(Dash {
                on: BEAD_ON,
                off: BEAD_PERIOD - BEAD_ON,
                phase: -travelled,
            }),
        );
    }
    for end in [points[0], points[points.len() - 1]] {
        ink.knob(end, HOSE_R * 1.4, highlighted(BRASS, hose.hot), hose.pick);
    }
}

/// The distance a dash pattern of `period` has moved when beads travelling at `speed` world
/// units per second have run for `step` more seconds from `travelled`, kept below the period.
pub fn advanced(travelled: f32, speed: f32, step: f32, period: f32) -> f32 {
    (travelled + speed * step).rem_euclid(period)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(unread: u64, datagram: bool) -> Option<Queue> {
        Some(Queue {
            unread,
            datagram_waiting: datagram,
        })
    }

    #[test]
    fn capsules_count_unread_bytes_per_four_kib_and_datagrams_show_one() {
        assert_eq!(capsules(None), None, "unmeasured is not an empty tube");
        assert_eq!(capsules(queue(0, false)), Some((0, 0)));
        assert_eq!(
            capsules(queue(1, false)),
            Some((1, 1)),
            "a started 4 KiB counts"
        );
        assert_eq!(capsules(queue(4096, false)), Some((1, 1)));
        assert_eq!(capsules(queue(4097, false)), Some((2, 2)));
        assert_eq!(
            capsules(queue(28_672, false)),
            Some((7, 7)),
            "28 KiB is seven"
        );
        // At most twelve are drawn, and the rest are counted for the label.
        assert_eq!(capsules(queue(4096 * 40, false)), Some((12, 40)));
        // A datagram waiting is one capsule, whatever the size of the packet.
        assert_eq!(capsules(queue(0, true)), Some((1, 1)));
        assert_eq!(
            capsules(queue(8192, true)),
            Some((3, 3)),
            "a stream and a datagram"
        );
    }

    #[test]
    fn capsules_wait_at_the_receiving_end_without_leaving_the_tube() {
        let at_end = slots(10.0, 4, true);
        assert_eq!(at_end.len(), 4);
        assert!(at_end[0] > at_end[1] && at_end[0] <= 10.0 - 0.45 + 1e-5);
        let at_start = slots(10.0, 4, false);
        assert!(at_start[0] < at_start[1] && at_start[0] >= 0.45 - 1e-5);
        // A short tube packs them closer rather than past its ends.
        for distance in slots(3.0, MOST_CAPSULES, true) {
            assert!((0.0..=3.0).contains(&distance), "{distance}");
        }
        let spacing = |d: &[f32]| d[0] - d[1];
        assert!(spacing(&slots(3.0, 4, true)) < spacing(&slots(10.0, 4, true)));
    }

    #[test]
    fn beads_advance_by_wall_clock_and_wrap_at_the_period() {
        let one_step = advanced(0.0, 2.0, 1.0, BEAD_PERIOD);
        let ten_steps = (0..10).fold(0.0, |at, _| advanced(at, 2.0, 0.1, BEAD_PERIOD));
        assert!(
            (one_step - ten_steps).abs() < 1e-4,
            "{one_step} against {ten_steps}"
        );
        assert!(advanced(0.5, 3.0, 1.0, BEAD_PERIOD) < BEAD_PERIOD);
        assert_eq!(
            advanced(0.3, 0.0, 1.0, BEAD_PERIOD),
            0.3,
            "stopped water does not move"
        );
    }
}
