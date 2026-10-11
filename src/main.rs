mod cells;
mod coop;
mod cores;
mod geo;
mod geometry;
mod globe;
mod glyphs;
mod gpu;
#[cfg(test)]
mod harness;
mod history;
mod journal;
mod layout;
mod matrix;
mod medium;
mod model;
mod palette;
mod picker;
mod places;
mod platform;
mod reef;
mod render;
mod simulation;
mod status;
mod stellar;
mod strata;
mod terminal;
mod views;

use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use clap::builder::{PossibleValuesParser, TypedValueParser};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use journal::Journal;
use model::{Collector, Demand, GpuMemory, Identity, Process, Snapshot, bytes, describe};
use picker::{Entry, Picker, Toggle};
use platform::Source;
use render::{
    Camera, Frame, ISOMETRIC, Links, ORIGIN_Y, Point, RenderInput, SCALE, Scene, Target, Thing,
    label_name,
};
use terminal::{Label, Popup, Terminal, Tone};
use views::{LOCAL_KEYS, Phase, REGISTRY, View, ViewInfo, ViewKey, Warmup, When};

const TOUR_STEP: Duration = Duration::from_secs(6);
/// How long the journal follower keeps running after the demand last contained
/// `Source::Journal` (Matrix was shown), so a look at another view does not restart it, and with
/// it the rain's backlog.
const JOURNAL_LINGER: Duration = Duration::from_secs(30);
/// Terminal rows of the status panel under the scene.
const PANEL_ROWS: u16 = 5;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Renderer {
    Auto,
    Gpu,
    Cpu,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Power {
    /// Integrated GPU where there is one: keeps a laptop's discrete GPU asleep
    Low,
    /// Discrete GPU (for example NVIDIA) where there is one
    High,
}

#[derive(Parser)]
#[command(
    version,
    about = "A living process city and orbital observatory inside your terminal"
)]
struct Options {
    /// Use a deterministic synthetic workload instead of /proc
    #[arg(long)]
    demo: bool,
    /// Initial visualization (city, or the first of --views)
    #[arg(long, value_parser = view_parser())]
    view: Option<View>,
    /// The views Tab and Shift+Tab cycle, as a comma-separated list such as reef,city: a name
    /// repeated or not supported on this platform is dropped. Without it, every supported view
    #[arg(long, value_delimiter = ',', value_parser = view_parser())]
    views: Vec<View>,
    /// Print the views this platform supports, one per line, and exit
    #[arg(long)]
    list_views: bool,
    /// Print startup notes to stderr, such as each view this platform does not support
    #[arg(long)]
    verbose: bool,
    /// Number of synthetic processes
    #[arg(long, default_value_t = 192, value_parser = clap::value_parser!(u32).range(1..=10000))]
    processes: u32,
    /// Maximum processes admitted into a scene
    #[arg(long, default_value_t = 1024, value_parser = clap::value_parser!(u32).range(1..=10000))]
    limit: u32,
    /// Maximum internal rendering width (height follows terminal aspect ratio)
    #[arg(long, default_value_t = 1920, value_parser = clap::value_parser!(u32).range(160..=2560))]
    width: u32,
    /// Target animation frame rate
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=60))]
    fps: u32,
    /// Process sampling interval in milliseconds
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(100..=10000))]
    sample_ms: u32,
    /// Number of snapshots retained for pause/rewind
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(2..=3600))]
    history: u32,
    /// Seconds without input before the guided tour starts (0 disables it)
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(0..=3600))]
    tour: u32,
    /// Render a single PNG and exit (no terminal required)
    #[arg(long)]
    output: Option<PathBuf>,
    /// Synthetic time for PNG output and benchmark
    #[arg(long, default_value_t = 12.0)]
    time: f64,
    /// Render this many frames headlessly and report renderer performance
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10000))]
    benchmark: Option<u32>,
    /// Bypass the terminal graphics capability query
    #[arg(long)]
    force_graphics: bool,
    /// Send frames inline (zlib + base64) even when the terminal can read shared memory
    #[arg(long)]
    direct: bool,
    /// Rasterizer: the GPU when one is available, falling back to the CPU
    #[arg(long, value_enum, default_value_t = Renderer::Auto)]
    renderer: Renderer,
    /// Which GPU to prefer on machines with more than one
    #[arg(long, value_enum, default_value_t = Power::High)]
    gpu_power: Power,
    /// Exit after this many seconds and report end-to-end presentation throughput
    #[arg(long)]
    duration: Option<f64>,
    /// City-level GeoIP database (MaxMind format, such as DB-IP Lite or GeoLite2 City) for the
    /// globe; found in ~/.local/share/isotop and /usr/share/GeoIP when not given
    #[arg(long)]
    geoip: Option<PathBuf>,
    /// This machine's location as LAT,LON for the globe; defaults to the system time zone's city
    #[arg(long, value_parser = parse_home, allow_hyphen_values = true)]
    home: Option<(f32, f32)>,
}

impl Options {
    /// Height of a headless frame: 16:9 at the requested width.
    fn headless_height(&self) -> u32 {
        self.width * 9 / 16
    }

    /// Whether the run draws without a terminal: `--output` or `--benchmark`.
    fn headless(&self) -> bool {
        self.output.is_some() || self.benchmark.is_some()
    }
}

/// Parses `--view` from the registry's names, so its help lists them and any other name is a
/// usage error.
fn view_parser() -> impl TypedValueParser<Value = View> {
    PossibleValuesParser::new(REGISTRY.iter().map(|row| row.name))
        .map(|name| View::named(&name).expect("the parser accepts only the registry's names"))
}

fn parse_home(text: &str) -> Result<(f32, f32), String> {
    let (latitude, longitude) = text
        .split_once(',')
        .ok_or("expected LAT,LON, for example 40.7,-74.0")?;
    let latitude: f32 = latitude
        .trim()
        .parse()
        .map_err(|_| "latitude is not a number")?;
    let longitude: f32 = longitude
        .trim()
        .parse()
        .map_err(|_| "longitude is not a number")?;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err("latitude must be within ±90 and longitude within ±180".into());
    }
    Ok((latitude, longitude))
}

#[derive(Clone, Copy)]
struct Layout {
    columns: u16,
    rows: u16,
    width: u32,
    height: u32,
    /// Terminal pixels per cell, when the terminal reports its window size.
    cell: Option<[f32; 2]>,
}

impl Layout {
    /// Sizes the scene to the terminal, leaving `PANEL_ROWS` at the bottom when the panel shows.
    fn measure(max_width: u32, panel: bool) -> io::Result<Self> {
        let (columns, rows) = crossterm::terminal::size()?;
        if columns < 40 || rows < 12 {
            return Err(io::Error::other(
                "isotop needs at least 40 columns and 12 rows",
            ));
        }
        let scene_rows = if panel { rows - PANEL_ROWS } else { rows };
        let cell = crossterm::terminal::window_size()
            .ok()
            .filter(|s| s.width > 0 && s.height > 0 && s.columns > 0 && s.rows > 0)
            .map(|s| {
                [
                    s.width as f32 / s.columns as f32,
                    s.height as f32 / s.rows as f32,
                ]
            });
        let [cell_width, cell_height] = cell.unwrap_or([8.0, 16.0]);
        let native_width = columns as f32 * cell_width;
        let width = native_width.min(max_width as f32).max(160.0) as u32;
        let height = (width as f32 * scene_rows as f32 * cell_height / native_width)
            .clamp(100.0, 1440.0) as u32;
        Ok(Self {
            columns,
            rows: scene_rows,
            width,
            height,
            cell,
        })
    }

    /// Frame pixels per terminal cell.
    fn cell_size(&self) -> [f32; 2] {
        [
            self.width as f32 / self.columns as f32,
            self.height as f32 / self.rows as f32,
        ]
    }

    fn cell_of(&self, point: [f32; 2]) -> (u16, u16) {
        let [w, h] = self.cell_size();
        ((point[0] / w) as u16, (point[1] / h) as u16)
    }

    /// Frame position and cell under a mouse report, which arrives in pixels when `pixels` is set.
    fn pointer(&self, column: u16, row: u16, pixels: bool) -> Pointer {
        match self.cell.filter(|_| pixels) {
            Some([cw, ch]) => {
                let (x, y) = (column as f32, row as f32);
                Pointer {
                    frame: [
                        x * self.width as f32 / (self.columns as f32 * cw),
                        y * self.height as f32 / (self.rows as f32 * ch),
                    ],
                    cell: ((x / cw) as u16, (y / ch) as u16),
                }
            }
            None => {
                let [w, h] = self.cell_size();
                Pointer {
                    frame: [(column as f32 + 0.5) * w, (row as f32 + 0.5) * h],
                    cell: (column, row),
                }
            }
        }
    }

    fn pick_radius(&self, pixels: bool) -> f32 {
        let [w, h] = self.cell_size();
        if pixels { w.max(h) * 0.6 } else { w.max(h) }
    }
}

#[derive(Clone, Copy)]
struct Pointer {
    frame: [f32; 2],
    cell: (u16, u16),
}

/// Screen cells already used by panels and labels, so later labels never overlap them.
#[derive(Default)]
struct Board(Vec<[u16; 4]>);

impl Board {
    /// Claims a label's cells if they are free and at least one column clear of every claimed
    /// rectangle on the same rows, so neighbouring labels never run together.
    fn claim(&mut self, column: u16, row: u16, width: u16, height: u16) -> bool {
        let rect = [column, row, column + width, row + height];
        let free = self
            .0
            .iter()
            .all(|r| rect[2] < r[0] || r[2] < rect[0] || rect[3] <= r[1] || r[3] <= rect[1]);
        if free {
            self.0.push(rect);
        }
        free
    }

    /// Claims a panel's cells if they are free. Panels may touch each other and labels, but a
    /// label keeps its gap from them.
    fn reserve(&mut self, column: u16, row: u16, width: u16, height: u16) -> bool {
        let rect = [column, row, column + width, row + height];
        let free = self
            .0
            .iter()
            .all(|r| rect[2] <= r[0] || r[2] <= rect[0] || rect[3] <= r[1] || r[3] <= rect[1]);
        if free {
            self.0.push(rect);
        }
        free
    }
}

struct Tour {
    index: usize,
    target: Identity,
    since: Instant,
}

struct App {
    scene: Scene,
    camera: Camera,
    goal: Camera,
    /// The camera that drew `scene.positions`, which the camera has since moved on from.
    rendered: Camera,
    view: View,
    selected: Option<Identity>,
    /// The selected thing; never Some while a process is selected.
    thing: Option<Thing>,
    focus: Option<Identity>,
    history: VecDeque<Snapshot>,
    paused: bool,
    history_index: usize,
    search: Option<String>,
    matches: Vec<Identity>,
    match_index: usize,
    fit: bool,
    /// The camera still frames the whole view, so views that grow are refitted as they do.
    framed: bool,
    snap: bool,
    fit_zoom: f32,
    show_help: bool,
    /// The status panel is wanted; `h` hides it so the scene fills the terminal.
    panel: bool,
    labels: bool,
    hover: Option<Pointer>,
    hovered: Option<Identity>,
    /// The thing under the pointer in the last frame, which the view highlights while no thing
    /// is selected.
    hovered_thing: Option<Thing>,
    dragging: Option<(MouseButton, [f32; 2])>,
    dismiss: bool,
    animation: f32,
    last_input: Instant,
    tour: Option<Tour>,
    gpu: Option<gpu::Gpu>,
    /// Which rasterizer drew the last frame, for the status strip.
    renderer: String,
    /// When the demand last contained `Source::Journal`; None while it has not.
    journal_demanded: Option<Instant>,
    /// Whether each of the scene's views runs on this platform, in row order.
    available: Vec<bool>,
    /// The views Tab and Shift+Tab cycle; never empty while any view is available.
    playlist: Vec<View>,
    /// The view picker, while `v` has it open.
    picker: Option<Picker>,
}

impl App {
    fn new(view: View, snapshot: Snapshot, available: Vec<bool>) -> Self {
        Self::with_scene(Scene::new(), view, snapshot, available)
    }

    /// An app drawing `scene`'s rows that starts on `view` as a switch to it would leave it.
    fn with_scene(scene: Scene, view: View, snapshot: Snapshot, available: Vec<bool>) -> Self {
        let mut app = Self {
            scene,
            camera: Camera::default(),
            goal: Camera::default(),
            rendered: Camera::default(),
            view,
            selected: None,
            thing: None,
            focus: None,
            history: VecDeque::from([snapshot]),
            paused: false,
            history_index: 0,
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
            gpu: None,
            renderer: "cpu".into(),
            journal_demanded: None,
            playlist: picker::playlist(&[], &available),
            picker: None,
            available,
        };
        app.switch(view);
        app
    }

