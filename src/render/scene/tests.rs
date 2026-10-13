use super::*;
use crate::model::demo;
use crate::simulation::GROW_SECONDS;
use crate::views::{testing, view};
fn render(scene: &mut Scene, snapshot: &Snapshot, view: View, time: f32) -> Frame {
    let mut frame = scene.render(&RenderInput::small(
        snapshot,
        view,
        &Camera::default(),
        time,
    ));
    frame.rasterize();
    frame
}

#[test]
fn spawned_bodies_grow_and_exits_leave_a_fading_flash() {
    let snapshot = demo(1.0, 32);
    let mut scene = Scene::new();
    scene.intro = true;
    render(&mut scene, &snapshot, view("orbit"), 10.0);
    let id = snapshot.processes[3].id;
    assert!(scene.growth(id, 10.0) < 0.01);
    assert!(scene.growth(id, 10.0 + GROW_SECONDS) > 0.99);
    let mut fewer = snapshot.clone();
    fewer.processes.remove(3);
    render(&mut scene, &fewer, view("orbit"), 10.5);
    assert_eq!(scene.deaths.len(), 1);
    render(
        &mut scene,
        &fewer,
        view("orbit"),
        10.5 + FADE_SECONDS + 0.01,
    );
    assert!(scene.deaths.is_empty());
    let mut quiet = Scene::new();
    render(&mut quiet, &snapshot, view("city"), 10.0);
    assert_eq!(
        quiet.growth(id, 10.0),
        1.0,
        "no intro: existing processes start settled"
    );
}

#[test]
fn links_glow_only_when_focused_mode_highlights_them() {
    let snapshot = demo(3.0, 64);
    let mut scene = Scene::new();
    let beams = |scene: &mut Scene| {
        let frame = scene.render(&RenderInput::small(
            &snapshot,
            view("orbit"),
            &Camera::default(),
            3.0,
        ));
        frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Beam(..)))
            .count()
    };
    let all = beams(&mut scene);
    assert!(all > 0);
    scene.links = Links::Focused;
    assert_eq!(beams(&mut scene), 0);
    scene.highlight = vec![snapshot.links[0].0];
    let focused = beams(&mut scene);
    assert!(focused > 0 && focused < all);
    scene.links = Links::Off;
    assert_eq!(beams(&mut scene), 0);
}

#[test]
fn a_row_with_link_strokes_draws_its_links_as_unpickable_triangles() {
    let snapshot = demo(3.0, 64);
    let count = |view: View| {
        let mut scene = Scene::with_rows(crate::views::testing::LINKED);
        let frame = scene.render(&RenderInput::small(
            &snapshot,
            view,
            &Camera::default(),
            3.0,
        ));
        let beams = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Beam(..)))
            .count();
        let strokes = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Triangle(_, _, NONE)))
            .count();
        (beams, strokes)
    };
    let (beams, strokes) = count(View::at(0));
    assert!(
        beams > 0 && strokes == 0,
        "{beams} beams, {strokes} strokes"
    );
    let (beams, strokes) = count(View::at(1));
    assert!(
        beams == 0 && strokes > 0,
        "{beams} beams, {strokes} strokes"
    );
}

#[test]
fn pressure_tints_the_sky() {
    let mut snapshot = demo(1.0, 16);
    snapshot.pressure = [0.0; 3];
    let calm = render(&mut Scene::new(), &snapshot, view("orbit"), 1.0);
    snapshot.pressure = [0.0, 60.0, 0.0];
    let stressed = render(&mut Scene::new(), &snapshot, view("orbit"), 1.0);
    let red = |frame: &Frame| {
        frame
            .pixels
            .iter()
            .step_by(3)
            .map(|&r| r as u64)
            .sum::<u64>()
    };
    assert!(red(&stressed) > red(&calm) * 2);
}

