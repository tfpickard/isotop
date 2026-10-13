use std::cmp::Reverse;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::model::{Identity, Process};
use crate::render::Point;

use super::App;

/// How long a stop of the tour lasts.
pub(super) const TOUR_STEP: Duration = Duration::from_secs(6);

/// The process the camera is visiting, by the tour or by hand.
pub(crate) struct Tour {
    pub(crate) target: Identity,
    pub(crate) since: Instant,
    /// Whether the tour steps on by itself; a visit made by hand with no tour running holds still.
    pub(crate) running: bool,
}

/// Which process of the cycle a visit goes to, from the cursor.
#[derive(Clone, Copy)]
pub(crate) enum Step {
    /// The one the cursor is on, or the first before any visit.
    Stay,
    /// The one after it.
    Next,
    /// Any other one.
    Random,
}

/// Where the last visit stood in the cycle. It follows the process, not the index, and outlives
/// the visit, so the next Space, `g` and the idle tour all carry on from it.
pub(crate) struct Cursor {
    pub(crate) index: usize,
    pub(crate) id: Identity,
}

impl App {
    /// Lets a held globe turn with time again from where it stands, and stands it upright.
    /// Does nothing unless the earth was held, or in a view with no sphere to turn, where a
    /// tour must leave the hold for the next visit to the globe.
    pub(super) fn release_globe(&mut self) {
        if self.row().turn.is_none() {
            return;
        }
        let time = self.render_time();
        self.both(|c| c.release(time));
        self.goal.globe_tilt = 0.0;
    }

    /// Notable processes to visit while idle: the view's own targets (a module view's
    /// `Stage::tour`, which leaves out what has no callout to show, such as the
    /// hub of an Orbit system), heaviest first, then the busiest and the largest.
    pub(super) fn tour_targets(&self) -> Vec<Identity> {
        let visible: Vec<&Process> = self
            .snapshot()
            .processes
            .iter()
            .filter(|p| self.scene.positions.contains_key(&p.id))
            .collect();
        let mut busiest = visible.clone();
        busiest.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        let mut largest = visible;
        largest.sort_by_key(|p| Reverse(p.memory));
        let mut stars: Vec<(Identity, f32)> = self
            .scene
            .tour
            .iter()
            .map(|target| (target.id, target.weight))
            .collect();
        stars.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut targets = Vec::new();
        for i in 0..6 {
            let picks = [
                stars.get(i).map(|s| s.0),
                busiest.get(i).map(|p| p.id),
                largest.get(i).map(|p| p.id),
            ];
            for id in picks.into_iter().flatten() {
                if !targets.contains(&id) {
                    targets.push(id);
                }
            }
        }
        targets
    }

    /// The processes Space steps through and the tour walks: `tour_targets`, then every other
    /// visible process by ascending identity, so each appears once.
    pub(super) fn visit_cycle(&self) -> Vec<Identity> {
        let mut cycle = self.tour_targets();
        let listed: HashSet<Identity> = cycle.iter().copied().collect();
        let mut rest: Vec<Identity> = self
            .snapshot()
            .processes
            .iter()
            .map(|p| p.id)
            .filter(|id| self.scene.positions.contains_key(id) && !listed.contains(id))
            .collect();
        rest.sort_unstable();
        cycle.extend(rest);
        cycle
    }

    /// Where the cursor stands in `cycle`, which is not empty: at the process it was left on, or
    /// at its old index, clamped, once that process is gone. None before the first visit.
    pub(super) fn cursor_position(&self, cycle: &[Identity]) -> Option<usize> {
        let cursor = self.cursor.as_ref()?;
        Some(
            cycle
                .iter()
                .position(|id| *id == cursor.id)
                .unwrap_or_else(|| cursor.index.min(cycle.len() - 1)),
        )
    }

