//! Stable placement shared by the views: members keep their seats and discs keep their places, so
//! a layout only changes where something new arrives or something outgrows its room.

use std::collections::HashMap;

use crate::model::Identity;
use crate::render::vacant;

/// Seat numbers within named groups. A member keeps its seat for life; a freed seat goes to the
/// next newcomer of that group.
#[derive(Default)]
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
