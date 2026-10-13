//! One frame of the plumbing view: the room, the runs between the tanks, the tanks and their
//! gauges, and everything the app is told about them (positions, titles, labels, notes, the
//! tour). It reads the view's state and the tanks `stand` chose, and writes the display list.

use std::collections::BTreeMap;

use super::card::{Subject, flow_lines};
use super::draw::pipes::{self, SPRAY_SECONDS, Water};
use super::draw::tank::{self, Reading};
use super::draw::tubes::{self, Beads, capsules};
use super::draw::{HOSE_Z, Ink, PIPE_Z, TUBE_Z, scenery};
use super::floor::CELL;
use super::flow::{Outflow, gauge_fill, water_speed};
use super::room::{HoseRun, TubeRun};
use super::route::{Footprint, Hull, Key, elbow};
use super::{Plumbing, Tank, bay_title, path, thing_key};
use crate::model::{Hose, Identity, Process, bytes};
use crate::platform::Wait;
use crate::render::{Point, Stage, Thing, Title, TourTarget, label_name};
use crate::simulation::Rng;

/// The radius runs go round a tank by, whatever its memory: the widest a tank is, so no run
/// moves when memory changes.
const ROUTE_RADIUS: f32 = 0.9;
/// How far from the middle of a tank's cap a tube's riser stands when the tank carries several,
/// the same for every tank, so no tube moves when memory changes.
const RISER: f32 = 0.2;
/// Seeds the droplets of a burst, apart from every other use of an identity.
const SPRAY_SALT: u64 = 0x0b0a_71e5_9100;
/// Tags the things of the room, so a pipe, a tube, a hose and a manifold of one pair of
/// processes are four keys.
const PIPE: u64 = 0x11;
const TUBE: u64 = 0x12;
const HOSE: u64 = 0x13;
const MANIFOLD: u64 = 0x14;
const SEVERED: u64 = 0x15;

/// Where the manifold of `tank` stands: a lattice axis from the tank, the one that lies most
/// toward `targets` of those that leave room for it, a little past the tank's wall.
fn manifold_site(tank: &Tank, targets: &[[f32; 2]], tanks: &[Tank]) -> [f32; 2] {
    let mean = if targets.is_empty() {
        [tank.at[0] + 1.0, tank.at[1]]
    } else {
        let sum = targets
            .iter()
            .fold([0.0, 0.0], |sum, p| [sum[0] + p[0], sum[1] + p[1]]);
        [sum[0] / targets.len() as f32, sum[1] / targets.len() as f32]
    };
    let (dx, dy) = (mean[0] - tank.at[0], mean[1] - tank.at[1]);
    let mut sides: [(f32, f32); 4] = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
    sides.sort_by(|a, b| (b.0 * dx + b.1 * dy).total_cmp(&(a.0 * dx + a.1 * dy)));
    let reach = ROUTE_RADIUS + 0.95;
    let site = |side: (f32, f32)| [tank.at[0] + side.0 * reach, tank.at[1] + side.1 * reach];
    sides
        .iter()
        .map(|&side| site(side))
        .find(|candidate| {
            tanks.iter().all(|other| {
                other.id == tank.id
                    || (candidate[0] - other.at[0]).hypot(candidate[1] - other.at[1])
                        >= ROUTE_RADIUS + 0.75
            })
        })
        .unwrap_or_else(|| site(sides[0]))
}

fn lifted(points: &[[f32; 2]], z: f32) -> Vec<Point> {
    path::lifted(points, z)
}

/// The words of one flow for a label: `1.4 MB/s`.
fn per_second(rate: f32) -> String {
    format!("{}/s", bytes(rate.max(0.0) as u64))
}

/// What the stages of one frame share: the stage's figures read once, the tanks by identity and
/// where the manifolds stand.
struct Pass<'a> {
    time: f32,
    step: f32,
    /// The processes whose links stand out.
    hot: Vec<Identity>,
    selected: Option<Thing>,
    by_id: BTreeMap<Identity, &'a Tank>,
    /// Where the manifold of each writer whose flow splits stands.
    sites: BTreeMap<Identity, [f32; 2]>,
}