#[test]
fn every_view_renders_the_demo_and_an_empty_machine() {
    let snapshot = demo(30.0, 128);
    // A view that draws no processes (the matrix draws the journal) has its own tests.
    for view in View::all().filter(|view| view.info().contract.draws_processes) {
        let mut scene = Scene::new();
        for k in 0..4 {
            render(&mut scene, &snapshot, view, 30.0 + k as f32 * 0.1);
        }
        assert!(scene.visible > 0, "{view:?} shows nothing");
        assert!(!scene.positions.is_empty(), "{view:?} has no positions");
        let mut camera = Camera::default();
        scene.fit(&mut camera, 320, 180);
        let mut frame = scene.render(&RenderInput::small(&snapshot, view, &camera, 30.5));
        frame.rasterize();
        let picks = frame.picks.iter().filter(|&&p| p != NONE).count();
        assert!(picks > 0, "{view:?} has nothing to click");
        assert!(
            frame
                .picks
                .iter()
                .all(|&p| p == NONE || frame.target(p).is_some())
        );
        render(&mut Scene::new(), &Snapshot::default(), view, 0.0);
    }
}

#[test]
fn a_module_view_draws_records_and_legends_through_the_trait() {
    let snapshot = demo(10.0, 32);
    let (probe, quiet) = (View::at(0), View::at(1));
    let mut scene = Scene::with_rows(testing::ROWS);
    assert_eq!(
        scene.legend(probe, &snapshot),
        " probe: 0 draws | 0 records | keys []"
    );
    let frame = render(&mut scene, &snapshot, probe, 10.0);
    assert_eq!(
        (scene.visible, scene.collapsed),
        (16, 16),
        "the processes the module did not show are collapsed"
    );
    assert_eq!(scene.positions.len(), 16);
    assert!(frame.picks.iter().any(|&pick| pick != NONE));
    assert_eq!(frame.sky.backdrop, Backdrop::Space, "the row's backdrop");
    scene.record(&snapshot, 10.0);
    assert_eq!(
        scene.legend(probe, &snapshot),
        " probe: 1 draws | 1 records | keys []"
    );
    assert_eq!(scene.inspector_line(probe), None);
    assert_eq!(scene.inspector_line(quiet).as_deref(), Some(" > quiet"));
    render(&mut scene, &snapshot, quiet, 10.1);
    assert_eq!(scene.positions.len(), 16, "each module keeps its own state");
    assert_eq!(
        scene.legend(quiet, &snapshot),
        " probe: 1 draws | 1 records | keys []"
    );
}

#[test]
fn a_modules_tests_can_read_its_state_by_type() {
    let snapshot = demo(12.0, 16);
    let camera = Camera::default();
    let mut scene = Scene::with_rows(testing::ROWS);
    let (probe, quiet) = (View::at(0), View::at(1));
    scene.render(&RenderInput::small(&snapshot, probe, &camera, 1.0));
    assert!(scene.module::<testing::Probe>(probe).is_some());
    assert!(
        scene.module::<testing::Quiet>(probe).is_none(),
        "the wrong type"
    );
    assert!(
        scene.module::<testing::Probe>(quiet).is_none(),
        "a view that shows nothing of itself"
    );
}

#[test]
fn a_view_that_draws_its_own_links_learns_the_link_mode_and_the_highlighted() {
    let snapshot = demo(12.0, 16);
    let camera = Camera::default();
    let mut scene = Scene::with_rows(testing::LINKER);
    let view = View::at(0);
    let legend = |scene: &mut Scene, links: Links, highlight: Vec<Identity>| {
        scene.links = links;
        scene.highlight = highlight;
        scene.render(&RenderInput::small(&snapshot, view, &camera, 1.0));
        scene.legend(view, &snapshot)
    };
    assert_eq!(
        legend(&mut scene, Links::All, Vec::new()),
        " linker: links All | highlight []"
    );
    let pids: Vec<u32> = snapshot
        .processes
        .iter()
        .take(2)
        .map(|p| p.id.pid)
        .collect();
    let hot = snapshot.processes.iter().take(2).map(|p| p.id).collect();
    assert_eq!(
        legend(&mut scene, Links::Focused, hot),
        format!(" linker: links Focused | highlight {pids:?}")
    );
    assert_eq!(
        legend(&mut scene, Links::Off, Vec::new()),
        " linker: links Off | highlight []"
    );
}

