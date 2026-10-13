//! Drawing a frame of the anthill, and telling the app what stands where: the camera's bounds, the
//! labels, the tour's stops and the lines the inspector adds. A frame draws the ground, the
//! tunnels, the piles, the mounds, the ants and the blankets, in that order; the depth buffer, not
//! the order, hides an ant that has gone into a mound.

use std::collections::HashSet;

use super::colony::Route;
use super::draw::{self, Cut, Heap, Lane, Mound};
use super::flows::{Medium, Way, file_key};
use super::slab::Region;
use super::{
    Anthill, FILE, LABELLED_PILES, PRESENCE, Record, SALT, TERMINAL, TOURED, blanket_key,
    nest_radius, words,
};
use crate::io_detail::Flow;
use crate::model::{Identity, Measured, Process};
use crate::palette::mix;
use crate::render::{Links, Stage, Thing, TourTarget, label_name};
use crate::simulation::{hash_identity, sprout};

/// A label for the crumb of a stream: where it stands on its route, and what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Callout {
    pub at: [f32; 2],
    pub text: String,
}

/// The labels of the two crumbs that differ most: the biggest and the smallest of `routes`, each
/// as bytes per call of its direction, standing a little past the middle of its route. One label
/// when they are the same size.
pub fn crumb_callouts<'a>(routes: impl IntoIterator<Item = &'a Route>) -> Vec<Callout> {
    let sized: Vec<(&Route, f32)> = routes
        .into_iter()
        .filter_map(|route| Some((route, route.crumb?)))
        .collect();
    let by_size = |a: &&(&Route, f32), b: &&(&Route, f32)| a.1.total_cmp(&b.1);
    let ends = [sized.iter().max_by(by_size), sized.iter().min_by(by_size)];
    let mut callouts: Vec<Callout> = Vec::new();
    for &(route, bytes) in ends.into_iter().flatten() {
        let word = match route.way {
            Way::Read => "read",
            Way::Write => "write",
            Way::Either => "call",
        };
        let at = route.at(0.6);
        let callout = Callout {
            at,
            text: format!("{}/{word}", words::bytes(bytes)),
        };
        if !callouts.iter().any(|other| other.text == callout.text) {
            callouts.push(callout);
        }
    }
    callouts
}

impl Anthill {
    /// Records the frame, `alpha` of the way from the colony's last step to its next, and fills
    /// the stage's outputs.
    pub(super) fn paint(&mut self, stage: &mut Stage, processes: &[&Process], alpha: f32) {
        if let (Some(ground), Some(colony)) = (&mut self.terrain, &self.colony)
            && std::mem::take(&mut self.stale)
        {
            ground.paint(&colony.pheromone);
        }
        if let Some(ground) = &self.terrain {
            ground.draw(stage.frame, stage.camera);
        }
        let sprite = self.slab.size() / 64.0;
        self.paint_tunnels(stage, sprite);
        self.paint_piles(stage);
        self.paint_mounds(stage, processes);
        if let Some(colony) = &self.colony {
            draw::ants(stage.frame, stage.camera, &colony.ants, alpha, sprite);
        }
        self.paint_blankets(stage);
        self.report(stage, processes);
    }

    /// The tunnels the `c` key shows: dashes that slide with the calls no file explains, a dotted
    /// still line where that is not known, two lanes one way each for sockets and one lane for a
    /// pipe.
    fn paint_tunnels(&self, stage: &mut Stage, unit: f32) {
        for conduit in &self.conduits {
            let tunnel = conduit.tunnel;
            let shown = match stage.links {
                Links::All => true,
                Links::Off => false,
                Links::Focused => {
                    stage.highlight.contains(&tunnel.from) || stage.highlight.contains(&tunnel.to)
                }
            };
            if !shown {
                continue;
            }
            let known = tunnel.rate.is_some();
            let (on, off) = match tunnel.medium {
                Medium::Pipe => (1.2, 0.8),
                Medium::Unix | Medium::Socket => (0.8, 0.6),
                Medium::Loopback => (0.45, 0.55),
            };
            let (cut, color) = if known {
                (
                    Cut {
                        on: on * unit,
                        off: off * unit,
                        width: 0.13 * unit,
                    },
                    conduit.color,
                )
            } else {
                (
                    Cut {
                        on: 0.25 * unit,
                        off: 0.6 * unit,
                        width: 0.08 * unit,
                    },
                    mix(conduit.color, [60, 48, 38], 0.55),
                )
            };
            let lane = Lane {
                offset: 0.0,
                cut,
                travelled: conduit.travelled * unit.max(1.0),
                forward: true,
                color,
            };
            if tunnel.one_way {
                draw::lane(stage.frame, stage.camera, &conduit.path, lane);
            } else {
                let side = 0.2 * unit;
                for (offset, forward) in [(side, true), (-side, false)] {
                    let lane = Lane {
                        offset,
                        forward,
                        ..lane
                    };
                    draw::lane(stage.frame, stage.camera, &conduit.path, lane);
                }
            }
        }
    }

