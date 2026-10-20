//! Stable placement shared by the views: members keep their seats and discs keep their places, so
//! a layout only changes where something new arrives or something outgrows its room. The
//! golden-angle spiral every placement builds on lives here too.

use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::model::Identity;
use crate::simulation::hash;

/// The golden angle in radians, 2 pi (1 - 1/phi): consecutive spiral slots never line up.
pub(crate) const GOLDEN_ANGLE: f32 = 2.399_963;

/// Where slot `slot` sits on a sunflower spiral, as (distance, angle) from its centre: out at
/// `spacing * sqrt(slot + offset)` and round by `slot` golden angles from `phase`. The offset
/// keeps slot 0 off the centre when something else sits there.
pub(crate) fn spiral(spacing: f32, offset: f32, slot: f32, phase: f32) -> (f32, f32) {
    (
        spacing * (slot + offset).sqrt(),
        slot * GOLDEN_ANGLE + phase,
    )
}

/// The first point on a golden-angle spiral out from the origin where a disc of `radius` clears
/// every placed (centre, radius) disc.
pub(crate) fn vacant(placed: &[([f32; 2], f32)], radius: f32) -> [f32; 2] {
    let step = (radius * 0.4).max(1.0);
    (0_u32..)
        .map(|k| {
            let (distance, angle) = spiral(step, 0.0, k as f32, 0.0);
            [distance * angle.cos(), distance * angle.sin()]
        })
        .find(|c| {
            placed
                .iter()
                .all(|(o, r)| (c[0] - o[0]).hypot(c[1] - o[1]) >= radius + r)
        })
        .expect("the spiral eventually clears every placed disc")
}

/// A stable angle per name, so clusters don't all share one spiral orientation.
pub(crate) fn spin(name: &str) -> f32 {
    let hash = hash::fnv1a_32(name);
    (hash >> 8) as f32 / 16_777_216.0 * TAU
}

/// Seat numbers within named groups. A member keeps its seat for life; a freed seat goes to the
/// next newcomer of that group.
#[derive(Clone, Default)]
pub struct Seats {
    taken: HashMap<String, HashMap<usize, Identity>>,
    seats: HashMap<Identity, (String, usize)>,
}

impl Seats {
    /// Seats every member in its group, releasing the seats of members that left or moved.
    pub fn assign<'a>(&mut self, members: impl IntoIterator<Item = (Identity, &'a str)>) {
        let members: HashMap<Identity, &str> = members.into_iter().collect();
        self.seats.retain(|id, (group, seat)| {
            let stays = members.get(id).is_some_and(|g| g == group);
            if !stays && let Some(taken) = self.taken.get_mut(group) {
                taken.remove(seat);
            }
            stays
        });
        self.taken.retain(|_, taken| !taken.is_empty());
        let mut newcomers: Vec<(&Identity, &&str)> = members
            .iter()
            .filter(|(id, _)| !self.seats.contains_key(id))
            .collect();
        newcomers.sort();
        for (&id, &group) in newcomers {
            let taken = self.taken.entry(group.to_owned()).or_default();
            let seat = (0..=taken.len())
                .find(|seat| !taken.contains_key(seat))
                .expect("one of len + 1 seats is free");
            taken.insert(seat, id);
            self.seats.insert(id, (group.to_owned(), seat));
        }
    }

    pub fn seat(&self, id: Identity) -> Option<usize> {
        self.seats.get(&id).map(|&(_, seat)| seat)
    }

    /// One past the highest seat in use in a group.
    pub fn span(&self, group: &str) -> usize {
        self.taken
            .get(group)
            .and_then(|taken| taken.keys().max())
            .map_or(0, |&seat| seat + 1)
    }
}

/// Named discs packed without overlap around the origin. A disc keeps its centre until it needs
/// more room than it reserved; then it is placed again with headroom.
#[derive(Default)]
pub struct Discs {
    placed: HashMap<String, ([f32; 2], f32)>,
}

impl Discs {
    /// Places discs of the given radii and returns each centre.
    pub fn arrange(&mut self, needs: &[(String, f32)]) -> HashMap<String, [f32; 2]> {
        let wanted: HashMap<&str, f32> = needs.iter().map(|(n, r)| (n.as_str(), *r)).collect();
        self.placed
            .retain(|name, (_, reserved)| wanted.get(name.as_str()).is_some_and(|r| r <= reserved));
        let mut pending: Vec<(&str, f32)> = wanted
            .iter()
            .filter(|(name, _)| !self.placed.contains_key(**name))
            .map(|(&name, &radius)| (name, radius))
            .collect();
        pending.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        for (name, radius) in pending {
            let reserved = radius * 1.15;
            let others: Vec<([f32; 2], f32)> = self.placed.values().copied().collect();
            self.placed
                .insert(name.to_owned(), (vacant(&others, reserved), reserved));
        }
        self.placed
            .iter()
            .map(|(name, &(center, _))| (name.clone(), center))
            .collect()
    }

