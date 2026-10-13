//! Anthill: syscalls as foraging ants. Every process is a nest, a mound whose size follows its
//! memory. Every open regular file is a pile of pebbles on a map of the filesystem, and one ant
//! walks for every K read and write calls: a read ant comes home carrying a crumb, a write ant
//! carries one out, and a crumb is as big as the bytes in one call. Pipes and sockets are dashed
//! tunnels between nests, a gingham blanket is a terminal, and the pheromone the ants lay glows
//! on the sand and fades.
//!
//! **What is data and what is not.** Where the ants go, how many go and what they carry is data:
//! the files a process holds and the bytes their positions advanced by (`io_detail`), the calls it
//! made (`Process::syscall_io`), and its tunnels (`Snapshot::pipes`, `links`). How an ant walks,
//! where a file's pile stands inside its directory's cell, the sand and the pebbles are not.
//! What nothing explains stays unattributed: it travels the tunnels, goes to a blanket or is
//! counted in the legend, and is never given to a file (`flows`).

mod colony;
mod draw;
mod flows;
mod ingest;
mod paint;
mod scale;
mod slab;
mod terrain;
mod words;

#[cfg(test)]
mod fixtures;

use std::collections::{HashMap, HashSet};

use self::colony::{Colony, Pheromone, STEP};
use self::flows::{MOST_BLANKETS, Plan, Tunnel};
use self::paint::Callout;
use self::scale::Scale;
use self::slab::{Packer, Parcels, Slab};
use self::terrain::Terrain;
use self::words::Facts;
use crate::model::{Identity, Process, Snapshot};
use crate::render::{Card, Color, Point, Stage, Thing};
use crate::simulation::{FixedStep, hash_identity};
use crate::views::ViewModule;

/// Seeds what an identity or a path hashes to here, apart from other views.
const SALT: u64 = 0x0a27_4110_fca5_7001;

/// What `Thing::kind` says of a pile and of a blanket.
const FILE: &str = "file";
const TERMINAL: &str = "terminal";

/// How much room a nest reserves over its radius, and the memory it reserves it for: twice what it
/// holds, so that a process that grows to double its memory keeps its place.
const RESERVE: f32 = 1.15;
const HEADROOM: f32 = 2.0;

/// The most piles that carry a label, and the most nests the tour is told about.
const LABELLED_PILES: usize = 12;
const TOURED: usize = 24;

/// How long a pile takes to appear or to fade, in seconds.
const PRESENCE: f32 = 1.5;

/// How fast the dashes of a tunnel slide, in world units per second for each decade of syscalls
/// per second.
const DASH_SPEED: f32 = 1.2;

/// How far a tunnel bends from the straight line between its nests, as a fraction of it, and how
/// high it arches over the ground, as a fraction of its length.
const BEND: f32 = 0.28;
const ARCH: f32 = 0.08;

/// The radius of a nest from its memory in bytes: 0.9 + 0.38 log2(max(MiB, 8) / 8) world units.
fn nest_radius(memory: f32) -> f32 {
    let mebibytes = (memory / 1_048_576.0).max(8.0);
    0.9 + 0.38 * (mebibytes / 8.0).log2()
}

/// The radius of a pile from the size of its file: 0.45 + 0.12 log2(size / 4096), from 0.45 to 3.
fn pile_radius(size: u64) -> f32 {
    (0.45 + 0.12 * (size as f32 / 4096.0).max(1.0).log2()).clamp(0.45, 3.0)
}

/// A nest: where its mound stands and what it is drawn with.
#[derive(Clone, Debug)]
struct Nest {
    at: [f32; 2],
    radius: f32,
    color: Color,
    /// The bearings of its entrances: one for each of up to four destinations.
    entrances: Vec<f32>,
}