    pub(crate) fn update_tour(&mut self, after: Option<Duration>) {
        if !self.row().tour {
            self.tour = None;
            return;
        }
        let now = Instant::now();
        let idle = || {
            after.is_some_and(|after| now.duration_since(self.last_input) >= after)
                // A globe held by hand stays where it was put until r or g.
                && !(self.row().turn.is_some() && self.goal.globe_held)
        };
        match &self.tour {
            Some(tour) if tour.running => {
                if now.duration_since(tour.since) >= TOUR_STEP
                    || self.tour_point(tour.target).is_none()
                {
                    self.visit(Step::Next, true);
                }
            }
            // A visit by hand stays on its process until input ends it or the idle tour takes over.
            Some(tour) => {
                if self.tour_point(tour.target).is_none() {
                    self.tour = None;
                } else if idle() {
                    self.visit(Step::Stay, true);
                }
            }
            None => {
                if idle() {
                    self.visit(Step::Stay, true);
                }
            }
        }
        if let Some(tour) = &self.tour
            && let Some(p) = self.tour_point(tour.target)
        {
            self.goal.center = [p[0], p[1]];
            self.release_globe();
            self.face(p);
        }
    }

    /// Where the tour shows `id`: the point the view gave with it as a tour target, or else
    /// where the process is drawn; None once it is neither.
    pub(super) fn tour_point(&self, id: Identity) -> Option<Point> {
        self.scene
            .tour
            .iter()
            .find(|target| target.id == id)
            .map(|target| target.position)
            .or_else(|| self.scene.positions.get(&id).copied())
    }

    /// In a view whose row can face a point, turns the camera so `point` is on the near side
    /// and centres the point itself. On the globe the world spins, so a process above home is
    /// behind the earth for half of every turn, and it floats far above the ground the camera
    /// otherwise centres on.
    ///
    /// The point was drawn by `rendered`, whose lean the camera is still easing away from, so
    /// on a sphere the wheel can lean the goal aims at where the point will be once the sphere
    /// stands upright.
    pub(super) fn face(&mut self, point: Point) {
        let Some(face) = self.row().face else {
            return;
        };
        let point = match self.row().turn {
            Some(turn) => {
                self.goal.globe_tilt = 0.0;
                (turn.upright)(point, &self.rendered)
            }
            None => point,
        };
        (self.goal.rotation, self.goal.pitch) = face(point, &self.goal);
        self.goal.look_at(point);
    }

