mod camera;
#[cfg(test)]
pub(crate) mod fixtures;
mod input;
mod overlay;
mod player;
mod pointer;
mod renderer;
mod screen;
mod search;
mod status;
mod switcher;
mod tour;

use std::time::{Duration, Instant};

use crossterm::event::MouseButton;

use crate::journal::{self, Journal};
use crate::model::{Demand, Identity, Snapshot};
use crate::picker::{self, Picker};
use crate::platform::Source;
use crate::render::{Camera, Frame, RenderInput, Scene, Thing};
use crate::simulation::Rng;
use crate::views::{View, ViewInfo};

pub(crate) use input::typed;
use player::HistoryPlayer;
use pointer::Pointer;
pub(crate) use renderer::Renderer;
pub(crate) use screen::Layout;
use tour::{Cursor, Tour};

/// How long the journal follower keeps running after the demand last contained
/// `Source::Journal` (Matrix was shown), so a look at another view does not restart it, and with
/// it the rain's backlog.
pub(crate) const JOURNAL_LINGER: Duration = Duration::from_secs(30);

pub(crate) struct App {
    pub(crate) scene: Scene,
    pub(crate) camera: Camera,
    pub(crate) goal: Camera,
    /// The camera that drew `scene.positions`, which the camera has since moved on from.
    pub(crate) rendered: Camera,
    pub(crate) view: View,
    pub(crate) selected: Option<Identity>,
    /// The selected thing; never Some while a process is selected.
    pub(crate) thing: Option<Thing>,
    pub(crate) focus: Option<Identity>,
    pub(crate) history: HistoryPlayer,
    pub(crate) search: Option<String>,
    pub(crate) matches: Vec<Identity>,
    pub(crate) match_index: usize,
    pub(crate) fit: bool,
    /// The camera still frames the whole view, so views that grow are refitted as they do.
    pub(crate) framed: bool,
    pub(crate) snap: bool,
    pub(crate) fit_zoom: f32,
    pub(crate) show_help: bool,
    /// The status panel is wanted; `h` hides it so the scene fills the terminal.
    pub(crate) panel: bool,
    pub(crate) labels: bool,
    pub(crate) hover: Option<Pointer>,
    pub(crate) hovered: Option<Identity>,
    /// The thing under the pointer in the last frame, which the view highlights while no thing
    /// is selected.
    pub(crate) hovered_thing: Option<Thing>,
    pub(crate) dragging: Option<(MouseButton, [f32; 2])>,
    pub(crate) dismiss: bool,
    pub(crate) animation: f32,
    pub(crate) last_input: Instant,
    pub(crate) tour: Option<Tour>,
    pub(crate) cursor: Option<Cursor>,
    /// Draws the position of a random visit.
    pub(crate) rng: Rng,
    /// The terminal reports Shift+Space apart from Space (the Kitty keyboard protocol is on).
    pub(crate) keyboard: bool,
    pub(crate) renderer: Renderer,
    /// When the demand last contained `Source::Journal`; None while it has not.
    pub(crate) journal_demanded: Option<Instant>,
    /// What `feed` has attached to samples so far, which numbers the next lines.
    pub(crate) journal_totals: journal::Totals,
    /// Whether each of the scene's views runs on this platform, in row order.
    pub(crate) available: Vec<bool>,
    /// The views Tab and Shift+Tab cycle; never empty while any view is available.
    pub(crate) playlist: Vec<View>,
    /// The view picker, while `v` has it open.
    pub(crate) picker: Option<Picker>,
}

impl App {
    pub(super) fn new(view: View, snapshot: Snapshot, available: Vec<bool>) -> Self {
        Self::with_scene(Scene::new(), view, snapshot, available)
    }