#[test]
fn rows_switch_the_haze_the_exit_flash_and_the_links_off() {
    let snapshot = demo(10.0, 32);
    let mut fewer = snapshot.clone();
    fewer.processes.truncate(8);
    let count = |frame: &Frame, beams: bool| {
        frame
            .items
            .iter()
            .filter(|item| match item {
                Item::Beam(..) => beams,
                Item::Glow { .. } => !beams,
                _ => false,
            })
            .count()
    };
    // Each row draws the full machine, then loses most of it, with every process highlighted.
    let draw = |view: View| {
        let mut scene = Scene::with_rows(testing::ROWS);
        scene.highlight = snapshot.processes.iter().map(|p| p.id).collect();
        let full = render(&mut scene, &snapshot, view, 10.0);
        let exits = render(&mut scene, &fewer, view, 10.1);
        (full.sky.haze, count(&full, true), count(&exits, false))
    };
    let (haze, beams, flashes) = draw(View::at(0));
    assert!(haze.iter().any(|&channel| channel > 0.0), "{haze:?}");
    assert!(beams > 0, "highlighted links are drawn");
    assert!(flashes > 0, "exits flash");
    assert_eq!(draw(View::at(1)), ([0.0; 3], 0, 0));
}

/// A view's state lives in its module, so `Scene` holds only frame-wide state, and a field
/// added to it is a decision someone reviews. The pattern below names every field of `Scene`
/// and has no `..`: a new field does not compile until it is listed here, and it belongs
/// here only if it is frame-wide (see the doc of `Scene`). The compiler reads the struct, so
/// this holds for any layout of it (multi-line types, attributes, comments), unlike a scan
/// of the source for field names, which a field named after no view slips past. The doc
/// comment each field needs, which is where the struct says why it is frame-wide, is the
/// `missing_docs_in_private_items` lint denied on `Scene`.
#[test]
fn scene_has_no_view_specific_fields() {
    let scene = Scene::new();
    let Scene {
        resources: _,
        frames: _,
        last_time: _,
        clock: _,
        births: _,
        arrivals: _,
        previous: _,
        deaths: _,
        headroom: _,
        splashes: _,
        newborn: _,
        started: _,
        intro: _,
        links: _,
        highlight: _,
        positions: _,
        places: _,
        titles: _,
        bounds: _,
        notes: _,
        sparklines: _,
        tour: _,
        popup: _,
        callout: _,
        history: _,
        rows: _,
        modules: _,
        spare: _,
        visible: _,
        collapsed: _,
    } = scene;
}

#[test]
fn record_hooks_run_for_every_view_whichever_is_shown() {
    let (probe, quiet) = (View::at(0), View::at(1));
    let mut scene = Scene::with_rows(testing::ROWS);
    for step in 0..3 {
        let snapshot = demo(step as f64, 16);
        scene.record(&snapshot, step as f32);
        render(&mut scene, &snapshot, probe, step as f32);
    }
    let snapshot = demo(3.0, 16);
    assert_eq!(
        scene.legend(probe, &snapshot),
        " probe: 3 draws | 3 records | keys []"
    );
    assert_eq!(
        scene.legend(quiet, &snapshot),
        " probe: 0 draws | 3 records | keys []",
        "a hidden view records too"
    );
}

