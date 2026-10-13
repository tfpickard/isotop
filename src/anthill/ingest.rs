//! Taking in a sample: who stands where, what the plan says, which ants go and how many, which
//! tunnels run and where the entrances face. It happens once for each sample and set of drawn
//! processes, never per frame, and everything it keeps is a function of the sample and of the
//! layout, which never moves for a change of CPU or memory.

use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;

use super::colony::{Colony, Route, Sending};
use super::draw::BLANKET;
use super::flows::{self, Goal, Medium, Plan, Tunnel};
use super::words::Facts;
use super::{
    ARCH, Anthill, BEND, Conduit, DASH_SPEED, Nest, PRESENCE, Record, SALT, nest_radius, paint,
    pile_radius,
};
use crate::model::{Identity, Measured, Process, Snapshot};
use crate::palette::{kind_color, mix};
use crate::render::{Point, Stage};
use crate::simulation::{hash, hash_identity};

/// A route and the units per second (calls, or KiB) of the stream that walks it.
type Routed = (Route, f32);

/// A way out of a nest: the units per second through it, None where that was not measured, and
/// the place it leads to.
type Door = (Option<f32>, [f32; 2]);

impl Anthill {
    /// Takes in the sample `snapshot`, of which `processes` are the drawn ones, at the stage's
    /// time.
    pub(super) fn ingest(&mut self, stage: &Stage, processes: &[&Process], snapshot: &Snapshot) {
        let key = (snapshot.elapsed.to_bits(), processes.len());
        if self.taken == Some(key) {
            return;
        }
        let first = self.taken.is_none();
        let new_sample = self.taken.is_none_or(|(elapsed, _)| elapsed != key.0);
        self.taken = Some(key);
        self.seat(stage, snapshot);
        self.ground();
        let plan = Plan::new(processes, snapshot);
        self.nests = self.nests_of(stage, processes);
        self.seat_blankets(processes, &plan);
        self.keep_piles(stage.time, first, &plan);
        let (routes, unplaced) = self.routes_of(&plan);
        self.callouts = paint::crumb_callouts(routes.iter().map(|(route, _)| route));
        self.send(&routes, new_sample);
        self.note_senders(processes, &plan);
        self.open_entrances(&routes, &plan);
        self.dig(&plan);
        self.facts = Facts {
            unit: Some(plan.unit),
            k: self.scale.k(),
            ants: self.colony.as_ref().map_or(0, |colony| colony.ants.len()),
            refused: self.colony.as_ref().map_or(0, |colony| colony.refused),
            nests: self.nests.len(),
            piles: plan.piles.len(),
            tally: flows::Tally {
                unplaced,
                ..plan.tally
            },
            read_any: plan.tally.measured > 0,
        };
        self.plan = Some(plan);
    }

    /// The nests of the drawn processes, each where the packer put it.
    fn nests_of(&self, stage: &Stage, processes: &[&Process]) -> HashMap<Identity, Nest> {
        processes
            .iter()
            .filter_map(|process| {
                let (at, _) = self.packer.get(process.id)?;
                Some((
                    process.id,
                    Nest {
                        at,
                        radius: nest_radius(stage.eased(process).1 as f32),
                        color: kind_color(process),
                        entrances: Vec::new(),
                    },
                ))
            })
            .collect()
    }