impl Pass<'_> {
    fn touched(&self, id: Identity) -> bool {
        self.hot.contains(&id)
    }
}

/// The words over a point of the room: a world point and the text.
type Label = (Point, String);

/// The hulls the router goes round: each tank, and each manifold.
fn hulls_of(tanks: &[Tank], sites: &BTreeMap<Identity, [f32; 2]>) -> BTreeMap<Hull, Footprint> {
    let mut hulls: BTreeMap<Hull, Footprint> = tanks
        .iter()
        .map(|tank| {
            (
                Hull {
                    owner: tank.id,
                    kind: 0,
                },
                Footprint {
                    center: tank.at,
                    radius: ROUTE_RADIUS,
                    wall: tank.radius,
                },
            )
        })
        .collect();
    for (&owner, &center) in sites {
        hulls.insert(
            Hull { owner, kind: 1 },
            Footprint {
                center,
                radius: 0.45,
                wall: 0.45,
            },
        );
    }
    hulls
}

impl Plumbing {
    /// Draws the frame and tells `stage` about it.
    pub(super) fn paint(&mut self, stage: &mut Stage, processes: &[&Process], tanks: &[Tank]) {
        self.drawn = super::Drawn {
            tanks: tanks.len(),
            ..super::Drawn::default()
        };
        let by_id: BTreeMap<Identity, &Tank> = tanks.iter().map(|tank| (tank.id, tank)).collect();
        let sites = self.manifold_sites(&by_id, tanks);
        let hulls = hulls_of(tanks, &sites);
        self.router.begin_frame();
        self.router.update(&hulls);
        let pass = Pass {
            time: stage.time,
            step: stage.dt,
            hot: stage.highlight.to_vec(),
            selected: stage.thing,
            by_id,
            sites,
        };

        let camera = stage.camera;
        let mut ink = Ink::new(&mut *stage.frame, camera);
        if let Some((low, high)) = self.floor.room() {
            scenery::room(&mut ink, low, high);
        }
        for bay in self.floor.bays() {
            scenery::bay(&mut ink, bay);
        }
        let mut labels: Vec<Label> = Vec::new();
        self.paint_pipes(&mut ink, &pass, &hulls, &mut labels);
        self.paint_severed(&mut ink, &pass, &mut labels);
        self.paint_tubes(&mut ink, &pass, &mut labels);
        self.paint_hoses(&mut ink, &pass, &mut labels);
        labels.extend(self.paint_manifolds(&mut ink, &pass));
        self.paint_stubs(&mut ink, &pass);
        let named = self.paint_tanks(&mut ink, &pass, tanks);
        ink.finish();
        self.announce(stage, processes, tanks, labels, named);
    }

    /// Where the manifolds stand: one for each writer whose flow splits among outlets that cannot
    /// be told apart, which with the tanks is what the runs go round.
    fn manifold_sites(
        &self,
        by_id: &BTreeMap<Identity, &Tank>,
        tanks: &[Tank],
    ) -> BTreeMap<Identity, [f32; 2]> {
        let mut sites: BTreeMap<Identity, [f32; 2]> = BTreeMap::new();
        for (&id, outflow) in &self.room.outflows {
            let (Some(tank), Outflow::Measured { outlets, .. }) = (by_id.get(&id), outflow) else {
                continue;
            };
            if !outlets.splits() {
                continue;
            }
            let targets: Vec<[f32; 2]> = self
                .room
                .pipes
                .iter()
                .filter(|run| run.writer == id)
                .filter_map(|run| by_id.get(&run.reader).map(|reader| reader.at))
                .collect();
            sites.insert(id, manifold_site(tank, &targets, tanks));
        }
        sites
    }

