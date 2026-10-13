//! Ouija: the newest line of the system journal, spelled one letter at a time by the planchette
//! of a talking board seen at an angle by candlelight. An error, or the end of a failed job,
//! sends the planchette to NO before it spells; a finished job, to YES. It trembles with CPU
//! pressure, and when the app is quit from the keyboard it glides to GOODBYE first.
//!
//! The view draws no process. The journal reaches it on the sample (`Snapshot::journal`), a
//! line at a time; `seance` queues the lines and moves the planchette, `layout` holds the board's
//! fixed geometry, `scenery` bakes the wood and the velvet into fields, and `paint`, `props` and
//! `ribbon` draw the ink, the candles and planchette, and the decoding line below.

mod layout;
mod paint;
mod props;
mod ribbon;
mod scenery;
mod seance;

use crate::journal::Delivery;
use crate::model::{Process, Snapshot};
use crate::platform::Source;
use crate::render::Stage;
use crate::status;
use crate::views::{ViewKey, ViewModule};

use layout::{HALF_HEIGHT, Mark, Plane, THICKNESS};
use paint::Ink;
use scenery::{CANDLES, FLAME_LIFT, Scenery};
use seance::{Farewell, Seance, jitter};

/// The planchette's slow sway, in radians, and how fast it turns: decoration.
const SWAY: f32 = 0.035;
/// How far the camera looks down on the board: steeper than isometric, so the letters read.
pub const PITCH: f32 = 0.9;
/// How far below the board the camera fit keeps clear for the ribbon, in board widths.
const RIBBON_ROOM: f32 = 0.3;

/// The talking board.
#[derive(Default)]
pub struct Ouija {
    seance: Seance,
    farewell: Farewell,
    scenery: Option<Scenery>,
    /// Journal lines taken in so far, and the follower's drops counted, which is how `take` tells
    /// a delivery it has seen from one it has not.
    taken: u64,
    counted: u64,
    /// Whether the first lines have arrived since the view was shown. They are the follower's
    /// backlog (`journalctl --lines=200`, or whatever gathered while another view was shown), so
    /// only the newest is spelled and none is counted as skipped.
    heard: bool,
    /// The `Stage::frames` of the last frame drawn, which tells a view hidden in between.
    drawn: Option<u64>,
}

impl Ouija {
    /// Whether the view was not drawn in the frame before this one (it has just been shown), and
    /// notes this frame.
    fn resumed(&mut self, frames: u64) -> bool {
        let continuing = self.drawn.is_some_and(|last| last + 1 == frames);
        self.drawn = Some(frames);
        !continuing
    }

    /// Takes in the lines of a sample's `Delivery` that earlier draws did not: each journal line is
    /// queued once however often a sample is drawn. The first lines to arrive are the backlog,
    /// of which only the newest is queued.
    fn take(&mut self, delivery: &Delivery) {
        let fresh = delivery.unseen(self.taken);
        if self.heard {
            for line in fresh {
                self.seance.push(line);
            }
            self.seance
                .skip(delivery.dropped.saturating_sub(self.counted));
        } else if let Some(newest) = fresh.last() {
            self.heard = true;
            self.seance.push(newest);
        }
        self.taken = self.taken.max(delivery.through);
        self.counted = self.counted.max(delivery.dropped);
    }

    /// How far the planchette trembles at `time`, in board widths: with the CPU stall percentage
    /// of the sample, and not at all on a machine that does not measure one.
    fn tremor(snapshot: &Snapshot, time: f32) -> layout::Spot {
        if snapshot.missing.contains(Source::CpuPressure) {
            [0.0; 2]
        } else {
            jitter(time, snapshot.pressure[0])
        }
    }

