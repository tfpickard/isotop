//! Tests of the plumbing view as a whole: scenes built by hand and the boiler room of the demo,
//! drawn through a `Scene` and read back through the view's own state.

use std::sync::Arc;

use super::*;
use crate::harness::{churned, rescaled};
use crate::io_detail::{IoDetail, Unattributed};
use crate::model::testing::process;
use crate::model::{DemoOptions, Measured, Pipe, Severed, demo_with};
use crate::platform::Wait;
use crate::render::{Camera, Frame, Item, RenderInput, Scene};
use crate::views::View;

const PLUMBING: DemoOptions = DemoOptions {
    plumbing: true,
    ..DemoOptions::NONE
};

fn view() -> View {
    View::named("plumbing").expect("plumbing is registered")
}

fn who(pid: u32) -> Identity {
    Identity {
        pid,
        start: u64::from(pid),
    }
}

/// A process that wrote `write` bytes per second no file position explains, and holds nothing
/// else: the whole of what B8 reads of it.
fn measured(pid: u32, write: f32) -> Process {
    Process {
        id: who(pid),
        name: format!("proc-{pid}"),
        memory: 64 << 20,
        io_detail: Measured::Known(Arc::new(IoDetail {
            unattributed: Some(Unattributed {
                read: 0.0,
                write,
                either: 0.0,
            }),
            pipes: 1,
            ..IoDetail::default()
        })),
        ..process(pid, 1)
    }
}

fn pipe(id: u64, writer: u32, reader: u32) -> Pipe {
    Pipe {
        id: PipeId::Anonymous(id),
        reader: who(reader),
        writer: who(writer),
        two_way: false,
        buffered: None,
        fifo: false,
    }
}

/// Two processes joined by one pipe, as the sample at `elapsed` reads them: process 1 writes,
/// 2 reads.
fn joined(elapsed: f64) -> Snapshot {
    let mut snapshot = Snapshot {
        processes: vec![measured(1, 2.0e6), measured(2, 0.0)],
        elapsed,
        sources: Source::Pipes.into(),
        ..Snapshot::default()
    };
    snapshot.pipes = vec![pipe(5, 1, 2)];
    snapshot.pipe_holds.insert(
        who(1),
        crate::model::PipeHold {
            writes: 1,
            reads: 0,
        },
    );
    snapshot.pipe_holds.insert(
        who(2),
        crate::model::PipeHold {
            writes: 0,
            reads: 1,
        },
    );
    snapshot
}

/// The same two processes after process 2 dropped its end: the pipe is severed.
fn cut(elapsed: f64) -> Snapshot {
    let mut snapshot = joined(elapsed);
    snapshot.pipes.clear();
    snapshot.pipe_holds.remove(&who(2));
    snapshot.severed = vec![Severed {
        writer: who(1),
        id: PipeId::Anonymous(5),
    }];
    snapshot
}