    /// The runs of copper between the tanks, with the water in them.
    fn paint_pipes(
        &mut self,
        ink: &mut Ink<'_>,
        pass: &Pass<'_>,
        hulls: &BTreeMap<Hull, Footprint>,
        labels: &mut Vec<Label>,
    ) {
        let runs = self.room.pipes.clone();
        for run in &runs {
            let (Some(writer), Some(reader)) =
                (pass.by_id.get(&run.writer), pass.by_id.get(&run.reader))
            else {
                continue;
            };
            let from = Hull {
                owner: run.writer,
                kind: u8::from(pass.sites.contains_key(&run.writer)),
            };
            let key = Key {
                from,
                to: Hull {
                    owner: run.reader,
                    kind: 0,
                },
                channel: 0,
            };
            let start = hulls.get(&from).map_or(writer.at, |hull| hull.center);
            let ground = self
                .router
                .polyline(key)
                .unwrap_or_else(|| elbow(start, reader.at));
            let points = lifted(&ground, PIPE_Z);
            let writer_outflow = self.room.outflows.get(&run.writer).copied();
            let reader_outflow = self.room.outflows.get(&run.reader).copied();
            let (writer_wait, reader_wait) = (
                self.room.waits.get(&run.writer).copied(),
                self.room.waits.get(&run.reader).copied(),
            );
            let speed = |outflow: Option<Outflow>| outflow.and_then(Outflow::speed);
            let forward = speed(writer_outflow);
            let backward = if run.two_way {
                speed(reader_outflow)
            } else {
                None
            };
            let units = |cells: f32| cells * CELL;
            // Water that was not measured does not move, and nothing is added to its phase.
            let (ahead, behind) = self.phases.entry((run.writer, run.reader)).or_default();
            for (travelled, speed) in [(&mut *ahead, forward), (&mut *behind, backward)] {
                if let Some(speed) = speed {
                    *travelled =
                        tubes::advanced(*travelled, units(speed), pass.step, pipes::DASH_PERIOD);
                }
            }
            let (ahead, behind) = (*ahead, *behind);
            let water = if writer_wait == Some(Wait::PipeWrite) {
                self.drawn.blocked += 1;
                Water::Packed
            } else if reader_wait == Some(Wait::PipeRead) {
                self.drawn.dry += 1;
                Water::Dry
            } else {
                match forward {
                    None => Water::Unmeasured,
                    Some(speed) if speed <= 0.0 => Water::Calm,
                    Some(_) => Water::Flowing {
                        forward: ahead,
                        backward: (run.two_way && backward.is_some_and(|speed| speed > 0.0))
                            .then_some(behind),
                    },
                }
            };
            let thing = Thing {
                kind: "pipe",
                key: thing_key(run.writer, run.reader, PIPE),
            };
            let pick = ink.frame.thing_pick(thing);
            let lit = pass.touched(run.writer)
                || pass.touched(run.reader)
                || pass.selected == Some(thing);
            pipes::pipe(
                ink,
                &pipes::Run {
                    points: &points,
                    water,
                    pick,
                    hot: lit,
                    time: pass.time,
                },
            );
            let middle = path::at(&points, path::length(&points) * 0.5).map_or(points[0], |m| m.0);
            self.drawn.pipes += 1;
            self.drawn.things.insert(
                thing.key,
                Subject::Pipe {
                    writer: run.writer,
                    reader: run.reader,
                    pipes: run.pipes.len(),
                    fifos: run.fifos(),
                    two_way: run.two_way,
                    buffered: run.buffered,
                    outflow: writer_outflow
                        .unwrap_or(Outflow::Unmeasured(super::flow::Why::NotRead)),
                    writer_wait,
                    reader_wait,
                    at: middle,
                },
            );
            match water {
                Water::Packed => labels.push((
                    [middle[0], middle[1], middle[2] + 0.9],
                    "writer blocked: pipe full".to_owned(),
                )),
                Water::Dry => labels.push((
                    [middle[0], middle[1], middle[2] + 0.9],
                    "reader waiting: pipe empty".to_owned(),
                )),
                _ => {}
            }
        }
    }