    /// Gives a blanket to each process that holds a terminal and has unattributed I/O with
    /// nowhere else to go. A holder keeps its corner while it lives and holds a terminal; the
    /// others take the free corners, the busiest first.
    fn seat_blankets(&mut self, processes: &[&Process], plan: &Plan) {
        let holders: HashSet<Identity> = processes
            .iter()
            .filter(
                |process| matches!(&process.io_detail, Measured::Known(detail) if detail.ttys > 0),
            )
            .map(|process| process.id)
            .collect();
        for slot in &mut self.slots {
            if slot.is_some_and(|holder| !holders.contains(&holder)) {
                *slot = None;
            }
        }
        for candidate in &plan.candidates {
            if self.slots.contains(&Some(candidate.holder)) {
                continue;
            }
            if let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none()) {
                *slot = Some(candidate.holder);
            }
        }
    }

    /// Records each pile of the plan where its directory put it and as big as its file makes it,
    /// lets one that left the samples start to fade, and forgets the faded.
    fn keep_piles(&mut self, now: f32, first: bool, plan: &Plan) {
        let current: HashSet<u64> = plan.piles.iter().map(|pile| pile.key).collect();
        for (key, record) in &mut self.records {
            if !current.contains(key) {
                record.gone.get_or_insert(now);
            }
        }
        for pile in &plan.piles {
            let at = self.parcels.pile(&self.slab, &pile.path);
            let record = self.records.entry(pile.key).or_insert(Record {
                at,
                radius: 0.0,
                seed: hash::fnv1a_64(&pile.path) ^ pile.key,
                first: if first { f32::NEG_INFINITY } else { now },
                gone: None,
                deleted: pile.deleted,
            });
            record.at = at;
            record.deleted = pile.deleted;
            record.radius = pile_radius(pile.size);
            record.gone = None;
        }
        self.records
            .retain(|_, record| record.gone.is_none_or(|gone| now - gone < PRESENCE));
    }

    /// The route each stream of the plan walks, and the bytes per second of unattributed I/O that
    /// have nowhere to go: those the plan counted, and those of a holder that found no blanket.
    fn routes_of(&self, plan: &Plan) -> (Vec<Routed>, f32) {
        let goal = |goal: Goal| -> Option<([f32; 2], f32)> {
            match goal {
                Goal::File(key) => self
                    .records
                    .get(&key)
                    .map(|record| (record.at, record.radius)),
                Goal::Blanket(holder) => {
                    let slot = self.slots.iter().position(|slot| *slot == Some(holder))?;
                    Some((self.slab.blanket(slot), BLANKET * 0.5))
                }
            }
        };
        let routes = plan
            .streams
            .iter()
            .filter_map(|stream| {
                let nest = self.nests.get(&stream.nest)?;
                let to = goal(stream.goal)?;
                let route = Route::new(
                    stream.nest,
                    stream.goal,
                    stream.way,
                    stream.crumb,
                    (nest.at, nest.radius),
                    to,
                    nest.color,
                );
                Some((route, stream.rate))
            })
            .collect();
        let homeless: f32 = plan
            .candidates
            .iter()
            .filter(|candidate| !self.slots.contains(&Some(candidate.holder)))
            .map(|candidate| candidate.bytes)
            .sum();
        (routes, plan.tally.unplaced + homeless)
    }

    /// Chooses K for the load of `routes` and tells the colony how many ants each sends a second.
    /// K changes only when `new_sample` brings the third sample in a row that agrees.
    fn send(&mut self, routes: &[Routed], new_sample: bool) {
        let load: f32 = routes.iter().map(|(route, rate)| rate * route.trip()).sum();
        let k = if new_sample {
            self.scale.observe(load) as f32
        } else {
            self.scale.k() as f32
        };
        let size = self.slab.size();
        let colony = self.colony.get_or_insert_with(|| Colony::new(size));
        colony.send(routes.iter().map(|&(route, rate)| Sending {
            route,
            per_second: rate / k,
        }));
        colony.retire(&self.nests.keys().copied().collect());
    }

    /// What each nest sends, which the tour ranks by, and how much of its bytes its files explain.
    fn note_senders(&mut self, processes: &[&Process], plan: &Plan) {
        self.busy.clear();
        for stream in &plan.streams {
            *self.busy.entry(stream.nest).or_default() += stream.rate;
        }
        self.explained = processes
            .iter()
            .filter_map(|process| {
                let Measured::Known(detail) = &process.io_detail else {
                    return None;
                };
                Some((process.id, flows::explained(detail)?))
            })
            .collect();
    }

    /// Opens an entrance on each nest for each of up to four distinct destinations, the busiest
    /// first, facing it: its files, its blanket and the nests at the other end of its tunnels.
    fn open_entrances(&mut self, routes: &[Routed], plan: &Plan) {
        let mut doors: HashMap<Identity, Vec<Door>> = HashMap::new();
        for &(route, rate) in routes {
            doors
                .entry(route.owner)
                .or_default()
                .push((Some(rate), route.goal));
        }
        for tunnel in &plan.tunnels {
            for (end, other) in [(tunnel.from, tunnel.to), (tunnel.to, tunnel.from)] {
                if let Some(to) = self.nests.get(&other) {
                    doors.entry(end).or_default().push((tunnel.rate, to.at));
                }
            }
        }
        for (id, mut list) in doors {
            let Some(nest) = self.nests.get_mut(&id) else {
                continue;
            };
            list.sort_by(|a, b| flows::busier_first(a.0, b.0));
            let mut seen: Vec<[f32; 2]> = Vec::new();
            for (_, at) in list {
                if seen.contains(&at) || seen.len() == 4 {
                    continue;
                }
                seen.push(at);
                nest.entrances
                    .push((at[1] - nest.at[1]).atan2(at[0] - nest.at[0]));
            }
        }
    }

    /// Lays out the tunnels of the plan, keeping how far each one's dashes have slid, and how
    /// fast they slide: with the calls per second no file explains, and not at all where that is
    /// not known.
    fn dig(&mut self, plan: &Plan) {
        let slid: HashMap<(Identity, Identity, Medium), f32> = self
            .conduits
            .iter()
            .map(|c| ((c.tunnel.from, c.tunnel.to, c.tunnel.medium), c.travelled))
            .collect();
        self.conduits = plan
            .tunnels
            .iter()
            .filter_map(|tunnel| {
                let (from, to) = (self.nests.get(&tunnel.from)?, self.nests.get(&tunnel.to)?);
                Some(Conduit {
                    tunnel: *tunnel,
                    path: tunnel_path(tunnel, from, to),
                    color: mix(from.color, [236, 240, 244], 0.35),
                    speed: tunnel
                        .rate
                        .map_or(0.0, |rate| DASH_SPEED * (1.0 + rate).log10()),
                    travelled: slid
                        .get(&(tunnel.from, tunnel.to, tunnel.medium))
                        .copied()
                        .unwrap_or_default(),
                })
            })
            .collect();
    }
}