/// A pile as the view keeps it between samples, to let it appear and fade.
#[derive(Clone, Debug)]
struct Record {
    at: [f32; 2],
    radius: f32,
    /// Places its pebbles: a hash of its path.
    seed: u64,
    /// When it first appeared, on the animation clock.
    first: f32,
    /// When it was first missing from a sample, if it is: it fades from then.
    gone: Option<f32>,
}

/// A tunnel as drawn: the path between two nests and how its dashes slide.
#[derive(Clone, Debug)]
struct Conduit {
    tunnel: Tunnel,
    path: Vec<Point>,
    color: Color,
    /// World units per second the dashes slide, from the calls per second no file explains.
    speed: f32,
    travelled: f32,
}

/// The anthill view: its layout, the colony and what the last sample said.
#[derive(Default)]
pub struct Anthill {
    slab: Slab,
    packer: Packer,
    parcels: Parcels,
    terrain: Option<Terrain>,
    colony: Option<Colony>,
    clock: Option<FixedStep>,
    scale: Scale,
    /// Whether the ground must be painted again: the pheromone moved.
    stale: bool,
    /// The sample taken in: its time, and how many processes were drawn from it.
    taken: Option<(u64, usize)>,
    plan: Option<Plan>,
    nests: HashMap<Identity, Nest>,
    records: HashMap<u64, Record>,
    /// The holder of each blanket, by slot.
    slots: [Option<Identity>; MOST_BLANKETS],
    conduits: Vec<Conduit>,
    /// Calls (or KiB) per second each nest sends, and the share of its bytes its files explain.
    busy: HashMap<Identity, f32>,
    explained: HashMap<Identity, f32>,
    /// The labels of the two crumbs that differ most.
    callouts: Vec<Callout>,
    facts: Facts,
}

impl Anthill {
    /// The room a nest of `memory` bytes reserves.
    fn reserve(memory: f32) -> f32 {
        nest_radius(memory * HEADROOM) * RESERVE
    }

    /// Gives every sampled process, drawn or not, a place, and the clearing room for them.
    fn seat(&mut self, stage: &Stage, snapshot: &Snapshot) {
        let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        self.packer.retain(|id| alive.contains(&id));
        let mut pending: Vec<(f32, Identity)> = Vec::new();
        for process in &snapshot.processes {
            let memory = stage.eased(process).1 as f32;
            let held = self.packer.get(process.id).map(|(_, room)| room);
            if held.is_none_or(|room| nest_radius(memory) * RESERVE > room) {
                pending.push((Self::reserve(memory), process.id));
            }
        }
        // The biggest first, ties by identity, walking the spiral outward together.
        pending.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut resume = 0.0;
        for (room, id) in pending {
            self.packer.place(id, room, &mut resume);
        }
        self.slab.grow_to_hold(self.packer.reach() + 1.0);
    }

    /// The ground and the colony's pheromone, built for the slab as it is now; rebuilt when the
    /// clearing grew.
    fn ground(&mut self) {
        if self
            .terrain
            .as_ref()
            .is_some_and(|ground| *ground.slab() == self.slab)
        {
            return;
        }
        let size = self.slab.size();
        let colony = self.colony.get_or_insert_with(|| Colony::new(size));
        colony.pheromone = Pheromone::new(size);
        self.terrain = Some(Terrain::new(&self.slab, &colony.pheromone));
        self.stale = true;
    }

    /// Advances the colony to `time` in whole steps, and says how far the frame lies into the
    /// next.
    fn advance(&mut self, time: f32) -> f32 {
        let clock = self
            .clock
            .get_or_insert_with(|| FixedStep::new(STEP, 10, 0.5));
        let Some(colony) = &mut self.colony else {
            clock.advance(time, |_| {});
            return 0.0;
        };
        let mut moved = false;
        let conduits = &mut self.conduits;
        let alpha = clock.advance(time, |dt| {
            colony.step(dt);
            for conduit in conduits.iter_mut() {
                conduit.travelled += conduit.speed * dt;
            }
            moved = true;
        });
        self.stale |= moved;
        alpha
    }
}