    /// The pipes nobody reads: capped, or spraying where their reader was seen to go.
    fn paint_severed(&mut self, ink: &mut Ink<'_>, pass: &Pass<'_>, labels: &mut Vec<Label>) {
        let severed = self.room.severed.clone();
        let mut per_writer: BTreeMap<Identity, usize> = BTreeMap::new();
        for &(writer_id, id) in &severed {
            let Some(writer) = pass.by_id.get(&writer_id) else {
                continue;
            };
            let nth = per_writer.entry(writer_id).or_insert(0);
            let side = (*nth + 3) % 4;
            *nth += 1;
            let known = self.breaks.get(&(writer_id, id)).copied();
            let from = Hull {
                owner: writer_id,
                kind: u8::from(pass.sites.contains_key(&writer_id)),
            };
            let routed = known.and_then(|brk| {
                let to = Hull {
                    owner: brk.reader,
                    kind: 0,
                };
                self.router.polyline(Key {
                    from,
                    to,
                    channel: 0,
                })
            });
            let points = match routed {
                Some(ground) => {
                    let full = lifted(&ground, PIPE_Z);
                    path::between(&full, 0.0, path::length(&full) * 0.45)
                }
                None => {
                    let (dx, dy) = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)][side];
                    let wall = [
                        writer.at[0] + dx * writer.radius * 0.9,
                        writer.at[1] + dy * writer.radius * 0.9,
                        PIPE_Z,
                    ];
                    vec![wall, [wall[0] + dx * 1.3, wall[1] + dy * 1.3, PIPE_Z]]
                }
            };
            if points.len() < 2 {
                continue;
            }
            let thing = Thing {
                kind: "pipe",
                key: thing_key(
                    writer_id,
                    Identity { pid: 0, start: 0 },
                    SEVERED ^ pipe_number(id),
                ),
            };
            let pick = ink.frame.thing_pick(thing);
            let age = known.map(|brk| pass.time - brk.since);
            let spraying = age.is_some_and(|age| (0.0..SPRAY_SECONDS).contains(&age));
            pipes::pipe(
                ink,
                &pipes::Run {
                    points: &points,
                    water: Water::Dry,
                    pick,
                    hot: pass.touched(writer_id) || pass.selected == Some(thing),
                    time: pass.time,
                },
            );
            let end = points[points.len() - 1];
            let before = points[points.len() - 2];
            let direction = path::at(&[before, end], 0.0).map_or([1.0, 0.0, 0.0], |d| d.1);
            if let (Some(age), true) = (age, known.is_some()) {
                if spraying {
                    self.drawn.bursts += 1;
                }
                pipes::burst(
                    ink,
                    end,
                    direction,
                    age,
                    Rng::for_identity(writer_id, SPRAY_SALT ^ pipe_number(id)),
                    pick,
                );
                labels.insert(
                    0,
                    (
                        [end[0], end[1], end[2] + 1.5],
                        "reader gone: EPIPE next".to_owned(),
                    ),
                );
                labels.insert(
                    1,
                    (
                        [end[0], end[1], end[2] + 1.0],
                        format!("{} still writing", label_name(&writer.name)),
                    ),
                );
            } else {
                pipes::cap(ink, end, pick);
            }
            self.drawn.things.insert(
                thing.key,
                Subject::Severed {
                    writer: writer_id,
                    reader: known.map(|brk| brk.reader),
                    age: age.filter(|_| spraying),
                    at: end,
                },
            );
        }
    }

    /// The Unix sockets, as brass tubes overhead with their capsules.
    fn paint_tubes(&mut self, ink: &mut Ink<'_>, pass: &Pass<'_>, labels: &mut Vec<Label>) {
        let tubes_here = self.room.tubes.clone();
        let mut risers: BTreeMap<Identity, Vec<Identity>> = BTreeMap::new();
        for tube in &tubes_here {
            risers.entry(tube.low).or_default().push(tube.high);
            risers.entry(tube.high).or_default().push(tube.low);
        }
        for tube in &tubes_here {
            let (Some(low), Some(high)) = (pass.by_id.get(&tube.low), pass.by_id.get(&tube.high))
            else {
                continue;
            };
            let along = tube_path(low, high, &risers, tube);
            let thing = Thing {
                kind: "unix socket",
                key: thing_key(tube.low, tube.high, TUBE),
            };
            let pick = ink.frame.thing_pick(thing);
            tubes::tube(
                ink,
                &tubes::Tube {
                    points: &along,
                    toward_end: tube.toward_high,
                    toward_start: tube.toward_low,
                    pick,
                    hot: pass.touched(tube.low)
                        || pass.touched(tube.high)
                        || pass.selected == Some(thing),
                },
            );
            let run = &along[1..along.len() - 1];
            let middle = path::at(run, path::length(run) * 0.5).map_or(along[1], |m| m.0);
            let waiting = [tube.toward_high, tube.toward_low]
                .into_iter()
                .map(|lane| capsules(lane).map_or(0, |(_, total)| total))
                .sum::<usize>();
            let measured = tube.toward_high.is_some() || tube.toward_low.is_some();
            let text = if !measured {
                "unix socket: queue not measured".to_owned()
            } else if waiting == 0 {
                "unix socket: nothing waits".to_owned()
            } else {
                let unread: u64 = [tube.toward_high, tube.toward_low]
                    .into_iter()
                    .flatten()
                    .map(|queue| queue.unread)
                    .sum();
                if unread == 0 {
                    "unix socket: a datagram waits".to_owned()
                } else {
                    format!(
                        "unix socket: {waiting} capsule{} = {} unread",
                        if waiting == 1 { "" } else { "s" },
                        bytes(unread)
                    )
                }
            };
            labels.push(([middle[0], middle[1], TUBE_Z + 0.7], text));
            self.drawn.tubes += 1;
            self.drawn.things.insert(
                thing.key,
                Subject::Tube {
                    low: tube.low,
                    high: tube.high,
                    toward_high: tube.toward_high,
                    toward_low: tube.toward_low,
                    at: [middle[0], middle[1], TUBE_Z],
                },
            );
        }
    }

    /// The loopback TCP connections, as hoses on the floor.
    fn paint_hoses(&mut self, ink: &mut Ink<'_>, pass: &Pass<'_>, labels: &mut Vec<Label>) {
        let hoses_here = self.room.hoses.clone();
        for hose in &hoses_here {
            let (Some(low), Some(high)) = (pass.by_id.get(&hose.low), pass.by_id.get(&hose.high))
            else {
                continue;
            };
            let key = Key {
                from: Hull {
                    owner: hose.low,
                    kind: 0,
                },
                to: Hull {
                    owner: hose.high,
                    kind: 0,
                },
                channel: 1,
            };
            let ground = self
                .router
                .polyline(key)
                .unwrap_or_else(|| elbow(low.at, high.at));
            let along = path::rounded(&lifted(&ground, HOSE_Z), 0.6, 4);
            let (forward, backward) = self.run_beads(hose, pass.step);
            let thing = Thing {
                kind: "loopback TCP",
                key: thing_key(hose.low, hose.high, HOSE),
            };
            let pick = ink.frame.thing_pick(thing);
            tubes::hose(
                ink,
                &tubes::Hose {
                    points: &along,
                    forward,
                    backward,
                    pick,
                    hot: pass.touched(hose.low)
                        || pass.touched(hose.high)
                        || pass.selected == Some(thing),
                },
            );
            let middle = path::at(&along, path::length(&along) * 0.5).map_or(along[0], |m| m.0);
            let text = match (hose.toward_high, hose.toward_low) {
                (None, None) => "loopback TCP: rate not measured".to_owned(),
                (up, down) => format!(
                    "loopback TCP: {} / {}",
                    up.map_or_else(|| "n/a".to_owned(), |hose| per_second(hose.rate)),
                    down.map_or_else(|| "n/a".to_owned(), |hose| per_second(hose.rate)),
                ),
            };
            labels.push(([middle[0], middle[1], 0.9], text));
            self.drawn.hoses += 1;
            self.drawn.things.insert(
                thing.key,
                Subject::Hose {
                    low: hose.low,
                    high: hose.high,
                    toward_high: hose.toward_high,
                    toward_low: hose.toward_low,
                    at: [middle[0], middle[1], HOSE_Z],
                },
            );
        }
    }

    /// The manifolds, each with a stub for every outlet nothing is drawn for; the words to stand
    /// over them.
    fn paint_manifolds(&mut self, ink: &mut Ink<'_>, pass: &Pass<'_>) -> Vec<Label> {
        let mut manifold_labels = Vec::new();
        for (&owner, &site) in &pass.sites {
            let (Some(tank), Some(&outflow)) =
                (pass.by_id.get(&owner), self.room.outflows.get(&owner))
            else {
                continue;
            };
            let Outflow::Measured { outlets, .. } = outflow else {
                continue;
            };
            let drawn_runs = self
                .room
                .pipes
                .iter()
                .filter(|run| run.writer == owner && pass.by_id.contains_key(&run.reader))
                .count() as u32;
            let wanted = outlets.branches().saturating_sub(drawn_runs).min(5) as usize;
            let free = self.router.free_sides(Hull { owner, kind: 1 });
            let stubs: Vec<[f32; 2]> = free
                .iter()
                .take(wanted)
                .map(|&(dx, dy)| [site[0] + dx as f32, site[1] + dy as f32])
                .collect();
            let thing = Thing {
                kind: "manifold",
                key: thing_key(owner, owner, MANIFOLD),
            };
            let pick = ink.frame.thing_pick(thing);
            pipes::manifold(
                ink,
                site,
                tank.at,
                &stubs,
                pick,
                pass.touched(owner) || pass.selected == Some(thing),
            );
            manifold_labels.push(([site[0], site[1], PIPE_Z + 1.4], "split unknown".to_owned()));
            self.drawn.manifolds += 1;
            self.drawn.things.insert(
                thing.key,
                Subject::Manifold {
                    owner,
                    outflow,
                    at: [site[0], site[1], PIPE_Z + 0.4],
                },
            );
        }
        manifold_labels
    }

    /// A single outlet that is not drawn (a file, a terminal, a socket nothing is drawn for) is
    /// a capped stub on the tank.
    fn paint_stubs(&self, ink: &mut Ink<'_>, pass: &Pass<'_>) {
        for (&owner, outflow) in &self.room.outflows {
            let (Some(tank), Outflow::Measured { outlets, .. }) = (pass.by_id.get(&owner), outflow)
            else {
                continue;
            };
            if outlets.is_single() && outlets.pipes == 0 && outlets.tubes == 0 {
                let right = (-ink.facing[1], ink.facing[0]);
                let (dx, dy) = if right.0.abs() >= right.1.abs() {
                    (right.0.signum(), 0.0)
                } else {
                    (0.0, right.1.signum())
                };
                let wall = [
                    tank.at[0] + dx * tank.radius * 0.9,
                    tank.at[1] + dy * tank.radius * 0.9,
                    PIPE_Z,
                ];
                let tip = [wall[0] + dx * 0.7, wall[1] + dy * 0.7, PIPE_Z];
                ink.pipe(&[wall, tip], 0.1, super::draw::BRASS, tank.index as u32);
                ink.knob(tip, 0.14, super::draw::BRASS, tank.index as u32);
            }
        }
    }

    /// The tanks and their gauges; the words over each, with its memory, which the app thins out
    /// by.
    fn paint_tanks(
        &self,
        ink: &mut Ink<'_>,
        pass: &Pass<'_>,
        tanks: &[Tank],
    ) -> Vec<(u64, Point, String)> {
        let mut named_labels: Vec<(u64, Point, String)> = Vec::new();
        for tank in tanks {
            let lit = pass.touched(tank.id);
            let wait = self.room.waits.get(&tank.id).copied();
            let blocked = wait == Some(Wait::PipeWrite);
            let reading = match self.room.outflows.get(&tank.id) {
                Some(Outflow::Measured { rate, .. }) => Reading::Needle {
                    fill: gauge_fill(*rate),
                    blocked,
                },
                Some(Outflow::Unmeasured(_)) | None if blocked => Reading::Needle {
                    fill: 0.0,
                    blocked: true,
                },
                _ => Reading::Missing,
            };
            let art = tank::Tank {
                at: tank.at,
                radius: tank.radius,
                height: tank.height,
                band: tank.color,
                pick: tank.index as u32,
                hot: lit,
            };
            tank::draw(ink, &art, reading);
            if blocked {
                tank::steam(ink, &art, pass.time);
            }
            named_labels.push((
                tank.memory,
                [tank.at[0], tank.at[1], tank::top_of(tank.radius) + 0.7],
                format!("{} {}", label_name(&tank.name), bytes(tank.memory)),
            ));
        }
        named_labels
    }

    /// Tells the app what the frame drew: the tanks' places, the room's bounds, the titles of the
    /// bays, the words over things (most memory first), the tour and the inspector lines.
    fn announce(
        &self,
        stage: &mut Stage,
        processes: &[&Process],
        tanks: &[Tank],
        labels: Vec<Label>,
        mut named_labels: Vec<(u64, Point, String)>,
    ) {
        let named: BTreeMap<Identity, &Process> = processes
            .iter()
            .filter(|process| tanks.iter().any(|tank| tank.id == process.id))
            .map(|process| (process.id, *process))
            .collect();
        for tank in tanks {
            stage
                .positions
                .insert(tank.id, [tank.at[0], tank.at[1], 0.0]);
        }
        if let Some((low, high)) = self.floor.room() {
            for [x, y] in [
                [low[0], low[1]],
                [high[0], low[1]],
                [high[0], high[1]],
                [low[0], high[1]],
            ] {
                stage.bounds.push([x, y, 0.0]);
                stage.bounds.push([x, y, TUBE_Z + 1.0]);
            }
        }
        for bay in self.floor.bays() {
            let text = named
                .values()
                .find(|process| super::bay_key(process) == bay.key)
                .map_or_else(|| "no cgroup".to_owned(), |process| bay_title(process));
            let centre = [
                (bay.low[0] + bay.high[0]) * 0.5,
                (bay.low[1] + bay.high[1]) * 0.5,
            ];
            let reach = ((bay.high[0] - bay.low[0]).max(bay.high[1] - bay.low[1])) * 0.5;
            stage.titles.push(Title {
                at: [centre[0], centre[1], 0.0],
                reach,
                text: label_name(&text).into_owned(),
            });
        }
        for (point, text) in labels {
            stage.places.push((point, text));
        }
        named_labels.sort_by_key(|label| std::cmp::Reverse(label.0));
        for (_, point, text) in named_labels {
            stage.places.push((point, text));
        }
        self.tell(stage, tanks);
    }

    /// The beads of a hose: how far each direction has travelled by `step` more seconds, and
    /// what each shows.
    fn run_beads(&mut self, hose: &HoseRun, step: f32) -> (Beads, Beads) {
        let (ahead, behind) = self.beads.entry((hose.low, hose.high)).or_default();
        let one = |travelled: &mut f32, measured: Option<Hose>| match measured {
            None => Beads::Unmeasured,
            Some(hose) if hose.rate <= 0.0 => Beads::Idle,
            Some(hose) => {
                *travelled =
                    tubes::advanced(*travelled, water_speed(hose.rate) * CELL, step, BEAD_PERIOD);
                Beads::Moving(*travelled)
            }
        };
        let forward = one(ahead, hose.toward_high);
        let backward = one(behind, hose.toward_low);
        (forward, backward)
    }

    /// The tour's targets, the inspector lines of each tank and the tour's callouts.
    fn tell(&self, stage: &mut Stage, tanks: &[Tank]) {
        let mut ranked: Vec<(f32, Identity)> = tanks
            .iter()
            .filter_map(|tank| {
                let links = *self.room.links.get(&tank.id)?;
                let rate = self
                    .room
                    .outflows
                    .get(&tank.id)
                    .and_then(|outflow| outflow.rate());
                // A flow that was not measured adds no weight to the tank's links.
                let weight = links as f32 * 1.0e9 + rate.map_or(0.0, |rate| rate.max(0.0));
                Some((weight, tank.id))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for &(weight, id) in ranked.iter().take(12) {
            let Some(tank) = tanks.iter().find(|tank| tank.id == id) else {
                continue;
            };
            stage.tour.push(TourTarget {
                id,
                position: [tank.at[0], tank.at[1], 0.0],
                weight,
            });
            if let Some(outflow) = self.room.outflows.get(&id) {
                let line = flow_lines("it", *outflow).remove(0);
                stage.callout.push((id, line));
            }
        }
        for tank in tanks {
            let Some(outflow) = self.room.outflows.get(&tank.id) else {
                continue;
            };
            for line in flow_lines("it", *outflow) {
                stage.popup.push((tank.id, line));
            }
            match self.room.waits.get(&tank.id) {
                Some(Wait::PipeWrite) => stage
                    .popup
                    .push((tank.id, "blocked writing to a full pipe".to_owned())),
                Some(Wait::PipeRead) => stage
                    .popup
                    .push((tank.id, "waiting to read an empty pipe".to_owned())),
                Some(Wait::Unreadable) => stage.popup.push((
                    tank.id,
                    "wait channel reads 0: running, or not readable".to_owned(),
                )),
                _ => {}
            }
        }
    }
}

/// The beads' pitch along a hose, in world units.
const BEAD_PERIOD: f32 = 0.56;

/// A number for a pipe, to tell the severed pipes of one writer apart in a key.
fn pipe_number(id: crate::platform::PipeId) -> u64 {
    match id {
        crate::platform::PipeId::Anonymous(inode) => inode,
        crate::platform::PipeId::Fifo { device, inode } => device ^ inode.rotate_left(17),
    }
}

/// The overhead path of a tube: up from the top of one tank, along x and then y (or the other
/// way, by the pair), and down to the other. Where a tank carries several tubes each leaves from
/// its own place on the cap.
fn tube_path(
    low: &Tank,
    high: &Tank,
    risers: &BTreeMap<Identity, Vec<Identity>>,
    tube: &TubeRun,
) -> Vec<Point> {
    let slot = |tank: &Tank, peer: Identity| -> [f32; 2] {
        let peers = &risers[&tank.id];
        if peers.len() < 2 {
            return tank.at;
        }
        let Some(nth) = peers.iter().position(|&p| p == peer) else {
            return tank.at;
        };
        let angle = nth as f32 * 2.399_963;
        [
            tank.at[0] + RISER * angle.cos(),
            tank.at[1] + RISER * angle.sin(),
        ]
    };
    let (a, b) = (slot(low, tube.high), slot(high, tube.low));
    let corner = if (a[0] - b[0]).abs() < 1e-3 || (a[1] - b[1]).abs() < 1e-3 {
        None
    } else if crate::simulation::hash_identity(tube.low, tube.high.pid.into()) & 1 == 0 {
        Some([b[0], a[1]])
    } else {
        Some([a[0], b[1]])
    };
    let mut points = vec![
        [a[0], a[1], tank::top_of(low.radius) - 0.1],
        [a[0], a[1], TUBE_Z],
    ];
    if let Some(corner) = corner {
        points.push([corner[0], corner[1], TUBE_Z]);
    }
    points.push([b[0], b[1], TUBE_Z]);
    points.push([b[0], b[1], tank::top_of(high.radius) - 0.1]);
    points
}