fn draw(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> Frame {
    let camera = Camera::default();
    let mut input = RenderInput::small(snapshot, view(), &camera, time);
    (input.width, input.height) = (640, 360);
    scene.render(&input)
}

fn state(scene: &Scene) -> &Plumbing {
    scene
        .module::<Plumbing>(view())
        .expect("the plumbing view has drawn")
}

/// The card text of the first thing of `kind` in `frame`.
fn card_of(scene: &Scene, frame: &Frame, snapshot: &Snapshot, kind: &str) -> String {
    let thing = frame
        .things
        .iter()
        .find(|thing| thing.kind == kind)
        .unwrap_or_else(|| panic!("no {kind} in the frame"));
    let card = scene
        .card(view(), *thing, snapshot)
        .expect("a card for a thing it drew");
    format!("{} | {}", card.title, card.lines.join(" | "))
}

/// Plays the boiler room of the demo for `seconds`, a sample a second with two frames each, as a
/// live run would have drawn them.
fn play(scene: &mut Scene, seconds: u32) -> Snapshot {
    let mut last = demo_with(0.0, 192, PLUMBING);
    for second in 0..=seconds {
        last = demo_with(f64::from(second), 192, PLUMBING);
        draw(scene, &last, second as f32);
        draw(scene, &last, second as f32 + 0.5);
    }
    last
}

#[test]
fn a_pipe_bursts_when_its_last_reader_goes_and_a_writer_remains() {
    let mut scene = Scene::new();
    draw(&mut scene, &joined(1.0), 1.0);
    assert_eq!(state(&scene).drawn.pipes, 1);
    assert_eq!(state(&scene).drawn.bursts, 0);
    let broken = cut(2.0);
    let frame = draw(&mut scene, &broken, 2.0);
    assert_eq!(state(&scene).drawn.bursts, 1, "the reader end is gone");
    assert_eq!(state(&scene).drawn.pipes, 0, "and nothing joins the two");
    let text = card_of(&scene, &frame, &broken, "pipe");
    assert!(text.contains("reader gone: EPIPE next"), "{text}");
    assert!(text.contains("proc-2 dropped its end"), "{text}");
    // The spray lasts six seconds; then the break is a capped end, still unread.
    draw(&mut scene, &broken, 7.0);
    assert_eq!(state(&scene).drawn.bursts, 1);
    draw(&mut scene, &broken, 9.0);
    assert_eq!(state(&scene).drawn.bursts, 0, "past the six seconds");
    assert_eq!(
        state(&scene).breaks.len(),
        1,
        "the pipe is still without a reader"
    );
    // A reader that comes back mends it.
    draw(&mut scene, &joined(4.0), 10.0);
    assert!(state(&scene).breaks.is_empty());
    assert_eq!(state(&scene).drawn.pipes, 1);
}

#[test]
fn a_pipe_the_view_never_saw_a_reader_of_is_unread_and_not_burst() {
    let mut scene = Scene::new();
    let broken = cut(1.0);
    let frame = draw(&mut scene, &broken, 1.0);
    assert_eq!(state(&scene).drawn.bursts, 0);
    assert!(state(&scene).breaks.is_empty());
    let text = card_of(&scene, &frame, &broken, "pipe");
    assert!(!text.contains("reader gone"), "{text}");
    assert!(text.contains("no process holds a read end"), "{text}");
}

#[test]
fn a_pipe_without_writers_does_not_burst() {
    // Process 2 holds the read end of a pipe nobody writes: nothing is severed and nothing joins
    // it to anything, so it is no tank and no pipe is drawn.
    let mut scene = Scene::new();
    let mut snapshot = joined(1.0);
    snapshot.pipes.clear();
    snapshot.pipe_holds.remove(&who(1));
    draw(&mut scene, &snapshot, 1.0);
    let drawn = &state(&scene).drawn;
    assert_eq!((drawn.tanks, drawn.pipes, drawn.bursts), (0, 0, 0));
    draw(&mut scene, &snapshot, 9.0);
    assert_eq!(state(&scene).drawn.bursts, 0);
}

/// Whether `frame` has a gauge needle pinned in the red: the colour only that needle has.
fn has_red_needle(frame: &Frame) -> bool {
    frame
        .items
        .iter()
        .any(|item| matches!(item, Item::Triangle(_, color, _) if *color == [150, 20, 20]))
}

#[test]
fn wchan_pipe_write_turns_the_gauge_red_and_pipe_read_runs_dry() {
    let calm = joined(1.0);
    let mut scene = Scene::new();
    let frame = draw(&mut scene, &calm, 1.0);
    let drawn = &state(&scene).drawn;
    assert_eq!((drawn.blocked, drawn.dry), (0, 0));
    assert!(!has_red_needle(&frame), "nothing is blocked");

    // The writer sleeps in pipe_write: its gauge is pinned in the red and the pipe is packed.
    let mut blocked = joined(2.0);
    blocked.waits.insert(who(1), Wait::PipeWrite);
    let mut scene = Scene::new();
    let frame = draw(&mut scene, &blocked, 2.0);
    assert_eq!(state(&scene).drawn.blocked, 1);
    assert!(has_red_needle(&frame), "the needle is in the red");
    let text = card_of(&scene, &frame, &blocked, "pipe");
    assert!(text.contains("blocked writing to a full pipe"), "{text}");

    // The reader sleeps in pipe_read: that run's glass is empty.
    let mut dry = joined(3.0);
    dry.waits.insert(who(2), Wait::PipeRead);
    let mut scene = Scene::new();
    let frame = draw(&mut scene, &dry, 3.0);
    let drawn = &state(&scene).drawn;
    assert_eq!((drawn.blocked, drawn.dry), (0, 1));
    assert!(
        !has_red_needle(&frame),
        "a starved reader is not a red gauge"
    );
    assert!(card_of(&scene, &frame, &dry, "pipe").contains("waiting to read an empty pipe"));

    // A wait channel that reads 0 is unknown: neither.
    let mut unknown = joined(4.0);
    unknown.waits.insert(who(1), Wait::Unreadable);
    unknown.waits.insert(who(2), Wait::Unreadable);
    let mut scene = Scene::new();
    let frame = draw(&mut scene, &unknown, 4.0);
    let drawn = &state(&scene).drawn;
    assert_eq!((drawn.blocked, drawn.dry), (0, 0));
    assert!(!has_red_needle(&frame));
}

#[test]
fn a_flow_that_was_not_measured_says_so_and_never_shows_a_number() {
    let mut snapshot = joined(1.0);
    snapshot.processes[0].io_detail = Measured::Pending;
    let mut scene = Scene::new();
    let frame = draw(&mut scene, &snapshot, 1.0);
    let text = card_of(&scene, &frame, &snapshot, "pipe");
    assert!(text.contains("flow not measured"), "{text}");
    assert!(text.contains("only the busiest processes are"), "{text}");
    assert!(!text.contains("B/s") && !text.contains("KiB/s"), "{text}");
    let popup: Vec<&str> = scene
        .popup
        .iter()
        .filter(|(owner, _)| *owner == who(1))
        .map(|(_, line)| line.as_str())
        .collect();
    assert!(
        popup.iter().any(|line| line.contains("not measured")),
        "{popup:?}"
    );
}

#[test]
fn the_demo_draws_its_cast_and_the_run_that_burst() {
    let mut scene = Scene::new();
    let snapshot = play(&mut scene, 12);
    let frame = draw(&mut scene, &snapshot, 12.5);
    let drawn = &state(&scene).drawn;
    assert_eq!(drawn.tanks, 16);
    assert_eq!(drawn.pipes, 8, "seven pipes between tanks and a FIFO");
    assert_eq!((drawn.tubes, drawn.hoses), (3, 1));
    assert_eq!(drawn.bursts, 1, "redis writes into a pipe gzip let go of");
    assert_eq!((drawn.blocked, drawn.dry), (1, 1));
    assert_eq!(scene.visible, 16);
    assert!(frame.things.iter().any(|thing| thing.kind == "manifold"));
    let legend = scene.legend(view(), &snapshot);
    assert!(
        legend.starts_with(" 16 tanks | 8 pipe runs, 3 unix sockets, 1 loopback TCP"),
        "{legend}"
    );
    assert!(legend.contains("1 burst"), "{legend}");
}

#[test]
fn a_manifold_stands_where_a_flow_splits_and_never_where_it_is_exact() {
    let mut scene = Scene::new();
    let snapshot = play(&mut scene, 12);
    let owners: Vec<String> = state(&scene)
        .drawn
        .things
        .values()
        .filter_map(|subject| match subject {
            card::Subject::Manifold { owner, .. } => snapshot
                .processes
                .iter()
                .find(|process| process.id == *owner)
                .map(|process| process.name.clone()),
            _ => None,
        })
        .collect();
    for split in ["compiler", "grep", "journald"] {
        assert!(
            owners.iter().any(|name| name == split),
            "{split} splits: {owners:?}"
        );
    }
    for exact in ["cat", "sort", "redis", "worker", "nginx"] {
        assert!(
            owners.iter().all(|name| name != exact),
            "{exact} has one outlet: {owners:?}"
        );
    }
}

type Places = Vec<(Identity, [f32; 3])>;
type Routes = Vec<(route::Key, Vec<[f32; 2]>)>;

/// The corners of every route and the places of the tanks, which a change of CPU, memory or
/// population must leave as they are.
fn layout(scene: &Scene) -> (Places, Routes) {
    let mut places: Places = scene
        .positions
        .iter()
        .map(|(id, place)| (*id, *place))
        .collect();
    places.sort_by_key(|(id, _)| *id);
    (places, state(scene).router.corners())
}

#[test]
fn tanks_and_routes_stay_put_when_cpu_memory_or_population_change() {
    let mut scene = Scene::new();
    let steady = play(&mut scene, 12);
    for step in 0..4 {
        draw(&mut scene, &steady, 13.0 + step as f32 * 0.25);
    }
    let (places, routes) = layout(&scene);
    assert_eq!(places.len(), 16);
    assert!(routes.len() >= 8, "{} routes", routes.len());

    // CPU scaled by 0.25 to 4 and memory by 0.5 to 2.
    let mut changed = rescaled(&steady);
    changed.elapsed = steady.elapsed + 1.0;
    for step in 0..12 {
        draw(&mut scene, &changed, 14.0 + step as f32 * 0.25);
    }
    let (after, after_routes) = layout(&scene);
    assert_eq!(after, places, "no tank moved");
    assert_eq!(after_routes, routes, "no route changed");

    // A tenth of the processes exit and as many start: whoever stays keeps its place and its runs.
    let (mut population, exited) = churned(&steady);
    population.elapsed = steady.elapsed + 5.0;
    // Inside the ten seconds a tank stays after its last link closed (gzip's, at the tenth second).
    for step in 0..12 {
        draw(&mut scene, &population, 17.0 + step as f32 * 0.1);
    }
    let (now, now_routes) = layout(&scene);
    for (id, place) in &places {
        if !exited.contains(id) {
            let found = now.iter().find(|(other, _)| other == id);
            assert_eq!(found.map(|(_, place)| place), Some(place), "{id:?}");
        }
    }
    for (key, corners) in &routes {
        let survives = !exited.contains(&key.from.owner) && !exited.contains(&key.to.owner);
        if survives {
            let again = now_routes.iter().find(|(other, _)| other == key);
            assert_eq!(again.map(|(_, corners)| corners), Some(corners), "{key:?}");
        }
    }
}

/// How far two distances along a dash pattern of `period` are apart, going the short way round.
fn apart(a: f32, b: f32, period: f32) -> f32 {
    let difference = (a - b).rem_euclid(period);
    difference.min(period - difference)
}

#[test]
fn water_moves_by_wall_clock_not_by_frame() {
    let sample = demo_with(20.0, 192, PLUMBING);
    let run = |frames_per_second: u32| {
        let mut scene = Scene::new();
        for step in 0..=(3 * frames_per_second) {
            draw(
                &mut scene,
                &sample,
                20.0 + step as f32 / frames_per_second as f32,
            );
        }
        state(&scene).phases.clone()
    };
    let (slow, fast) = (run(10), run(60));
    assert!(!slow.is_empty());
    assert_eq!(
        slow.keys().collect::<Vec<_>>(),
        fast.keys().collect::<Vec<_>>()
    );
    let mut moved = false;
    for (key, (ahead, behind)) in &slow {
        let (quick_ahead, quick_behind) = fast[key];
        assert!(
            apart(*ahead, quick_ahead, draw::pipes::DASH_PERIOD) < 1e-2,
            "{key:?}"
        );
        assert!(
            apart(*behind, quick_behind, draw::pipes::DASH_PERIOD) < 1e-2,
            "{key:?}"
        );
        moved |= *ahead > 0.0;
    }
    assert!(moved, "some water flows");
    let hoses = |scene_beads: &BTreeMap<(Identity, Identity), (f32, f32)>| scene_beads.len();
    let mut scene = Scene::new();
    draw(&mut scene, &sample, 20.0);
    assert_eq!(hoses(&state(&scene).beads), 1);
}

#[test]
fn a_tank_stays_ten_seconds_after_its_last_link_closes() {
    let mut scene = Scene::new();
    draw(&mut scene, &joined(1.0), 1.0);
    assert_eq!(state(&scene).drawn.tanks, 2);
    let mut alone = joined(2.0);
    alone.pipes.clear();
    draw(&mut scene, &alone, 2.0);
    assert_eq!(state(&scene).drawn.tanks, 2, "still there a second later");
    draw(&mut scene, &alone, 10.9);
    assert_eq!(state(&scene).drawn.tanks, 2, "and at 9.9 s");
    draw(&mut scene, &alone, 11.5);
    assert_eq!(state(&scene).drawn.tanks, 0, "gone after ten");
}

#[test]
fn the_selected_process_is_a_tank_even_with_no_link() {
    let mut scene = Scene::new();
    let mut alone = joined(1.0);
    alone.pipes.clear();
    let camera = Camera::default();
    let mut input = RenderInput::small(&alone, view(), &camera, 1.0);
    input.selected = Some(who(2));
    scene.render(&input);
    assert_eq!(state(&scene).drawn.tanks, 1);
    assert!(scene.positions.contains_key(&who(2)));
}

#[test]
fn the_most_tanks_are_the_ones_with_the_most_links() {
    // A ring of 300 processes, each writing to the next, with one more link for the first twenty.
    let count = 300_u32;
    let mut snapshot = Snapshot {
        processes: (1..=count).map(|pid| measured(pid, 1.0e5)).collect(),
        elapsed: 1.0,
        sources: Source::Pipes.into(),
        ..Snapshot::default()
    };
    for pid in 1..=count {
        snapshot
            .pipes
            .push(pipe(u64::from(pid), pid, pid % count + 1));
    }
    for pid in 1..=20 {
        snapshot
            .pipes
            .push(pipe(1000 + u64::from(pid), pid, pid + 100));
    }
    let mut scene = Scene::new();
    let camera = Camera::default();
    let mut input = RenderInput::small(&snapshot, view(), &camera, 1.0);
    input.limit = 4096;
    scene.render(&input);
    assert_eq!(state(&scene).drawn.tanks, MOST_TANKS);
    assert_eq!(scene.visible, MOST_TANKS);
    assert_eq!(scene.collapsed, count as usize - MOST_TANKS);
    for pid in 1..=20 {
        assert!(
            scene.positions.contains_key(&who(pid)),
            "{pid} has three links"
        );
    }
    // A process outside the cap that is selected is kept, and the cap is not exceeded.
    let mut input = RenderInput::small(&snapshot, view(), &camera, 1.1);
    input.limit = 4096;
    input.selected = Some(who(count));
    scene.render(&input);
    assert_eq!(state(&scene).drawn.tanks, MOST_TANKS);
    assert!(scene.positions.contains_key(&who(count)));
}

#[test]
fn the_tour_offers_the_tanks_with_the_most_links_first() {
    let mut scene = Scene::new();
    let snapshot = play(&mut scene, 12);
    draw(&mut scene, &snapshot, 12.6);
    assert!(!scene.tour.is_empty() && scene.tour.len() <= 12);
    let first = scene.tour[0].id;
    let links = |id: Identity| state(&scene).room.links.get(&id).copied().unwrap_or(0);
    for target in &scene.tour {
        assert!(
            scene.positions.contains_key(&target.id),
            "a tour stop is a tank"
        );
        assert!(links(target.id) <= links(first) || target.weight <= scene.tour[0].weight);
    }
    let heaviest = scene
        .tour
        .iter()
        .max_by(|a, b| a.weight.total_cmp(&b.weight))
        .expect("a target");
    assert_eq!(heaviest.id, first, "heaviest first");
}

#[test]
fn the_legend_names_what_is_missing_and_what_it_does_instead() {
    let mut scene = Scene::new();
    let mut snapshot = joined(1.0);
    snapshot.missing = Source::WaitChannel.into();
    draw(&mut scene, &snapshot, 1.0);
    let legend = scene.legend(view(), &snapshot);
    assert!(
        legend.contains(" | no wait channel: no red or dry gauges"),
        "{legend}"
    );
    assert!(legend.contains(" | sources: pipes"), "{legend}");
    assert!(
        legend.find("2 tanks").unwrap() < legend.find("no wait channel").unwrap()
            && legend.find("no wait channel").unwrap() < legend.find("tank = process").unwrap(),
        "dynamic parts, then what is missing, then the mappings: {legend}"
    );
}

#[test]
fn an_empty_machine_draws_no_tanks() {
    let mut scene = Scene::new();
    let frame = draw(&mut scene, &Snapshot::default(), 1.0);
    assert_eq!(state(&scene).drawn.tanks, 0);
    assert!(frame.things.is_empty());
}