    /// The shown view's registry row.
    fn row(&self) -> &'static ViewInfo {
        self.scene.row(self.view)
    }

    /// Whether the journal follower should run at `now`: while the demand contains
    /// `Source::Journal` and for `JOURNAL_LINGER` after it last did.
    fn journal_wanted(&mut self, now: Instant) -> bool {
        if self.demand().contains(Source::Journal) {
            self.journal_demanded = Some(now);
        }
        self.journal_demanded
            .is_some_and(|shown| now.saturating_duration_since(shown) < JOURNAL_LINGER)
    }

    /// The optional sources the shown view reads.
    fn demand(&self) -> Demand {
        Demand::from(self.row().sources)
    }

    /// The view Tab (or Shift+Tab, when `forward` is false) goes to: the shown view's neighbour
    /// in the playlist, or the shown view itself when the playlist is empty.
    fn cycle(&self, forward: bool) -> View {
        picker::cycle(&self.playlist, self.view, forward).unwrap_or(self.view)
    }

    /// The views the picker lists: every available one, in registry order.
    fn entries(&self) -> Vec<Entry> {
        View::all()
            .filter(|view| self.available.get(view.index()) == Some(&true))
            .map(|view| {
                let row = self.scene.row(view);
                Entry {
                    view,
                    name: row.name,
                    title: row.title,
                }
            })
            .collect()
    }

    /// Handles a key while the picker is open: it takes every key, so nothing reaches the scene
    /// or the shown view. Returns whether to quit.
    fn picker_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        let entries = self.entries();
        let Some(picker) = self.picker.as_mut() else {
            return false;
        };
        match code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter => {
                if let Some(view) = picker.highlighted(&entries) {
                    self.picker = None;
                    self.switch(view);
                }
            }
            KeyCode::Up => picker.step(&entries, false),
            KeyCode::Down => picker.step(&entries, true),
            KeyCode::Backspace => picker.backspace(&entries),
            KeyCode::Char(' ') => match picker.highlighted(&entries) {
                Some(view) => {
                    if picker::toggle(&mut self.playlist, view) == Toggle::Refused {
                        picker.refuse();
                    } else {
                        picker.quiet();
                    }
                }
                None => picker.quiet(),
            },
            KeyCode::Char(character)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                picker.type_character(&entries, character);
            }
            _ => picker.quiet(),
        }
        false
    }

    /// The picker as a popup in the scene's top right corner, listing as many rows as fit.
    fn picker_popup(&self, layout: &Layout) -> Option<Popup> {
        let picker = self.picker.as_ref()?;
        // The popup also holds a border above and below, the filter line and the key line.
        let capacity = usize::from(layout.rows).saturating_sub(4).max(1);
        let lines = picker.lines(&self.entries(), &self.playlist, capacity);
        place_panel(lines, None, layout, false, 40)
    }

    /// Shows `view`, framed, at its row's pitch if it has one. Things belong to the view that
    /// drew them, so a selected or hovered one is let go.
    fn switch(&mut self, view: View) {
        self.view = view;
        self.fit = true;
        self.thing = None;
        self.hovered_thing = None;
        if let Some(pitch) = self.row().pitch {
            self.goal.pitch = pitch;
        }
    }

    fn choose_renderer(&mut self, renderer: Renderer, power: Power) -> Result<(), String> {
        if renderer == Renderer::Cpu {
            self.renderer = "cpu".into();
            return Ok(());
        }
        let preference = match power {
            Power::Low => wgpu::PowerPreference::LowPower,
            Power::High => wgpu::PowerPreference::HighPerformance,
        };
        match gpu::Gpu::new(preference) {
            Ok(gpu) => {
                self.renderer = format!("gpu {}", gpu.name);
                self.gpu = Some(gpu);
                Ok(())
            }
            Err(error) if renderer == Renderer::Auto => {
                self.renderer = format!("cpu ({error})");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn snapshot(&self) -> &Snapshot {
        &self.history[if self.paused {
            self.history_index
        } else {
            self.history.len() - 1
        }]
    }

    fn both(&mut self, change: impl Fn(&mut Camera)) {
        change(&mut self.camera);
        change(&mut self.goal);
    }

    /// The time the scene is drawn at: the sample's own while paused or rewound, else the
    /// animation clock.
    fn render_time(&self) -> f32 {
        if self.paused {
            self.history[self.history_index].elapsed as f32
        } else {
            self.animation
        }
    }

    /// Lets a held globe turn with time again from where it stands, and stands it upright.
    /// Does nothing unless the earth was held, or in a view with no sphere to turn, where a
    /// tour must leave the hold for the next visit to the globe.
    fn release_globe(&mut self) {
        if self.row().turn.is_none() {
            return;
        }
        let time = self.render_time();
        self.both(|c| c.release(time));
        self.goal.globe_tilt = 0.0;
    }

    fn render(&mut self, width: u32, height: u32, limit: usize) -> Frame {
        let index = if self.paused {
            self.history_index
        } else {
            self.history.len() - 1
        };
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
            snapshot: &self.history[index],
            view: self.view,
            camera: &self.camera,
            width,
            height,
            selected: self.selected,
            thing: self.thing.or(self.hovered_thing),
            time,
            limit,
            focus: self.focus,
            paused: self.paused,
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
        let failure = match self.gpu.as_mut() {
            Some(gpu) => gpu.render(&mut frame, self.hover.map(|h| h.frame)).err(),
            None => {
                frame.rasterize();
                None
            }
        };
        if let Some(error) = failure {
            self.gpu = None;
            self.renderer = format!("cpu (GPU failed: {error})");
            frame.rasterize();
        }
        frame
    }

    fn search(&mut self) {
        let query = self.search.as_deref().unwrap_or("").to_lowercase();
        self.matches = self
            .snapshot()
            .processes
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query)
                    || p.command.to_lowercase().contains(&query)
                    || p.id.pid.to_string() == query
            })
            .map(|p| p.id)
            .collect();
        self.match_index = 0;
        if !query.is_empty() {
            self.select_match();
        }
    }

    fn select_match(&mut self) {
        if let Some(&id) = self.matches.get(self.match_index) {
            self.selected = Some(id);
            self.thing = None;
            if let Some(&p) = self.scene.positions.get(&id) {
                self.goal.center = [p[0], p[1]];
                self.face(p);
            } else {
                self.focus = Some(id);
                self.fit = true;
            }
        }
    }

    fn touch(&mut self) {
        self.last_input = Instant::now();
        self.tour = None;
    }

    /// Whether the status panel is drawn: when wanted, and always while typing a search.
    fn panel_shown(&self) -> bool {
        self.panel || self.search.is_some()
    }

    /// Notable processes to visit while idle: the view's own targets (Orbit's system stars, and
    /// a module view's `Stage::tour`), heaviest first, then the busiest and the largest. Hubs
    /// are not processes, so they have no callout to show and are passed over.
    fn tour_targets(&self) -> Vec<Identity> {
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
            .stars
            .iter()
            .map(|&(id, members, _)| (id, members as f32))
            .chain(
                self.scene
                    .tour
                    .iter()
                    .map(|target| (target.id, target.weight)),
            )
            .filter(|(id, _)| !render::is_hub(id))
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

    fn update_tour(&mut self, after: Option<Duration>) {
        if !self.row().tour {
            self.tour = None;
            return;
        }
        let now = Instant::now();
        let due = match &self.tour {
            // A globe held by hand stays where it was put until r or g.
            None => {
                after.is_some_and(|after| now.duration_since(self.last_input) >= after)
                    && !(self.row().turn.is_some() && self.goal.globe_held)
            }
            Some(tour) => {
                now.duration_since(tour.since) >= TOUR_STEP
                    || self.tour_point(tour.target).is_none()
            }
        };
        if due {
            self.visit(self.tour.as_ref().map_or(0, |tour| tour.index + 1));
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
    fn tour_point(&self, id: Identity) -> Option<Point> {
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
    fn face(&mut self, point: Point) {
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

    fn visit(&mut self, index: usize) {
        self.release_globe();
        let targets = self.tour_targets();
        self.tour = (!targets.is_empty()).then(|| Tour {
            index,
            target: targets[index % targets.len()],
            since: Instant::now(),
        });
        if self.tour.is_some() {
            self.selected = None;
            self.thing = None;
            self.goal.zoom = self.fit_zoom * 2.2;
        }
    }

    fn text(&self, render_ms: f32, transport: &str, fps: u32, limit: usize) -> Vec<String> {
        let s = self.snapshot();
        let row = self.row();
        let (visible, collapsed) = row
            .header
            .map_or((self.scene.visible, self.scene.collapsed), |header| {
                header(&self.scene)
            });
        let mode = match (self.paused, &self.tour) {
            (true, _) => "PAUSED",
            (false, Some(_)) => "TOUR",
            (false, None) => "LIVE",
        };
        let mut lines = vec![format!(
            " ISOTOP / {} / {mode}   {} processes{}{} | {visible} visible | {collapsed} collapsed | {:.1}/{} CPU cores | RAM {} / {}",
            row.title,
            s.processes.len(),
            unreadable_text(s),
            sampler_text(s),
            s.processes.iter().map(|p| p.cpu).sum::<f32>() / 100.0,
            s.cores,
            known_bytes(
                s.memory_total
                    .zip(s.memory_available)
                    .map(|(total, available)| total.saturating_sub(available))
            ),
            known_bytes(s.memory_total)
        )];
        let inspector = if let Some(line) = self.scene.inspector_line(self.view) {
            line
        } else if let Some(id) = self.inspected() {
            if let Some(p) = s.processes.iter().find(|p| p.id == id) {
                format!(
                    " {} [pid {} / parent {} / {}] CPU {:.1}%  {} {}  IO {}  {}",
                    p.name,
                    id.pid,
                    p.parent,
                    p.state,
                    p.cpu,
                    platform::MEMORY_LABEL,
                    bytes(p.memory),
                    p.io_rate.map_or_else(
                        || "unavailable".into(),
                        |v| format!("{}/s", bytes(v as u64))
                    ),
                    p.group
                )
            } else {
                format!(" pid {} exited / unavailable in this snapshot", id.pid)
            }
        } else {
            " Click or hover a building/body to inspect; / searches name, command or PID".into()
        };
        lines.push(inspector);
        lines.push(
            self.inspected()
                .and_then(|id| s.processes.iter().find(|p| p.id == id))
                .map_or_else(
                    || self.scene.legend(self.view, s),
                    |p| format!(" {}", p.command),
                ),
        );
        if let Some(search) = &self.search {
            lines.push(format!(
                " /{search}_  {} matches | Enter accept | Esc cancel | Tab next",
                self.matches.len()
            ));
        } else if self.picker.is_some() {
            lines.push(" Type to filter views | Up/Down move | Enter show | Space add or remove from the Tab playlist | Esc close".into());
        } else if self.show_help {
            lines.push(" Arrows/WASD pan | +/- zoom | Q/E rotate | PgUp/PgDn tilt | t top-down | Home fit | Tab/Shift-Tab or 0-9 view | v views | g tour | f focus | l labels | c links".into());
        } else {
            let scroll = if self.row().turn.is_some() {
                "scroll turn/tilt | drag pan"
            } else {
                "scroll pan"
            };
            lines.push(format!(" Tab next view | v views | g tour | {scroll} | Ctrl-scroll zoom | click inspect | / search | c links | Space pause | h hide panel | ? help | q quit"));
        }
        let links = match self.scene.links {
            Links::All => "",
            Links::Focused => " | links: focused",
            Links::Off => " | links: off",
        };
        lines.push(if self.show_help {
            let scroll = if self.row().turn.is_some() {
                "Scroll turn/tilt | drag pan"
            } else {
                "Scroll pan"
            };
            format!(" {scroll} | Ctrl-scroll zoom | Alt-scroll or right-drag rotate/tilt | n next match | [/] rewind | r reset | Ctrl-C quit")
        } else { format!(" {render_ms:.1}ms {} | {transport} | target {fps}fps | {} samples | t={:.1}s | cap {limit} | {}{links}{}",
            self.renderer, self.history.len(), s.elapsed, pressure_text(s),
            if self.focus.is_some() { " | SUBTREE FOCUS" } else { "" }) });
        lines
    }

    /// Notes the process or thing under the pointer in `frame`, for the next frame to highlight.
    fn hover_over(&mut self, frame: &Frame, radius: f32) {
        let target = self
            .hover
            .and_then(|h| frame.target_near(h.frame[0], h.frame[1], radius));
        (self.hovered, self.hovered_thing) = match target {
            Some(Target::Process(id)) => (Some(id), None),
            Some(Target::Thing(thing)) => (None, Some(thing)),
            None => (None, None),
        };
    }

    fn screen(&self, frame: &Frame, id: Identity) -> Option<[f32; 2]> {
        let &p = self.scene.positions.get(&id)?;
        frame.locate(&self.camera, p)
    }

    /// The selected process, unless the view draws no processes and shows a line of its own in
    /// place of the inspector; the selection itself is kept for when a process view returns.
    fn inspected(&self) -> Option<Identity> {
        self.selected
            .filter(|_| self.scene.inspector_line(self.view).is_none())
    }

    /// The selected thing's popup: its card's title and lines beside the point the card names,
    /// or a note that the view no longer shows it.
    fn thing_popup(&self, thing: Thing, frame: &Frame, layout: &Layout) -> Option<Popup> {
        let (lines, anchor) = match self.scene.card(self.view, thing, self.snapshot()) {
            Some(card) => {
                let mut lines = vec![card.title];
                lines.extend(card.lines);
                lines.push("Esc close".into());
                (lines, frame.locate(&self.camera, card.at))
            }
            None => (
                vec![
                    format!("{} no longer shown", thing.kind),
                    "Esc close".into(),
                ],
                None,
            ),
        };
        place_panel(lines, anchor.map(|a| layout.cell_of(a)), layout, false, 32)
    }

    fn popup(&self, frame: &Frame, layout: &Layout) -> Option<Popup> {
        if let Some(thing) = self.thing {
            return self.thing_popup(thing, frame, layout);
        }
        let id = self.inspected()?;
        let snapshot = self.snapshot();
        let lines = match snapshot.processes.iter().find(|p| p.id == id) {
            Some(p) => {
                let mut lines = vec![
                    p.name.clone(),
                    describe(p),
                    format!(
                        "pid {} | parent {} | state {} | {} thread{}",
                        id.pid,
                        p.parent,
                        p.state,
                        p.threads,
                        if p.threads == 1 { "" } else { "s" }
                    ),
                    format!(
                        "CPU {:.1}% | {} {} | IO {}",
                        p.cpu,
                        platform::MEMORY_LABEL,
                        bytes(p.memory),
                        p.io_rate.map_or_else(
                            || "unavailable".into(),
                            |v| format!("{}/s", bytes(v as u64))
                        )
                    ),
                ];
                match p.gpu_memory {
                    None => {}
                    Some(GpuMemory::Unreported) => {
                        lines.push("NVIDIA GPU in use (memory not reported)".into());
                    }
                    Some(GpuMemory::Bytes(gpu)) => {
                        lines.push(format!("NVIDIA GPU memory {}", bytes(gpu)));
                    }
                }
                if let Some(&(total, count)) = self.scene.mass.get(&id) {
                    lines.push(format!(
                        "system {} {} across {count} processes",
                        platform::MEMORY_LABEL,
                        bytes(total)
                    ));
                }
                lines.extend(
                    self.scene
                        .popup
                        .iter()
                        .filter(|(owner, _)| *owner == id)
                        .map(|(_, line)| line.clone()),
                );
                let local = snapshot
                    .links
                    .iter()
                    .filter(|(a, b, _)| *a == id || *b == id)
                    .map(|&(_, _, n)| n)
                    .sum::<u32>();
                let outside = snapshot.outside.get(&id).copied().unwrap_or(0);
                if local + outside > 0 {
                    lines.push(format!(
                        "sockets: {local} to local processes, {outside} leaving the machine"
                    ));
                }
                lines.extend(self.scene.notes.get(&id).into_iter().flatten().cloned());
                lines.extend([
                    p.group.clone(),
                    p.command.clone(),
                    "f focus subtree | Esc close".into(),
                ]);
                lines
            }
            None => vec![
                format!("pid {}", id.pid),
                "exited / unavailable in this snapshot".into(),
                "Esc close".into(),
            ],
        };
        place_panel(
            lines,
            frame.anchor.map(|a| layout.cell_of(a)),
            layout,
            false,
            32,
        )
    }

    /// The label of an orbit system's star with `members` processes: its process's name, or
    /// for a hub the name of the parent it stands for, which isotop could not read.
    fn star_label(
        &self,
        lookup: &HashMap<Identity, &Process>,
        id: Identity,
        members: usize,
    ) -> Option<String> {
        if let Some(p) = lookup.get(&id) {
            let name = match (p.name.as_str(), p.id.pid) {
                ("systemd", 1) => "init",
                ("systemd", _) => "systemd --user",
                (name, _) => name,
            };
            let name = label_name(name);
            return Some(if members > 1 {
                format!("{name} ({members})")
            } else {
                name.into_owned()
            });
        }
        let name = self.scene.hub_names.get(&id)?;
        Some(format!("{} (unreadable) ({members})", label_name(name)))
    }

    /// Text drawn over the scene, highest priority first: the inspector popup, the tour
    /// callout (with a pointer drawn into the frame), the hover name, then system and busy labels.
    fn overlay(&self, frame: &mut Frame, layout: &Layout, radius: f32) -> (Vec<Label>, Vec<Popup>) {
        let snapshot = self.snapshot();
        let lookup: HashMap<Identity, &Process> =
            snapshot.processes.iter().map(|p| (p.id, p)).collect();
        let mut board = Board::default();
        let mut labels = Vec::new();
        let mut panels = Vec::new();
        // The open picker covers the inspector popup, which comes back when it closes.
        let popup = if self.picker.is_some() {
            self.picker_popup(layout)
        } else {
            self.popup(frame, layout)
        };
        if let Some(popup) = popup {
            board.reserve(popup.column, popup.row, popup.width, popup.height());
            panels.push(popup);
        }
        if let Some(tour) = &self.tour
            && let Some(p) = lookup.get(&tour.target)
            && let Some(anchor) = self
                .tour_point(tour.target)
                .and_then(|point| frame.locate(&self.camera, point))
        {
            let lines = vec![
                p.name.clone(),
                describe(p),
                format!(
                    "CPU {:.1}% | {} {} | pid {}",
                    p.cpu,
                    platform::MEMORY_LABEL,
                    bytes(p.memory),
                    p.id.pid
                ),
            ];
            if let Some(panel) = place_panel(lines, Some(layout.cell_of(anchor)), layout, true, 24)
            {
                let [w, h] = layout.cell_size();
                let x = anchor[0].clamp(
                    panel.column as f32 * w,
                    (panel.column + panel.width) as f32 * w,
                );
                let y = anchor[1].clamp(
                    panel.row as f32 * h,
                    (panel.row + panel.height()) as f32 * h,
                );
                pointer(frame, [x, y], anchor);
                board.reserve(panel.column, panel.row, panel.width, panel.height());
                panels.push(panel);
            }
        }
        let tag = match self
            .hover
            .and_then(|hover| frame.target_near(hover.frame[0], hover.frame[1], radius))
        {
            Some(Target::Process(id))
                if Some(id) != self.selected
                    && self.tour.as_ref().is_none_or(|tour| tour.target != id) =>
            {
                lookup
                    .get(&id)
                    .map(|p| format!(" {} - {} ", p.name, describe(p)))
            }
            Some(Target::Thing(thing)) if Some(thing) != self.thing => self
                .scene
                .card(self.view, thing, snapshot)
                .map(|card| format!(" {} ", card.title)),
            _ => None,
        };
        if let Some(hover) = self.hover
            && let Some(text) = tag
        {
            let width = text.chars().count() as u16;
            let column = (hover.cell.0 + 2).min(layout.columns.saturating_sub(width));
            let row = hover.cell.1.saturating_sub(1);
            if board.claim(column, row, width, 1) {
                labels.push(Label {
                    column,
                    row,
                    text,
                    tone: Tone::Tag,
                });
            }
        }
        if !self.labels {
            return (labels, panels);
        }
        for (point, text) in &self.scene.places {
            if let Some(at) = frame.locate(&self.camera, *point) {
                let width = text.chars().count() as u16;
                let (column, row) = layout.cell_of(at);
                let column = column.saturating_sub(width / 2);
                if row < layout.rows && board.claim(column, row, width, 1) {
                    labels.push(Label {
                        column,
                        row,
                        text: text.clone(),
                        tone: Tone::Quiet,
                    });
                }
            }
        }
        for &(id, members, reach) in &self.scene.stars {
            if let Some(text) = self.star_label(&lookup, id, members)
                && let Some(at) = self.screen(frame, id)
            {
                let width = text.chars().count() as u16;
                // Small systems are labelled just below their outer edge, large ones at the star.
                let star = self.scene.positions[&id];
                let edge = frame
                    .locate(&self.camera, self.camera.front(star, reach))
                    .filter(|edge| edge[1] - at[1] < layout.cell_size()[1] * 6.0)
                    .unwrap_or(at);
                let (column, row) = layout.cell_of([at[0], edge[1]]);
                let column = column.saturating_sub(width / 2);
                if row + 1 < layout.rows && board.claim(column, row + 1, width, 1) {
                    labels.push(Label {
                        column,
                        row: row + 1,
                        text,
                        tone: Tone::Bright,
                    });
                }
            }
        }
        let mut busy: Vec<&Process> = lookup
            .values()
            .copied()
            .filter(|p| p.cpu >= 2.0 && self.scene.positions.contains_key(&p.id))
            .collect();
        busy.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        for p in busy.into_iter().take(6) {
            if let Some(at) = self.screen(frame, p.id) {
                let text = format!("{} {:.0}%", label_name(&p.name), p.cpu);
                let (column, row) = layout.cell_of(at);
                if board.claim(column + 2, row, text.chars().count() as u16, 1) {
                    labels.push(Label {
                        column: column + 2,
                        row,
                        text,
                        tone: Tone::Quiet,
                    });
                }
            }
        }
        (labels, panels)
    }

    fn key(&mut self, code: KeyCode, modifiers: KeyModifiers, step: f32) -> bool {
        let before = self.goal.clone();
        let quit = self.command(code, modifiers, step);
        self.follow(&before);
        quit
    }

    /// Moving the camera by hand stops it following the view; fitting it again resumes.
    fn follow(&mut self, before: &Camera) {
        if self.fit {
            self.framed = true;
        } else if self.goal != *before {
            self.framed = false;
        }
    }

    fn command(&mut self, code: KeyCode, modifiers: KeyModifiers, step: f32) -> bool {
        let touring = self.tour.is_some();
        self.touch();
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            return true;
        }
        if self.picker.is_some() {
            return self.picker_key(code, modifiers);
        }
        if self.search.is_some() {
            match code {
                KeyCode::Esc | KeyCode::Enter => self.search = None,
                KeyCode::Backspace => {
                    self.search.as_mut().unwrap().pop();
                    self.search();
                }
                KeyCode::Tab => {
                    if !self.matches.is_empty() {
                        self.match_index = (self.match_index + 1) % self.matches.len();
                        self.select_match();
                    }
                }
                KeyCode::Char(c) => {
                    self.search.as_mut().unwrap().push(c);
                    self.search();
                }
                _ => {}
            }
            return false;
        }
        if code == KeyCode::Esc && self.thing.take().is_some() {
            return false;
        }
        if let Some(key) = view_key(code) {
            let consumed = self.scene.key(self.view, key);
            match key {
                ViewKey::Local(_) => return false,
                ViewKey::Escape if consumed => return false,
                ViewKey::Escape | ViewKey::Reset => {}
            }
        }
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Tab => self.switch(self.cycle(true)),
            KeyCode::BackTab => self.switch(self.cycle(false)),
            KeyCode::Left | KeyCode::Char('a') => self.goal.pan(-step, 0.0),
            KeyCode::Right | KeyCode::Char('d') => self.goal.pan(step, 0.0),
            KeyCode::Up | KeyCode::Char('w') => self.goal.pan(0.0, -step),
            KeyCode::Down | KeyCode::Char('s') => self.goal.pan(0.0, step),
            KeyCode::Char('+') | KeyCode::Char('=') => self.goal.scale(1.15),
            KeyCode::Char('-') => self.goal.scale(1.0 / 1.15),
            KeyCode::Char('e') => self.goal.rotation += FRAC_PI_2,
            KeyCode::Char('Q') => self.goal.rotation -= FRAC_PI_2,
            KeyCode::PageUp => self.goal.tilt(0.12),
            KeyCode::PageDown => self.goal.tilt(-0.12),
            KeyCode::Char('t') => {
                self.goal.pitch = if self.goal.pitch > 1.2 {
                    ISOMETRIC
                } else {
                    FRAC_PI_2
                };
            }
            KeyCode::Char('v') => {
                self.picker = Some(Picker::open(&self.entries(), self.view));
            }
            KeyCode::Char('l') => self.labels = !self.labels,
            KeyCode::Char('h') => self.panel = !self.panel,
            KeyCode::Char(digit @ '0'..='9') => {
                // 1 to 9 pick the first nine rows and 0 the tenth.
                let index = (digit as usize + 10 - '1' as usize) % 10;
                if self.available.get(index) == Some(&true) {
                    self.switch(View::at(index));
                }
            }
            KeyCode::Char('g') if !touring && self.row().tour => self.visit(0),
            KeyCode::Char('c') => self.scene.links.cycle(),
            KeyCode::Home => self.fit = true,
            KeyCode::Char('r') => {
                // Release first: the camera must stop being held before it eases to the default
                // camera, or it would copy the goal's release and spin a second time.
                let time = self.render_time();
                self.camera.release(time);
                self.goal = Camera::default();
                if let Some(pitch) = self.row().pitch {
                    self.goal.pitch = pitch;
                }
                self.fit = true;
            }
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char('/') => {
                self.search = Some(String::new());
                self.matches.clear();
            }
            KeyCode::Char('n') => {
                if !self.matches.is_empty() {
                    self.match_index = (self.match_index + 1) % self.matches.len();
                    self.select_match();
                }
            }
            KeyCode::Char('f') => {
                if self.selected.is_some() {
                    self.focus = self.selected;
                    self.fit = true;
                }
            }
            KeyCode::Esc => {
                if self.selected.take().is_none() {
                    self.focus = None;
                    self.fit = true;
                }
            }
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                self.history_index = self.history.len() - 1;
            }
            KeyCode::Char('[') => {
                if !self.paused {
                    self.history_index = self.history.len() - 1;
                    self.paused = true;
                }
                self.history_index = self.history_index.saturating_sub(1);
            }
            KeyCode::Char(']') if self.paused => {
                self.history_index = (self.history_index + 1).min(self.history.len() - 1)
            }
            _ => {}
        }
        false
    }

    fn mouse(
        &mut self,
        mouse: MouseEvent,
        layout: &Layout,
        frame: &Frame,
        panels: &[Popup],
        pixels: bool,
    ) {
        if self.picker.is_some() {
            return;
        }
        let before = self.goal.clone();
        self.pointer_event(mouse, layout, frame, panels, pixels);
        self.follow(&before);
    }

    fn pointer_event(
        &mut self,
        mouse: MouseEvent,
        layout: &Layout,
        frame: &Frame,
        panels: &[Popup],
        pixels: bool,
    ) {
        let pointer = layout.pointer(mouse.column, mouse.row, pixels);
        let (column, row) = pointer.cell;
        let in_scene = row < layout.rows;
        self.hover = in_scene.then_some(pointer);
        let scroll = match mouse.kind {
            MouseEventKind::ScrollUp => Some((0.0, -1.0)),
            MouseEventKind::ScrollDown => Some((0.0, 1.0)),
            MouseEventKind::ScrollLeft => Some((-1.0, 0.0)),
            MouseEventKind::ScrollRight => Some((1.0, 0.0)),
            _ => None,
        };
        if let Some((x, y)) = scroll {
            // Terminals deliver touchpad two-finger scrolling as wheel events; pinch never arrives.
            if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                let offset = [
                    pointer.frame[0] - layout.width as f32 * 0.5,
                    pointer.frame[1] - layout.height as f32 * ORIGIN_Y,
                ];
                let factor = 1.1_f32.powf(-(x + y));
                self.both(|c| c.scale_at(factor, offset[0], offset[1]));
            } else if mouse.modifiers.contains(KeyModifiers::ALT) {
                self.both(|c| c.rotation += 0.08 * (x + y));
            } else if let Some(turn) = self.row().turn {
                // The surface follows the fingers the way panning moves content: scrolling right
                // moves it left, which turns it westward, and scrolling down moves it up, which
                // brings the south up. The angle moves the surface under the pointer by `step`.
                let step = layout.height as f32 * 0.04;
                let angle = (step / (turn.radius * SCALE * self.camera.zoom)).min(0.25);
                let time = self.render_time();
                self.both(|c| {
                    c.hold(time);
                    c.globe_turn = (c.globe_turn + x * angle).rem_euclid(TAU);
                    c.globe_tilt =
                        (c.globe_tilt + y * angle).clamp(c.pitch - FRAC_PI_2, c.pitch + FRAC_PI_2);
                });
            } else {
                let step = layout.height as f32 * 0.04;
                self.both(|c| c.pan(x * step, y * step));
            }
            return;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left)
                if in_scene && !panels.iter().any(|p| p.contains(column, row)) =>
            {
                let picked = frame.target_near(
                    pointer.frame[0],
                    pointer.frame[1],
                    layout.pick_radius(pixels),
                );
                // Clicking empty space closes the popup, unless the press turns into a drag.
                self.dismiss = picked.is_none();
                match picked {
                    Some(Target::Process(id)) => (self.selected, self.thing) = (Some(id), None),
                    Some(Target::Thing(thing)) => (self.selected, self.thing) = (None, Some(thing)),
                    None => {}
                }
                self.dragging = Some((MouseButton::Left, pointer.frame));
            }
            MouseEventKind::Down(button) if in_scene && button != MouseButton::Left => {
                self.dragging = Some((button, pointer.frame));
            }
            MouseEventKind::Drag(button) => {
                if let Some((held, last)) = self.dragging
                    && held == button
                {
                    let dx = pointer.frame[0] - last[0];
                    let dy = pointer.frame[1] - last[1];
                    if button == MouseButton::Left {
                        self.both(|c| c.pan(-dx, -dy));
                        self.dismiss = false;
                    } else {
                        self.both(|c| {
                            c.rotation += dx * 0.01;
                            c.tilt(dy * 0.008);
                        });
                    }
                    self.dragging = Some((held, pointer.frame));
                }
            }
            MouseEventKind::Up(button) => {
                self.dragging = None;
                if button == MouseButton::Left && std::mem::take(&mut self.dismiss) {
                    self.selected = None;
                    self.thing = None;
                }
            }
            _ => {}
        }
    }
}

