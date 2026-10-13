mod camera;
mod feed;
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

use crate::model::{Demand, Identity, Snapshot};
use crate::picker::{self, Picker};
use crate::render::{Frame, RenderInput, Scene, Thing};
use crate::simulation::Rng;
use crate::views::{View, ViewInfo};

use camera::CameraController;
use feed::JournalFeed;
pub(crate) use input::typed;
use player::HistoryPlayer;
use pointer::Pointer;
pub(crate) use renderer::Renderer;
pub(crate) use screen::Layout;
use search::Search;
use tour::{Cursor, Tour};

pub(crate) struct App {
    pub(crate) scene: Scene,
    pub(crate) camera: CameraController,
    pub(crate) view: View,
    pub(crate) selected: Option<Identity>,
    /// The selected thing; never Some while a process is selected.
    pub(crate) thing: Option<Thing>,
    pub(crate) focus: Option<Identity>,
    pub(crate) history: HistoryPlayer,
    pub(crate) search: Search,
    pub(crate) show_help: bool,
    /// The status panel is wanted; `h` hides it so the scene fills the terminal.
    pub(crate) panel: bool,
    pub(crate) labels: bool,
    pub(crate) pointer: Pointer,
    pub(crate) animation: f32,
    pub(crate) last_input: Instant,
    pub(crate) tour: Option<Tour>,
    pub(crate) cursor: Option<Cursor>,
    /// Draws the position of a random visit.
    pub(crate) rng: Rng,
    /// The terminal reports Shift+Space apart from Space (the Kitty keyboard protocol is on).
    pub(crate) keyboard: bool,
    pub(crate) renderer: Renderer,
    pub(crate) journal: JournalFeed,
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
            camera: CameraController::default(),
            view,
            selected: None,
            thing: None,
            focus: None,
            history: HistoryPlayer::new(snapshot),
            search: Search::default(),
            show_help: false,
            panel: true,
            labels: true,
            pointer: Pointer::default(),
            animation: 0.0,
            last_input: Instant::now(),
            tour: None,
            cursor: None,
            rng: Rng::new(clock_seed()),
            keyboard: false,
            renderer: Renderer::cpu(),
            journal: JournalFeed::default(),
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

    /// Takes in a sample the live loop collected. A camera that still frames the whole view,
    /// with no tour under way, is refitted when the view grows by itself.
    pub(crate) fn take_sample(&mut self, snapshot: Snapshot, capacity: usize) {
        self.push_sample(snapshot, capacity);
        if self.camera.is_framed() && self.tour.is_none() && self.scene.bounded() {
            self.camera.refit();
        }
    }

    /// Moves the camera on by `elapsed` seconds and takes the tour a step, which starts after
    /// `tour_after` without input.
    pub(crate) fn steer(&mut self, elapsed: f32, tour_after: Option<Duration>) {
        self.camera.approach(elapsed);
        self.update_tour(tour_after);
    }

    /// The optional sources the shown view reads.
    pub(super) fn demand(&self) -> Demand {
        Demand::from(self.row().sources)
    }

    pub(super) fn snapshot(&self) -> &Snapshot {
        self.history.snapshot()
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
            .rasterize(frame, self.pointer.hover.map(|hover| hover.frame));
    }

    /// Replaces the rasterizer, which starts as the CPU.
    pub(crate) fn set_renderer(&mut self, renderer: Renderer) {
        self.renderer = renderer;
    }

    /// Records the frame's display list, which `rasterize` then turns into pixels: everything
    /// that moves a view's state, without the pixels.
    pub(crate) fn draw(&mut self, width: u32, height: u32, limit: usize) -> Frame {
        self.scene.highlight = [
            self.selected,
            self.pointer.hovered,
            self.tour.as_ref().map(|tour| tour.target),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut frame = self.record(width, height, limit, false);
        if self.camera.fit_to(&self.scene, width, height) {
            // The camera snaps to the fitted goal, and this frame is drawn from there.
            frame = self.record(width, height, limit, true);
            self.camera.snap_to_goal();
        }
        self.camera.drawn();
        frame
    }

    /// The scene's display list for the shown sample, seen from the camera, or from its goal.
    fn record(&mut self, width: u32, height: u32, limit: usize, from_goal: bool) -> Frame {
        let camera = if from_goal {
            &self.camera.goal
        } else {
            &self.camera.current
        };
        let input = RenderInput {
            snapshot: self.history.snapshot(),
            view: self.view,
            camera,
            width,
            height,
            selected: self.selected,
            thing: self.thing.or(self.pointer.hovered_thing),
            time: self.render_time(),
            limit,
            focus: self.focus,
            paused: self.history.is_paused(),
            toured: self.tour.as_ref().map(|tour| tour.target),
        };
        self.scene.render(&input)
    }

    pub(super) fn touch(&mut self) {
        self.last_input = Instant::now();
        self.tour = None;
    }

    /// Whether the status panel is drawn: when wanted, and always while typing a search.
    pub(super) fn panel_shown(&self) -> bool {
        self.panel || self.search.is_open()
    }
}

/// A seed for the random visit that differs from run to run.
fn clock_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64)
}

#[cfg(test)]
mod tests {

    use crate::app::fixtures::*;

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
