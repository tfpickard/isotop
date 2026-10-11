mod cells;
mod coop;
mod cores;
mod geo;
mod globe;
mod glyphs;
mod gpu;
mod journal;
mod matrix;
mod medium;
mod model;
mod pack;
mod places;
mod platform;
mod reef;
mod render;
mod simulation;
mod strata;
mod terminal;

use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use journal::Journal;
use model::{Collector, Identity, Process, Snapshot, bytes, describe};
use render::{Camera, Frame, ISOMETRIC, Links, ORIGIN_Y, SCALE, Scene, View, label_name};
use terminal::{Label, Popup, Terminal, Tone};

const TOUR_STEP: Duration = Duration::from_secs(6);
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
    #[arg(long, value_enum, default_value_t = View::City)]
    view: View,
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
}

impl App {
    fn new(view: View, snapshot: Snapshot) -> Self {
        Self {
            scene: Scene::new(),
            camera: Camera::default(),
            goal: Camera::default(),
            rendered: Camera::default(),
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
    /// Does nothing unless the earth was held.
    fn release_globe(&mut self) {
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
        let mut frame = self.scene.render(
            &self.history[index],
            self.view,
            &self.camera,
            width,
            height,
            self.selected,
            time,
            limit,
            self.focus,
        );
        if self.fit {
            self.scene.fit(&mut self.goal, width, height);
            self.fit_zoom = self.goal.zoom;
            self.fit = false;
            if std::mem::take(&mut self.snap) {
                self.camera = self.goal.clone();
                frame = self.scene.render(
                    &self.history[index],
                    self.view,
                    &self.camera,
                    width,
                    height,
                    self.selected,
                    time,
                    limit,
                    self.focus,
                );
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

    /// Notable processes to visit while idle: system stars, the busiest and the largest. Hubs
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
        let mut stars = self.scene.stars.clone();
        stars.retain(|(id, _, _)| !render::is_hub(id));
        stars.sort_by_key(|&(_, members, _)| Reverse(members));
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
        let now = Instant::now();
        let due = match &self.tour {
            // A globe held by hand stays where it was put until r or g.
            None => {
                after.is_some_and(|after| now.duration_since(self.last_input) >= after)
                    && !(self.view == View::Globe && self.goal.globe_held)
            }
            Some(tour) => {
                now.duration_since(tour.since) >= TOUR_STEP
                    || !self.scene.positions.contains_key(&tour.target)
            }
        };
        if due {
            self.visit(self.tour.as_ref().map_or(0, |tour| tour.index + 1));
        }
        if let Some(tour) = &self.tour
            && let Some(&p) = self.scene.positions.get(&tour.target)
        {
            self.goal.center = [p[0], p[1]];
            self.release_globe();
            self.face(p);
        }
    }

    /// On the globe, turns the camera so `point` is on the near side and centres the point
    /// itself: the world spins, so a process above home is behind the earth for half of every
    /// turn, and it floats far above the ground the camera otherwise centres on.
    ///
    /// The point was drawn by `rendered`, whose lean the camera is still easing away from, so
    /// the goal aims at where the point will be once the earth stands upright.
    fn face(&mut self, point: [f32; 3]) {
        if self.view == View::Globe {
            let point = globe::upright(point, &self.rendered);
            self.goal.globe_tilt = 0.0;
            (self.goal.rotation, self.goal.pitch) = globe::face(point, &self.goal);
            self.goal.look_at(point);
        }
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
            self.goal.zoom = self.fit_zoom * 2.2;
        }
    }

    fn text(&self, render_ms: f32, transport: &str, fps: u32, limit: usize) -> Vec<String> {
        let s = self.snapshot();
        let view = match self.view {
            View::City => "CITY",
            View::Orbit => "ORBIT",
            View::Ripple => "RIPPLE",
            View::Flow => "FLOW",
            View::Cores => "CORES",
            View::Cells => "CELLS",
            View::Strata => "STRATA",
            View::Globe => "GLOBE",
            View::Reef => "REEF",
            View::Matrix => "MATRIX",
            View::Coop => "COOP",
        };
        let mode = match (self.paused, &self.tour) {
            (true, _) => "PAUSED",
            (false, Some(_)) => "TOUR",
            (false, None) => "LIVE",
        };
        let mut lines = vec![format!(
            " ISOTOP / {view} / {mode}   {} processes{} | {} visible | {} collapsed | {:.1}/{} CPU cores | RAM {} / {}",
            s.processes.len(),
            unreadable_text(s),
            self.scene.visible,
            self.scene.collapsed,
            s.processes.iter().map(|p| p.cpu).sum::<f32>() / 100.0,
            s.cores,
            bytes(s.memory_total.saturating_sub(s.memory_available)),
            bytes(s.memory_total)
        )];
        let inspector = if let Some(id) = self.inspected() {
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
        } else if self.view == View::Matrix {
            format!(
                " > {}",
                self.scene
                    .matrix
                    .latest()
                    .unwrap_or("waiting for the journal...")
            )
        } else {
            " Click or hover a building/body to inspect; / searches name, command or PID".into()
        };
        lines.push(inspector);
        lines.push(self.inspected().and_then(|id| s.processes.iter().find(|p| p.id == id)).map_or_else(
            || match self.view {
                View::City => format!(" Height = CPU | footprint = {} | district = cgroup | amber lights = CPU | cyan pulses = IO", platform::MEMORY_LABEL),
                View::Orbit => format!(" Size = memory (stars: whole system) | rings = threads | glow + trail = CPU | green = NVIDIA GPU | cyan arcs = sockets, pink = outside{}", hub_note(&self.scene)),
                View::Ripple => " Pebbles = processes, clustered by cgroup | ripples = CPU, each at its own pitch | size = memory | water tint = nearest process | drops = births, splashes = exits | swell = pressure".into(),
                View::Flow => format!(" Wells = memory | whirlpools + coloured particles = CPU | two-lane rivers = sockets | rising sparks = outside | turbulence = pressure{}", hub_note(&self.scene)),
                View::Cores => cores::legend(s),
                View::Cells => cells::legend(s),
                View::Strata => " Ridge = process, height = CPU over the last minute, newest at the front | rows: kernel, system, session, containers".into(),
                View::Globe => globe::legend(s),
                View::Reef => " Coral = system services | fish = your session's apps | crabs = containers | plankton = kernel threads | glow = CPU | size = memory | bubbles = I/O".into(),
                View::Coop => self.scene.coop.legend(),
                View::Matrix => format!(
                    " Journal lines decode as the rain passes | red = error, amber = warning, green = info, teal = debug | {} queued{}",
                    self.scene.matrix.pending(),
                    match self.scene.matrix.skipped {
                        0 => String::new(),
                        skipped => format!(", {skipped} skipped"),
                    }
                ),
            }, |p| format!(" {}", p.command)));
        if let Some(search) = &self.search {
            lines.push(format!(
                " /{search}_  {} matches | Enter accept | Esc cancel | Tab next",
                self.matches.len()
            ));
        } else if self.show_help {
            lines.push(" Arrows/WASD pan | +/- zoom | Q/E rotate | PgUp/PgDn tilt | t top-down | Home fit | Tab/Shift-Tab or 0-9 view | g tour | f focus | l labels | c links".into());
        } else {
            let scroll = if self.view == View::Globe {
                "scroll turn/tilt | drag pan"
            } else {
                "scroll pan"
            };
            lines.push(format!(" Tab next view | g tour | {scroll} | Ctrl-scroll zoom | click inspect | / search | c links | Space pause | h hide panel | ? help | q quit"));
        }
        let links = match self.scene.links {
            Links::All => "",
            Links::Focused => " | links: focused",
            Links::Off => " | links: off",
        };
        lines.push(if self.show_help {
            let scroll = if self.view == View::Globe {
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

    fn screen(&self, frame: &Frame, id: Identity) -> Option<[f32; 2]> {
        let &p = self.scene.positions.get(&id)?;
        frame.locate(&self.camera, p)
    }

    /// The selected process, unless the view draws no processes; the selection itself is kept for
    /// when a process view returns.
    fn inspected(&self) -> Option<Identity> {
        self.selected.filter(|_| self.view != View::Matrix)
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
                    0 => {}
                    1 => lines.push("NVIDIA GPU in use (memory not reported)".into()),
                    gpu => lines.push(format!("NVIDIA GPU memory {}", bytes(gpu))),
                }
                if let Some(&(total, count)) = self.scene.mass.get(&id) {
                    lines.push(format!(
                        "system {} {} across {count} processes",
                        platform::MEMORY_LABEL,
                        bytes(total)
                    ));
                }
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
            && let Some(anchor) = self.screen(frame, tour.target)
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
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Tab => {
                self.view.next();
                self.fit = true;
            }
            KeyCode::BackTab => {
                self.view.previous();
                self.fit = true;
            }
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
                self.view = View::ALL[(digit as usize + 10 - '1' as usize) % 10];
                self.fit = true;
            }
            KeyCode::Char('g') if !touring => self.visit(0),
            KeyCode::Char('c') => self.scene.links.cycle(),
            KeyCode::Home => self.fit = true,
            KeyCode::Char('r') => {
                // Release first: the camera must stop being held before it eases to the default
                // camera, or it would copy the goal's release and spin a second time.
                let time = self.render_time();
                self.camera.release(time);
                self.goal = Camera::default();
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
            } else if self.view == View::Globe {
                // The surface follows the fingers the way panning moves content: scrolling right
                // moves it left, which turns it westward, and scrolling down moves it up, which
                // brings the south up. The angle moves the surface under the pointer by `step`.
                let step = layout.height as f32 * 0.04;
                let angle = (step / (globe::RADIUS * SCALE * self.camera.zoom)).min(0.25);
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

/// The pressure readings for the status line, `n/a` for each one the platform cannot measure.
fn pressure_text(snapshot: &Snapshot) -> String {
    let [cpu, memory, io] = snapshot.pressure;
    let reading = |name: &str, value: f32| {
        if snapshot
            .missing
            .contains(&format!("{name} pressure").as_str())
        {
            "n/a".to_owned()
        } else {
            format!("{value:.0}%")
        }
    };
    format!(
        "pressure cpu {} mem {} io {}",
        reading("cpu", cpu),
        reading("memory", memory),
        reading("io", io)
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

/// What the Orbit and Flow legends add while the frame draws a hub; nothing otherwise, so a
/// focused subtree or a platform that reads every process gets the legend it always had.
fn hub_note(scene: &Scene) -> &'static str {
    if scene.hub_names.is_empty() {
        ""
    } else {
        " | hollow star = a parent isotop can't read (run with sudo)"
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

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    if !options.time.is_finite() || options.time < 0.0 {
        return Err("--time must be finite and nonnegative".into());
    }
    if options.duration.is_some_and(|v| !v.is_finite() || v <= 0.0) {
        return Err("--duration must be finite and positive".into());
    }
    let mut collector = Collector::new(options.geoip.clone(), options.home);
    collector.locate(options.view == View::Globe);
    let mut journal = (options.view == View::Matrix && !options.demo).then(Journal::start);
    let mut snapshot = if options.demo {
        model::demo(
            if options.output.is_some() || options.benchmark.is_some() {
                options.time
            } else {
                0.0
            },
            options.processes as usize,
        )
    } else {
        collector.sample()?
    };
    if !options.demo && (options.output.is_some() || options.benchmark.is_some()) {
        std::thread::sleep(Duration::from_millis(options.sample_ms as u64));
        snapshot = collector.sample()?;
    }
    let mut app = App::new(options.view, snapshot);
    app.choose_renderer(options.renderer, options.gpu_power)?;
    let limit = options.limit as usize;
    if options.output.is_some() || options.benchmark.is_some() {
        let height = options.width * 9 / 16;
        // Simulated media (ripple, flow, the reef's fish) start empty: let them develop before
        // the captured frame. Strata needs a history, which the demo workload can replay.
        if options.view == View::Strata && options.demo && options.output.is_some() {
            for back in (1..=60).rev() {
                let at = options.time - back as f64;
                app.history = VecDeque::from([model::demo(at, options.processes as usize)]);
                app.animation = at as f32;
                let warmup = app.render(options.width, height, limit);
                app.scene.spare = warmup.release();
            }
            app.history = VecDeque::from([model::demo(options.time, options.processes as usize)]);
        }
        // The coop's CPU history and fox baselines come from the samples before the frame.
        if options.view == View::Coop && options.demo && options.output.is_some() {
            for back in (1..=30).rev() {
                let at = options.time - back as f64;
                if at >= 0.0 {
                    let snapshot = model::demo(at, options.processes as usize);
                    app.scene.record(&snapshot, at as f32);
                }
            }
        }
        if matches!(
            options.view,
            View::Ripple | View::Flow | View::Reef | View::Cores | View::Coop
        ) && options.output.is_some()
        {
            for step in 0..80 {
                app.animation = options.time as f32 - 4.0 + step as f32 * 0.05;
                let warmup = app.render(options.width, height, limit);
                app.scene.spare = warmup.release();
            }
        }
        // The rain needs several seconds to fill the screen.
        if options.view == View::Matrix {
            let mut fed = options.time - 8.0;
            for step in 0..=160 {
                let at = options.time - 8.0 + step as f64 * 0.05;
                feed(&mut app, journal.as_ref(), options.demo, fed, at);
                fed = at;
                app.animation = at as f32;
                let warmup = app.render(options.width, height, limit);
                app.scene.spare = warmup.release();
            }
        }
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
        return Ok(());
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
    while !quit {
        let frame_start = Instant::now();
        let elapsed = last_frame.elapsed().as_secs_f32();
        last_frame = Instant::now();
        if !app.paused {
            app.animation += elapsed;
            if last_sample.elapsed() >= Duration::from_millis(options.sample_ms as u64) {
                let snapshot = if options.demo {
                    model::demo(origin.elapsed().as_secs_f64(), options.processes as usize)
                } else {
                    collector.locate(app.view == View::Globe);
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
        if app.view == View::Matrix {
            if !options.demo && journal.is_none() {
                journal = Some(Journal::start());
            }
            let now = origin.elapsed().as_secs_f64();
            // Paused, the view keeps what it shows; the journal backlog is bounded meanwhile.
            if !app.paused {
                fed = f64::max(fed, now - 2.0);
                feed(&mut app, journal.as_ref(), options.demo, fed, now);
                fed = now;
            }
        }
        app.camera.approach(&goal, 1.0 - (-elapsed / 0.2).exp());
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
    Ok(())
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
    if let Err(error) = run(Options::parse()) {
        eprintln!("isotop: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut app = App::new(View::Orbit, snapshot);
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
        for view in [View::Orbit, View::Flow] {
            let mut app = App::new(view, snapshot.clone());
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
            let mut app = App::new(view, plain.clone());
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
        for view in [View::Orbit, View::Flow] {
            let mut app = App::new(view, snapshot.clone());
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
        let mut app = App::new(View::Orbit, model::demo(1.0, 64));
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
            let mut app = App::new(View::Globe, model::demo(1.0, 64));
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
        let mut app = App::new(View::City, model::demo(1.0, 16));
        app.key(KeyCode::BackTab, KeyModifiers::SHIFT, 10.0);
        assert_eq!(app.view, *View::ALL.last().unwrap());
        app.key(KeyCode::Tab, KeyModifiers::NONE, 10.0);
        assert_eq!(app.view, View::City);
    }

    #[test]
    fn matrix_status_ignores_a_process_selected_in_another_view() {
        let snapshot = model::demo(1.0, 16);
        let selected = snapshot.processes[0].id;
        let mut app = App::new(View::Matrix, snapshot);
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
        let mut app = App::new(View::City, snapshot);
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
        let mut app = App::new(View::City, model::demo(1.0, 16));
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
        let mut app = App::new(View::Cells, model::demo(1.0, 64));
        app.render(320, 180, 512);
        assert!(app.framed && app.scene.bounded());
        app.key(KeyCode::Left, KeyModifiers::NONE, 10.0);
        assert!(!app.framed, "panning takes over the camera");
        app.key(KeyCode::Char('8'), KeyModifiers::NONE, 10.0);
        assert!(
            app.framed && app.view == View::Globe,
            "switching view frames it again"
        );
    }

    /// The macOS gaps, as the collector reports them.
    const MACOS: [&str; 8] = [
        "cpu pressure",
        "io pressure",
        "last cpu",
        "cpu clock",
        "run queue",
        "cgroups",
        "socket traffic",
        "file locks",
    ];

    /// The status text of `view` over the demo workload with the given gaps, after one frame so
    /// views that learn the gaps while drawing have seen them.
    fn lines_with(view: View, missing: &[&'static str], unreadable: usize) -> Vec<String> {
        let mut snapshot = model::demo(1.0, 16);
        snapshot.missing = missing.to_vec();
        snapshot.unreadable = unreadable;
        if missing.contains(&"last cpu") {
            // A Mac without the rusage counters that place marbles by cluster: no share of
            // performance-core time and no waiting threads.
            for process in &mut snapshot.processes {
                process.performance_share = None;
                process.waiting = None;
            }
        }
        if missing.contains(&"file locks") {
            // No lock table: every process's locks are unreadable and none waits on one.
            for process in &mut snapshot.processes {
                process.locks_held = model::Measured::Unreadable;
                process.blocked_on = None;
            }
            snapshot.unattributed_locks = 0;
        }
        let mut app = App::new(view, snapshot);
        app.render(320, 180, 512);
        app.text(0.0, "test", 20, 512)
    }

    #[test]
    fn status_line_shows_n_a_for_each_missing_pressure() {
        let [cpu, memory, io] = model::demo(1.0, 16)
            .pressure
            .map(|value| format!("{value:.0}%"));
        let full = lines_with(View::City, &[], 0);
        assert!(
            full[4].contains(&format!("pressure cpu {cpu} mem {memory} io {io}")),
            "{}",
            full[4]
        );
        let mac = lines_with(View::City, &MACOS, 0);
        assert!(
            mac[4].contains(&format!("pressure cpu n/a mem {memory} io n/a")),
            "{}",
            mac[4]
        );
        let none = lines_with(
            View::City,
            &["cpu pressure", "memory pressure", "io pressure"],
            0,
        );
        assert!(
            none[4].contains("pressure cpu n/a mem n/a io n/a"),
            "{}",
            none[4]
        );
    }

    #[test]
    fn status_line_counts_unreadable_processes_only_when_there_are_some() {
        let none = lines_with(View::City, &[], 0);
        assert!(!none[0].contains("unreadable") && !none[4].contains("unreadable"));
        let one = lines_with(View::City, &[], 1);
        assert!(
            one[0].contains(" processes (+1 unreadable: run with sudo) | "),
            "{}",
            one[0]
        );
        let many = lines_with(View::City, &MACOS, 37);
        assert!(
            many[0].contains(" processes (+37 unreadable: run with sudo) | "),
            "{}",
            many[0]
        );
        assert!(!many[4].contains("unreadable"), "{}", many[4]);
    }

    #[test]
    fn unreadable_notice_survives_an_80_column_terminal() {
        let first = &lines_with(View::City, &MACOS, 212)[0];
        let visible: String = first.chars().take(80).collect();
        assert!(
            visible.contains("(+212 unreadable: run with sudo)"),
            "{visible}"
        );
    }

    #[test]
    fn legends_name_what_the_platform_cannot_measure() {
        let cores = lines_with(View::Cores, &MACOS, 0)[2].clone();
        assert!(
            cores.contains("lanes = load; macOS reports no last CPU per process, so no marbles"),
            "{cores}"
        );
        assert!(cores.contains("no clock readings"), "{cores}");
        assert!(!cores.contains("marble ="), "{cores}");
        let cells = lines_with(View::Cells, &MACOS, 0)[2].clone();
        assert!(
            cells.contains(
                "groups by app and user; macOS has no cgroups, so no limits, quotas or pressure"
            ),
            "{cells}"
        );
        let globe = lines_with(View::Globe, &MACOS, 0)[2].clone();
        assert!(
            globe.contains("no per-connection rates or RTT on macOS"),
            "{globe}"
        );
        let coop = lines_with(View::Coop, &MACOS, 0)[2].clone();
        assert!(
            coop.contains("no last CPU: running chickens share one trough"),
            "{coop}"
        );
        assert!(coop.contains("no CPU pressure"), "{coop}");
    }

    #[test]
    fn coop_legend_names_app_groups_and_drops_foxes_without_cgroups() {
        let mac = lines_with(View::Coop, &MACOS, 0)[2].clone();
        assert!(mac.contains("flock = app or user group"), "{mac}");
        assert!(
            mac.contains("dust = reads | eyes = memory pressure"),
            "{mac}"
        );
        assert!(mac.contains("no cgroups: no OOM foxes"), "{mac}");
        assert!(
            !mac.contains("flock = cgroup") && !mac.contains("fox ="),
            "{mac}"
        );
        let linux = lines_with(View::Coop, &[], 0)[2].clone();
        assert!(
            linux.contains("flock = cgroup | ") && linux.contains("fox = OOM kill, eyes = "),
            "{linux}"
        );
        assert!(!linux.contains("no cgroups"), "{linux}");
    }

    #[test]
    fn coop_legend_on_macos_counts_open_files_but_has_no_brooding() {
        let mac = lines_with(View::Coop, &MACOS, 0)[2].clone();
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
        let linux = lines_with(View::Coop, &[], 0)[2].clone();
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
            text(View::Cores).contains("chevrons = clock | red queue = waiting tasks | marble =")
        );
        assert!(text(View::Cells).contains("dashed ring = memory limit | arc = CPU vs quota"));
        assert!(text(View::Globe).contains("brighter with traffic | cyan = mostly download"));
        assert!(!text(View::Coop).contains("share one trough"));
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
        let mut app = App::new(View::Globe, model::demo(1.0, 64));
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
        for view in [View::City, View::Orbit, View::Reef] {
            let mut app = App::new(view, model::demo(1.0, 64));
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
        let mut app = App::new(View::City, model::demo(1.0, 64));
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
    fn g_releases_a_held_globe_even_when_there_is_nobody_to_visit() {
        let mut app = App::new(View::Globe, {
            let mut snapshot = model::demo(1.0, 64);
            snapshot.remotes.clear();
            snapshot
        });
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
        let mut globe = App::new(View::Globe, model::demo(1.0, 64));
        let mut city = App::new(View::City, model::demo(1.0, 64));
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