/// Places a bordered panel beside (or above) an anchor cell, staying inside the scene.
fn place_panel(
    lines: Vec<String>,
    anchor: Option<(u16, u16)>,
    layout: &Layout,
    above: bool,
    min_width: usize,
) -> Option<Popup> {
    let (columns, rows) = (layout.columns, layout.rows);
    let longest = lines.iter().map(String::len).max().unwrap_or(0);
    let width = (longest + 4).clamp(min_width, 64).min(columns as usize) as u16;
    let height = lines.len() as u16 + 2;
    if height > rows {
        return None;
    }
    let (column, row) = match anchor {
        Some((x, y)) => {
            let column = if x + 4 + width <= columns {
                x + 4
            } else {
                x.saturating_sub(4 + width)
            };
            let row = match (above, y >= height + 2) {
                (true, true) => y - height - 2,
                (true, false) => y + 3,
                (false, _) => y.saturating_sub(height / 2),
            };
            (column, row.min(rows - height))
        }
        None => (columns - width, 0),
    };
    Some(Popup {
        column,
        row,
        width,
        lines,
    })
}

/// A byte count for the status line, `n/a` when it was not measured.
fn known_bytes(value: Option<u64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), bytes)
}

/// The pressure readings for the status line, `n/a` for each one the platform cannot measure.
fn pressure_text(snapshot: &Snapshot) -> String {
    let [cpu, memory, io] = snapshot.pressure;
    let reading = |source: Source, value: f32| {
        if snapshot.missing.contains(source) {
            "n/a".to_owned()
        } else {
            format!("{value:.0}%")
        }
    };
    format!(
        "pressure cpu {} mem {} io {}",
        reading(Source::CpuPressure, cpu),
        reading(Source::MemoryPressure, memory),
        reading(Source::IoPressure, io)
    )
}

/// A short note beside the process count for processes of other users that could not be
/// measured at all. It sits on the first status line, which is the one least likely to be
/// clipped on a narrow terminal.
fn unreadable_text(snapshot: &Snapshot) -> String {
    match snapshot.unreadable {
        0 => String::new(),
        count => format!(" (+{count} unreadable: run with sudo)"),
    }
}

/// A short note beside the process count when the background sampler has stopped or stalled:
/// sockets, GPU memory, cgroups, open files and locks then keep what it measured last.
fn sampler_text(snapshot: &Snapshot) -> String {
    let sampler = &snapshot.sampler;
    if !sampler.running {
        " (background sampler stopped: links, files and cgroups frozen)".into()
    } else if sampler.stalled() {
        let seconds = sampler.quiet.as_secs();
        format!(" (background sampler stalled {seconds}s)")
    } else {
        String::new()
    }
}

/// What a key press offers the shown view's key hook, if anything.
fn view_key(code: KeyCode) -> Option<ViewKey> {
    match code {
        KeyCode::Char(character) if LOCAL_KEYS.contains(character) => {
            Some(ViewKey::Local(character))
        }
        KeyCode::Home | KeyCode::Char('r') => Some(ViewKey::Reset),
        KeyCode::Esc => Some(ViewKey::Escape),
        _ => None,
    }
}

/// Leader line from a callout to its target, with an arrowhead and a ring around the target.
fn pointer(frame: &mut Frame, from: [f32; 2], to: [f32; 2]) {
    const COLOR: [u8; 3] = [255, 214, 150];
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let length = dx.hypot(dy);
    if length > 16.0 {
        let (ux, uy) = (dx / length, dy / length);
        let tip = [to[0] - ux * 12.0, to[1] - uy * 12.0];
        frame.stroke(from, tip, COLOR);
        for side in [-0.45_f32, 0.45] {
            let (s, c) = side.sin_cos();
            let back = [-(ux * c - uy * s) * 9.0, -(ux * s + uy * c) * 9.0];
            frame.stroke(tip, [tip[0] + back[0], tip[1] + back[1]], COLOR);
        }
    }
    frame.circle(to, 10.0, COLOR);
}

/// What `run` does after the startup that comes before the terminal is touched.
#[derive(Debug, PartialEq, Eq)]
enum Preflight {
    /// Exit at once with this status.
    Exit(i32),
    /// Run, starting on this view.
    Start(View),
}