impl ViewModule for Anthill {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let alpha = self.advance(stage.time);
        self.ingest(stage, processes, snapshot);
        self.paint(stage, processes, alpha);
        processes.len()
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        words::legend(&self.facts, snapshot)
    }

    fn card(&self, thing: Thing, snapshot: &Snapshot) -> Option<Card> {
        let plan = self.plan.as_ref()?;
        match thing.kind {
            FILE => {
                let pile = plan.piles.iter().find(|pile| pile.key == thing.key)?;
                let record = self.records.get(&pile.key)?;
                let (title, lines) = words::pile_card(pile, snapshot);
                Some(Card {
                    title,
                    lines,
                    at: [record.at[0], record.at[1], 0.5 * record.radius],
                })
            }
            TERMINAL => {
                let (slot, holder) = self.slots.iter().enumerate().find_map(|(slot, holder)| {
                    let holder = (*holder)?;
                    (blanket_key(holder) == thing.key).then_some((slot, holder))
                })?;
                let at = self.slab.blanket(slot);
                let candidate = plan.candidates.iter().find(|c| c.holder == holder);
                let mut lines = vec![format!(
                    "terminal descriptors held: {}",
                    candidate.map_or(0, |c| c.ttys)
                )];
                if let Some(candidate) = candidate {
                    lines.push(format!(
                        "{} that no file, pipe or socket explains",
                        words::per_second(candidate.bytes)
                    ));
                }
                Some(Card {
                    title: format!("terminal held by {}", words::name_of(snapshot, holder)),
                    lines,
                    at: [at[0], at[1], 0.5],
                })
            }
            _ => None,
        }
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// The key of the thing that is the blanket of `holder`.
fn blanket_key(holder: Identity) -> u64 {
    hash_identity(holder, SALT ^ 2)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::fixtures::*;
    use super::*;
    use crate::app::fixtures::SCENE;
    use crate::io_detail::Flow;
    use crate::model::Measured;
    use crate::platform::{AccessMode, Source, Sources};
    use crate::render::{Camera, NONE, RenderInput, Scene, Target};
    use crate::simulation::hash;
    use crate::views::View;

    /// Three nests that read, write and hold a terminal, and one that was not read.
    fn colony_of_four() -> Snapshot {
        let reader = holding(
            counting(process(1, 200), 400.0, 4096.0, 0.0, 0.0),
            vec![
                file(
                    "/usr/lib/libc.so.6",
                    AccessMode::Read,
                    moved(1_000_000.0, 0.0),
                ),
                file("/var/log/db.log", AccessMode::Read, Flow::Unmeasured),
            ],
            Some(left(600_000.0, 0.0, 0.0)),
            0,
        );
        let writer = holding(
            counting(process(2, 50), 0.0, 0.0, 200.0, 8192.0),
            vec![file(
                "/var/log/db.log",
                AccessMode::Write,
                moved(0.0, 1_200_000.0),
            )],
            Some(left(0.0, 400_000.0, 0.0)),
            0,
        );
        let shell = holding(
            counting(process(3, 8), 20.0, 1.0, 0.0, 0.0),
            vec![file(
                "/home/tom/.bash_history",
                AccessMode::Write,
                moved(0.0, 100.0),
            )],
            Some(left(20.0, 0.0, 0.0)),
            1,
        );
        let sleeper = process(4, 30);
        snapshot(vec![reader, writer, shell, sleeper], 5.0)
    }

    fn drawn_ants(scene: &Scene) -> usize {
        state(scene)
            .colony
            .as_ref()
            .map_or(0, |colony| colony.ants.len())
    }

    fn rasterized(snapshot: &Snapshot) -> crate::render::Frame {
        let mut scene = scene();
        let mut frame = play(&mut scene, snapshot, 5.0, 9.0, 0.1);
        frame.rasterize();
        frame
    }

    #[test]
    fn anthill_renders_the_same_frame_twice_from_the_same_inputs() {
        let snapshot = colony_of_four();
        let (one, two) = (rasterized(&snapshot), rasterized(&snapshot));
        assert_eq!(one.pixels, two.pixels);
        assert_eq!(one.picks, two.picks);
        assert!(one.pixels.iter().any(|&byte| byte != 0));
    }

    #[test]
    fn piles_and_nests_are_pickable_and_ants_are_not() {
        let snapshot = colony_of_four();
        let mut scene = scene();
        let mut frame = play(&mut scene, &snapshot, 5.0, 9.0, 0.1);
        assert!(drawn_ants(&scene) > 20, "{} ants", drawn_ants(&scene));
        frame.rasterize();
        let targets: Vec<Target> = frame
            .picks
            .iter()
            .filter(|&&pick| pick != NONE)
            .filter_map(|&pick| frame.target(pick))
            .collect();
        assert!(
            targets
                .iter()
                .any(|target| matches!(target, Target::Process(_)))
        );
        assert!(
            targets
                .iter()
                .any(|target| matches!(target, Target::Thing(thing) if thing.kind == FILE))
        );
        assert!(
            targets
                .iter()
                .any(|target| matches!(target, Target::Thing(thing) if thing.kind == TERMINAL)),
            "the terminal's blanket"
        );
        // Every pick names a nest the view drew or a thing of its own.
        assert!(
            frame
                .things
                .iter()
                .all(|thing| thing.kind == FILE || thing.kind == TERMINAL)
        );
        // Ants and their crumbs write no pick: every item that is neither mound, pile nor blanket
        // is unpickable.
        let ants = frame
            .items
            .iter()
            .filter(
                |item| matches!(item, crate::render::Item::Sphere { pick, .. } if *pick == NONE),
            )
            .count();
        assert!(ants > 0, "crumbs are spheres that pick nothing");
    }

    #[test]
    fn nests_and_piles_keep_their_places_when_cpu_memory_or_population_changes() {
        let before = colony_of_four();
        let mut scene = scene();
        play(&mut scene, &before, 5.0, 8.0, 0.1);
        let nests: HashMap<Identity, [f32; 3]> = scene.positions.clone();
        let piles: HashMap<u64, [f32; 2]> = state(&scene)
            .records
            .iter()
            .map(|(key, record)| (*key, record.at))
            .collect();
        assert_eq!(nests.len(), 4);
        assert_eq!(piles.len(), 3);
        // Every CPU and memory changes, within what a nest reserved; two processes exit and two
        // others start, one of them holding the same file.
        let mut after = before.clone();
        for (k, process) in after.processes.iter_mut().enumerate() {
            process.cpu = 3.0 * k as f32 + 1.0;
            process.memory = if k % 2 == 0 {
                process.memory * 2
            } else {
                process.memory / 2
            };
        }
        after.processes.truncate(2);
        after.processes.push(holding(
            counting(process(10, 40), 50.0, 512.0, 0.0, 0.0),
            vec![
                file("/usr/lib/libc.so.6", AccessMode::Read, moved(10_000.0, 0.0)),
                file("/opt/new/file", AccessMode::Read, moved(10_000.0, 0.0)),
            ],
            Some(left(0.0, 0.0, 0.0)),
            0,
        ));
        after.processes.push(process(11, 400));
        after.elapsed = 6.0;
        play(&mut scene, &after, 8.0, 12.0, 0.1);
        for id in [id(1), id(2)] {
            assert_eq!(scene.positions[&id], nests[&id], "{id:?} moved");
        }
        for (key, at) in &piles {
            if let Some(record) = state(&scene).records.get(key) {
                assert_eq!(record.at, *at, "the pile of {key:x} moved");
            }
        }
        assert!(scene.positions.contains_key(&id(10)) && scene.positions.contains_key(&id(11)));
    }

    #[test]
    fn a_nest_that_outgrows_the_room_it_reserved_is_placed_again_and_no_other_moves() {
        let mut snapshot = colony_of_four();
        let mut scene = scene();
        play(&mut scene, &snapshot, 5.0, 7.0, 0.1);
        let places = scene.positions.clone();
        // The sleeper grows 64 times its memory: far past the double it reserved for.
        snapshot.processes[3].memory *= 64;
        snapshot.elapsed = 6.0;
        play(&mut scene, &snapshot, 7.0, 8.0, 0.1);
        // The easing of memory takes a moment, and the next sample sees the nest has outgrown it.
        snapshot.elapsed = 7.0;
        play(&mut scene, &snapshot, 8.0, 10.0, 0.1);
        assert_ne!(scene.positions[&id(4)], places[&id(4)]);
        for id in [id(1), id(2), id(3)] {
            assert_eq!(scene.positions[&id], places[&id]);
        }
    }

    #[test]
    fn the_clearing_grows_when_nests_overflow_and_the_nests_stay_where_they_were() {
        let mut scene = scene();
        let small = colony_of_four();
        play(&mut scene, &small, 5.0, 7.0, 0.1);
        let before = scene.positions.clone();
        assert_eq!(state(&scene).slab.size(), 64.0);
        let mut crowd = small.clone();
        crowd
            .processes
            .extend((100..400).map(|pid| process(pid, 500)));
        crowd.elapsed = 6.0;
        play(&mut scene, &crowd, 7.0, 9.0, 0.1);
        assert!(state(&scene).slab.size() > 64.0);
        for (id, at) in before {
            assert_eq!(scene.positions[&id], at);
        }
        // And what stands on it fits: every nest's room is inside the clearing.
        let slab = state(&scene).slab;
        assert!(state(&scene).packer.reach() <= slab.clearing() + 1.0);
    }

    #[test]
    fn a_hidden_process_keeps_its_seat_so_a_limit_moves_nothing() {
        let snapshot = colony_of_four();
        let place = |limit: usize| {
            let mut scene = scene();
            let camera = Camera::default();
            for k in 0..30 {
                let input = RenderInput {
                    limit,
                    ..RenderInput::small(&snapshot, View::at(0), &camera, 5.0 + k as f32 * 0.1)
                };
                scene.render(&input);
            }
            scene.positions.clone()
        };
        let (all, few) = (place(512), place(2));
        assert_eq!(few.len(), 2);
        for (id, at) in few {
            assert_eq!(all[&id], at);
        }
    }

    #[test]
    fn colony_follows_the_same_path_at_ten_and_sixty_frames_per_second() {
        let snapshot = colony_of_four();
        let run = |fps: u32| {
            let mut scene = scene();
            let camera = Camera::default();
            let mut states = HashMap::new();
            for k in 0..=6 * fps {
                let time = 5.0 + k as f32 / fps as f32;
                scene.render(&RenderInput::small(&snapshot, View::at(0), &camera, time));
                let colony = state(&scene).colony.as_ref().expect("a colony");
                let ants: Vec<([f32; 2], bool)> = colony
                    .ants
                    .iter()
                    .map(|ant| (ant.position, ant.homeward))
                    .collect();
                states.insert(colony.steps, ants);
            }
            states
        };
        let (slow, fast) = (run(10), run(60));
        let mut compared = 0;
        for (steps, ants) in &slow {
            if let Some(other) = fast.get(steps) {
                assert_eq!(ants, other, "after {steps} steps");
                compared += 1;
            }
        }
        assert!(compared >= 50, "compared {compared} states");
        assert!(
            slow.values().any(|ants| ants.len() > 10),
            "the colony had ants"
        );
    }

    /// Twenty heavy readers, each far from the file it reads.
    fn heavy() -> Snapshot {
        let processes = (1..=20)
            .map(|pid| {
                holding(
                    counting(process(pid, 100), 40_000.0, 4096.0, 0.0, 0.0),
                    vec![file(
                        &format!("/usr/lib/lib{pid}.so"),
                        AccessMode::Read,
                        moved(40_000.0 * 4096.0, 0.0),
                    )],
                    Some(left(0.0, 0.0, 0.0)),
                    0,
                )
            })
            .collect();
        snapshot(processes, 5.0)
    }

    #[test]
    fn k_keeps_ants_on_screen_near_three_thousand() {
        let snapshot = heavy();
        let mut scene = scene();
        play(&mut scene, &snapshot, 5.0, 45.0, 0.1);
        let colony = state(&scene).colony.as_ref().expect("a colony");
        let k = state(&scene).scale.k();
        assert!(k >= 100, "800,000 calls a second need K well above 1: {k}");
        assert!(
            (900..=3750).contains(&colony.ants.len()),
            "{} ants at K = {k}",
            colony.ants.len()
        );
        assert_eq!(colony.refused, 0, "the cap was never reached");
        let legend = scene.legend(View::at(0), &snapshot);
        assert!(
            legend.contains(&format!("(K = {})", scale::text(k))),
            "{legend}"
        );
    }

    #[test]
    fn the_legend_names_k_the_nests_and_every_mapping() {
        let snapshot = colony_of_four();
        let mut scene = scene();
        play(&mut scene, &snapshot, 5.0, 7.0, 0.1);
        let legend = scene.legend(View::at(0), &snapshot);
        assert!(legend.starts_with(" ant = K syscalls (K = "), "{legend}");
        assert!(legend.contains("4 nests, 3 piles"), "{legend}");
        for mapping in [
            "mound = process (size ~ memory)",
            "pile = open file",
            "crumb = bytes per syscall",
            "pale = read, coral = write",
            "glow = pheromone",
            "dashes = pipes, sockets",
        ] {
            assert!(legend.contains(mapping), "{mapping} in {legend}");
        }
    }

    #[test]
    fn without_open_file_data_it_draws_nests_and_no_ants_and_says_so() {
        let snapshot = snapshot(vec![process(1, 100), process(2, 20)], 5.0);
        let mut scene = scene();
        play(&mut scene, &snapshot, 5.0, 8.0, 0.1);
        assert_eq!(scene.positions.len(), 2);
        assert_eq!(drawn_ants(&scene), 0);
        assert!(state(&scene).records.is_empty());
        let legend = scene.legend(View::at(0), &snapshot);
        assert!(legend.contains(" | reading open files"), "{legend}");
        // The source is demanded and gone: the note says what that means.
        let mut gone = snapshot.clone();
        gone.missing = Source::IoDetail.into();
        let legend = scene.legend(View::at(0), &gone);
        assert!(
            legend.contains(" | no io detail: no piles, no ants"),
            "{legend}"
        );
    }

    #[test]
    fn missing_syscall_counts_switch_ants_to_bytes_and_say_so() {
        let mut snapshot = colony_of_four();
        for process in &mut snapshot.processes {
            process.syscall_io = None;
            if let Measured::Known(detail) = &process.io_detail {
                let mut plain = (**detail).clone();
                plain.unattributed = None;
                process.io_detail = Measured::Known(Arc::new(plain));
            }
        }
        snapshot.missing = Sources::of(&[Source::SyscallIo, Source::UnattributedIo]);
        let mut scene = scene();
        play(&mut scene, &snapshot, 5.0, 12.0, 0.1);
        let colony = state(&scene).colony.as_ref().expect("a colony");
        assert!(!colony.ants.is_empty());
        assert!(
            colony.ants.iter().all(|ant| ant.route.crumb.is_none()),
            "crumbs are unsized"
        );
        let legend = scene.legend(View::at(0), &snapshot);
        assert!(legend.starts_with(" ant = K KiB moved (K = "), "{legend}");
        assert!(
            legend.contains(" | no syscall io: ant = K KiB moved, crumbs unsized"),
            "{legend}"
        );
    }

    #[test]
    fn a_pile_opens_and_closes_by_growing_and_fading() {
        let open = colony_of_four();
        let mut scene = scene();
        play(&mut scene, &open, 5.0, 7.0, 0.1);
        let key = flows::file_key(7, hash::fnv1a_64("/home/tom/.bash_history") >> 8);
        assert!(state(&scene).records.contains_key(&key));
        let mut closed = open.clone();
        closed.processes[2] = holding(
            counting(process(3, 8), 20.0, 1.0, 0.0, 0.0),
            vec![],
            Some(left(20.0, 0.0, 0.0)),
            1,
        );
        closed.elapsed = 6.0;
        play(&mut scene, &closed, 7.0, 7.5, 0.1);
        let record = state(&scene).records.get(&key).expect("still fading");
        assert!(record.gone.is_some());
        play(&mut scene, &closed, 7.5, 9.0, 0.1);
        assert!(
            !state(&scene).records.contains_key(&key),
            "gone once it has faded"
        );
        // And a file that opens grows in rather than appearing: nothing at first, then pebbles that
        // swell until the pile is whole.
        let mut reopened = open.clone();
        reopened.elapsed = 7.0;
        let widest = |frame: &crate::render::Frame| {
            frame
                .items
                .iter()
                .filter_map(|item| match item {
                    crate::render::Item::Sphere { radius, pick, .. }
                        if pick & crate::render::THING != 0 =>
                    {
                        Some(*radius)
                    }
                    _ => None,
                })
                .fold(0.0_f32, f32::max)
        };
        let first = play(&mut scene, &reopened, 9.0, 9.05, 0.1);
        let middle = play(&mut scene, &reopened, 9.05, 9.8, 0.05);
        let whole = play(&mut scene, &reopened, 9.8, 11.0, 0.1);
        // The other piles are whole all the while; the one that opened is the difference.
        let spheres = |frame: &crate::render::Frame| {
            frame
                .items
                .iter()
                .filter(|item| matches!(item, crate::render::Item::Sphere { pick, .. } if pick & crate::render::THING != 0))
                .count()
        };
        assert!(spheres(&first) < spheres(&middle) && spheres(&middle) <= spheres(&whole));
        assert!(widest(&middle) <= widest(&whole));
    }

    #[test]
    fn the_tour_visits_the_nests_that_send_the_most_first() {
        let snapshot = colony_of_four();
        let mut scene = scene();
        play(&mut scene, &snapshot, 5.0, 7.0, 0.1);
        let mut weights: Vec<(Identity, f32)> = scene
            .tour
            .iter()
            .map(|stop| (stop.id, stop.weight))
            .collect();
        assert_eq!(
            weights.len(),
            3,
            "the sleeper sends nothing and is not a stop"
        );
        weights.sort_by(|a, b| b.1.total_cmp(&a.1));
        assert_eq!(weights[0].0, id(1));
        for stop in &scene.tour {
            assert_eq!(stop.position[0], scene.positions[&stop.id][0]);
        }
    }

    #[test]
    fn a_pile_and_a_blanket_have_cards_that_name_their_holders() {
        let snapshot = colony_of_four();
        let mut scene = scene();
        let frame = play(&mut scene, &snapshot, 5.0, 7.0, 0.1);
        let view = View::at(0);
        let log = frame
            .things
            .iter()
            .copied()
            .filter(|thing| thing.kind == FILE)
            .find_map(|thing| {
                let card = scene.card(view, thing, &snapshot)?;
                (card.title == "/var/log/db.log").then_some(card)
            })
            .expect("the log's pile has a card");
        assert!(
            log.lines[1].starts_with("readers 1, writers 1: p1, p2"),
            "{:?}",
            log.lines
        );
        assert!(
            log.lines
                .iter()
                .any(|line| line.starts_with("moved at least"))
        );
        assert!(
            log.lines
                .iter()
                .any(|line| line.contains("1 without an earlier reading")),
            "{:?}",
            log.lines
        );
        let terminal = frame
            .things
            .iter()
            .copied()
            .find(|thing| thing.kind == TERMINAL)
            .expect("a blanket");
        let card = scene.card(view, terminal, &snapshot).expect("its card");
        assert_eq!(card.title, "terminal held by p3");
        assert!(
            card.lines[0].starts_with("terminal descriptors held: 1"),
            "{:?}",
            card.lines
        );
        // A thing the view does not show has no card.
        let other = Thing { kind: FILE, key: 1 };
        assert!(scene.card(view, other, &snapshot).is_none());
    }

    #[test]
    fn the_stage_gets_the_slab_for_the_camera_and_labels_for_the_regions_and_the_busiest_files() {
        let snapshot = colony_of_four();
        let mut scene = scene();
        play(&mut scene, &snapshot, 5.0, 7.0, 0.1);
        let texts: Vec<&str> = scene.places.iter().map(|(_, text)| text.as_str()).collect();
        for region in ["/usr", "/home", "/var", "/tmp", "other"] {
            assert!(texts.contains(&region), "{texts:?}");
        }
        assert!(
            texts.contains(&"libc.so.6") && texts.contains(&"db.log"),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.ends_with("/read"))
                && texts.iter().any(|text| text.ends_with("/write")),
            "the crumbs that differ most: {texts:?}"
        );
        assert!(!texts.contains(&".bash_history") || texts.len() <= 5 + 12 + 2);
        let bounds = scene.bounded();
        assert!(bounds, "the slab's corners frame the camera");
    }

    #[test]
    fn file_paths_are_sanitized_before_display() {
        const PATH: &str = "/tmp/ev\x1b]0;pwned\x07il\x1b[2J\u{9b}31m\u{85}\u{202e}\r\nname";
        let reading = holding(
            Process {
                name: "ev\x1b]0;pwned\x07il\u{202e}".to_owned(),
                ..counting(process(1, 100), 100.0, 4096.0, 0.0, 0.0)
            },
            vec![file(PATH, AccessMode::Read, moved(409_600.0, 0.0))],
            Some(left(0.0, 0.0, 0.0)),
            0,
        );
        let snapshot = snapshot(vec![reading], 5.0);
        let mut app = crate::app::App::new(View::at(0), snapshot.clone(), vec![true]);
        app.scene = scene();
        for k in 0..40 {
            app.animation = 5.0 + k as f32 * 0.1;
            app.render(SCENE.width, SCENE.height, 512);
        }
        let frame = app.render(SCENE.width, SCENE.height, 512);
        // The view hands the path on as the system gave it...
        let card = app
            .scene
            .card(View::at(0), frame.things[0], &snapshot)
            .expect("the pile has a card");
        assert_eq!(card.title, PATH);
        // ...and the app makes it safe wherever it reaches the terminal.
        let thing = frame.things[0];
        app.thing = Some(thing);
        let mut frame2 = app.render(SCENE.width, SCENE.height, 512);
        let (labels, panels) = app.overlay(&mut frame2, &SCENE, SCENE.pick_radius(false));
        let mut seen = String::new();
        for text in labels.iter().map(|label| &label.text) {
            seen.push_str(text.as_str());
        }
        for text in panels.iter().flat_map(|panel| &panel.lines) {
            seen.push_str(text.as_str());
        }
        assert!(
            seen.contains("/tmp/ev?]0;pwned?i"),
            "the path is on screen, cleaned: {seen}"
        );
        assert!(
            seen.chars()
                .all(|character| character.is_ascii_graphic() || character == ' '),
            "{seen:?}"
        );
    }
}