    /// Visits the process `step` names in the cycle, as a tour step does: nothing selected, the
    /// camera zoomed in and facing it, and its callout shown. A `running` visit goes on by itself
    /// every `TOUR_STEP` from here; otherwise it holds still. Does nothing when no process can
    /// be visited.
    pub(super) fn visit(&mut self, step: Step, running: bool) {
        self.release_globe();
        let cycle = self.visit_cycle();
        if cycle.is_empty() {
            self.tour = None;
            return;
        }
        let count = cycle.len();
        let index = match (step, self.cursor_position(&cycle)) {
            (Step::Stay, Some(index)) => index,
            (Step::Next, Some(index)) => (index + 1) % count,
            // Another position, uniform among the others when there are any.
            (Step::Random, Some(index)) if count > 1 => {
                (index + 1 + self.rng.below(count - 1)) % count
            }
            (Step::Random, _) => self.rng.below(count),
            (Step::Stay | Step::Next, None) => 0,
        };
        self.cursor = Some(Cursor {
            index,
            id: cycle[index],
        });
        self.tour = Some(Tour {
            target: cycle[index],
            since: Instant::now(),
            running,
        });
        self.selected = None;
        self.thing = None;
        self.goal.zoom = self.fit_zoom * 2.2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;

    #[test]
    fn the_tour_passes_over_hubs_and_opens_on_a_process_with_a_callout() {
        let mut snapshot = model::demo(1.0, 20);
        snapshot.links.clear();
        for (index, process) in snapshot.processes.iter_mut().enumerate() {
            process.id = Identity {
                pid: 100 + index as u32,
                start: 1,
            };
            process.parent = 1;
        }
        snapshot.shadows = vec![platform::Shadow {
            pid: 1,
            parent: 0,
            name: "launchd".into(),
        }];
        let layout = Layout {
            columns: 200,
            rows: 56,
            width: 1600,
            height: 900,
            cell: None,
        };
        for view in [view("orbit"), view("flow")] {
            let mut app = App::new(view, snapshot.clone(), all_available());
            app.render(1600, 900, 512);
            assert!(
                app.scene
                    .titles
                    .iter()
                    .any(|title| title.text.contains("(unreadable)")),
                "{view:?} draws the hub"
            );
            let targets = app.tour_targets();
            assert!(!targets.is_empty());
            for target in &targets {
                assert!(
                    snapshot.processes.iter().any(|p| p.id == *target),
                    "{view:?} visits {target:?}"
                );
            }
            app.visit(Step::Stay, true);
            let mut frame = app.render(1600, 900, 512);
            frame.rasterize();
            let (_, panels) = app.overlay(&mut frame, &layout, layout.pick_radius(false));
            assert_eq!(panels.len(), 1, "{view:?} shows the first stop's callout");
        }
    }

    #[test]
    fn tour_key_toggles_a_tour_that_runs_even_with_the_idle_tour_off() {
        let mut app = App::new(view("orbit"), model::demo(1.0, 64), all_available());
        app.render(320, 180, 512);
        let press = |app: &mut App, c: char| app.key(KeyCode::Char(c), KeyModifiers::NONE, 10.0);
        press(&mut app, 'g');
        let target = app.tour.as_ref().expect("g starts the tour").target;
        app.update_tour(None);
        let at = app.scene.positions[&target];
        assert_eq!(app.goal.center, [at[0], at[1]]);
        let tour = app.tour.as_mut().unwrap();
        tour.since = tour.since.checked_sub(TOUR_STEP).unwrap();
        app.update_tour(None);
        assert_eq!(
            app.cursor.as_ref().unwrap().index,
            1,
            "steps without --tour"
        );
        press(&mut app, 'g');
        assert!(app.tour.is_none(), "g again ends it");
        press(&mut app, 'g');
        press(&mut app, '+');
        assert!(app.tour.is_none(), "any other key ends it");
    }

    fn cursor_index(app: &App) -> usize {
        app.cursor.as_ref().expect("a visit was made").index
    }

    fn visited(app: &App) -> Identity {
        app.tour.as_ref().expect("a visit is shown").target
    }

    fn press_with(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        app.key(code, modifiers, 10.0);
    }

    fn age_the_step(app: &mut App) {
        let tour = app.tour.as_mut().expect("a visit is shown");
        tour.since = tour.since.checked_sub(TOUR_STEP).unwrap();
    }

    #[test]
    fn the_cycle_lists_tour_targets_first_then_every_other_visible_process_once() {
        let app = visiting_app();
        let targets = app.tour_targets();
        let cycle = app.visit_cycle();
        assert_eq!(cycle[..targets.len()], targets[..]);
        let rest = &cycle[targets.len()..];
        assert!(rest.windows(2).all(|pair| pair[0] < pair[1]), "ascending");
        let mut drawn: Vec<Identity> = app
            .snapshot()
            .processes
            .iter()
            .map(|p| p.id)
            .filter(|id| app.scene.positions.contains_key(id))
            .collect();
        drawn.sort_unstable();
        let mut sorted = cycle.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, drawn, "every visible process, once");
        assert!(cycle.len() > targets.len());
    }

    #[test]
    fn space_advances_the_tour_without_stopping_it() {
        let mut app = visiting_app();
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(cursor_index(&app), 0);
        press(&mut app, KeyCode::Char(' '));
        let cycle = app.visit_cycle();
        assert_eq!(cursor_index(&app), 1);
        assert_eq!(visited(&app), cycle[1]);
        assert!(app.tour.as_ref().unwrap().running, "the tour goes on");
        assert!(app.text(0.0, "test", 20, 512)[0].contains("/ TOUR"));
        age_the_step(&mut app);
        app.update_tour(None);
        assert_eq!(cursor_index(&app), 2, "the next automatic step follows");
    }

    #[test]
    fn a_visit_centres_a_flock_that_flies_above_the_ground() {
        let row = view("murmuration").info();
        let snapshot = demo_snapshot(row, 11.0, 12.0, 64);
        let mut app = App::new(view("murmuration"), snapshot, all_available());
        app.rng = Rng::new(7);
        let frame = app.render(640, 360, 512);
        press(&mut app, KeyCode::Char('g'));
        app.update_tour(None);
        let flock = app
            .tour_point(visited(&app))
            .expect("the flock has a centroid");
        assert!(flock[2] > 5.0, "it flies above the ground: {flock:?}");
        // The point itself, not the ground below it, is where the camera looks.
        let goal = &app.goal;
        let middle = frame.project(goal, [goal.center[0], goal.center[1], 0.0]);
        let seen = frame.project(goal, flock);
        assert!(
            (seen[0] - middle[0]).abs() < 1.0 && (seen[1] - middle[1]).abs() < 1.0,
            "the flock is at {seen:?}, the middle of the screen at {middle:?}"
        );
    }