/// The startup before the terminal is touched. With `--verbose` it names each view this platform
/// does not support on `err`; `--list-views` lists the supported ones on `out`. A headless run
/// of an unsupported `--view` fails with status 1. An interactive one, and any run without
/// `--view` whose default (city) is unsupported, starts on the first supported view instead (the
/// first of `--views` when given). `--views` with no supported name fails with status 1. `available` holds each registry row's answer.
fn preflight(
    options: &Options,
    available: &[bool],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Preflight {
    // Writes to the standard streams only fail when they are closed; there is no one to tell.
    let supported = |view: View| available.get(view.index()) == Some(&true);
    if options.verbose {
        for view in View::all().filter(|&view| !supported(view)) {
            let _ = writeln!(
                err,
                "skipping view {}: not supported on this platform",
                view.name()
            );
        }
    }
    if options.list_views {
        for view in View::all().filter(|&view| supported(view)) {
            let _ = writeln!(out, "{}", view.name());
        }
        return Preflight::Exit(0);
    }
    let playlist = picker::playlist(&options.views, available);
    if !options.views.is_empty() && playlist.is_empty() {
        let _ = writeln!(err, "isotop: no available view in --views");
        return Preflight::Exit(1);
    }
    let requested = options.view.unwrap_or_else(|| {
        if options.views.is_empty() {
            View::named("city").expect("the registry has the city view")
        } else {
            playlist[0]
        }
    });
    if supported(requested) {
        return Preflight::Start(requested);
    }
    if options.headless() && options.view.is_some() {
        let _ = writeln!(
            err,
            "isotop: view {} is not supported on this platform",
            requested.name()
        );
        return Preflight::Exit(1);
    }
    match playlist.first() {
        Some(&view) => Preflight::Start(view),
        None => {
            let _ = writeln!(err, "isotop: no view is supported on this platform");
            Preflight::Exit(1)
        }
    }
}

/// Runs isotop and returns the exit status for `main`.
#[expect(clippy::too_many_lines, reason = "split in #79 (A20)")]
fn run(options: Options) -> Result<i32, Box<dyn Error>> {
    let available = views::availability();
    let view = match preflight(&options, &available, &mut io::stdout(), &mut io::stderr()) {
        Preflight::Exit(status) => return Ok(status),
        Preflight::Start(view) => view,
    };
    if !options.time.is_finite() || options.time < 0.0 {
        return Err("--time must be finite and nonnegative".into());
    }
    if options.duration.is_some_and(|v| !v.is_finite() || v <= 0.0) {
        return Err("--duration must be finite and positive".into());
    }
    let demand = Demand::from(view.info().sources);
    let mut collector = Collector::new(options.geoip.clone(), options.home);
    collector.demand(demand);
    let mut journal = (demand.contains(Source::Journal) && !options.demo).then(Journal::start);
    let mut snapshot = if options.demo {
        demo_snapshot(
            view.info(),
            if options.headless() {
                options.time
            } else {
                0.0
            },
            options.processes as usize,
        )
    } else {
        collector.sample()?
    };
    if !options.demo && options.headless() {
        std::thread::sleep(Duration::from_millis(options.sample_ms as u64));
        snapshot = collector.sample()?;
    }
    let playlist = picker::playlist(&options.views, &available);
    let mut app = App::new(view, snapshot, available);
    app.playlist = playlist;
    app.choose_renderer(options.renderer, options.gpu_power)?;
    let limit = options.limit as usize;
    if options.headless() {
        let height = options.headless_height();
        warm_up(&mut app, &options, journal.as_ref());
        app.animation = options.time as f32;
        let mut frame = app.render(options.width, height, limit);
        if let Some(count) = options.benchmark {
            let start = Instant::now();
            for i in 0..count {
                app.animation = options.time as f32 + i as f32 / options.fps as f32;
                app.scene.spare = frame.release();
                frame = app.render(options.width, height, limit);
                std::hint::black_box(&frame.pixels);
            }
            let elapsed = start.elapsed().as_secs_f64();
            println!(
                "{} frames | {}x{} | {} visible | {:.2} ms/frame | {:.1} renderer fps on {} (excludes terminal transport)",
                count,
                frame.width,
                frame.height,
                app.scene.visible,
                elapsed * 1000.0 / count as f64,
                count as f64 / elapsed,
                app.renderer
            );
        }
        if let Some(path) = options.output {
            frame.write_png(&path)?;
            println!(
                "Wrote {} ({}x{}, {} visible)",
                path.display(),
                frame.width,
                frame.height,
                app.scene.visible
            );
        }
        return Ok(0);
    }
    let mut terminal = Terminal::enter(options.force_graphics, options.direct)?;
    app.scene.intro = true;
    let mut layout = Layout::measure(options.width, app.panel_shown())?;
    let tour_after = (options.tour > 0).then(|| Duration::from_secs(options.tour as u64));
    let origin = Instant::now();
    let mut last_sample = Instant::now();
    let mut last_frame = Instant::now();
    let frame_interval = Duration::from_secs_f64(1.0 / options.fps as f64);
    let mut wire = 0;
    let mut quit = false;
    let mut frames = 0_u64;
    let mut total_wire = 0_u64;
    let mut fed = 0.0;
    let mut demanded = app.demand();
    while !quit {
        let frame_start = Instant::now();
        let elapsed = last_frame.elapsed().as_secs_f32();
        last_frame = Instant::now();
        if !app.paused {
            app.animation += elapsed;
            if last_sample.elapsed() >= Duration::from_millis(options.sample_ms as u64) {
                let snapshot = if options.demo {
                    demo_snapshot(
                        app.row(),
                        origin.elapsed().as_secs_f64(),
                        options.processes as usize,
                    )
                } else {
                    collector.sample()?
                };
                app.scene.record(&snapshot, app.animation);
                app.history.push_back(snapshot);
                if app.history.len() > options.history as usize {
                    app.history.pop_front();
                }
                last_sample = Instant::now();
                if app.framed && app.tour.is_none() && app.scene.bounded() {
                    app.fit = true;
                }
            }
        }
        let goal = app.goal.clone();
        if !options.demo {
            if !app.journal_wanted(Instant::now()) {
                // Dropping it stops the follower and its reader.
                journal = None;
            } else if journal.is_none() {
                journal = Some(Journal::start());
            }
        }
        if app.demand().contains(Source::Journal) {
            let now = origin.elapsed().as_secs_f64();
            // Paused, the view keeps what it shows; the journal backlog is bounded meanwhile.
            if !app.paused {
                fed = f64::max(fed, now - 2.0);
                feed(&mut app, journal.as_ref(), options.demo, fed, now);
                fed = now;
            }
        }
        app.camera
            .approach(&goal, simulation::smoothing(elapsed, 0.2));
        app.update_tour(tour_after);
        let mut frame = app.render(layout.width, layout.height, limit);
        let render_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        let pixels = terminal.pixel_mouse && layout.cell.is_some();
        app.hover_over(&frame, layout.pick_radius(pixels));
        let (labels, panels) = app.overlay(&mut frame, &layout, layout.pick_radius(pixels));
        let transport = if terminal.shared_memory {
            "shared memory".to_owned()
        } else {
            format!("{:.1} KiB/frame inline", wire as f32 / 1024.0)
        };
        let text = if app.panel_shown() {
            app.text(render_ms, &transport, options.fps, limit)
        } else {
            Vec::new()
        };
        wire = terminal.present(&frame, layout.columns, layout.rows, &labels, &panels, &text)?;
        frames += 1;
        total_wire += wire as u64;
        let deadline = frame_start + frame_interval;
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            if !event::poll(wait)? {
                break;
            }
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    let shown = app.panel_shown();
                    quit = app.key(key.code, key.modifiers, layout.width as f32 * 0.04);
                    if app.panel_shown() != shown {
                        layout = Layout::measure(options.width, app.panel_shown())?;
                        terminal.clear()?;
                    }
                }
                Event::Resize(_, _) => {
                    layout = Layout::measure(options.width, app.panel_shown())?;
                    terminal.clear()?;
                }
                Event::Mouse(mouse) => {
                    app.touch();
                    app.mouse(mouse, &layout, &frame, &panels, pixels);
                }
                _ => {}
            }
            // Past the deadline, poll(0) keeps draining queued input; motion floods must not delay keys.
            if quit {
                break;
            }
        }
        if app.demand() != demanded {
            demanded = app.demand();
            collector.demand(demanded);
        }
        if options
            .duration
            .is_some_and(|duration| origin.elapsed().as_secs_f64() >= duration)
        {
            quit = true;
        }
        app.scene.spare = frame.release();
    }
    drop(terminal);
    if options.duration.is_some() {
        let seconds = origin.elapsed().as_secs_f64();
        eprintln!(
            "isotop: {frames} frames in {seconds:.2}s | {:.1} presented fps | {:.2} MiB/s inline graphics (display latency not measured)",
            frames as f64 / seconds,
            total_wire as f64 / 1048576.0 / seconds
        );
    }
    Ok(0)
}

/// The demo workload at `time` for `count` processes, with what `row`, the shown view's registry
/// row, asks the demo to synthesize. Every demo snapshot goes through here, so a view sees the
/// same workload whether it is the first sample, a warm-up replay or a live one.
fn demo_snapshot(row: &ViewInfo, time: f64, count: usize) -> Snapshot {
    model::demo_with(time, count, row.demo)
}

/// Settles what a headless `--output` or `--benchmark` frame depends on before it is captured:
/// the shared history, then the shown view's warm-up phases (`ViewInfo::warmup`) whose condition
/// holds. The golden render test runs this too, so the hashes it checks are the PNGs' frames.
fn warm_up(app: &mut App, options: &Options, journal: Option<&Journal>) {
    replay_history(app, options);
    run_phases(app, options, journal);
}

/// Whether a warm-up phase runs in this run.
fn due(warmup: &Warmup, options: &Options) -> bool {
    match warmup.when {
        When::DemoOutput => options.demo && options.output.is_some(),
        When::Output => options.output.is_some(),
        When::Headless => options.headless(),
    }
}

/// Fills the scene's shared history, which a live run has by the time a view opens, with the
/// demo samples of the `history::SERIES` whole seconds before `--time`, skipping those before 0.
/// The samples go straight into `scene.history`, not through `Scene::record`, so no view's own
/// record hook sees them and the views that warm up through `Phase::Record` are unchanged. A row
/// whose due `Phase::History` already fills the history while it rasterizes skips the replay.
fn replay_history(app: &mut App, options: &Options) {
    let row = app.row();
    let filled = row
        .warmup
        .iter()
        .any(|warmup| matches!(warmup.phase, Phase::History { .. }) && due(warmup, options));
    if !options.demo || !options.headless() || filled {
        return;
    }
    let count = options.processes as usize;
    for back in (1..=history::SERIES).rev() {
        let at = options.time - back as f64;
        if at >= 0.0 {
            app.scene
                .history
                .record(&demo_snapshot(row, at, count), at as f32);
        }
    }
}

/// Runs the shown view's warm-up phases whose condition holds.
fn run_phases(app: &mut App, options: &Options, journal: Option<&Journal>) {
    let (width, height, limit) = (
        options.width,
        options.headless_height(),
        options.limit as usize,
    );
    let count = options.processes as usize;
    let row = app.row();
    for warmup in row.warmup {
        if !due(warmup, options) {
            continue;
        }
        match warmup.phase {
            Phase::History { seconds } => {
                for back in (1..=seconds).rev() {
                    let at = options.time - back as f64;
                    app.history = VecDeque::from([demo_snapshot(row, at, count)]);
                    app.animation = at as f32;
                    let frame = app.render(width, height, limit);
                    app.scene.spare = frame.release();
                }
                app.history = VecDeque::from([demo_snapshot(row, options.time, count)]);
            }
            Phase::Record { seconds } => {
                for back in (1..=seconds).rev() {
                    let at = options.time - back as f64;
                    if at >= 0.0 {
                        app.scene.record(&demo_snapshot(row, at, count), at as f32);
                    }
                }
            }
            Phase::Settle { frames, step, lead } => {
                for index in 0..frames {
                    app.animation = options.time as f32 - lead + index as f32 * step;
                    let frame = app.render(width, height, limit);
                    app.scene.spare = frame.release();
                }
            }
            Phase::Journal { frames, step, lead } => {
                let mut fed = options.time - lead;
                for index in 0..frames {
                    let at = options.time - lead + index as f64 * step;
                    feed(app, journal, options.demo, fed, at);
                    fed = at;
                    app.animation = at as f32;
                    let frame = app.render(width, height, limit);
                    app.scene.spare = frame.release();
                }
            }
        }
    }
}

/// Hands the matrix view its journal lines: synthetic ones arriving in `(from, to]` demo seconds,
/// or whatever `journalctl` has written since the last call.
fn feed(app: &mut App, source: Option<&Journal>, demo: bool, from: f64, to: f64) {
    let lines = match (demo, source) {
        (true, _) => journal::demo(from, to),
        (false, Some(source)) => source.drain(),
        (false, None) => Vec::new(),
    };
    for line in lines {
        app.scene.matrix.push(line);
    }
    if let Some(source) = source {
        app.scene.matrix.skipped += source.dropped();
    }
}