    /// The piles, each one a thing the view can be asked about; one that has faded is let go.
    fn paint_piles(&mut self, stage: &mut Stage) {
        let time = stage.time;
        self.records
            .retain(|_, record| record.gone.is_none_or(|gone| time - gone < PRESENCE));
        let mut piles: Vec<(&u64, &Record)> = self.records.iter().collect();
        piles.sort_by_key(|(key, _)| **key);
        for (&key, record) in piles {
            let appear = ((time - record.first) / PRESENCE).clamp(0.0, 1.0);
            let leave = record
                .gone
                .map_or(1.0, |gone| (1.0 - (time - gone) / PRESENCE).clamp(0.0, 1.0));
            let thing = Thing { kind: FILE, key };
            let pick = stage.frame.thing_pick(thing);
            draw::pile(
                stage.frame,
                stage.camera,
                &Heap {
                    at: record.at,
                    radius: record.radius,
                    pick,
                    seed: record.seed,
                    presence: appear.min(leave),
                    highlight: stage.thing == Some(thing),
                    deleted: record.deleted,
                },
            );
        }
    }

    /// A mound for each drawn process, which picks the process by its index, and its place.
    fn paint_mounds(&self, stage: &mut Stage, processes: &[&Process]) {
        for (index, process) in processes.iter().enumerate() {
            let Some(nest) = self.nests.get(&process.id) else {
                continue;
            };
            let radius = nest_radius(process.memory as f32) * sprout(stage.growth(process.id));
            draw::mound(
                stage.frame,
                stage.camera,
                &Mound {
                    at: nest.at,
                    radius,
                    pick: index as u32,
                    seed: hash_identity(process.id, SALT),
                    highlight: stage.highlight.contains(&process.id),
                    awake: matches!(process.io_detail, Measured::Known(_)),
                    glow: nest.color,
                    entrances: &nest.entrances,
                },
            );
            stage
                .positions
                .insert(process.id, [nest.at[0], nest.at[1], 0.0]);
        }
    }

    /// The blankets, each one a thing the view can be asked about.
    fn paint_blankets(&self, stage: &mut Stage) {
        for (slot, holder) in self.slots.iter().enumerate() {
            let Some(holder) = *holder else { continue };
            let thing = Thing {
                kind: TERMINAL,
                key: blanket_key(holder),
            };
            let pick = stage.frame.thing_pick(thing);
            draw::blanket(
                stage.frame,
                stage.camera,
                self.slab.blanket(slot),
                pick,
                stage.thing == Some(thing),
            );
        }
    }

    /// The camera's bounds, the labels, the tour's stops and the lines the inspector adds.
    fn report(&mut self, stage: &mut Stage, processes: &[&Process]) {
        stage.bounds.extend(
            self.slab
                .corners()
                .iter()
                .map(|corner| [corner[0], corner[1], 0.0]),
        );
        self.label(stage);
        // The tour visits the nests that send the most.
        let mut busiest: Vec<(f32, Identity)> = self
            .busy
            .iter()
            .filter(|(id, _)| self.nests.contains_key(id))
            .map(|(&id, &rate)| (rate, id))
            .collect();
        busiest.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (rate, id) in busiest.into_iter().take(TOURED) {
            let at = self.nests[&id].at;
            stage.tour.push(TourTarget {
                id,
                position: [at[0], at[1], 0.0],
                weight: rate,
            });
        }
        for &id in stage.highlight {
            if let Some(process) = processes.iter().find(|process| process.id == id) {
                self.inspect(stage, process);
            }
        }
        if let Some(colony) = &self.colony {
            self.facts.ants = colony.ants.len();
            self.facts.refused = colony.refused;
        }
    }

    /// The labels: the regions' names, then the piles with the most traffic, then the two crumbs
    /// that differ most.
    fn label(&self, stage: &mut Stage) {
        for region in Region::ALL {
            let at = self.slab.rect(region).center();
            stage
                .places
                .push(([at[0], at[1], 0.0], region.name().to_owned()));
        }
        for pile in self
            .plan
            .iter()
            .flat_map(|plan| &plan.piles)
            .filter(|pile| pile.traffic() > 0.0)
            .take(LABELLED_PILES)
        {
            if let Some(record) = self.records.get(&pile.key) {
                stage.places.push((
                    [record.at[0], record.at[1] + record.radius + 0.6, 0.0],
                    label_name(words::file_name(&pile.path)).into_owned(),
                ));
            }
        }
        for callout in &self.callouts {
            stage
                .places
                .push(([callout.at[0], callout.at[1], 0.0], callout.text.clone()));
        }
    }

    /// What the inspector adds for `process`: its calls and files, and its busiest file for the
    /// tour's callout.
    fn inspect(&self, stage: &mut Stage, process: &Process) {
        let Measured::Known(detail) = &process.io_detail else {
            return;
        };
        let kept: HashSet<u64> = self
            .plan
            .iter()
            .flat_map(|plan| plan.piles.iter().map(|pile| pile.key))
            .collect();
        let piles = detail
            .files
            .iter()
            .filter(|file| kept.contains(&file_key(file.device, file.inode)))
            .count();
        let explained = self.explained.get(&process.id).copied();
        for line in words::nest_lines(process, piles, detail.files.len(), explained) {
            stage.popup.push((process.id, line));
        }
        let loudest = detail
            .files
            .iter()
            .filter_map(|file| match file.flow {
                Flow::Measured {
                    read,
                    written,
                    either,
                } => Some((read + written + either, file)),
                _ => None,
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((rate, file)) = loudest.filter(|(rate, _)| *rate > 0.0) {
            stage.callout.push((
                process.id,
                format!("busiest file {} at {}", file.path, words::per_second(rate)),
            ));
        }
    }
}