    #[test]
    fn the_press_of_a_modifier_key_does_not_end_the_tour_before_shift_space_arrives() {
        let mut app = visiting_app();
        press(&mut app, KeyCode::Char('g'));
        // A terminal that reports every key sends Shift on its own first.
        for modifier in [
            KeyCode::Modifier(event::ModifierKeyCode::LeftShift),
            KeyCode::CapsLock,
        ] {
            assert!(!press(&mut app, modifier));
            assert!(app.tour.as_ref().unwrap().running, "{modifier:?}");
        }
        assert!(!app.key(KeyCode::Char(' '), KeyModifiers::SHIFT, 1.0));
        assert!(
            app.tour.as_ref().unwrap().running,
            "the tour resumes from the jump"
        );
    }

    #[test]
    fn space_restarts_the_tour_step_timer() {
        let mut app = visiting_app();
        press(&mut app, KeyCode::Char('g'));
        let tour = app.tour.as_mut().unwrap();
        tour.since = tour.since.checked_sub(TOUR_STEP / 2).unwrap();
        press(&mut app, KeyCode::Char(' '));
        assert!(app.tour.as_ref().unwrap().since.elapsed() < TOUR_STEP / 4);
        app.update_tour(None);
        assert_eq!(cursor_index(&app), 1, "no step is due yet");
    }