fn main() {
    match run(Options::parse()) {
        Ok(0) => {}
        Ok(status) => std::process::exit(status),
        Err(error) => {
            eprintln!("isotop: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DemoOptions;
    use crate::simulation::hash::fnv1a_64_bytes as fnv1a;
    use crate::views::{testing, view};

    /// Every registered view available, as on Linux and macOS today.
    fn all_available() -> Vec<bool> {
        vec![true; REGISTRY.len()]
    }

    #[test]
    fn labels_on_one_row_keep_a_free_column_between_them() {
        let mut board = Board::default();
        assert!(board.claim(10, 4, 5, 1));
        assert!(!board.claim(15, 4, 5, 1), "touching on the right");
        assert!(!board.claim(5, 4, 5, 1), "touching on the left");
        assert!(board.claim(16, 4, 5, 1), "one free column is enough");
        assert!(board.claim(4, 4, 5, 1), "one free column on the left");
        assert!(board.claim(10, 5, 5, 1), "other rows are not neighbours");
        assert!(!board.claim(12, 4, 2, 1), "overlap is still refused");
    }

    #[test]
    fn labels_keep_a_free_column_from_panels_but_panels_may_touch() {
        let mut board = Board::default();
        assert!(board.reserve(20, 2, 10, 6));
        assert!(board.reserve(30, 2, 5, 6), "panels keep their old rule");
        assert!(!board.reserve(32, 2, 5, 6), "panels still never overlap");
        assert!(!board.claim(35, 4, 6, 1), "label against a panel's edge");
        assert!(!board.claim(14, 4, 6, 1), "label ending at a panel's edge");
        assert!(board.claim(36, 4, 6, 1));
        assert!(board.claim(13, 4, 6, 1));
    }

    #[test]
    fn long_process_names_are_cut_in_labels_but_not_in_the_hover_tag() {
        let mut snapshot = model::demo(1.0, 64);
        for process in &mut snapshot.processes {
            process.name = "com.google.BatteriesAvocadoWidgetExtension".into();
            process.cpu = 30.0;
        }
        let hovered = snapshot.processes[0].id;
        let mut app = App::new(view("orbit"), snapshot, all_available());
        let mut frame = app.render(1600, 900, 512);
        frame.rasterize();
        let layout = Layout {
            columns: 200,
            rows: 56,
            width: 1600,
            height: 900,
            cell: None,
        };
        let at = app
            .screen(&frame, hovered)
            .expect("the process is on screen");
        app.hover = Some(Pointer {
            frame: at,
            cell: layout.cell_of(at),
        });
        let (labels, _) = app.overlay(&mut frame, &layout, layout.pick_radius(false));
        let busy: Vec<&Label> = labels.iter().filter(|l| l.tone == Tone::Quiet).collect();
        assert!(!busy.is_empty());
        for label in &busy {
            assert!(label.text.starts_with("com.google.Ba.. "), "{}", label.text);
            assert!(label.text.chars().count() <= 15 + 5, "{}", label.text);
        }
        let tags: Vec<&Label> = labels.iter().filter(|l| l.tone == Tone::Tag).collect();
        assert_eq!(tags.len(), 1, "the hovered process has a tag");
        assert!(
            tags[0].text.contains("WidgetExtension"),
            "the tag keeps the full name"
        );
        for label in labels.iter().filter(|l| l.tone == Tone::Bright) {
            assert!(!label.text.contains("WidgetExtension"), "{}", label.text);
        }
    }

    #[test]
    fn a_hub_is_labelled_as_unreadable_and_explained_only_while_one_is_drawn() {
        let mut snapshot = model::demo(1.0, 20);
        snapshot.links.clear();
        for (index, process) in snapshot.processes.iter_mut().enumerate() {
            process.id = Identity {
                pid: 100 + index as u32,
                start: 1,
            };
            process.parent = 1;
        }
        let plain = snapshot.clone();
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
            let mut frame = app.render(1600, 900, 512);
            frame.rasterize();
            let (labels, _) = app.overlay(&mut frame, &layout, layout.pick_radius(false));
            let stars: Vec<&str> = labels
                .iter()
                .filter(|label| label.tone == Tone::Bright)
                .map(|label| label.text.as_str())
                .collect();
            assert_eq!(stars, ["launchd (unreadable) (20)"], "{view:?}");
            let legend = &app.text(0.0, "test", 20, 512)[2];
            assert!(
                legend.ends_with(" | hollow star = a parent isotop can't read (run with sudo)"),
                "{legend}"
            );
            app.focus = Some(snapshot.processes[0].id);
            app.render(1600, 900, 512);
            let legend = &app.text(0.0, "test", 20, 512)[2];
            assert!(!legend.contains("hollow star"), "focused: {legend}");
            let mut app = App::new(view, plain.clone(), all_available());
            app.render(1600, 900, 512);
            let legend = &app.text(0.0, "test", 20, 512)[2];
            assert!(!legend.contains("hollow star"), "{legend}");
        }
    }

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
            assert!(!app.scene.hub_names.is_empty(), "{view:?} draws the hub");
            let targets = app.tour_targets();
            assert!(!targets.is_empty());
            for target in &targets {
                assert!(
                    snapshot.processes.iter().any(|p| p.id == *target),
                    "{view:?} visits {target:?}"
                );
            }
            app.visit(0);
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
        assert_eq!(app.tour.as_ref().unwrap().index, 1, "steps without --tour");
        press(&mut app, 'g');
        assert!(app.tour.is_none(), "g again ends it");
        press(&mut app, 'g');
        press(&mut app, '+');
        assert!(app.tour.is_none(), "any other key ends it");
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
    fn shift_tab_steps_back_through_the_views_and_wraps() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.key(KeyCode::BackTab, KeyModifiers::SHIFT, 10.0);
        assert_eq!(app.view, View::all().last().unwrap());
        app.key(KeyCode::Tab, KeyModifiers::NONE, 10.0);
        assert_eq!(app.view, view("city"));
    }

    #[test]
    fn matrix_status_ignores_a_process_selected_in_another_view() {
        let snapshot = model::demo(1.0, 16);
        let selected = snapshot.processes[0].id;
        let mut app = App::new(view("matrix"), snapshot, all_available());
        app.selected = Some(selected);
        app.scene
            .matrix
            .push(journal::Line::new(6, "sshd[1]: hello"));
        let text = app.text(0.0, "test", 20, 512);
        assert_eq!(text[1], " > sshd[1]: hello");
        assert!(text[2].contains("queued"), "{}", text[2]);
        let frame = app.render(320, 180, 512);
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
    fn memory_is_labelled_with_what_the_platform_measures() {
        let snapshot = model::demo(1.0, 16);
        let selected = snapshot.processes[0].id;
        let mut app = App::new(view("city"), snapshot, all_available());
        let label = platform::MEMORY_LABEL;
        assert!(app.text(0.0, "test", 20, 512)[2].contains(&format!("footprint = {label} |")));
        app.selected = Some(selected);
        let text = app.text(0.0, "test", 20, 512);
        assert!(text[1].contains(&format!("%  {label} ")), "{}", text[1]);
        let frame = app.render(320, 180, 512);
        let layout = Layout {
            columns: 120,
            rows: 40,
            width: 320,
            height: 180,
            cell: None,
        };
        let popup = app
            .popup(&frame, &layout)
            .expect("a popup for the selection");
        assert!(
            popup
                .lines
                .iter()
                .any(|line| line.contains(&format!("| {label} "))),
            "{:?}",
            popup.lines
        );
        // macOS reports the physical footprint, which is not a resident set size.
        if cfg!(target_os = "macos") {
            assert!(!text[1].contains("RSS") && popup.lines.iter().all(|l| !l.contains("RSS")));
        }
    }

    #[test]
    fn h_hides_the_panel_except_while_typing_a_search() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        let press = |app: &mut App, c: char| app.key(KeyCode::Char(c), KeyModifiers::NONE, 10.0);
        assert!(app.panel_shown());
        press(&mut app, 'h');
        assert!(!app.panel_shown());
        press(&mut app, '/');
        assert!(app.panel_shown(), "the search line stays visible");
        press(&mut app, 'h');
        assert_eq!(app.search.as_deref(), Some("h"), "h is typed, not a toggle");
        app.key(KeyCode::Esc, KeyModifiers::NONE, 10.0);
        assert!(!app.panel_shown(), "hidden again after the search");
        press(&mut app, 'h');
        assert!(app.panel_shown());
    }

    #[test]
    fn camera_follows_growing_views_until_moved_by_hand() {
        let mut app = App::new(view("cells"), model::demo(1.0, 64), all_available());
        app.render(320, 180, 512);
        assert!(app.framed && app.scene.bounded());
        app.key(KeyCode::Left, KeyModifiers::NONE, 10.0);
        assert!(!app.framed, "panning takes over the camera");
        app.key(KeyCode::Char('8'), KeyModifiers::NONE, 10.0);
        assert!(
            app.framed && app.view == view("globe"),
            "switching view frames it again"
        );
    }

    /// The macOS gaps, as the collector reports them on a Mac without cluster clocks.
    const MACOS: [Source; 9] = [
        Source::CpuPressure,
        Source::IoPressure,
        Source::LastCpu,
        Source::CpuClock,
        Source::RunQueue,
        Source::Cgroups,
        Source::SocketTraffic,
        Source::FileLocks,
        Source::GpuMemory,
    ];

    /// The status text of `view` over the demo workload with the given gaps, after one frame so
    /// views that learn the gaps while drawing have seen them.
    fn lines_with(view: View, missing: &[Source], unreadable: usize) -> Vec<String> {
        let mut snapshot = model::demo(1.0, 16);
        snapshot.missing = missing.iter().copied().collect();
        snapshot.unreadable = unreadable;
        if missing.contains(&Source::LastCpu) {
            // A Mac without the rusage counters that place marbles by cluster: no share of
            // performance-core time and no waiting threads.
            for process in &mut snapshot.processes {
                process.performance_share = None;
                process.waiting = None;
            }
        }
        if missing.contains(&Source::FileLocks) {
            // No lock table: every process's locks are unreadable and none waits on one.
            for process in &mut snapshot.processes {
                process.locks_held = model::Measured::Unreadable;
                process.blocked_on = None;
            }
            snapshot.unattributed_locks = 0;
        }
        let mut app = App::new(view, snapshot, all_available());
        app.render(320, 180, 512);
        app.text(0.0, "test", 20, 512)
    }

    #[test]
    fn status_line_shows_n_a_for_memory_it_could_not_read() {
        let status = |total: Option<u64>, available: Option<u64>| {
            let mut snapshot = model::demo(1.0, 16);
            (snapshot.memory_total, snapshot.memory_available) = (total, available);
            let mut app = App::new(view("city"), snapshot, all_available());
            app.render(320, 180, 512);
            app.text(0.0, "test", 20, 512).swap_remove(0)
        };
        let line = status(Some(3 << 30), Some(1 << 30));
        assert!(
            line.ends_with(&format!("RAM {} / {}", bytes(2 << 30), bytes(3 << 30))),
            "{line}"
        );
        let line = status(Some(3 << 30), None);
        assert!(
            line.ends_with(&format!("RAM n/a / {}", bytes(3 << 30))),
            "{line}"
        );
        let line = status(None, None);
        assert!(line.ends_with("RAM n/a / n/a"), "{line}");
    }

    #[test]
    fn status_line_shows_n_a_for_each_missing_pressure() {
        let [cpu, memory, io] = model::demo(1.0, 16)
            .pressure
            .map(|value| format!("{value:.0}%"));
        let full = lines_with(view("city"), &[], 0);
        assert!(
            full[4].contains(&format!("pressure cpu {cpu} mem {memory} io {io}")),
            "{}",
            full[4]
        );
        let mac = lines_with(view("city"), &MACOS, 0);
        assert!(
            mac[4].contains(&format!("pressure cpu n/a mem {memory} io n/a")),
            "{}",
            mac[4]
        );
        let none = lines_with(
            view("city"),
            &[
                Source::CpuPressure,
                Source::MemoryPressure,
                Source::IoPressure,
            ],
            0,
        );
        assert!(
            none[4].contains("pressure cpu n/a mem n/a io n/a"),
            "{}",
            none[4]
        );
    }

    #[test]
    fn status_line_says_when_the_background_sampler_stopped_or_stalled() {
        let status = |sampler: model::SamplerHealth| {
            let mut snapshot = model::demo(1.0, 16);
            snapshot.sampler = sampler;
            App::new(view("city"), snapshot, all_available()).text(0.0, "test", 20, 512)[0].clone()
        };
        let healthy = status(model::demo(1.0, 16).sampler);
        assert!(!healthy.contains("sampler"), "{healthy}");
        let stopped = status(model::SamplerHealth::default());
        assert!(
            stopped.contains(" processes (background sampler stopped: "),
            "{stopped}"
        );
        let stalled = status(model::SamplerHealth {
            running: true,
            last_cycle: Some(Duration::from_secs(95)),
            quiet: Duration::from_secs(95),
            scan_latency: Vec::new(),
        });
        assert!(
            stalled.contains("(background sampler stalled 95s)"),
            "{stalled}"
        );
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

    #[test]
    fn status_line_counts_unreadable_processes_only_when_there_are_some() {
        let none = lines_with(view("city"), &[], 0);
        assert!(!none[0].contains("unreadable") && !none[4].contains("unreadable"));
        let one = lines_with(view("city"), &[], 1);
        assert!(
            one[0].contains(" processes (+1 unreadable: run with sudo) | "),
            "{}",
            one[0]
        );
        let many = lines_with(view("city"), &MACOS, 37);
        assert!(
            many[0].contains(" processes (+37 unreadable: run with sudo) | "),
            "{}",
            many[0]
        );
        assert!(!many[4].contains("unreadable"), "{}", many[4]);
    }

    #[test]
    fn unreadable_notice_survives_an_80_column_terminal() {
        let first = &lines_with(view("city"), &MACOS, 212)[0];
        let visible: String = first.chars().take(80).collect();
        assert!(
            visible.contains("(+212 unreadable: run with sudo)"),
            "{visible}"
        );
    }

    #[test]
    fn legends_name_what_the_platform_cannot_measure() {
        let cores = lines_with(view("cores"), &MACOS, 0)[2].clone();
        assert!(
            cores.contains("lanes = load; macOS reports no last CPU per process, so no marbles"),
            "{cores}"
        );
        assert!(cores.contains("no clock readings"), "{cores}");
        assert!(!cores.contains("marble ="), "{cores}");
        let cells = lines_with(view("cells"), &MACOS, 0)[2].clone();
        assert!(
            cells.contains(&format!(
                "{}, so no limits, quotas or pressure",
                platform::WITHOUT_CGROUPS
            )),
            "{cells}"
        );
        let orbit = lines_with(view("orbit"), &MACOS, 0)[2].clone();
        assert!(
            orbit.contains("glow + trail = CPU | cyan arcs") && !orbit.contains("NVIDIA"),
            "{orbit}"
        );
        let globe = lines_with(view("globe"), &MACOS, 0)[2].clone();
        assert!(
            globe.contains("no per-connection rates or RTT on macOS"),
            "{globe}"
        );
        let coop = lines_with(view("coop"), &MACOS, 0)[2].clone();
        assert!(
            coop.contains("no last CPU: running chickens share one trough"),
            "{coop}"
        );
        assert!(coop.contains("no CPU pressure"), "{coop}");
    }

    #[test]
    fn coop_legend_names_app_groups_and_drops_foxes_without_cgroups() {
        let mac = lines_with(view("coop"), &MACOS, 0)[2].clone();
        assert!(
            mac.contains(&format!("flock = {} | ", platform::GROUP_LABEL)),
            "{mac}"
        );
        assert!(
            mac.contains("dust = reads | eyes = memory pressure"),
            "{mac}"
        );
        assert!(mac.contains("no cgroups: no OOM foxes"), "{mac}");
        assert!(
            !mac.contains("flock = cgroup") && !mac.contains("fox ="),
            "{mac}"
        );
        let linux = lines_with(view("coop"), &[], 0)[2].clone();
        assert!(
            linux.contains("flock = cgroup | ") && linux.contains("fox = OOM kill, eyes = "),
            "{linux}"
        );
        assert!(!linux.contains("no cgroups"), "{linux}");
    }

    #[test]
    fn coop_legend_on_macos_counts_open_files_but_has_no_brooding() {
        let mac = lines_with(view("coop"), &MACOS, 0)[2].clone();
        assert!(
            mac.contains("eggs = open files (log2), cracked = deleted but open | chicks = threads"),
            "{mac}"
        );
        assert!(
            mac.contains("no file lock table: no brooding or queueing"),
            "{mac}"
        );
        assert!(
            !mac.contains("brooding =") && !mac.contains("file locks unreadable"),
            "{mac}"
        );
        assert!(!mac.contains("without a process (OFD)"), "{mac}");
        let linux = lines_with(view("coop"), &[], 0)[2].clone();
        assert!(
            linux.contains("brooding = holds a file lock, queue at a nest = blocked on one"),
            "{linux}"
        );
        assert!(!linux.contains("no file lock table"), "{linux}");
    }

    #[test]
    fn legends_keep_their_full_text_when_nothing_is_missing() {
        let text = |view| lines_with(view, &[], 0)[2].clone();
        assert!(
            text(view("cores")).contains("chevrons = clock | red queue = waiting tasks | marble =")
        );
        assert!(text(view("cells")).contains("dashed ring = memory limit | arc = CPU vs quota"));
        assert!(text(view("globe")).contains("brighter with traffic | cyan = mostly download"));
        assert!(
            text(view("orbit")).contains("glow + trail = CPU | green = NVIDIA GPU | cyan arcs")
        );
        assert!(!text(view("coop")).contains("share one trough"));
    }

    /// Options as `isotop` parses `arguments`.
    fn options(arguments: &[&str]) -> Options {
        Options::parse_from(std::iter::once("isotop").chain(arguments.iter().copied()))
    }

    /// What `preflight` returns and writes to standard output and error for `arguments`, with
    /// the views named in `missing` unavailable.
    fn preflight_with(arguments: &[&str], missing: &[&str]) -> (Preflight, String, String) {
        let available: Vec<bool> = View::all()
            .map(|view| !missing.contains(&view.name()))
            .collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = preflight(&options(arguments), &available, &mut out, &mut err);
        let text = |bytes: Vec<u8>| String::from_utf8(bytes).expect("the output is text");
        (result, text(out), text(err))
    }

    /// An app over the test module views of `views::testing::ROWS`, showing `probe`.
    fn probe_app() -> App {
        let rows = views::testing::ROWS;
        let mut app = App::new(View::at(0), model::demo(1.0, 16), vec![true; rows.len()]);
        app.scene = Scene::with_rows(rows);
        app.render(320, 180, 512);
        app
    }

    #[test]
    fn demo_snapshots_follow_the_shown_views_options() {
        let rows = views::testing::DEMO_ROWS;
        let (plain, demanding) = (View::at(0), View::at(1));
        let asked = DemoOptions {
            churn: true,
            exact_cpu_time: true,
        };
        let mut app = App::new(plain, model::demo(1.0, 16), vec![true; rows.len()]);
        app.scene = Scene::with_rows(rows);
        assert_eq!(app.row().demo, DemoOptions::default());
        assert_eq!(app.scene.row(demanding).demo, asked);
        let processes = |app: &App, time: f64| {
            let row = app.row();
            demo_snapshot(row, time, 64).processes
        };
        // The first view asks for nothing, so it gets `demo` exactly.
        assert_eq!(
            format!("{:?}", processes(&app, 40.0)),
            format!("{:?}", model::demo(40.0, 64).processes)
        );
        // Switching to a view that asks for more changes the next sample, not the history.
        let kept = app.history.len();
        app.switch(demanding);
        assert_eq!(app.history.len(), kept, "the history is kept as it was");
        let busy = processes(&app, 40.0);
        assert!(busy.len() > 64, "transients follow the base processes");
        assert_eq!(
            f64::from(busy[5].cpu_time),
            f64::from(model::demo_cpu_seconds(40.0, 5) as f32)
        );
        assert_eq!(
            format!("{:?}", demo_snapshot(app.row(), 40.0, 64).processes),
            format!("{:?}", model::demo_with(40.0, 64, asked).processes)
        );
        // And back again.
        app.switch(plain);
        assert_eq!(processes(&app, 40.0).len(), 64);
        // The headless warm-up replays history through the same options.
        app.switch(demanding);
        let options = Options::parse_from([
            "isotop",
            "--demo",
            "--time",
            "12",
            "--processes",
            "64",
            "--width",
            "160",
            "--output",
            "warm.png",
        ]);
        warm_up(&mut app, &options, None);
        assert_eq!(app.history.len(), 1);
        assert_eq!(
            format!("{:?}", app.history[0].processes),
            format!("{:?}", model::demo_with(12.0, 64, asked).processes)
        );
    }

    /// The frame a headless demo run of `name` captures, and the app it came from.
    fn warmed_frame(name: &str, replay: bool) -> (App, Vec<u8>) {
        let options = Options::parse_from([
            "isotop",
            "--demo",
            "--time",
            "75",
            "--processes",
            "64",
            "--width",
            "160",
            "--renderer",
            "cpu",
            "--output",
            "warm.png",
            "--view",
            name,
        ]);
        let view = View::all().find(|view| view.name() == name).unwrap();
        let mut app = App::new(
            view,
            demo_snapshot(view.info(), options.time, 64),
            all_available(),
        );
        if replay {
            warm_up(&mut app, &options, None);
        } else {
            run_phases(&mut app, &options, None);
        }
        app.animation = options.time as f32;
        let frame = app.render(160, options.headless_height(), options.limit as usize);
        (app, frame.pixels.to_vec())
    }

    #[test]
    fn headless_replay_leaves_coop_warm_up_unchanged() {
        let (replayed, with) = warmed_frame("coop", true);
        let (bare, without) = warmed_frame("coop", false);
        assert!(with == without, "the replay changed coop's frame");
        // The replay fills the shared history, one sample a second before the captured one; coop's
        // own `Record` phase reaches it through `Scene::record` for its 30 seconds only.
        assert_eq!(replayed.scene.history.window(75.0, 60.0).count(), 61);
        assert_eq!(bare.scene.history.window(75.0, 60.0).count(), 31);
        // Strata fills it by rasterizing, so it is not replayed on top.
        let (strata, _) = warmed_frame("strata", true);
        assert_eq!(strata.scene.history.window(75.0, 60.0).count(), 61);
    }

    #[test]
    fn registry_names_are_unique_lowercase_and_parse_on_the_command_line() {
        let names: Vec<&str> = View::all().map(View::name).collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        for view in View::all() {
            let name = view.name();
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()),
                "{name}"
            );
            assert_eq!(options(&["--view", name]).view, Some(view));
        }
        let error = Options::try_parse_from(["isotop", "--view", "atlantis"])
            .err()
            .expect("an unknown view is a usage error");
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
        assert!(
            error
                .to_string()
                .contains("[possible values: city, orbit, ripple"),
            "{error}"
        );
    }

    #[test]
    fn existing_views_keep_their_names_titles_tab_order_and_digits() {
        let existing = [
            ("city", "CITY"),
            ("orbit", "ORBIT"),
            ("ripple", "RIPPLE"),
            ("flow", "FLOW"),
            ("cores", "CORES"),
            ("cells", "CELLS"),
            ("strata", "STRATA"),
            ("globe", "GLOBE"),
            ("reef", "REEF"),
            ("matrix", "MATRIX"),
            ("coop", "COOP"),
        ];
        let rows: Vec<(&str, &str)> = REGISTRY.iter().map(|row| (row.name, row.title)).collect();
        assert_eq!(rows[..existing.len()], existing);
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        let press = |app: &mut App, code: KeyCode| app.key(code, KeyModifiers::NONE, 10.0);
        for (name, title) in existing {
            assert_eq!(app.view.name(), name);
            assert!(
                app.text(0.0, "test", 20, 512)[0].starts_with(&format!(" ISOTOP / {title} / "))
            );
            press(&mut app, KeyCode::Tab);
        }
        for (digit, (name, _)) in "1234567890".chars().zip(existing) {
            press(&mut app, KeyCode::Char(digit));
            assert_eq!(app.view.name(), name, "digit {digit}");
        }
    }

    #[test]
    fn view_defaults_to_city() {
        assert_eq!(options(&[]).view, None);
        assert_eq!(preflight_with(&[], &[]).0, Preflight::Start(view("city")));
    }

    #[test]
    fn tab_skips_views_that_are_unavailable() {
        let shown = ["city", "flow", "matrix"];
        let available = View::all()
            .map(|view| shown.contains(&view.name()))
            .collect();
        let mut app = App::new(view("city"), model::demo(1.0, 16), available);
        let press = |app: &mut App, code: KeyCode| {
            app.key(code, KeyModifiers::NONE, 10.0);
            app.view.name()
        };
        assert_eq!(press(&mut app, KeyCode::Tab), "flow");
        assert_eq!(press(&mut app, KeyCode::Tab), "matrix");
        assert_eq!(press(&mut app, KeyCode::Tab), "city", "wraps round");
        assert_eq!(press(&mut app, KeyCode::BackTab), "matrix");
        assert_eq!(press(&mut app, KeyCode::BackTab), "flow");
        let mut alone = App::new(
            view("reef"),
            model::demo(1.0, 16),
            vec![false; REGISTRY.len()],
        );
        alone.available[view("reef").index()] = true;
        assert_eq!(
            press(&mut alone, KeyCode::Tab),
            "reef",
            "nowhere else to go"
        );
    }

    #[test]
    fn a_digit_bound_to_an_unavailable_row_does_nothing() {
        let mut available = all_available();
        available[view("orbit").index()] = false;
        let mut app = App::new(view("cells"), model::demo(1.0, 16), available);
        app.render(320, 180, 512);
        assert!(!app.fit);
        app.key(KeyCode::Char('2'), KeyModifiers::NONE, 10.0);
        assert_eq!((app.view, app.fit), (view("cells"), false));
        app.key(KeyCode::Char('3'), KeyModifiers::NONE, 10.0);
        assert_eq!((app.view, app.fit), (view("ripple"), true));
    }

    #[test]
    fn verbose_names_each_unavailable_view_once() {
        let (result, out, err) = preflight_with(&["--verbose"], &["orbit", "reef"]);
        assert_eq!(result, Preflight::Start(view("city")));
        assert_eq!(out, "");
        assert_eq!(
            err,
            "skipping view orbit: not supported on this platform\n\
             skipping view reef: not supported on this platform\n"
        );
        assert_eq!(
            preflight_with(&[], &["orbit"]).2,
            "",
            "silent without --verbose"
        );
        assert_eq!(preflight_with(&["--verbose"], &[]).2, "");
    }

    #[test]
    fn list_views_prints_only_available_views_in_registry_order() {
        let (result, out, err) = preflight_with(&["--list-views"], &["orbit", "matrix"]);
        assert_eq!(result, Preflight::Exit(0));
        let expected: String = REGISTRY
            .iter()
            .filter(|row| !["orbit", "matrix"].contains(&row.name))
            .map(|row| format!("{}\n", row.name))
            .collect();
        assert_eq!(out, expected);
        assert!(out.starts_with("city\nripple\nflow\n"), "{out}");
        assert_eq!(err, "");
        let (_, every, _) = preflight_with(&["--list-views", "--view", "globe"], &[]);
        assert_eq!(
            every.lines().collect::<Vec<_>>(),
            View::all().map(View::name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_unavailable_view_on_the_command_line_falls_back_to_the_first_available() {
        let (result, out, err) = preflight_with(&["--view", "orbit"], &["city", "orbit"]);
        assert_eq!(result, Preflight::Start(view("ripple")));
        assert_eq!((out.as_str(), err.as_str()), ("", ""));
        assert_eq!(
            preflight_with(&["--view", "coop"], &["city"]).0,
            Preflight::Start(view("coop")),
            "an available view starts as asked"
        );
    }

    #[test]
    fn an_unavailable_view_in_a_headless_run_exits_with_status_one() {
        for headless in [["--output", "orbit.png"], ["--benchmark", "5"]] {
            let arguments = [&["--view", "orbit"][..], &headless].concat();
            let (result, out, err) = preflight_with(&arguments, &["orbit"]);
            assert_eq!(result, Preflight::Exit(1), "{arguments:?}");
            assert_eq!(out, "");
            assert_eq!(
                err,
                "isotop: view orbit is not supported on this platform\n"
            );
        }
        let (result, _, err) = preflight_with(&["--view", "orbit", "--output", "x.png"], &["city"]);
        assert_eq!(
            (result, err.as_str()),
            (Preflight::Start(view("orbit")), "")
        );
    }

    #[test]
    fn a_headless_run_without_view_falls_back_when_the_default_is_unavailable() {
        for headless in [["--output", "x.png"], ["--benchmark", "5"]] {
            let (result, out, err) = preflight_with(&headless, &["city"]);
            assert_eq!(result, Preflight::Start(view("orbit")), "{headless:?}");
            assert_eq!((out.as_str(), err.as_str()), ("", ""));
        }
    }

    #[test]
    fn view_local_keys_reach_only_the_shown_view_and_never_during_search() {
        let mut app = probe_app();
        let (probe, quiet) = (View::at(0), View::at(1));
        let press = |app: &mut App, code: KeyCode| app.key(code, KeyModifiers::NONE, 10.0);
        let keys = |app: &App, view: View| {
            let legend = app.scene.legend(view, app.snapshot());
            let start = legend.find("keys [").expect("the probe lists its keys") + 6;
            legend[start..legend.len() - 1].to_owned()
        };
        for character in "x,z".chars() {
            press(&mut app, KeyCode::Char(character));
        }
        let center = app.goal.center;
        press(&mut app, KeyCode::Char('a'));
        assert_ne!(app.goal.center, center, "a global key keeps its meaning");
        assert_eq!(
            (keys(&app, probe), keys(&app, quiet)),
            ("x,z".into(), "".into())
        );
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('y'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            (keys(&app, probe), keys(&app, quiet)),
            ("x,z".into(), "y".into())
        );
        for code in [KeyCode::Char('/'), KeyCode::Char('b'), KeyCode::Char('k')] {
            press(&mut app, code);
        }
        assert_eq!(app.search.as_deref(), Some("bk"), "typed into the search");
        press(&mut app, KeyCode::Esc);
        assert_eq!(
            keys(&app, probe),
            "x,z",
            "nothing reached the view during the search"
        );
        app.fit = false;
        press(&mut app, KeyCode::Char('r'));
        assert!(app.fit, "the global reset happens as well");
        press(&mut app, KeyCode::Home);
        assert_eq!(keys(&app, probe), "x,zRR");
        // The probe's mode takes the first Esc; the next one closes the selection as usual.
        let selected = app.snapshot().processes[0].id;
        app.selected = Some(selected);
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(
            app.selected,
            Some(selected),
            "the view consumed the first Esc"
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.selected, None);
        assert_eq!(keys(&app, probe), "x,zRRmEE");
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

    /// An app over `views::testing::GUIDED`, showing `guide`, and the process in its last seat,
    /// which the guide tours. That process is made idle and small, so that nothing but the
    /// guide's target puts it first.
    fn guided_app() -> (App, Identity) {
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
    fn popup_lines_from_stage_outputs_reach_the_inspector() {
        let (mut app, last) = guided_app();
        let layout = Layout {
            columns: 80,
            rows: 24,
            width: 320,
            height: 180,
            cell: None,
        };
        let popup = |app: &mut App, id: Identity| {
            app.selected = Some(id);
            let frame = app.render(320, 180, 512);
            app.popup(&frame, &layout)
                .expect("the selection has a popup")
                .lines
        };
        let lines = popup(&mut app, last);
        let at = lines
            .iter()
            .position(|line| line == views::testing::GUIDE_LINE)
            .expect("the view's line is in the popup");
        assert!(
            lines[at - 1].starts_with("CPU "),
            "right after the process's own figures: {lines:?}"
        );
        let other = app.snapshot().processes[0].id;
        assert!(
            !popup(&mut app, other)
                .iter()
                .any(|line| line == views::testing::GUIDE_LINE)
        );
    }

    /// The checked-in golden hashes, which are those of Linux on x86_64.
    const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden.txt");

    /// FNV-1a 64 of a render's colour, pick and depth buffers.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Hashes {
        pixels: u64,
        picks: u64,
        depth: u64,
    }

    /// The frame `isotop --demo --time 12 --renderer cpu --width 480 --output` writes for `view`.
    fn golden_render(view: View) -> Hashes {
        let name = view.name();
        let options = Options::parse_from([
            "isotop",
            "--demo",
            "--time",
            "12",
            "--renderer",
            "cpu",
            "--width",
            "480",
            "--output",
            "golden.png",
            "--view",
            name,
        ]);
        let mut app = App::new(
            view,
            demo_snapshot(view.info(), options.time, options.processes as usize),
            all_available(),
        );
        // The matrix greets `$USER`, which differs between machines.
        app.scene.matrix.user = Some("Neo".into());
        warm_up(&mut app, &options, None);
        app.animation = options.time as f32;
        let frame = app.render(
            options.width,
            options.headless_height(),
            options.limit as usize,
        );
        Hashes {
            pixels: fnv1a(frame.pixels.iter().copied()),
            picks: crate::simulation::hash::fnv1a_64_bytes(
                frame.picks.iter().flat_map(|pick| pick.to_le_bytes()),
            ),
            depth: fnv1a(
                frame
                    .depth()
                    .iter()
                    .flat_map(|depth| depth.to_bits().to_le_bytes()),
            ),
        }
    }

    /// Every registered view's golden render, in registry order, each view on its own thread.
    fn render_every_view() -> Vec<(String, Hashes)> {
        std::thread::scope(|scope| {
            let renders: Vec<_> = View::all()
                .map(|view| scope.spawn(move || (view.name().to_owned(), golden_render(view))))
                .collect();
            renders
                .into_iter()
                .map(|render| render.join().expect("the view renders"))
                .collect()
        })
    }

    /// The renders shared by the tests below, so each view is rendered once for both of them.
    fn first_renders() -> &'static [(String, Hashes)] {
        static RENDERS: std::sync::OnceLock<Vec<(String, Hashes)>> = std::sync::OnceLock::new();
        RENDERS.get_or_init(render_every_view)
    }

    fn golden_line(hashes: &Hashes) -> String {
        format!(
            "{:016x} {:016x} {:016x}",
            hashes.pixels, hashes.picks, hashes.depth
        )
    }

    /// Compares `renders` with the golden file at `path`, or rewrites the file when `bless` is set.
    fn check_golden(
        path: &std::path::Path,
        renders: &[(String, Hashes)],
        bless: bool,
    ) -> Result<(), String> {
        if bless {
            let mut text = String::from(
                "# view pixels picks depth: FNV-1a 64 of `--demo --time 12 --renderer cpu --width 480`\n",
            );
            for (name, hashes) in renders {
                text += &format!("{name} {}\n", golden_line(hashes));
            }
            return std::fs::write(path, text)
                .map_err(|error| format!("{}: {error}", path.display()));
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let stored: HashMap<&str, Vec<&str>> = text
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                Some((fields.next()?, fields.collect()))
            })
            .collect();
        let mut changed = Vec::new();
        for (name, hashes) in renders {
            let rendered = golden_line(hashes);
            match stored.get(name.as_str()) {
                None => changed.push(format!("{name}: no golden hash")),
                Some(fields) => {
                    let buffers: Vec<&str> = ["pixels", "picks", "depth"]
                        .into_iter()
                        .zip(rendered.split(' '))
                        .enumerate()
                        .filter(|(index, (_, hash))| fields.get(*index) != Some(hash))
                        .map(|(_, (buffer, _))| buffer)
                        .collect();
                    if !buffers.is_empty() {
                        changed.push(format!("{name}: {} changed", buffers.join(", ")));
                    }
                }
            }
        }
        let mut retired: Vec<&str> = stored
            .keys()
            .copied()
            .filter(|name| !renders.iter().any(|(rendered, _)| rendered == name))
            .collect();
        retired.sort_unstable();
        changed.extend(
            retired
                .iter()
                .map(|name| format!("{name}: no longer a view")),
        );
        if changed.is_empty() {
            return Ok(());
        }
        Err(format!(
            "golden render hashes in {} differ:\n  {}\nIf the change is intended, rewrite the file with \
             `ISOTOP_BLESS=1 cargo test every_view_matches` and say why in the PR description, with the renders.",
            path.display(),
            changed.join("\n  ")
        ))
    }

    /// Renders every view as a headless `--output` run does and compares the FNV-1a 64 hashes of
    /// its colour, pick and depth buffers with `tests/golden.txt`. `ISOTOP_BLESS=1` rewrites it.
    ///
    /// Transcendental functions (`sin`, `exp`, `powf`) are not guaranteed to round the same way
    /// across libm implementations and architectures, so the checked-in hashes are those of Linux
    /// on x86_64, which CI's `ubuntu-latest` runs, and only that platform compares them. Elsewhere
    /// every view still renders, which catches panics, and the comparison is skipped.
    ///
    /// `ISOTOP_GOLDEN_FILE=<path>` reads, and in bless mode writes, that file instead, and compares
    /// on any platform: bless a local baseline before a change and compare against it after.
    #[test]
    fn every_view_matches_its_golden_hash() {
        let renders = first_renders();
        let bless = std::env::var_os("ISOTOP_BLESS").is_some_and(|value| value == "1");
        let (path, compare) = match std::env::var_os("ISOTOP_GOLDEN_FILE") {
            Some(path) => (PathBuf::from(path), true),
            None => (
                PathBuf::from(GOLDEN),
                cfg!(all(target_os = "linux", target_arch = "x86_64")),
            ),
        };
        if !compare {
            assert!(
                !bless,
                "the checked-in hashes are blessed on Linux x86_64 only; \
                 set ISOTOP_GOLDEN_FILE to bless a local baseline"
            );
            return;
        }
        if let Err(message) = check_golden(&path, renders, bless) {
            panic!("{message}");
        }
    }

    #[test]
    fn every_view_renders_the_same_frame_twice() {
        let second = render_every_view();
        for ((name, first), (_, second)) in first_renders().iter().zip(&second) {
            assert_eq!(
                first, second,
                "{name} drew a different frame from the same inputs"
            );
        }
    }

    #[test]
    fn bless_mode_rewrites_the_golden_file() {
        let path = std::env::temp_dir().join(format!("isotop-golden-{}.txt", std::process::id()));
        std::fs::write(&path, "city 0 0 0\nretired 0 0 0\n").expect("the file is written");
        let hashes = |pixels, picks, depth| Hashes {
            pixels,
            picks,
            depth,
        };
        let mut renders = vec![
            ("city".to_owned(), hashes(1, 2, 3)),
            ("orbit".to_owned(), hashes(4, 5, 6)),
        ];
        let stale = check_golden(&path, &renders, false);
        let blessed = check_golden(&path, &renders, true);
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        let fresh = check_golden(&path, &renders, false);
        renders[1].1.picks = 7;
        let changed = check_golden(&path, &renders, false);
        std::fs::remove_file(&path).ok();
        let stale = stale.expect_err("a stale file fails");
        assert!(
            stale.contains("city: pixels, picks, depth changed"),
            "{stale}"
        );
        assert!(stale.contains("orbit: no golden hash"), "{stale}");
        assert!(stale.contains("retired: no longer a view"), "{stale}");
        assert!(stale.contains("ISOTOP_BLESS=1"), "{stale}");
        assert_eq!(blessed, Ok(()));
        assert!(
            written.contains("\norbit 0000000000000004 0000000000000005 0000000000000006\n"),
            "{written}"
        );
        assert!(!written.contains("retired"), "{written}");
        assert_eq!(fresh, Ok(()));
        let changed = changed.expect_err("a changed buffer fails");
        assert!(changed.contains("orbit: picks changed"), "{changed}");
        assert!(!changed.contains("city:"), "{changed}");
    }

    /// Paths relative to `src/` that the first column of AGENTS.md's tables names. Only table rows
    /// count, so a path mentioned in prose does not.
    fn agents_table_paths() -> Vec<String> {
        let guide = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/AGENTS.md"))
            .expect("AGENTS.md is readable");
        guide
            .lines()
            .filter(|line| line.starts_with("| `"))
            .filter_map(|line| line.split('|').nth(1))
            .flat_map(|cell| cell.split('`').skip(1).step_by(2))
            .map(str::to_owned)
            .collect()
    }

    fn source_files(directory: &std::path::Path, prefix: &str, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(directory).expect("src is readable") {
            let entry = entry.expect("directory entries are readable");
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_dir() {
                source_files(&path, &format!("{prefix}{name}/"), found);
            } else if name.ends_with(".rs") {
                found.push(format!("{prefix}{name}"));
            }
        }
    }

    fn press(app: &mut App, code: KeyCode) -> bool {
        app.key(code, KeyModifiers::NONE, 10.0)
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    /// Opens the picker, types `name` and presses Enter, as the smoke test does.
    fn pick(app: &mut App, name: &str) {
        press(app, KeyCode::Char('v'));
        type_text(app, name);
        press(app, KeyCode::Enter);
    }

    fn playlist_names(app: &App) -> Vec<&'static str> {
        app.playlist.iter().map(|view| view.name()).collect()
    }

    /// The picker's popup lines in a layout of `rows` scene rows.
    fn picker_lines(app: &App, rows: u16) -> Vec<String> {
        let layout = Layout {
            columns: 120,
            rows,
            width: 960,
            height: rows as u32 * 16,
            cell: None,
        };
        app.picker_popup(&layout).expect("the picker fits").lines
    }

    #[test]
    fn the_picker_filters_by_name_and_title_ignoring_case() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        press(&mut app, KeyCode::Char('v'));
        let lines = picker_lines(&app, 31);
        assert_eq!(lines.len(), REGISTRY.len() + 2, "{lines:?}");
        assert!(lines[1].starts_with("> "), "the highlight starts on city");
        type_text(&mut app, "RE");
        let entries = app.entries();
        let matching = |text: &str| {
            let names: Vec<_> = picker::filter(&entries, text)
                .iter()
                .map(|entry| entry.name)
                .collect();
            names
        };
        assert_eq!(matching("RE"), ["cores", "reef"]);
        assert_eq!(matching("CoRe"), ["cores"]);
        assert_eq!(matching("matrix"), ["matrix"]);
        let lines = picker_lines(&app, 31);
        assert!(lines[0].starts_with("View: RE_"), "{lines:?}");
        assert!(lines[1].contains("cores") && lines[1].contains("CORES"));
        assert!(lines[2].contains("reef") && lines[2].contains("REEF"));
        assert!(
            lines[2].contains(" 9 "),
            "reef is the ninth view: {lines:?}"
        );
        press(&mut app, KeyCode::Backspace);
        type_text(&mut app, "eef");
        assert_eq!(picker_lines(&app, 31)[1].trim_end(), "> *  9 reef   REEF");
        assert_eq!(app.view, view("city"), "typing never switches");
    }

    #[test]
    fn an_empty_filter_shows_no_view_matches_and_enter_does_nothing() {
        let mut app = App::new(view("orbit"), model::demo(1.0, 16), all_available());
        app.render(320, 180, 512);
        press(&mut app, KeyCode::Char('v'));
        type_text(&mut app, "zzz");
        let lines = picker_lines(&app, 31);
        assert_eq!(lines[1], "no view matches", "{lines:?}");
        assert_eq!(lines.len(), 3);
        assert!(!app.fit);
        press(&mut app, KeyCode::Enter);
        assert!(app.picker.is_some(), "Enter keeps the picker open");
        assert_eq!((app.view, app.fit), (view("orbit"), false));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(
            playlist_names(&app).len(),
            REGISTRY.len(),
            "nothing toggled"
        );
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        assert_eq!(picker_lines(&app, 31).len(), REGISTRY.len() + 2);
    }

    #[test]
    fn tab_cycles_the_playlist_and_resumes_after_a_picked_view() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.playlist =
            picker::playlist(&[view("city"), view("flow"), view("reef")], &app.available);
        for expected in ["flow", "reef", "city", "flow"] {
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.view.name(), expected);
        }
        pick(&mut app, "orbit");
        assert_eq!(app.view, view("orbit"), "not in the playlist");
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.view.name(),
            "flow",
            "the first playlist view after orbit"
        );
        pick(&mut app, "coop");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view.name(), "city", "wraps round past the last row");
        pick(&mut app, "ripple");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view.name(), "flow");
    }

    #[test]
    fn shift_tab_mirrors_tab_around_a_view_outside_the_playlist() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.playlist = picker::playlist(&[view("flow"), view("reef")], &app.available);
        pick(&mut app, "orbit");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(
            app.view.name(),
            "reef",
            "wraps round to the last playlist view"
        );
        pick(&mut app, "ripple");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(
            app.view.name(),
            "reef",
            "nothing before ripple, so the last"
        );
        pick(&mut app, "matrix");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view.name(), "reef", "the nearest before matrix");
        pick(&mut app, "coop");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view.name(), "reef");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view.name(), "flow", "inside the playlist it steps back");
        pick(&mut app, "cells");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view.name(), "flow", "the nearest before cells");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view.name(), "reef");
    }

    #[test]
    fn the_playlist_cannot_be_emptied() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.playlist = picker::playlist(&[view("city"), view("orbit")], &app.available);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(playlist_names(&app), ["orbit"], "city leaves");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(playlist_names(&app), ["orbit"], "the last view stays");
        let lines = picker_lines(&app, 31);
        assert!(
            lines.last().unwrap().contains("keeps at least one view"),
            "{lines:?}"
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(playlist_names(&app), ["orbit", "ripple"]);
        assert!(!picker_lines(&app, 31).last().unwrap().contains("keeps"));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(
            playlist_names(&app),
            ["city", "orbit", "ripple"],
            "back in registry order"
        );
        press(&mut app, KeyCode::Esc);
        assert!(app.picker.is_none());
        assert_eq!(app.view, view("city"), "Esc leaves the view as it was");
    }

    #[test]
    fn digits_ignore_the_playlist() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.playlist = picker::playlist(&[view("reef")], &app.available);
        for (digit, name) in [
            ('2', "orbit"),
            ('7', "strata"),
            ('0', "matrix"),
            ('1', "city"),
        ] {
            press(&mut app, KeyCode::Char(digit));
            assert_eq!(app.view.name(), name, "digit {digit}");
        }
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view.name(), "reef");
        assert_eq!(
            playlist_names(&app),
            ["reef"],
            "digits leave the playlist alone"
        );
    }

    #[test]
    fn views_flag_rejects_unknown_names_and_drops_duplicates() {
        let parse = |arguments: &[&str]| {
            Options::try_parse_from(std::iter::once("isotop").chain(arguments.iter().copied()))
        };
        let error = parse(&["--views", "city,nope"])
            .err()
            .expect("nope is unknown");
        let message = error.to_string();
        assert!(message.contains("nope"), "{message}");
        assert!(
            message.contains("[possible values: city, orbit, ripple"),
            "{message}"
        );
        let named = options(&["--views", "reef,city,reef,coop,city"]);
        let every = all_available();
        let names: Vec<_> = picker::playlist(&named.views, &every)
            .iter()
            .map(|view| view.name())
            .collect();
        assert_eq!(
            names,
            ["reef", "city", "coop"],
            "first occurrences, in order"
        );
        let (result, _, err) = preflight_with(&["--views", "reef,city,reef"], &[]);
        assert_eq!((result, err.as_str()), (Preflight::Start(view("reef")), ""));
        let (result, _, _) = preflight_with(&["--views", "reef,city", "--view", "orbit"], &[]);
        assert_eq!(result, Preflight::Start(view("orbit")), "--view wins");
        let (result, _, err) = preflight_with(&["--views", "orbit,reef", "--verbose"], &["orbit"]);
        assert_eq!(result, Preflight::Start(view("reef")), "orbit is skipped");
        assert!(
            err.contains("skipping view orbit: not supported on this platform\n"),
            "{err}"
        );
        let headless = ["--views", "coop,city", "--output", "x.png"];
        assert_eq!(
            preflight_with(&headless, &[]).0,
            Preflight::Start(view("coop"))
        );
        let mut app = App::new(view("reef"), model::demo(1.0, 16), every);
        app.playlist = picker::playlist(&named.views, &app.available);
        assert_eq!(playlist_names(&app), ["reef", "city", "coop"]);
    }

    #[test]
    fn views_flag_with_no_available_name_exits_with_an_error() {
        let (result, out, err) = preflight_with(&["--views", "orbit,matrix"], &["orbit", "matrix"]);
        assert_eq!(result, Preflight::Exit(1));
        assert_eq!(out, "");
        assert_eq!(err, "isotop: no available view in --views\n");
        let (result, _, err) = preflight_with(
            &["--views", "orbit", "--verbose", "--output", "x.png"],
            &["orbit"],
        );
        assert_eq!(result, Preflight::Exit(1));
        assert_eq!(
            err,
            "skipping view orbit: not supported on this platform\n\
             isotop: no available view in --views\n"
        );
        let (result, _, err) = preflight_with(&["--views", "orbit", "--view", "city"], &["orbit"]);
        assert_eq!(
            (result, err.as_str()),
            (Preflight::Exit(1), "isotop: no available view in --views\n")
        );
    }

    #[test]
    fn keys_typed_into_the_picker_do_not_reach_the_scene() {
        let mut app = probe_app();
        let legend = |app: &App| app.scene.legend(View::at(0), app.snapshot());
        let before = (
            legend(&app),
            app.goal.clone(),
            (app.labels, app.panel, app.paused),
        );
        press(&mut app, KeyCode::Char('v'));
        assert!(app.picker.is_some());
        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('x'),
            KeyCode::Char('a'),
            KeyCode::Char('m'),
        ] {
            assert!(!press(&mut app, code), "{code:?} must not quit");
        }
        for code in [
            KeyCode::Char('1'),
            KeyCode::Char('/'),
            KeyCode::Char('+'),
            KeyCode::Char('l'),
            KeyCode::Char('h'),
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Home,
            KeyCode::PageUp,
            KeyCode::Left,
        ] {
            assert!(!press(&mut app, code));
        }
        assert_eq!(app.view, View::at(0));
        assert_eq!(app.search, None);
        let lines = picker_lines(&app, 31);
        assert!(lines[0].starts_with("View: qxam1/+lh_"), "{lines:?}");
        assert_eq!(legend(&app), before.0, "the view saw no key");
        assert!(app.goal == before.1, "the camera did not move");
        assert_eq!((app.labels, app.panel, app.paused), before.2);
        assert!(
            app.key(KeyCode::Char('c'), KeyModifiers::CONTROL, 10.0),
            "Ctrl-C still quits"
        );
        press(&mut app, KeyCode::Esc);
        assert!(app.picker.is_none());
        assert!(!press(&mut app, KeyCode::Char('x')));
        assert_ne!(
            legend(&app),
            before.0,
            "closed, the key reaches the view again"
        );
        let mut plain = App::new(view("city"), model::demo(1.0, 16), all_available());
        press(&mut plain, KeyCode::Char('v'));
        assert!(
            !press(&mut plain, KeyCode::Char('q')),
            "q filters and does not quit"
        );
        assert_eq!(picker_lines(&plain, 31)[0], "View: q_  0/11");
    }

    /// A 1600 by 896 scene of 8 by 16 pixel cells whose mouse reports arrive in pixels.
    const THING_LAYOUT: Layout = Layout {
        columns: 200,
        rows: 56,
        width: 1600,
        height: 896,
        cell: Some([8.0, 16.0]),
    };

    /// An app on the lantern view of `testing::THINGS`, and its first frame.
    fn lantern_app() -> (App, Frame) {
        let scene = Scene::with_rows(testing::THINGS);
        let mut app = App::with_scene(scene, View::at(0), model::demo(1.0, 16), vec![true; 2]);
        let frame = app.render(THING_LAYOUT.width, THING_LAYOUT.height, 512);
        (app, frame)
    }

    /// Where lantern `index` is drawn in `frame`.
    fn lantern_at(app: &App, frame: &Frame, index: usize) -> [f32; 2] {
        frame
            .locate(&app.camera, testing::Lanterns::position(index))
            .expect("the lantern is on screen")
    }

    /// A left click at frame pixel `at`, pressed and released in place.
    fn click(app: &mut App, frame: &Frame, at: [f32; 2]) {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            let event = MouseEvent {
                kind,
                column: at[0] as u16,
                row: at[1] as u16,
                modifiers: KeyModifiers::NONE,
            };
            app.mouse(event, &THING_LAYOUT, frame, &[], true);
        }
    }

    /// The text of the open popup, if one is open.
    fn popup_lines(app: &App, frame: &mut Frame) -> Option<Vec<String>> {
        let radius = THING_LAYOUT.pick_radius(true);
        let (_, panels) = app.overlay(frame, &THING_LAYOUT, radius);
        panels.first().map(|panel| panel.lines.clone())
    }

    #[test]
    fn a_selected_thing_opens_its_card_and_esc_closes_it() {
        let (mut app, mut frame) = lantern_app();
        assert_eq!(frame.things.len(), 3);
        let hovered = lantern_at(&app, &frame, 1);
        app.hover = Some(Pointer {
            frame: hovered,
            cell: THING_LAYOUT.cell_of(hovered),
        });
        let radius = THING_LAYOUT.pick_radius(true);
        let (labels, _) = app.overlay(&mut frame, &THING_LAYOUT, radius);
        let tags: Vec<String> = labels
            .into_iter()
            .filter(|label| label.tone == Tone::Tag)
            .map(|label| label.text)
            .collect();
        assert_eq!(
            tags,
            [format!(" {} ", testing::lantern_title(101))],
            "a hovered thing shows its title"
        );
        app.hover = None;
        let lantern = Thing {
            kind: testing::LANTERN,
            key: 100,
        };
        let spot = lantern_at(&app, &frame, 0);
        click(&mut app, &frame, spot);
        assert_eq!((app.thing, app.selected), (Some(lantern), None));
        assert_eq!(
            popup_lines(&app, &mut frame).expect("the card is open"),
            [
                testing::lantern_title(100).as_str(),
                testing::WICK,
                "Esc close"
            ]
        );
        app.render(THING_LAYOUT.width, THING_LAYOUT.height, 512);
        assert_eq!(
            app.scene.legend(app.view, app.snapshot()),
            " lanterns | highlighted 100",
            "the view highlights the selected thing"
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.thing, None);
        assert_eq!(popup_lines(&app, &mut frame), None);
        app.render(THING_LAYOUT.width, THING_LAYOUT.height, 512);
        assert_eq!(app.scene.legend(app.view, app.snapshot()), " lanterns");
        let spot = lantern_at(&app, &frame, 0);
        click(&mut app, &frame, spot);
        let process = app.snapshot().processes[0].id;
        let at = app
            .screen(&frame, process)
            .expect("the process is on screen");
        click(&mut app, &frame, at);
        assert_eq!(
            (app.thing, app.selected),
            (None, Some(process)),
            "selecting a process lets the thing go"
        );
    }

    #[test]
    fn a_thing_that_vanishes_says_so() {
        let (mut app, mut frame) = lantern_app();
        let spot = lantern_at(&app, &frame, 2);
        click(&mut app, &frame, spot);
        assert_eq!(app.thing.map(|thing| thing.key), Some(102));
        press(&mut app, KeyCode::Char('x'));
        frame = app.render(THING_LAYOUT.width, THING_LAYOUT.height, 512);
        assert_eq!(frame.things.len(), 2, "the last lantern went out");
        assert_eq!(
            popup_lines(&app, &mut frame).expect("the popup stays open"),
            ["lantern no longer shown", "Esc close"]
        );
    }

    #[test]
    fn switching_views_clears_the_selected_thing() {
        let (mut app, frame) = lantern_app();
        let spot = lantern_at(&app, &frame, 0);
        click(&mut app, &frame, spot);
        assert!(app.thing.is_some());
        press(&mut app, KeyCode::Tab);
        assert_eq!((app.view, app.thing), (View::at(1), None));
        press(&mut app, KeyCode::Tab);
        assert_eq!((app.view, app.thing), (View::at(0), None));
    }

    #[test]
    fn thing_card_text_cannot_inject_terminal_controls() {
        let (mut app, mut frame) = lantern_app();
        let radius = THING_LAYOUT.pick_radius(true);
        let at = lantern_at(&app, &frame, 0);
        app.hover = Some(Pointer {
            frame: at,
            cell: THING_LAYOUT.cell_of(at),
        });
        let (labels, _) = app.overlay(&mut frame, &THING_LAYOUT, radius);
        click(&mut app, &frame, at);
        app.hover = None;
        let (_, panels) = app.overlay(&mut frame, &THING_LAYOUT, radius);
        let tag = labels.iter().find(|label| label.tone == Tone::Tag);
        assert!(tag.expect("the hover tag").text.contains('\x1b'));
        assert!(panels[0].lines.iter().any(|line| line == testing::WICK));
        let mut output = Vec::new();
        for label in &labels {
            terminal::draw_label(&mut output, label, THING_LAYOUT.columns).unwrap();
        }
        for panel in &panels {
            terminal::draw_popup(&mut output, panel).unwrap();
        }
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains("lantern 100?]0;flare?"),
            "the tag: {output:?}"
        );
        assert!(
            output.contains("wick ?[2J?lit"),
            "the card's line: {output:?}"
        );
        for control in ["\x1b]0;", "\x1b[2J", "\x07"] {
            assert!(
                !output.contains(control),
                "{control:?} reached the terminal"
            );
        }
    }

    #[test]
    fn the_mouse_is_ignored_while_the_picker_is_open() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        let frame = app.render(320, 180, 512);
        let layout = Layout {
            columns: 40,
            rows: 12,
            width: 320,
            height: 180,
            cell: None,
        };
        let scroll = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let center = app.goal.center;
        press(&mut app, KeyCode::Char('v'));
        app.mouse(scroll, &layout, &frame, &[], false);
        assert_eq!((app.goal.center, app.hover.is_none()), (center, true));
        press(&mut app, KeyCode::Esc);
        app.mouse(scroll, &layout, &frame, &[], false);
        assert_ne!(
            app.goal.center, center,
            "the same event pans once it is closed"
        );
    }

    #[test]
    fn enter_in_the_picker_switches_through_the_usual_path() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.render(320, 180, 512);
        assert!(app.demand().is_empty());
        pick(&mut app, "matrix");
        assert_eq!(app.view, view("matrix"));
        assert!(app.picker.is_none() && app.fit);
        assert_eq!(app.demand(), Demand::from(view("matrix").info().sources));
        assert!(app.demand().contains(Source::Journal));
        pick(&mut app, "ripple");
        assert_eq!(app.goal.pitch, app.row().pitch.unwrap_or(app.goal.pitch));
        assert!(app.demand().is_empty());
    }

    #[test]
    fn the_picker_scrolls_to_keep_the_highlight_visible() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        press(&mut app, KeyCode::Char('v'));
        for _ in 0..REGISTRY.len() - 1 {
            press(&mut app, KeyCode::Down);
        }
        let lines = picker_lines(&app, 8);
        assert_eq!(lines.len(), 6, "filter, four rows and the keys: {lines:?}");
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("> ") && line.contains("coop"))
        );
        press(&mut app, KeyCode::Down);
        let lines = picker_lines(&app, 8);
        assert!(
            lines[1].starts_with("> ")
                || lines
                    .iter()
                    .any(|line| line.starts_with("> ") && line.contains("city")),
            "{lines:?}"
        );
        press(&mut app, KeyCode::Up);
        assert!(
            picker_lines(&app, 8)
                .iter()
                .any(|line| line.starts_with("> ") && line.contains("coop"))
        );
    }

    #[test]
    fn agents_file_table_lists_every_source_file() {
        let listed = agents_table_paths();
        let mut found = Vec::new();
        source_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            "",
            &mut found,
        );
        found.sort();
        let missing: Vec<&String> = found.iter().filter(|file| !listed.contains(file)).collect();
        assert!(
            missing.is_empty(),
            "AGENTS.md's file table has no row for: {missing:?}. Add one, naming the file by its \
             path under src/ in the first column"
        );
    }

    #[test]
    fn agents_file_table_names_only_files_that_exist() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let gone: Vec<String> = agents_table_paths()
            .into_iter()
            .filter(|file| !source.join(file).is_file())
            .collect();
        assert!(
            gone.is_empty(),
            "AGENTS.md's file table names files that do not exist under src/: {gone:?}"
        );
    }

    const SCENE: Layout = Layout {
        columns: 80,
        rows: 24,
        width: 640,
        height: 360,
        cell: None,
    };

    fn scroll(app: &mut App, kind: MouseEventKind, times: usize) {
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
    fn globe_at(time: f32) -> App {
        let mut app = App::new(view("globe"), model::demo(1.0, 64), all_available());
        app.animation = time;
        app.render(SCENE.width, SCENE.height, 512);
        app
    }

    /// Where the globe's home marker falls on screen with the camera as it stands.
    fn home_on_screen(app: &mut App) -> [f32; 2] {
        let frame = app.render(SCENE.width, SCENE.height, 512);
        let at = frame.project(&app.camera, app.scene.places[0].0);
        [at[0], at[1]]
    }

    /// Where a world point falls on screen if the view is panned by (dx, dy) frame pixels.
    fn panned_by(camera: &Camera, point: [f32; 3], dx: f32, dy: f32) -> [f32; 2] {
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

    #[test]
    fn scrolling_turns_and_tilts_the_globe_the_way_panning_moves_the_view() {
        let step = SCENE.height as f32 * 0.04;
        for (kind, sideways) in [
            (MouseEventKind::ScrollRight, true),
            (MouseEventKind::ScrollLeft, true),
            (MouseEventKind::ScrollDown, false),
            (MouseEventKind::ScrollUp, false),
        ] {
            let mut app = globe_at(0.0);
            let centre = app.goal.center;
            let before = home_on_screen(&mut app);
            let point = app.scene.places[0].0;
            let (dx, dy) = match kind {
                MouseEventKind::ScrollRight => (step, 0.0),
                MouseEventKind::ScrollLeft => (-step, 0.0),
                MouseEventKind::ScrollDown => (0.0, step),
                _ => (0.0, -step),
            };
            // What panning by the same scroll does to the same point is the way it must move.
            let by_pan = panned_by(&app.camera, point, dx, dy);
            let pan_moves = [by_pan[0] - before[0], by_pan[1] - before[1]];
            scroll(&mut app, kind, 1);
            let after = home_on_screen(&mut app);
            let moved = [after[0] - before[0], after[1] - before[1]];
            let (along, across) = if sideways { (0, 1) } else { (1, 0) };
            assert_eq!(moved[along].signum(), pan_moves[along].signum(), "{kind:?}");
            let ratio = moved[along] / pan_moves[along];
            assert!(
                (0.3..1.05).contains(&ratio),
                "{kind:?} moved {ratio} of a step"
            );
            assert!(
                moved[across].abs() < 0.35 * step,
                "{kind:?} drifted {moved:?}"
            );
            assert!(
                app.goal.globe_held && app.camera.globe_held,
                "{kind:?} holds"
            );
            assert!(app.goal.center == centre, "{kind:?} panned");
        }
    }

    #[test]
    fn scrolling_the_globe_moves_the_surface_by_one_step_whatever_the_zoom() {
        let step = SCENE.height as f32 * 0.04;
        for zoom in [1.0, 5.0, 30.0] {
            let mut app = globe_at(0.0);
            app.both(|c| c.zoom = zoom);
            let before = home_on_screen(&mut app);
            scroll(&mut app, MouseEventKind::ScrollRight, 1);
            let after = home_on_screen(&mut app);
            let moved = (after[0] - before[0]).abs();
            // The surface at home is a little foreshortened and the angle is capped at 0.25 rad.
            let reach = (0.25 * globe::RADIUS * render::SCALE * zoom).min(step);
            assert!(
                moved > 0.5 * reach && moved <= reach * 1.02,
                "zoom {zoom}: {moved} px for {reach}"
            );
        }
    }

    #[test]
    fn the_tilt_stops_at_the_poles_for_the_camera_it_is_clamped_against() {
        let mut app = globe_at(0.0);
        scroll(&mut app, MouseEventKind::ScrollDown, 80);
        let pitch = app.camera.pitch;
        assert!((app.camera.globe_tilt - (pitch + FRAC_PI_2)).abs() < 1e-5);
        scroll(&mut app, MouseEventKind::ScrollUp, 160);
        assert!((app.camera.globe_tilt - (pitch - FRAC_PI_2)).abs() < 1e-5);
        assert_eq!(app.camera.globe_tilt, app.goal.globe_tilt);
    }

    #[test]
    fn dragging_still_pans_the_globe() {
        let mut app = globe_at(0.0);
        let frame = app.render(SCENE.width, SCENE.height, 512);
        let mouse = |kind, column| MouseEvent {
            kind,
            column,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let centre = app.goal.center;
        for event in [
            mouse(MouseEventKind::Down(MouseButton::Left), 10),
            mouse(MouseEventKind::Drag(MouseButton::Left), 14),
            mouse(MouseEventKind::Up(MouseButton::Left), 14),
        ] {
            app.mouse(event, &SCENE, &frame, &[], false);
        }
        assert_ne!(app.goal.center, centre, "dragging pans");
        assert_eq!(app.goal.globe_turn, 0.0);
        assert_eq!(app.goal.globe_tilt, 0.0);
        assert!(!app.goal.globe_held, "dragging leaves the earth turning");
    }

    #[test]
    fn other_views_still_pan_when_scrolled() {
        for view in ["city", "orbit", "reef"].map(view) {
            let mut app = App::new(view, model::demo(1.0, 64), all_available());
            let frame = app.render(SCENE.width, SCENE.height, 512);
            let mut expected = app.camera.clone();
            let step = SCENE.height as f32 * 0.04;
            expected.pan(step, -step);
            for kind in [MouseEventKind::ScrollRight, MouseEventKind::ScrollUp] {
                let event = MouseEvent {
                    kind,
                    column: 10,
                    row: 5,
                    modifiers: KeyModifiers::NONE,
                };
                app.mouse(event, &SCENE, &frame, &[], false);
            }
            for k in 0..2 {
                assert!(
                    (app.goal.center[k] - expected.center[k]).abs() < 1e-3,
                    "{view:?}: {:?} against {:?}",
                    app.goal.center,
                    expected.center
                );
            }
            assert!(!app.goal.globe_held, "{view:?}");
        }
    }

    #[test]
    fn a_turned_globe_holds_still_until_reset_and_then_spins_on_without_a_jump() {
        let mut app = globe_at(10.0);
        // Holding the earth freezes it where it is, however far the clock has run.
        let free = home_on_screen(&mut app);
        app.both(|c| c.hold(10.0));
        let frozen = home_on_screen(&mut app);
        assert!(
            (free[0] - frozen[0]).abs() < 0.01 && (free[1] - frozen[1]).abs() < 0.01,
            "holding jumped {free:?} to {frozen:?}"
        );
        app.both(|c| c.release(10.0));
        scroll(&mut app, MouseEventKind::ScrollRight, 3);
        scroll(&mut app, MouseEventKind::ScrollDown, 2);
        let held = home_on_screen(&mut app);
        for time in [40.0, 150.0] {
            app.animation = time;
            let later = home_on_screen(&mut app);
            assert!(
                (held[0] - later[0]).abs() < 0.01 && (held[1] - later[1]).abs() < 0.01,
                "the held earth moved by {held:?} to {later:?} at {time} s"
            );
        }
        // r renders straight after the key, before the camera has eased anywhere.
        app.animation = 150.0;
        app.key(KeyCode::Char('r'), KeyModifiers::NONE, 10.0);
        assert!(!app.camera.globe_held && !app.goal.globe_held);
        let straight_after = home_on_screen(&mut app);
        assert!(
            (held[0] - straight_after[0]).abs() < 0.01
                && (held[1] - straight_after[1]).abs() < 0.01,
            "r made the earth jump from {held:?} to {straight_after:?}"
        );
        // Then the camera eases back to the default while the earth turns on.
        let mut last = straight_after;
        for frame in 1..=120 {
            let elapsed = 1.0 / 20.0;
            app.animation += elapsed;
            let goal = app.goal.clone();
            app.camera.approach(&goal, 1.0 - (-elapsed / 0.2_f32).exp());
            let now = home_on_screen(&mut app);
            let jump = (now[0] - last[0]).hypot(now[1] - last[1]);
            assert!(jump < 100.0, "frame {frame} jumped {jump} px");
            last = now;
        }
        assert!(app.camera.globe_tilt.abs() < 1e-3);
        assert!(
            (app.camera.globe_turn.rem_euclid(TAU) - app.goal.globe_turn).abs() < 1e-3
                || (app.camera.globe_turn.rem_euclid(TAU) - TAU).abs() < 1e-3
        );
        // It is the globe a new session shows at that time: nothing doubled the spin.
        let mut fresh = globe_at(app.animation);
        let (a, b) = (home_on_screen(&mut app), home_on_screen(&mut fresh));
        assert!(
            (a[0] - b[0]).abs() < 2.0 && (a[1] - b[1]).abs() < 2.0,
            "{a:?} {b:?}"
        );
    }

    #[test]
    fn holding_and_releasing_while_paused_or_rewound_does_not_jump() {
        let mut app = globe_at(0.0);
        app.history.push_back(model::demo(50.0, 64));
        app.history.push_back(model::demo(100.0, 64));
        app.animation = 500.0;
        // [ pauses and steps back to the sample at 50 s, whatever the animation clock says.
        app.key(KeyCode::Char('['), KeyModifiers::NONE, 10.0);
        assert!(app.paused);
        assert_eq!(app.render_time(), 50.0);
        let free = home_on_screen(&mut app);
        let time = app.render_time();
        app.both(|c| c.hold(time));
        let held = home_on_screen(&mut app);
        assert!(
            (free[0] - held[0]).abs() < 0.01 && (free[1] - held[1]).abs() < 0.01,
            "holding jumped {free:?} to {held:?}"
        );
        scroll(&mut app, MouseEventKind::ScrollRight, 2);
        let turned = home_on_screen(&mut app);
        assert!(turned[0] < held[0], "the scroll turned it");
        // g releases it, and the earth stays where the hand left it.
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        assert!(!app.camera.globe_held);
        let released = home_on_screen(&mut app);
        assert!(
            (turned[0] - released[0]).abs() < 0.01 && (turned[1] - released[1]).abs() < 0.01,
            "releasing jumped {turned:?} to {released:?}"
        );
        // Rewinding further carries the free earth with the sample time, not the clock.
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        app.key(KeyCode::Char('['), KeyModifiers::NONE, 10.0);
        assert_eq!(app.render_time(), 1.0);
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

    #[test]
    fn searching_keeps_the_hold_but_stands_the_earth_upright_on_the_match() {
        let mut app = globe_at(30.0);
        scroll(&mut app, MouseEventKind::ScrollDown, 6);
        scroll(&mut app, MouseEventKind::ScrollRight, 3);
        app.render(640, 360, 512);
        let target = *app.scene.positions.keys().next().unwrap();
        app.matches = vec![target];
        app.select_match();
        assert!(app.goal.globe_held, "the found process must not drift away");
        assert_eq!(app.goal.globe_tilt, 0.0);
        let goal = app.goal.clone();
        app.camera.approach(&goal, 1.0);
        app.camera = app.goal.clone();
        let frame = app.render(640, 360, 512);
        let at = frame.project(&app.camera, app.scene.positions[&target]);
        assert!((at[0] - 320.0).abs() < 1.5 && (at[1] - 360.0 * ORIGIN_Y).abs() < 1.5);
    }

    #[test]
    fn globe_hints_say_scroll_turns_and_drag_pans() {
        let mut globe = App::new(view("globe"), model::demo(1.0, 64), all_available());
        let mut city = App::new(view("city"), model::demo(1.0, 64), all_available());
        let line = |app: &App, index: usize| app.text(0.0, "test", 20, 512)[index].clone();
        assert!(line(&globe, 3).contains(" | scroll turn/tilt | drag pan | Ctrl-scroll zoom"));
        assert!(!line(&globe, 3).contains("scroll pan"));
        assert!(line(&city, 3).contains(" | scroll pan | Ctrl-scroll zoom"));
        globe.show_help = true;
        city.show_help = true;
        assert!(line(&globe, 4).starts_with(" Scroll turn/tilt | drag pan | Ctrl-scroll zoom"));
        assert!(line(&city, 4).starts_with(" Scroll pan | Ctrl-scroll zoom"));
    }
}