/// The path of a tunnel between two nests: a quadratic Bezier bent to one side by a hashed sign and
/// arched a little over the ground, from the edge of one mound to the edge of the other.
fn tunnel_path(tunnel: &Tunnel, from: &Nest, to: &Nest) -> Vec<Point> {
    let seed = hash_identity(tunnel.from, SALT) ^ hash_identity(tunnel.to, SALT ^ 3);
    let sign = if seed & 1 == 0 { BEND } else { -BEND };
    let (dx, dy) = (to.at[0] - from.at[0], to.at[1] - from.at[1]);
    let control = [
        (from.at[0] + to.at[0]) * 0.5 - dy * sign,
        (from.at[1] + to.at[1]) * 0.5 + dx * sign,
    ];
    let rise = (ARCH * dx.hypot(dy)).clamp(0.8, 5.0);
    (0..=32)
        .map(|k| {
            let t = k as f32 / 32.0;
            let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * (1.0 - t) * t, t * t);
            [
                a * from.at[0] + b * control[0] + c * to.at[0],
                a * from.at[1] + b * control[1] + c * to.at[1],
                0.3 + rise * (PI * t).sin(),
            ]
        })
        .filter(|point| {
            let outside = |nest: &Nest| {
                (point[0] - nest.at[0]).hypot(point[1] - nest.at[1]) >= nest.radius * 0.9
            };
            outside(from) && outside(to)
        })
        .collect()
}