    #[test]
    fn shift_space_jumps_to_a_random_other_position_and_the_tour_resumes_from_it() {
        let mut app = visiting_app();
        press(&mut app, KeyCode::Char('g'));
        for _ in 0..3 {
            press(&mut app, KeyCode::Char(' '));
        }
        let count = app.visit_cycle().len();
        let mut jumps = std::collections::BTreeSet::new();
        for _ in 0..20 {
            let before = cursor_index(&app);
            press_with(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
            let after = cursor_index(&app);
            assert_ne!(after, before, "never the current position");
            assert!(app.tour.as_ref().unwrap().running);
            jumps.insert(after);
            age_the_step(&mut app);
            app.update_tour(None);
            assert_eq!(cursor_index(&app), (after + 1) % count);
        }
        assert!(jumps.len() > 5, "the jumps vary: {jumps:?}");
    }

    #[test]
    fn space_after_a_random_jump_visits_the_next_position() {
        let mut app = visiting_app();
        let count = app.visit_cycle().len();
        for key in [KeyCode::Char('*'), KeyCode::Char(' ')] {
            press_with(&mut app, KeyCode::Char('*'), KeyModifiers::NONE);
            let jumped = cursor_index(&app);
            press(&mut app, key);
            if key == KeyCode::Char(' ') {
                assert_eq!(cursor_index(&app), (jumped + 1) % count);
                assert_eq!(visited(&app), app.visit_cycle()[(jumped + 1) % count]);
            }
        }
    }

    #[test]
    fn space_without_a_tour_visits_without_starting_one() {
        let mut app = visiting_app();
        press(&mut app, KeyCode::Char(' '));
        let first = visited(&app);
        assert_eq!(cursor_index(&app), 0);
        assert!(!app.tour.as_ref().unwrap().running);
        assert!(app.text(0.0, "test", 20, 512)[0].contains("/ LIVE"));
        assert!(app.selected.is_none());
        assert_eq!(app.goal.zoom, app.fit_zoom * 2.2);
        age_the_step(&mut app);
        app.update_tour(None);
        assert_eq!(visited(&app), first, "a visit by hand holds still");
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(cursor_index(&app), 1);
        press_with(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
        assert!(!app.tour.as_ref().unwrap().running);
        // Another key ends the visit but not the cursor, and `g` carries on from it.
        let at = cursor_index(&app);
        let id = visited(&app);
        press(&mut app, KeyCode::Char('+'));
        assert!(app.tour.is_none());
        press(&mut app, KeyCode::Char('g'));
        assert_eq!((cursor_index(&app), visited(&app)), (at, id));
        assert!(app.tour.as_ref().unwrap().running);
    }

    #[test]
    fn the_idle_tour_starts_from_the_cursor() {
        let mut app = visiting_app();
        for _ in 0..3 {
            press(&mut app, KeyCode::Char(' '));
        }
        let id = visited(&app);
        app.update_tour(Some(Duration::ZERO));
        assert!(app.tour.as_ref().unwrap().running);
        assert_eq!((cursor_index(&app), visited(&app)), (2, id));
    }

    #[test]
    fn the_cursor_follows_the_visited_process_when_the_cycle_changes() {
        let mut app = visiting_app();
        let targets = app.tour_targets().len();
        for _ in 0..targets + 3 {
            press(&mut app, KeyCode::Char(' '));
        }
        let id = visited(&app);
        assert_eq!(cursor_index(&app), targets + 2);
        // A process ahead of it that is no target exits, so it moves up one place.
        let ahead = app.visit_cycle()[targets];
        app.history[0].processes.retain(|p| p.id != ahead);
        app.render(320, 180, 512);
        let cycle = app.visit_cycle();
        let at = cycle.iter().position(|&other| other == id).unwrap();
        assert_eq!(at, targets + 1, "the visited process changed places");
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(visited(&app), cycle[at + 1]);
        // The visited process itself exits: the stored index, clamped, stands in for it.
        let gone = visited(&app);
        let index = cursor_index(&app);
        app.history[0].processes.retain(|p| p.id != gone);
        app.render(320, 180, 512);
        let cycle = app.visit_cycle();
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(visited(&app), cycle[(index + 1) % cycle.len()]);
    }

    #[test]
    fn space_does_nothing_with_no_visitable_process() {
        let mut app = visiting_app();
        app.history[0].processes.clear();
        app.render(320, 180, 512);
        assert!(app.visit_cycle().is_empty());
        for (code, modifiers) in [
            (KeyCode::Char(' '), KeyModifiers::NONE),
            (KeyCode::Char(' '), KeyModifiers::SHIFT),
            (KeyCode::Char('*'), KeyModifiers::NONE),
        ] {
            press_with(&mut app, code, modifiers);
            assert!(app.tour.is_none() && app.cursor.is_none());
        }
    }

    #[test]
    fn space_does_nothing_in_a_view_that_opts_out_of_the_tour() {
        let mut app = App::new(view("necropolis"), model::demo(1.0, 40), all_available());
        app.render(320, 180, 512);
        assert!(!app.row().tour);
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('*'));
        assert!(app.tour.is_none() && app.cursor.is_none());
    }

    #[test]
    fn p_pauses_and_space_no_longer_does() {
        let mut app = visiting_app();
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.paused);
        press_with(&mut app, KeyCode::Char('P'), KeyModifiers::SHIFT);
        assert!(app.paused);
        assert!(app.text(0.0, "test", 20, 512)[0].contains("/ PAUSED"));
        press(&mut app, KeyCode::Char('P'));
        assert!(!app.paused);
        press(&mut app, KeyCode::Char('p'));
        assert!(!app.paused, "lowercase p belongs to the views");
    }

