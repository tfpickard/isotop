//! Test fixtures shared by the tests of the `app` modules.

pub(crate) use std::f32::consts::{FRAC_PI_2, TAU};
pub(crate) use std::time::{Duration, Instant};

pub(crate) use crossterm::event::{
    self, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

pub(crate) use crate::app::App;
pub(crate) use crate::app::pointer::Spot;
pub(crate) use crate::app::screen::Layout;
pub(crate) use crate::cli::Options;
pub(crate) use crate::model::{self, Demand, Identity, bytes};
pub(crate) use crate::platform::{self, Source};
pub(crate) use crate::render::{self, Camera, Frame, ISOMETRIC, ORIGIN_Y, Scene, Thing};
pub(crate) use crate::simulation::Rng;
pub(crate) use crate::terminal::{Label, SafeText, Tone};
pub(crate) use crate::views::{self, REGISTRY, View, testing, view};
pub(crate) use crate::{demo_snapshot, globe, picker, terminal};
pub(crate) use clap::Parser;

/// Every registered view available, as on Linux and macOS today.
pub(crate) fn all_available() -> Vec<bool> {
    vec![true; REGISTRY.len()]
}

/// An app over the test module views of `views::testing::ROWS`, showing `probe`.
pub(crate) fn probe_app() -> App {
    app_over(views::testing::ROWS)
}

/// An app over the test module views of `rows`, showing the first, drawn once.
pub(crate) fn app_over(rows: &'static [views::ViewInfo]) -> App {
    let mut app = App::new(View::at(0), model::demo(1.0, 16), vec![true; rows.len()]);
    app.scene = Scene::with_rows(rows);
    app.render(320, 180, 512);
    app
}

/// An app over `views::testing::GUIDED`, showing `guide`, and the process in its last seat,
/// which the guide tours. That process is made idle and small, so that nothing but the
/// guide's target puts it first.
pub(crate) fn guided_app() -> (App, Identity) {
    let mut snapshot = model::demo(1.0, 16);
    let last = snapshot
        .processes
        .iter()
        .map(|p| p.id)
        .max()
        .expect("the demo has processes");
    for process in snapshot.processes.iter_mut().filter(|p| p.id == last) {
        process.cpu = 0.0;
        process.memory = 1;
        process.gpu_memory = None;
    }
    let rows = views::testing::GUIDED;
    let mut app = App::new(View::at(0), snapshot, vec![true; rows.len()]);
    app.scene = Scene::with_rows(rows);
    app.render(320, 180, 512);
    (app, last)
}

pub(crate) fn press(app: &mut App, code: KeyCode) -> bool {
    app.key(code, KeyModifiers::NONE, 10.0)
}

pub(crate) fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, KeyCode::Char(character));
    }
}

pub(crate) fn playlist_names(app: &App) -> Vec<&'static str> {
    app.playlist.iter().map(|view| view.name()).collect()
}

pub(crate) const SCENE: Layout = Layout {
    columns: 80,
    rows: 24,
    width: 640,
    height: 360,
    cell: None,
};

pub(crate) fn scroll(app: &mut App, kind: MouseEventKind, times: usize) {
    let frame = app.render(SCENE.width, SCENE.height, 512);
    let event = MouseEvent {
        kind,
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    for _ in 0..times {
        app.mouse(event, &SCENE, &frame, &[], false);
    }
}

/// A globe at animation time `time`, drawn once so its camera has settled.
pub(crate) fn globe_at(time: f32) -> App {
    let mut app = App::new(view("globe"), model::demo(1.0, 64), all_available());
    app.animation = time;
    app.render(SCENE.width, SCENE.height, 512);
    app
}

/// Where the globe's home marker falls on screen with the camera as it stands.
pub(crate) fn home_on_screen(app: &mut App) -> [f32; 2] {
    let frame = app.render(SCENE.width, SCENE.height, 512);
    let at = frame.project(&app.camera.current, app.scene.places[0].0);
    [at[0], at[1]]
}

/// Where a world point falls on screen if the view is panned by (dx, dy) frame pixels.
pub(crate) fn panned_by(camera: &Camera, point: [f32; 3], dx: f32, dy: f32) -> [f32; 2] {
    let mut panned = camera.clone();
    panned.pan(dx, dy);
    let frame = Frame::new(
        SCENE.width,
        SCENE.height,
        render::Sky {
            backdrop: render::Backdrop::Void,
            haze: [0.0; 3],
            time: 0.0,
        },
        Default::default(),
    );
    let at = frame.project(&panned, point);
    [at[0], at[1]]
}

/// A city of demo processes with a fixed random seed, drawn once so the cycle is known.
pub(crate) fn visiting_app() -> App {
    let mut app = App::new(view("city"), model::demo(1.0, 40), all_available());
    app.rng = Rng::new(7);
    app.render(320, 180, 512);
    app
}

/// Options as `isotop` parses `arguments`.
pub(crate) fn options(arguments: &[&str]) -> Options {
    Options::parse_from(std::iter::once("isotop").chain(arguments.iter().copied()))
}