    /// An app drawing `scene`'s rows that starts on `view` as a switch to it would leave it.
    pub(super) fn with_scene(
        scene: Scene,
        view: View,
        snapshot: Snapshot,
        available: Vec<bool>,
    ) -> Self {
        let mut app = Self {
            scene,
            camera: Camera::default(),
            goal: Camera::default(),
            rendered: Camera::default(),
            view,
            selected: None,
            thing: None,
            focus: None,
            history: HistoryPlayer::new(snapshot),
            search: None,
            matches: Vec::new(),
            match_index: 0,
            fit: true,
            framed: true,
            snap: true,
            fit_zoom: 5.0,
            show_help: false,
            panel: true,
            labels: true,
            hover: None,
            hovered: None,
            hovered_thing: None,
            dragging: None,
            dismiss: false,
            animation: 0.0,
            last_input: Instant::now(),
            tour: None,
            cursor: None,
            rng: Rng::new(clock_seed()),
            keyboard: false,
            renderer: Renderer::cpu(),
            journal_demanded: None,
            journal_totals: journal::Totals::default(),
            playlist: picker::playlist(&[], &available),
            picker: None,
            available,
        };
        app.switch(view);
        app
    }

    /// The shown view's registry row.
    pub(super) fn row(&self) -> &'static ViewInfo {
        self.scene.row(self.view)
    }

    /// Takes in a new sample as the newest, keeping at most `capacity` in the history. The sample
    /// it replaces as the newest lets go of its journal lines: they were drawn, and only the
    /// newest sample is ever fed more.
    pub(super) fn push_sample(&mut self, snapshot: Snapshot, capacity: usize) {
        self.scene.record(&snapshot, self.animation);
        self.history.push(snapshot, capacity);
    }

    /// Whether the journal follower should run at `now`: while the demand contains
    /// `Source::Journal` and for `JOURNAL_LINGER` after it last did.
    pub(super) fn journal_wanted(&mut self, now: Instant) -> bool {
        if self.demand().contains(Source::Journal) {
            self.journal_demanded = Some(now);
        }
        self.journal_demanded
            .is_some_and(|shown| now.saturating_duration_since(shown) < JOURNAL_LINGER)
    }

    /// The optional sources the shown view reads.
    pub(super) fn demand(&self) -> Demand {
        Demand::from(self.row().sources)
    }

    pub(super) fn snapshot(&self) -> &Snapshot {
        self.history.snapshot()
    }

    pub(super) fn both(&mut self, change: impl Fn(&mut Camera)) {
        change(&mut self.camera);
        change(&mut self.goal);
    }

    /// The time the scene is drawn at: the sample's own while paused or rewound, else the
    /// animation clock.
    pub(super) fn render_time(&self) -> f32 {
        if self.history.is_paused() {
            self.history.snapshot().elapsed as f32
        } else {
            self.animation
        }
    }

    /// Draws one frame: records its display list, then rasterizes it.
    pub(crate) fn render(&mut self, width: u32, height: u32, limit: usize) -> Frame {
        let mut frame = self.draw(width, height, limit);
        self.rasterize(&mut frame);
        frame
    }

    /// Rasterizes a frame `draw` recorded, on the GPU when there is one.
    pub(crate) fn rasterize(&mut self, frame: &mut Frame) {
        self.renderer
            .rasterize(frame, self.hover.map(|hover| hover.frame));
    }

    /// Replaces the rasterizer, which starts as the CPU.
    pub(crate) fn set_renderer(&mut self, renderer: Renderer) {
        self.renderer = renderer;
    }

    /// Records the frame's display list, which `render` then rasterizes: everything that moves a
    /// view's state, without the pixels.
    pub(super) fn draw(&mut self, width: u32, height: u32, limit: usize) -> Frame {
        let time = self.render_time();
        self.scene.highlight = [
            self.selected,
            self.hovered,
            self.tour.as_ref().map(|tour| tour.target),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut input = RenderInput {
            snapshot: self.history.snapshot(),
            view: self.view,
            camera: &self.camera,
            width,
            height,
            selected: self.selected,
            thing: self.thing.or(self.hovered_thing),
            time,
            limit,
            focus: self.focus,
            paused: self.history.is_paused(),
            toured: self.tour.as_ref().map(|tour| tour.target),
        };
        let mut frame = self.scene.render(&input);
        if self.fit {
            self.scene.fit(&mut self.goal, width, height);
            self.fit_zoom = self.goal.zoom;
            self.fit = false;
            if std::mem::take(&mut self.snap) {
                // The camera snaps to the fitted goal, and this frame is drawn from there.
                input.camera = &self.goal;
                frame = self.scene.render(&input);
                self.camera = self.goal.clone();
            }
        }
        self.rendered = self.camera.clone();
        frame
    }

    pub(super) fn touch(&mut self) {
        self.last_input = Instant::now();
        self.tour = None;
    }

    /// Whether the status panel is drawn: when wanted, and always while typing a search.
    pub(super) fn panel_shown(&self) -> bool {
        self.panel || self.search.is_some()
    }
}