    #[test]
    fn space_still_toggles_the_playlist_inside_the_picker_and_types_in_search() {
        let mut app = visiting_app();
        press(&mut app, KeyCode::Char('v'));
        let before = playlist_names(&app).len();
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(playlist_names(&app).len(), before - 1);
        assert!(app.tour.is_none() && app.cursor.is_none());
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('*'));
        assert_eq!(app.search.as_deref(), Some("a *"));
        assert!(app.tour.is_none() && app.cursor.is_none());
    }

    #[test]
    fn globe_tour_turns_the_earth_so_its_target_faces_the_viewer() {
        // The world turns once in five minutes, so these times put home on every side of it.
        for time in [0.0, 75.0, 150.0, 225.0] {
            let mut app = App::new(view("globe"), model::demo(1.0, 64), all_available());
            app.animation = time;
            app.render(640, 360, 512);
            app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
            let target = app
                .tour
                .as_ref()
                .expect("the globe has processes to visit")
                .target;
            app.update_tour(None);
            app.camera = app.goal.clone();
            let frame = app.render(640, 360, 512);
            let at = frame
                .locate(&app.camera, app.scene.positions[&target])
                .expect("the target is on screen");
            assert_eq!(
                frame.pick_near(at[0], at[1], 3.0),
                Some(target),
                "the target is in front of the earth at {time} s"
            );
        }
    }

    #[test]
    fn optional_row_fields_set_the_tour_pitch_counts_and_inspector() {
        let mut app = probe_app();
        let press = |app: &mut App, code: KeyCode| app.key(code, KeyModifiers::NONE, 10.0);
        press(&mut app, KeyCode::Char('g'));
        assert!(app.tour.is_some(), "the probe takes part in the tour");
        let pitch = app.goal.pitch;
        app.view = View::at(1);
        app.update_tour(Some(Duration::ZERO));
        assert!(
            app.tour.is_none(),
            "showing a view without the tour ends it"
        );
        press(&mut app, KeyCode::Char('g'));
        assert!(app.tour.is_none());
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            (app.view, app.goal.pitch),
            (View::at(0), pitch),
            "no pitch of its own"
        );
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.goal.pitch, FRAC_PI_2, "switching sets the row's pitch");
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.goal.pitch, ISOMETRIC);
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.goal.pitch, FRAC_PI_2, "so does r");
        let rows = views::testing::ROWS;
        let started = App::with_scene(
            Scene::with_rows(rows),
            View::at(1),
            model::demo(1.0, 16),
            vec![true; rows.len()],
        );
        assert_eq!(started.goal.pitch, FRAC_PI_2, "and so does starting on it");
        app.render(320, 180, 512);
        app.selected = Some(app.snapshot().processes[0].id);
        let text = app.text(0.0, "test", 20, 512);
        assert!(
            text[0].contains(" | 7 visible | 3 collapsed | "),
            "{}",
            text[0]
        );
        assert_eq!(text[1], " > quiet");
        assert!(
            text[2].starts_with(" probe: "),
            "the legend, not the command line"
        );
        let layout = Layout {
            columns: 80,
            rows: 24,
            width: 320,
            height: 180,
            cell: None,
        };
        let frame = app.render(320, 180, 512);
        assert!(app.popup(&frame, &layout).is_none());
    }

    #[test]
    fn tour_targets_from_stage_outputs_reach_the_app() {
        let (mut app, last) = guided_app();
        assert_eq!(
            app.tour_targets().first(),
            Some(&last),
            "the view's target leads, though it is neither busy nor large"
        );
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        assert_eq!(app.tour.as_ref().map(|tour| tour.target), Some(last));
        app.update_tour(None);
        let seat = app.scene.positions[&last];
        assert_eq!(
            app.goal.center,
            [seat[0], 3.0],
            "the camera goes to the point the view gave, beside the seat"
        );
        app.view = View::at(1);
        app.render(320, 180, 512);
        assert!(app.scene.tour.is_empty());
        assert_ne!(
            app.tour_targets().first(),
            Some(&last),
            "a view that gives no targets leaves the busiest first"
        );
    }

    #[test]
    fn the_idle_tour_waits_while_the_globe_is_held() {
        let idle = Duration::from_secs(1);
        let long_ago = Instant::now().checked_sub(Duration::from_secs(60)).unwrap();
        let mut app = globe_at(0.0);
        app.last_input = long_ago;
        app.update_tour(Some(idle));
        assert!(app.tour.is_some(), "an idle free globe tours");
        let mut app = globe_at(0.0);
        scroll(&mut app, MouseEventKind::ScrollRight, 1);
        app.last_input = long_ago;
        app.update_tour(Some(idle));
        assert!(app.tour.is_none(), "a held globe is left alone");
        app.key(KeyCode::Char('r'), KeyModifiers::NONE, 10.0);
        app.last_input = long_ago;
        app.update_tour(Some(idle));
        assert!(app.tour.is_some(), "r lets the tour start again");
        // A hold left on the globe does not stop other views touring.
        let mut app = App::new(view("city"), model::demo(1.0, 64), all_available());
        app.render(SCENE.width, SCENE.height, 512);
        app.goal.globe_held = true;
        app.last_input = long_ago;
        app.update_tour(Some(idle));
        assert!(app.tour.is_some());
    }

    #[test]
    fn a_tour_that_finds_the_globe_held_lets_it_go() {
        let mut app = globe_at(20.0);
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        assert!(app.tour.is_some());
        // A hold that arrives without the input handler ending the tour first.
        let time = app.render_time();
        app.both(|c| {
            c.hold(time);
            c.globe_tilt = 0.4;
        });
        app.update_tour(None);
        assert!(!app.goal.globe_held && !app.camera.globe_held);
        assert_eq!(app.goal.globe_tilt, 0.0);
    }

    #[test]
    fn a_tour_on_another_view_leaves_a_held_globe_as_it_was() {
        let mut app = globe_at(20.0);
        scroll(&mut app, MouseEventKind::ScrollDown, 3);
        scroll(&mut app, MouseEventKind::ScrollRight, 3);
        let tilt = app.goal.globe_tilt;
        assert!(app.goal.globe_held && tilt > 0.0);
        app.view = view("city");
        app.render(SCENE.width, SCENE.height, 512);
        app.update_tour(Some(Duration::ZERO));
        assert!(app.tour.is_some(), "an idle City tours");
        app.update_tour(None);
        assert!(app.goal.globe_held && app.camera.globe_held);
        assert_eq!(app.goal.globe_tilt, tilt);
        app.tour = None;
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        assert!(app.tour.is_some());
        assert!(app.goal.globe_held && app.goal.globe_tilt == tilt);
    }

    #[test]
    fn g_releases_a_held_globe_even_when_there_is_nobody_to_visit() {
        let mut app = App::new(
            view("globe"),
            {
                let mut snapshot = model::demo(1.0, 64);
                snapshot.remotes.clear();
                snapshot
            },
            all_available(),
        );
        app.render(SCENE.width, SCENE.height, 512);
        scroll(&mut app, MouseEventKind::ScrollDown, 3);
        assert!(app.goal.globe_held);
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        assert!(app.tour.is_none(), "no talkers, no tour");
        assert!(!app.goal.globe_held && !app.camera.globe_held);
        assert_eq!(app.goal.globe_tilt, 0.0);
    }

    /// The world point of the tour's target, and where the goal camera puts it on screen.
    fn target_on_screen(app: &mut App) -> ([f32; 2], Option<Identity>) {
        app.camera = app.goal.clone();
        let frame = app.render(640, 360, 512);
        let target = app.tour.as_ref().expect("a tour").target;
        let at = frame.project(&app.camera, app.scene.positions[&target]);
        let picked = frame.pick_near(at[0], at[1], 3.0);
        ([at[0], at[1]], picked)
    }

    #[test]
    fn the_tour_still_faces_its_target_after_the_globe_was_tilted() {
        for time in [0.0, 75.0, 150.0, 225.0] {
            for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp] {
                let mut app = globe_at(time);
                scroll(&mut app, MouseEventKind::ScrollRight, 4);
                scroll(&mut app, kind, 7);
                assert!(app.camera.globe_tilt.abs() > 0.5);
                app.render(640, 360, 512);
                app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
                app.update_tour(None);
                let target = app.tour.as_ref().expect("a tour").target;
                let ([x, y], picked) = target_on_screen(&mut app);
                assert_eq!(picked, Some(target), "{time} s {kind:?}");
                let (cx, cy) = (320.0, 360.0 * ORIGIN_Y);
                assert!(
                    (x - cx).abs() < 1.5 && (y - cy).abs() < 1.5,
                    "{time} s {kind:?}: the target is at {x},{y}, not the centre"
                );
            }
        }
    }

    #[test]
    fn the_tour_aims_true_while_the_camera_is_still_easing() {
        let mut app = globe_at(30.0);
        scroll(&mut app, MouseEventKind::ScrollDown, 7);
        scroll(&mut app, MouseEventKind::ScrollRight, 5);
        app.render(640, 360, 512);
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        // The loop's order: the camera eases, then the tour aims, then the frame is drawn, so
        // the positions the tour reads were drawn by a camera that has since moved.
        let goal = app.goal.clone();
        app.camera.approach(&goal, 0.25);
        assert!(app.camera.globe_tilt > 0.1 && app.camera.globe_tilt != app.rendered.globe_tilt);
        app.update_tour(None);
        let ([x, y], _) = target_on_screen(&mut app);
        let (cx, cy) = (320.0, 360.0 * ORIGIN_Y);
        assert!(
            (x - cx).abs() < 1.5 && (y - cy).abs() < 1.5,
            "the target is at {x},{y}, not the centre"
        );
    }
}