    /// Records the frame: the table, the board and its ink, the planchette, the candles and the
    /// ribbon, and the room the camera fit must keep.
    fn paint(&mut self, stage: &mut Stage, snapshot: &Snapshot) {
        let plane = Plane::new();
        let flicker = scenery::flicker(stage.time);
        let scenery = self.scenery.get_or_insert_with(Scenery::new);
        scenery.relight(flicker);
        let (camera, time) = (stage.camera, stage.time);
        let frame = &mut *stage.frame;
        scenery.draw(frame, camera, &plane);
        props::board_edge(frame, camera, &plane);
        let (at, lit) = match self.farewell.position() {
            Some(at) => (at, self.farewell.arrived().then_some(Mark::Goodbye)),
            None => (self.seance.position(), self.seance.lit()),
        };
        let shake = Self::tremor(snapshot, time);
        let at = [at[0] + shake[0], at[1] + shake[1]];
        let mut ink = Ink {
            frame: &mut *frame,
            camera,
            plane: &plane,
            flicker,
        };
        ink.lettering(lit);
        ink.ornaments();
        let sway = SWAY * (time * 1.7).sin();
        props::planchette(frame, camera, &plane, (at, sway), (flicker, lit.is_some()));
        props::candles(frame, camera, &plane, flicker, time);
        let nearest = plane
            .corners([1.65, 1.05], -THICKNESS)
            .into_iter()
            .map(|corner| frame.project(camera, corner)[2])
            .fold(f32::NEG_INFINITY, f32::max);
        ribbon::draw(frame, self.seance.reading(), nearest + 5.0, time);
        stage.bounds.extend(Self::envelope(&plane));
    }

    /// The points the camera fit must include: the board's corners, the candles with their
    /// flames, and a strip below the board for the ribbon.
    fn envelope(plane: &Plane) -> Vec<[f32; 3]> {
        let mut points: Vec<[f32; 3]> = plane.corners([0.5, HALF_HEIGHT], 0.0).to_vec();
        for candle in CANDLES {
            points.push(plane.world(candle.spot, -THICKNESS));
            points.push(plane.world(candle.spot, -THICKNESS + candle.height + FLAME_LIFT + 0.05));
        }
        points.push(plane.world([0.0, -HALF_HEIGHT - RIBBON_ROOM], 0.0));
        points
    }
}

impl ViewModule for Ouija {
    /// Takes in the journal lines the sample brings, steps the planchette (and the farewell, if
    /// one is playing) and draws the board. It draws no processes, so none count as shown. When
    /// a farewell has played out it ends the app through `Stage::quit`.
    fn draw(&mut self, stage: &mut Stage, _processes: &[&Process], snapshot: &Snapshot) -> usize {
        if self.resumed(stage.frames) {
            // A farewell left half played, and a journal that gathered lines while hidden.
            self.farewell.cancel();
            self.heard = false;
        }
        self.take(&snapshot.journal);
        self.seance.advance(stage.dt);
        if self.farewell.advance(stage.dt) {
            *stage.quit = true;
        }
        self.paint(stage, snapshot);
        0
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let skipped = match self.seance.skipped() {
            0 => String::new(),
            skipped => format!(", {skipped} skipped"),
        };
        let notes = [
            status::missing_note(snapshot, Source::CpuPressure, "planchette steady"),
            status::missing_note(snapshot, Source::JobResults, "no visits to YES"),
        ];
        format!(
            " Planchette = newest journal line, letter by letter | NO = error or failed job | YES = job done | jitter = cpu pressure | {} queued{skipped}{}",
            self.seance.queued(),
            notes.into_iter().flatten().collect::<String>()
        )
    }

    /// The newest journal line, in place of the inspector: the board draws no processes.
    fn inspector_line(&self) -> Option<String> {
        Some(format!(
            " > {}",
            self.seance.newest().unwrap_or("waiting for the journal...")
        ))
    }