#[test]
fn history_records_while_another_view_is_shown() {
    let mut scene = Scene::new();
    let mut snapshot = demo(0.0, 16);
    // Samples the main loop records, and a render of a sample it never recorded.
    for step in 0..3 {
        snapshot.elapsed = step as f64;
        scene.record(&snapshot, step as f32);
        render(&mut scene, &snapshot, view("city"), step as f32);
    }
    snapshot.elapsed = 3.0;
    render(&mut scene, &snapshot, view("city"), 3.0);
    let elapsed: Vec<f64> = scene.history.window(3.0, 60.0).map(|s| s.elapsed).collect();
    assert_eq!(elapsed, [0.0, 1.0, 2.0, 3.0]);
    let id = snapshot.processes[0].id;
    let series = scene.history.series(id, 3.0, 4);
    assert_eq!(series.len(), 4);
    assert!(series.iter().all(|reading| reading.is_some()));
    assert_eq!(scene.history.arrived(), 3.0);
}

#[test]
fn both_scenes_render_and_focus_preserves_descendants() {
    let snapshot = demo(1.0, 128);
    let branch = snapshot.processes[16].id;
    let set = descendants(&snapshot, branch);
    assert_eq!(set.len(), 16);
    for view in [view("city"), view("orbit")] {
        let mut scene = Scene::new();
        let mut frame = scene.render(&RenderInput {
            selected: Some(branch),
            focus: Some(branch),
            ..RenderInput::small(&snapshot, view, &Camera::default(), 1.0)
        });
        frame.rasterize();
        assert_eq!(frame.pixels.len(), 320 * 180 * 3);
        assert_eq!(scene.visible, 16);
    }
}

#[test]
fn long_names_are_cut_to_fifteen_cells_with_an_ascii_ellipsis() {
    let cut = label_name("BatteriesAvocadoWidgetExtension");
    assert_eq!(cut, "BatteriesAvoc..");
    assert_eq!(cut.chars().count(), 15);
    assert_eq!(label_name("fifteen_chars_ok"), "fifteen_chars..");
    assert_eq!(label_name("exactly15chars!"), "exactly15chars!");
    assert_eq!(label_name("kworker/u16:2-e"), "kworker/u16:2-e");
    assert_eq!(label_name("systemd"), "systemd");
    assert_eq!(label_name(""), "");
}

#[test]
fn labels_count_characters_not_bytes() {
    let name = "\u{00e9}".repeat(16);
    let cut = label_name(&name);
    assert_eq!(cut.chars().count(), 15);
    assert!(cut.ends_with(".."));
    assert_eq!(label_name(&"\u{00e9}".repeat(15)).chars().count(), 15);
}

#[test]
fn apples_reverse_dns_prefix_is_dropped_from_labels() {
    assert_eq!(label_name("com.apple.dock"), "dock");
    assert_eq!(
        label_name("com.apple.accessibility.mediaac"),
        "accessibility.."
    );
    assert_eq!(
        label_name("com.apple.dock.external.extra.a"),
        "dock.external.."
    );
    assert_eq!(label_name("com.apple."), "com.apple.");
    assert_eq!(
        without_apple_prefix("com.apple.accessibility.mediaac"),
        "accessibility.mediaac"
    );
    assert_eq!(
        without_apple_prefix("user@1000.service"),
        "user@1000.service"
    );
}

#[test]
fn other_vendors_reverse_dns_names_stay_whole_up_to_the_cut() {
    assert_eq!(label_name("org.mozilla.fx"), "org.mozilla.fx");
    assert_eq!(label_name("com.google.Chrome.helper"), "com.google.Ch..");
}

#[test]
fn cut_labels_survive_terminal_text_cleaning_unchanged() {
    for name in [
        "BatteriesAvocadoWidgetExtension",
        "com.apple.accessibility.mediaac",
        "com.google.Chrome.helper",
        "systemd",
    ] {
        let label = label_name(name);
        let shown = crate::terminal::clean_text(&label, 64);
        assert_eq!(
            shown, label,
            "{name} lost characters on the way to the terminal"
        );
        assert!(!shown.contains('?'), "{shown}");
        assert!(shown.chars().count() <= LABEL_NAME_CELLS);
    }
}