/// A seed for the random visit that differs from run to run.
fn clock_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64)
}

/// Attaches the journal lines of this frame to the newest sample (`Snapshot::journal`), where the
/// view that draws them finds them: synthetic ones arriving in `(from, to]` demo seconds, or
/// whatever `journalctl` has written since the last call.
pub(crate) fn feed(app: &mut App, source: Option<&Journal>, demo: bool, from: f64, to: f64) {
    let lines = match (demo, source) {
        (true, _) => journal::demo(from, to),
        (false, Some(source)) => source.drain(),
        (false, None) => Vec::new(),
    };
    let dropped = source.map_or(0, Journal::dropped);
    if !lines.is_empty() || dropped > 0 {
        let App {
            history,
            journal_totals,
            ..
        } = app;
        history
            .newest_mut()
            .journal
            .add(journal_totals, lines, dropped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;

    #[test]
    fn matrix_status_ignores_a_process_selected_in_another_view() {
        let snapshot = model::demo(1.0, 16);
        let selected = snapshot.processes[0].id;
        let mut app = App::new(view("matrix"), snapshot, all_available());
        app.selected = Some(selected);
        let App {
            history,
            journal_totals,
            ..
        } = &mut app;
        history.samples_mut()[0].journal.add(
            journal_totals,
            vec![journal::Line::new(6, "sshd[1]: hello")],
            0,
        );
        // The line reaches the view when it draws the sample that carries it.
        let frame = app.render(320, 180, 512);
        let text = app.text(0.0, "test", 20, 512);
        assert_eq!(text[1], " > sshd[1]: hello");
        assert!(text[2].contains("queued"), "{}", text[2]);
        let layout = Layout {
            columns: 80,
            rows: 24,
            width: 320,
            height: 180,
            cell: None,
        };
        assert!(app.popup(&frame, &layout).is_none());
        assert_eq!(app.selected, Some(selected), "kept for the process views");
    }

    #[test]
    fn matrix_lines_reach_the_view_through_the_snapshot() {
        let mut app = App::new(view("matrix"), model::demo(1.0, 16), all_available());
        let line = |app: &App| {
            app.scene
                .inspector_line(app.view)
                .expect("the matrix has one")
        };
        let legend = |app: &App| app.scene.legend(app.view, app.snapshot());
        app.render(320, 180, 512);
        assert_eq!(line(&app), " > waiting for the journal...");

        // `main.rs` hands the frame's lines to the sample; the view takes them in when it draws.
        feed(&mut app, None, true, 0.0, 1.0);
        let lines = journal::demo(0.0, 1.0);
        assert_eq!(app.history.newest().journal.lines, lines);
        assert_eq!(line(&app), " > waiting for the journal...", "not drawn yet");
        app.render(320, 180, 512);
        assert_eq!(line(&app), format!(" > {}", lines.last().unwrap().text));
        let queued = legend(&app);

        // Drawing the same sample again, or a newer one that brought no lines, queues none twice.
        app.render(320, 180, 512);
        app.history.samples_mut().push_back(model::demo(2.0, 16));
        app.render(320, 180, 512);
        assert_eq!(legend(&app), queued);
        assert_eq!(line(&app), format!(" > {}", lines.last().unwrap().text));

        // Lines attached to the newer sample are the next ones taken.
        feed(&mut app, None, true, 1.0, 2.0);
        app.render(320, 180, 512);
        let all = journal::demo(0.0, 2.0);
        assert_eq!(line(&app), format!(" > {}", all.last().unwrap().text));
        assert_eq!(app.history.newest().journal.through as usize, all.len());
    }

    #[test]
    fn feed_attaches_each_frames_journal_lines_to_the_newest_sample() {
        let mut app = App::new(view("matrix"), model::demo(1.0, 16), all_available());
        app.history.samples_mut().push_front(model::demo(0.0, 16));
        feed(&mut app, None, true, 0.0, 1.0);
        assert!(
            app.history.samples()[0].journal.lines.is_empty(),
            "an older sample gets none"
        );
        assert_eq!(
            app.history.samples()[1].journal.lines,
            journal::demo(0.0, 1.0)
        );
        feed(&mut app, None, true, 1.0, 2.0);
        let newest = &app.history.samples()[1].journal;
        assert_eq!(newest.lines, journal::demo(0.0, 2.0));
        assert_eq!(newest.through as usize, newest.lines.len());
        // A frame that brought nothing leaves the sample as it was.
        let before = newest.clone();
        feed(&mut app, None, true, 2.0, 2.05);
        assert_eq!(app.history.samples()[1].journal, before);
    }

    #[test]
    fn the_history_keeps_the_journal_lines_of_the_newest_sample_only() {
        let mut app = App::new(view("matrix"), model::demo(1.0, 16), all_available());
        let held = |app: &App| -> usize {
            app.history
                .samples()
                .iter()
                .map(|s| s.journal.lines.len())
                .sum()
        };
        for sample in 0..40 {
            app.push_sample(model::demo(f64::from(sample), 16), 24);
            // A flood bigger than a sample keeps arrives every frame, twice.
            for _ in 0..2 {
                let flood: Vec<journal::Line> = (0..journal::KEPT)
                    .map(|n| journal::Line::new(6, format!("line {n}")))
                    .collect();
                let App {
                    history,
                    journal_totals,
                    ..
                } = &mut app;
                let newest = history.newest_mut();
                newest.journal.add(journal_totals, flood, 0);
            }
            assert!(held(&app) <= journal::KEPT, "{} lines held", held(&app));
        }
        assert_eq!(app.history.len(), 24);
        assert_eq!(held(&app), journal::KEPT, "the newest sample's lines");
        // The numbering outlives the lines: a sample that was the newest says how many had
        // arrived by then, and offers none of them to a view that has not taken them.
        for older in app.history.samples().iter().take(23) {
            assert!(older.journal.lines.is_empty());
            assert!(older.journal.through > 0);
            assert!(older.journal.unseen(0).is_empty());
        }
    }

    #[test]
    fn the_journal_stops_after_matrix_has_been_hidden_for_a_while() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        assert!(!app.journal_wanted(at(0)), "Matrix was never shown");
        app.view = view("matrix");
        assert!(app.journal_wanted(at(10)));
        assert!(app.journal_wanted(at(500)), "shown all along");
        app.view = view("globe");
        assert!(app.journal_wanted(at(501)));
        assert!(app.journal_wanted(at(529)), "a look at another view");
        assert!(!app.journal_wanted(at(530)));
        assert!(!app.journal_wanted(at(900)));
        app.view = view("matrix");
        assert!(app.journal_wanted(at(901)), "back on Matrix");
    }

    #[test]
    fn changing_views_updates_the_demand() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        assert!(app.demand().is_empty(), "City reads no optional source");
        app.switch(view("globe"));
        assert!(app.demand().contains(Source::Locations));
        assert!(!app.demand().contains(Source::Journal));
        app.switch(view("matrix"));
        assert!(app.demand().contains(Source::Journal));
        assert!(
            !app.demand().contains(Source::Locations),
            "only the shown view"
        );
        app.switch(view("cores"));
        assert!(app.demand().is_empty());
        // Every view demands exactly what its registry row declares.
        for (index, row) in REGISTRY.iter().enumerate() {
            app.switch(View::at(index));
            assert_eq!(app.demand(), Demand::from(row.sources));
        }
    }
}