    pub fn reserved(&self, name: &str) -> f32 {
        self.placed.get(name).map_or(0.0, |&(_, reserved)| reserved)
    }

    /// The radius around the origin that holds every disc.
    pub fn reach(&self) -> f32 {
        self.placed
            .values()
            .map(|(c, r)| c[0].hypot(c[1]) + r)
            .fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_spiral_places_the_same_points_as_before() {
        // (slot, phase, angle bits, distance bits at spacing 1.6 and offset 0.5, distance bits at
        // spacing 0.9 and offset 1.0), captured from the hand-written spirals this replaced.
        for (slot, phase, angle, wide, tight) in [
            (0.0, 0.0, 0, 1066455235, 1063675494),
            (0.0, 1.234, 1067316150, 1066455235, 1063675494),
            (1.0, 0.0, 1075419390, 1073402856, 1067641562),
            (1.0, 1.234, 1080595161, 1073402856, 1067641562),
            (2.0, 0.0, 1083807998, 1075964059, 1070041153),
            (2.0, 1.234, 1086395884, 1075964059, 1070041153),
            (7.0, 0.0, 1099326942, 1082931084, 1076030170),
            (7.0, 1.234, 1099973913, 1082931084, 1076030170),
            (40.0, 0.0, 1119878974, 1092807387, 1085827316),
            (40.0, 1.234, 1120040717, 1092807387, 1085827316),
        ] {
            let (distance, got) = spiral(1.6, 0.5, slot, phase);
            assert_eq!(got.to_bits(), angle, "angle at slot {slot} phase {phase}");
            assert_eq!(distance.to_bits(), wide, "distance at slot {slot}");
            assert_eq!(spiral(0.9, 1.0, slot, phase).0.to_bits(), tight);
        }
        let discs = [([0.0, 0.0], 5.0), ([9.0, 1.0], 4.0), ([-3.0, 8.0], 2.5)];
        for (placed, radius, x, y) in [
            (&discs[..0], 3.0, 0, 0),
            (&discs[..1], 3.0, 1078084000, 1089377526),
            (&discs[..], 6.5, 3236849091, 3238975150),
            (
                &[([0.0, 0.0], 12.0), ([20.0, 5.0], 7.0)][..],
                0.5,
                1095015110,
                3223119600,
            ),
        ] {
            let found = vacant(placed, radius);
            assert_eq!(
                (found[0].to_bits(), found[1].to_bits()),
                (x, y),
                "vacant({radius})"
            );
        }
    }

    #[test]
    fn spin_returns_the_angles_it_returned_before_hashing_moved() {
        for (name, bits) in [
            ("", 1_078_644_461),
            ("a", 1_085_479_852),
            ("dbus.service", 1_084_250_095),
            ("System /system.slice Ünï", 1_085_165_909),
        ] {
            assert_eq!(spin(name).to_bits(), bits, "spin({name:?})");
        }
    }

    fn id(pid: u32) -> Identity {
        Identity {
            pid,
            start: pid as u64,
        }
    }

    #[test]
    fn seats_survive_departures_and_discs_survive_small_growth() {
        let mut seats = Seats::default();
        seats.assign([(id(1), "a"), (id(2), "a"), (id(3), "a"), (id(4), "b")]);
        assert_eq!((seats.seat(id(3)), seats.span("a")), (Some(2), 3));
        seats.assign([(id(1), "a"), (id(3), "a"), (id(4), "b"), (id(5), "a")]);
        assert_eq!(seats.seat(id(3)), Some(2));
        assert_eq!(seats.seat(id(5)), Some(1), "takes the freed seat");
        let mut discs = Discs::default();
        let first = discs.arrange(&[("x".into(), 5.0), ("y".into(), 3.0)]);
        let (x, y) = (first["x"], first["y"]);
        assert!((x[0] - y[0]).hypot(x[1] - y[1]) >= 5.0 + 3.0);
        let second = discs.arrange(&[("x".into(), 5.5), ("y".into(), 3.0)]);
        assert_eq!(second["x"], x, "growth within the headroom keeps the place");
        assert!(discs.reach() >= 5.0);
    }
}
