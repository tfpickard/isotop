mod cells;
mod coop;
mod cores;
mod geo;
mod geometry;
mod globe;
mod glyphs;
mod gpu;
mod journal;
mod layout;
mod matrix;
mod medium;
mod model;
mod palette;
mod places;
mod platform;
mod reef;
mod render;
mod simulation;
mod status;
mod strata;
mod terminal;
mod views;

use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::f32::consts::FRAC_PI_2;
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
use platform::Source;
use render::{Camera, Frame, ISOMETRIC, Links, ORIGIN_Y, Point, RenderInput, Scene, label_name};
use terminal::{Label, Popup, Terminal, Tone};
use views::{LOCAL_KEYS, Phase, REGISTRY, View, ViewInfo, ViewKey, When};

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
    /// Initial visualization
    #[arg(long, default_value = "city", value_parser = view_parser())]
    view: View,
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
    view: View,
    selected: Option<Identity>,
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
            view,
            selected: None,
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
            dragging: None,
            dismiss: false,
            animation: 0.0,
            last_input: Instant::now(),
            tour: None,
            gpu: None,
            renderer: "cpu".into(),
            journal_demanded: None,
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

    /// The next available view after the shown one in Tab order, or before it when `forward` is
    /// false, wrapping round; the shown view itself when no other is available.
    fn cycle(&self, forward: bool) -> View {
        let count = self.available.len();
        let shown = self.view.index();
        (1..=count)
            .map(|offset| {
                if forward {
                    (shown + offset) % count
                } else {
                    (shown + count - offset) % count
                }
            })
            .find(|&index| self.available[index])
            .map_or(self.view, View::at)
    }

    /// Shows `view`, framed, at its row's pitch if it has one.
    fn switch(&mut self, view: View) {
        self.view = view;
        self.fit = true;
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

    fn render(&mut self, width: u32, height: u32, limit: usize) -> Frame {
        let index = if self.paused {
            self.history_index
        } else {
            self.history.len() - 1
        };
        let time = if self.paused {
            self.history[index].elapsed as f32
        } else {
            self.animation
        };
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
            None => after.is_some_and(|after| now.duration_since(self.last_input) >= after),
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
    fn face(&mut self, point: [f32; 3]) {
        if let Some(face) = self.row().face {
            (self.goal.rotation, self.goal.pitch) = face(point, &self.goal);
            self.goal.look_at(point);
        }
    }

    fn visit(&mut self, index: usize) {
        let targets = self.tour_targets();
        self.tour = (!targets.is_empty()).then(|| Tour {
            index,
            target: targets[index % targets.len()],
            since: Instant::now(),
        });
        if self.tour.is_some() {
            self.selected = None;
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
        } else if self.show_help {
            lines.push(" Arrows/WASD pan | +/- zoom | Q/E rotate | PgUp/PgDn tilt | t top-down | Home fit | Tab/Shift-Tab or 0-9 view | g tour | f focus | l labels | c links".into());
        } else {
            lines.push(" Tab next view | g tour | scroll pan | Ctrl-scroll zoom | click inspect | / search | c links | Space pause | h hide panel | ? help | q quit".into());
        }
        let links = match self.scene.links {
            Links::All => "",
            Links::Focused => " | links: focused",
            Links::Off => " | links: off",
        };
        lines.push(if self.show_help {
            " Scroll pan | Ctrl-scroll zoom | Alt-scroll or right-drag rotate/tilt | n next match | [/] rewind | r reset | Ctrl-C quit".into()
        } else { format!(" {render_ms:.1}ms {} | {transport} | target {fps}fps | {} samples | t={:.1}s | cap {limit} | {}{links}{}",
            self.renderer, self.history.len(), s.elapsed, pressure_text(s),
            if self.focus.is_some() { " | SUBTREE FOCUS" } else { "" }) });
        lines
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

    fn popup(&self, frame: &Frame, layout: &Layout) -> Option<Popup> {
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
        if let Some(popup) = self.popup(frame, layout) {
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
        if let Some(hover) = self.hover
            && let Some(id) = frame.pick_near(hover.frame[0], hover.frame[1], radius)
            && Some(id) != self.selected
            && self.tour.as_ref().is_none_or(|tour| tour.target != id)
            && let Some(p) = lookup.get(&id)
        {
            let text = format!(" {} - {} ", p.name, describe(p));
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
                let picked = frame.pick_near(
                    pointer.frame[0],
                    pointer.frame[1],
                    layout.pick_radius(pixels),
                );
                // Clicking empty space closes the popup, unless the press turns into a drag.
                self.dismiss = picked.is_none();
                if picked.is_some() {
                    self.selected = picked;
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
/// of an unsupported `--view` fails with status 1, and an interactive one starts on the first
/// supported view instead. `available` holds each registry row's answer.
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
    if supported(options.view) {
        return Preflight::Start(options.view);
    }
    if options.headless() {
        let _ = writeln!(
            err,
            "isotop: view {} is not supported on this platform",
            options.view.name()
        );
        return Preflight::Exit(1);
    }
    match View::all().find(|&view| supported(view)) {
        Some(view) => Preflight::Start(view),
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
    let mut app = App::new(view, snapshot, available);
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
        app.hovered = app
            .hover
            .and_then(|h| frame.pick_near(h.frame[0], h.frame[1], layout.pick_radius(pixels)));
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

/// Settles what a headless `--output` or `--benchmark` frame depends on before it is captured,
/// by running the shown view's warm-up phases (`ViewInfo::warmup`) whose condition holds. The
/// golden render test runs this too, so the hashes it checks are the PNGs' frames.
fn warm_up(app: &mut App, options: &Options, journal: Option<&Journal>) {
    let (width, height, limit) = (
        options.width,
        options.headless_height(),
        options.limit as usize,
    );
    let count = options.processes as usize;
    let row = app.row();
    for warmup in row.warmup {
        let due = match warmup.when {
            When::DemoOutput => options.demo && options.output.is_some(),
            When::Output => options.output.is_some(),
            When::Headless => options.headless(),
        };
        if !due {
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
    use crate::views::view;

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
            assert_eq!(options(&["--view", name]).view, view);
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
        assert_eq!(options(&[]).view, view("city"));
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
}