    /// `q` starts the glide to GOODBYE and holds the quit until it ends; `q` again, during it,
    /// quits at once. No other key is the view's.
    fn key(&mut self, key: ViewKey) -> bool {
        key == ViewKey::Quit && self.farewell.press(self.seance.position())
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;
    use crate::app::fixtures::{all_available, app_over, options, press};
    use crate::headless::warm_up;
    use crate::journal::{JobResult, Line, Totals};
    use crate::platform::Sources;
    use crate::render::{Backdrop, Camera, Frame, Item, RenderInput, Scene};
    use crate::views::testing::Probe;
    use crate::views::{View, ViewInfo, view};

    /// The board alone, and the board with a probe to switch away to.
    static ROWS: &[ViewInfo] = &[ViewInfo::new("ouija", "OUIJA", Backdrop::Void, || {
        Box::new(Ouija::default())
    })];
    static WITH_PROBE: &[ViewInfo] = &[
        ViewInfo::new("ouija", "OUIJA", Backdrop::Void, || {
            Box::new(Ouija::default())
        }),
        ViewInfo::new("probe", "PROBE", Backdrop::Space, || {
            Box::new(Probe::default())
        }),
    ];

    /// A sample that carries `lines`, numbered on from `totals`.
    fn sample(totals: &mut Totals, lines: Vec<Line>) -> Snapshot {
        let mut snapshot = Snapshot::default();
        snapshot.journal.add(totals, lines, 0);
        snapshot
    }

    fn lines(count: usize) -> Vec<Line> {
        (0..count)
            .map(|number| Line::new(6, format!("line {number}")))
            .collect()
    }

    fn draw(scene: &mut Scene, shown: usize, snapshot: &Snapshot, time: f32) -> Frame {
        let camera = Camera::default();
        let input = RenderInput::small(snapshot, View::at(shown), &camera, time);
        scene.render(&input)
    }

    fn board(scene: &Scene) -> &Ouija {
        scene
            .module::<Ouija>(View::at(0))
            .expect("the board's state")
    }

    #[test]
    fn the_startup_backlog_keeps_only_the_newest_line_and_counts_no_skips() {
        let mut scene = Scene::with_rows(ROWS);
        let mut totals = Totals::default();
        let mut backlog = sample(&mut totals, lines(200));
        backlog.journal.dropped = 7;
        draw(&mut scene, 0, &backlog, 1.0);
        let seance = &board(&scene).seance;
        assert_eq!(seance.reading().expect("a line").entry.text, "line 199");
        assert_eq!((seance.queued(), seance.skipped()), (0, 0));
        // Everything after the backlog is spelled in its turn, and drops that follow it count.
        let mut next = sample(&mut totals, lines(3));
        next.journal.dropped = 9;
        draw(&mut scene, 0, &next, 1.1);
        draw(&mut scene, 0, &next, 1.2);
        let seance = &board(&scene).seance;
        assert_eq!(
            seance.queued(),
            3,
            "each line is taken in once however often it is drawn"
        );
        assert_eq!(
            seance.skipped(),
            2,
            "two more were dropped after the backlog"
        );
        assert_eq!(seance.newest(), Some("line 2"));
        let legend = scene.legend(View::at(0), &Snapshot::default());
        assert!(legend.contains("| 3 queued, 2 skipped"), "{legend}");
    }

    #[test]
    fn a_view_shown_again_takes_what_gathered_meanwhile_as_a_backlog_and_drops_a_half_played_farewell()
     {
        let mut scene = Scene::with_rows(WITH_PROBE);
        let mut totals = Totals::default();
        let first = sample(&mut totals, lines(1));
        draw(&mut scene, 0, &first, 1.0);
        draw(&mut scene, 0, &first, 1.1);
        assert!(
            scene.key(View::at(0), ViewKey::Quit),
            "the first q takes the quit"
        );
        draw(&mut scene, 0, &first, 1.2);
        // Another view is shown for a while, and the follower's lines gather.
        let empty = Snapshot::default();
        for step in 0..5 {
            draw(&mut scene, 1, &empty, 2.0 + step as f32 * 0.1);
        }
        let gathered = sample(&mut totals, lines(50));
        draw(&mut scene, 0, &gathered, 3.0);
        let seance = &board(&scene).seance;
        assert_eq!(seance.newest(), Some("line 49"), "the newest of the lot");
        assert_eq!(
            (seance.queued(), seance.skipped()),
            (1, 0),
            "it waits behind the line being spelled, and the other 49 are not counted"
        );
        assert!(
            scene.key(View::at(0), ViewKey::Quit),
            "the farewell left half played is over: this q starts it afresh"
        );
        let mut fresh = Ouija::default();
        assert!(fresh.resumed(1) && !fresh.resumed(2) && !fresh.resumed(3));
        assert!(fresh.resumed(9), "a frame that does not follow the last");
    }

    #[test]
    fn goodbye_is_skipped_by_a_second_q_and_never_runs_without_a_terminal() {
        // No key is pressed in a headless run, a benchmark or a recorded frame, and a board that
        // is only drawn never starts its farewell, however long it runs.
        let mut scene = Scene::with_rows(ROWS);
        let mut totals = Totals::default();
        let busy = sample(&mut totals, lines(30));
        for frame in 0..400 {
            draw(&mut scene, 0, &busy, 1.0 + frame as f32 * 0.25);
            assert!(!scene.quit, "frame {frame} ended the app by itself");
        }
        assert!(board(&scene).farewell.position().is_none());

        // The first q glides to GOODBYE for a second and then ends the app.
        let mut scene = Scene::with_rows(ROWS);
        draw(&mut scene, 0, &busy, 1.0);
        assert!(scene.key(View::at(0), ViewKey::Quit));
        let mut frames = 0;
        while !scene.quit {
            frames += 1;
            draw(&mut scene, 0, &busy, 1.0 + frames as f32 * 0.1);
            assert!(frames < 20, "the farewell never ended");
        }
        assert!(
            (10..=11).contains(&frames),
            "{frames} frames of a tenth of a second"
        );
        // A second q ends it at once; no other key is the board's.
        let mut scene = Scene::with_rows(ROWS);
        draw(&mut scene, 0, &busy, 1.0);
        assert!(scene.key(View::at(0), ViewKey::Quit));
        draw(&mut scene, 0, &busy, 1.1);
        assert!(!scene.key(View::at(0), ViewKey::Quit));
        assert!(!scene.key(View::at(0), ViewKey::Escape));
        assert!(!scene.key(View::at(0), ViewKey::Reset));
    }

    #[test]
    fn a_quit_in_the_app_waits_for_the_glide_and_a_second_q_or_ctrl_c_does_not() {
        let mut app = app_over(ROWS);
        assert!(
            !press(&mut app, KeyCode::Char('q')),
            "the board takes the quit"
        );
        assert!(app.quitting && !app.farewell_over());
        for _ in 0..9 {
            app.animation += 0.1;
            app.render(320, 180, 512);
            assert!(!app.farewell_over());
        }
        for _ in 0..2 {
            app.animation += 0.1;
            app.render(320, 180, 512);
        }
        assert!(app.farewell_over(), "the glide has landed on GOODBYE");
        let mut again = app_over(ROWS);
        press(&mut again, KeyCode::Char('q'));
        assert!(
            press(&mut again, KeyCode::Char('q')),
            "a second q quits at once"
        );
        let mut hurried = app_over(ROWS);
        press(&mut hurried, KeyCode::Char('q'));
        assert!(
            hurried.key(KeyCode::Char('c'), KeyModifiers::CONTROL, 10.0),
            "so does Ctrl-C"
        );
    }

    #[test]
    fn a_headless_run_of_the_registered_row_plays_no_farewell() {
        let ouija = view("ouija");
        let options = options(&[
            "--demo",
            "--output",
            "/dev/null",
            "--time",
            "12",
            "--processes",
            "16",
            "--width",
            "320",
        ]);
        let snapshot = crate::demo_snapshot(ouija.info(), 11.0, 12.0, 16);
        let mut app = crate::app::App::new(ouija, snapshot, all_available());
        warm_up(&mut app, &options, None);
        let frame = app.render(320, 180, 512);
        assert!(!frame.items.is_empty());
        assert!(!app.scene.quit && !app.quitting);
        let board = app.scene.module::<Ouija>(ouija).expect("the board");
        assert!(board.farewell.position().is_none());
        assert!(
            board.seance.reading().is_some(),
            "the warm-up fed the journal, and the frame lands in the middle of a line"
        );
    }

    #[test]
    fn jitter_follows_cpu_pressure_and_stops_when_it_is_unreadable() {
        let mut snapshot = Snapshot {
            pressure: [40.0, 0.0, 0.0],
            ..Snapshot::default()
        };
        let shaken = Ouija::tremor(&snapshot, 1.3);
        assert!(shaken != [0.0; 2]);
        assert!(shaken[0].hypot(shaken[1]) <= 0.025 * 0.4 * 1.5);
        snapshot.pressure[0] = 80.0;
        let twice = Ouija::tremor(&snapshot, 1.3);
        assert!(
            (twice[0] - 2.0 * shaken[0]).abs() < 1e-6 && (twice[1] - 2.0 * shaken[1]).abs() < 1e-6
        );
        snapshot.missing = Sources::from(Source::CpuPressure);
        assert_eq!(
            Ouija::tremor(&snapshot, 1.3),
            [0.0; 2],
            "the machine does not measure it"
        );
        snapshot.missing = Sources::default();
        snapshot.pressure[0] = 0.0;
        assert_eq!(Ouija::tremor(&snapshot, 1.3), [0.0; 2], "a calm machine");
    }

    #[test]
    fn the_legend_counts_the_queue_and_names_what_the_machine_lacks() {
        let mut scene = Scene::with_rows(ROWS);
        let mut totals = Totals::default();
        let plain = " Planchette = newest journal line, letter by letter | NO = error or failed job | YES = job done | jitter = cpu pressure | 0 queued";
        assert_eq!(scene.legend(View::at(0), &Snapshot::default()), plain);
        assert_eq!(
            scene.inspector_line(View::at(0)).as_deref(),
            Some(" > waiting for the journal...")
        );
        draw(&mut scene, 0, &sample(&mut totals, lines(1)), 1.0);
        draw(&mut scene, 0, &sample(&mut totals, lines(20)), 1.1);
        let gaps = Snapshot {
            missing: Sources::from([Source::CpuPressure, Source::JobResults]),
            ..Snapshot::default()
        };
        let counted = plain.replace("| 0 queued", "| 8 queued, 12 skipped");
        assert_eq!(
            scene.legend(View::at(0), &gaps),
            format!(
                "{counted} | no cpu pressure: planchette steady | no job results: no visits to YES"
            )
        );
        assert_eq!(
            scene.inspector_line(View::at(0)).as_deref(),
            Some(" > line 19"),
            "the newest line, spelled yet or not"
        );
    }

    #[test]
    fn the_board_keeps_every_letter_in_place_whatever_the_journal_or_machine_does() {
        let fields = |frame: &Frame| -> Vec<String> {
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Field { .. }))
                .map(|item| format!("{item:?}"))
                .collect()
        };
        let mut calm = Scene::with_rows(ROWS);
        let calm_frame = draw(&mut calm, 0, &Snapshot::default(), 5.0);
        let mut busy = Scene::with_rows(ROWS);
        let mut totals = Totals::default();
        let mut crowded = crate::model::demo(5.0, 200);
        crowded.pressure = [90.0, 40.0, 40.0];
        crowded.journal.add(&mut totals, lines(300), 4);
        let busy_frame = draw(&mut busy, 0, &crowded, 5.0);
        assert_eq!(
            fields(&calm_frame),
            fields(&busy_frame),
            "the table and the board are where they were"
        );
        let (mut a, mut b) = (Camera::default(), Camera::default());
        calm.fit(&mut a, 320, 180);
        busy.fit(&mut b, 320, 180);
        assert!(a == b, "the camera fit does not follow the journal");
        let placed = || {
            layout::letters()
                .map(|placed| (placed.character, placed.at))
                .collect::<Vec<_>>()
        };
        let before = placed();
        for step in 0..100 {
            draw(&mut busy, 0, &crowded, 6.0 + step as f32 * 0.1);
        }
        assert_eq!(placed(), before);
    }

    #[test]
    fn ouija_renders_the_same_frame_twice_from_the_same_inputs() {
        let render = || {
            let mut scene = Scene::with_rows(ROWS);
            let mut totals = Totals::default();
            let first = sample(
                &mut totals,
                vec![Line::new(3, "kernel: nvme0: I/O 42 QID 6 timeout")],
            );
            let mut frame = draw(&mut scene, 0, &first, 2.0);
            for step in 1..30 {
                frame = draw(&mut scene, 0, &first, 2.0 + step as f32 * 0.1);
            }
            frame.rasterize();
            (frame.pixels, frame.picks)
        };
        let (pixels, picks) = render();
        assert_eq!((pixels.clone(), picks.clone()), render());
        assert!(pixels.iter().any(|&byte| byte > 0));
        assert!(
            picks.iter().all(|&pick| pick == crate::render::NONE),
            "nothing on the board is pickable"
        );
    }

    #[test]
    fn a_failed_job_sends_the_planchette_to_no_and_a_finished_one_to_yes_on_the_screen() {
        for (job, mark, priority) in [
            (JobResult::Failed, Mark::No, 3),
            (JobResult::Done, Mark::Yes, 6),
        ] {
            let mut scene = Scene::with_rows(ROWS);
            let mut totals = Totals::default();
            let line = Line::new(priority, "systemd[1]: job ended").with_job(Some(job));
            let first = sample(&mut totals, vec![line]);
            for step in 0..14 {
                draw(&mut scene, 0, &first, 1.0 + step as f32 * 0.04);
            }
            assert_eq!(board(&scene).seance.lit(), Some(mark), "{job:?}");
        }
    }
}
