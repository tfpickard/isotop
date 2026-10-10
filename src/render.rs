use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use crate::medium::{Flow, Grid, TRAIL, Wave, mix};
use crate::model::{Identity, Kind, Process, Snapshot, bounded};
use crate::{cells, coop, cores, globe, glyphs, matrix, reef, strata};

pub type Color = [u8; 3];
pub type Point = [f32; 3];
pub const NONE: u32 = u32::MAX;
const PALETTE: [Color; 8] = [
    [72, 183, 199],
    [122, 145, 229],
    [190, 135, 215],
    [211, 159, 91],
    [98, 185, 149],
    [212, 116, 140],
    [149, 171, 213],
    [166, 185, 104],
];
/// Screen pixels per world unit at zoom 1: sqrt(3/2), the isometric foreshortening.
pub(crate) const SCALE: f32 = 1.224_745;
/// Elevation of a true isometric view, asin(1/sqrt(3)).
pub const ISOMETRIC: f32 = 0.615_480;
/// The camera centre sits slightly below the middle to leave headroom for buildings.
pub const ORIGIN_Y: f32 = 0.57;
/// Depth assigned to background stars, below every scene object.
pub const BACKGROUND: f32 = -10000.0;
pub(crate) const MIN_BODY_PX: f32 = 3.5;
const GROW_SECONDS: f32 = 0.9;
const FADE_SECONDS: f32 = 1.2;
pub(crate) const WARM: Color = [255, 190, 110];
pub(crate) const NVIDIA: Color = [118, 214, 60];
const LINK: Color = [80, 175, 235];
const LINK_HOT: Color = [150, 232, 255];
const OUTSIDE: Color = [255, 110, 200];

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum View {
    City,
    Orbit,
    Ripple,
    Flow,
    Cores,
    Cells,
    Strata,
    Globe,
    Reef,
    Matrix,
    Coop,
}

impl View {
    /// Every view in Tab order; number keys 1 to 9, then 0, pick from this list.
    pub const ALL: [View; 11] = [
        Self::City,
        Self::Orbit,
        Self::Ripple,
        Self::Flow,
        Self::Cores,
        Self::Cells,
        Self::Strata,
        Self::Globe,
        Self::Reef,
        Self::Matrix,
        Self::Coop,
    ];

    pub fn next(&mut self) {
        let index = Self::ALL.iter().position(|v| v == self).unwrap_or(0);
        *self = Self::ALL[(index + 1) % Self::ALL.len()];
    }

    pub fn previous(&mut self) {
        let index = Self::ALL.iter().position(|v| v == self).unwrap_or(0);
        *self = Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()];
    }

    fn backdrop(self) -> Backdrop {
        match self {
            Self::City | Self::Cells | Self::Strata | Self::Coop => Backdrop::Dusk,
            Self::Reef => Backdrop::Sea,
            Self::Matrix => Backdrop::Void,
            Self::Orbit | Self::Ripple | Self::Flow | Self::Cores | Self::Globe => Backdrop::Space,
        }
    }
}

/// The sky behind a view. The GPU shader receives the discriminant as `globals.sky.x`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    Dusk = 0,
    Space = 1,
    Sea = 2,
    Void = 3,
}

/// Which socket links to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Links {
    All,
    Focused,
    Off,
}

impl Links {
    pub fn cycle(&mut self) {
        *self = match self {
            Self::All => Self::Focused,
            Self::Focused => Self::Off,
            Self::Off => Self::All,
        };
    }
}

#[derive(Clone, PartialEq)]
pub struct Camera {
    pub center: [f32; 2],
    pub zoom: f32,
    pub rotation: f32,
    /// Elevation above the ground plane: isometric by default, top-down at pi/2.
    pub pitch: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            zoom: 5.0,
            rotation: 0.0,
            pitch: ISOMETRIC,
        }
    }
}

impl Camera {
    /// Orthographic view space: screen x and y offsets before zoom, and depth (larger is nearer).
    fn view(&self, p: Point) -> Point {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        let x = p[0] - self.center[0];
        let y = p[1] - self.center[1];
        let across = x * c - y * s;
        let along = x * s + y * c;
        let (ps, pc) = self.pitch.sin_cos();
        [
            across * SCALE,
            (along * ps - p[2] * pc) * SCALE,
            along * pc + p[2] * ps,
        ]
    }

    /// Ground-plane offset that appears at a screen offset of (dx, dy) frame pixels.
    fn ground(&self, dx: f32, dy: f32) -> [f32; 2] {
        let across = dx / (SCALE * self.zoom);
        let along = dy / (SCALE * self.zoom * self.pitch.sin());
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        [across * c + along * s, along * c - across * s]
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        let [x, y] = self.ground(dx, dy);
        self.center[0] += x;
        self.center[1] += y;
    }

    pub fn scale(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.3, 65.0);
    }

    /// Zooms while keeping the ground point under the screen offset (dx, dy) in place.
    pub fn scale_at(&mut self, factor: f32, dx: f32, dy: f32) {
        let before = self.ground(dx, dy);
        self.scale(factor);
        let after = self.ground(dx, dy);
        self.center[0] += before[0] - after[0];
        self.center[1] += before[1] - after[1];
    }

    pub fn tilt(&mut self, delta: f32) {
        self.pitch = (self.pitch + delta).clamp(0.35, FRAC_PI_2);
    }

    /// Unit vector from the scene towards the viewer, for specular highlights.
    pub(crate) fn viewer(&self) -> Point {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        let (ps, pc) = self.pitch.sin_cos();
        [s * pc, c * pc, ps]
    }

    /// The ground point `distance` world units from `p` towards the viewer (lowest on screen).
    pub fn front(&self, p: Point, distance: f32) -> Point {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        [p[0] + distance * s, p[1] + distance * c, p[2]]
    }

    /// Moves `amount` of the way towards `goal`, turning the short way round.
    pub fn approach(&mut self, goal: &Camera, amount: f32) {
        for i in 0..2 {
            self.center[i] += (goal.center[i] - self.center[i]) * amount;
        }
        self.zoom *= (goal.zoom / self.zoom).powf(amount);
        self.rotation += ((goal.rotation - self.rotation + PI).rem_euclid(TAU) - PI) * amount;
        self.pitch += (goal.pitch - self.pitch) * amount;
    }
}

/// Background gradient plus pressure haze.
#[derive(Clone, Copy, Debug)]
pub struct Sky {
    pub backdrop: Backdrop,
    /// Haze colour already scaled by its strength, added where the haze field is dense.
    pub haze: [f32; 3],
    pub time: f32,
}

impl Sky {
    pub fn base(&self, y: f32) -> [f32; 3] {
        match self.backdrop {
            Backdrop::Space => [5.0, 8.0, 18.0],
            Backdrop::Dusk => [8.0 + y * 6.0, 13.0 + y * 7.0, 25.0 + y * 8.0],
            Backdrop::Sea => [24.0 - y * 18.0, 72.0 - y * 50.0, 96.0 - y * 58.0],
            Backdrop::Void => [0.0; 3],
        }
    }

    /// Haze density along one normalised screen axis; the field is the product of both axes.
    pub fn profile(&self, t: f32, phase: f32) -> f32 {
        0.5 + 0.5
            * (t * 7.3 + self.time * 0.11 + phase).sin()
            * (t * 2.9 - self.time * 0.07 + phase * 1.7).cos()
    }

    pub fn hazy(&self) -> bool {
        self.haze.iter().any(|&c| c > 0.5)
    }
}

/// Screen-space drawing primitives, recorded in order and rasterized by a backend.
#[derive(Clone, Copy, Debug)]
pub enum Item {
    /// Depth-tested, pickable triangle; points are (x, y, depth).
    Triangle([Point; 3], Color, u32),
    /// Depth-tested one-pixel line that clears picking where it is drawn.
    Line(Point, Point, Color),
    /// Shaded sphere impostor: centre (x, y, depth), pixel radius, world radius for depth.
    Sphere {
        center: Point,
        radius: f32,
        depth: f32,
        color: Color,
        pick: u32,
        selected: bool,
    },
    /// Soft halo blended over whatever is already drawn, without depth or picking.
    Glow {
        center: [f32; 2],
        radius: f32,
        color: Color,
        strength: f32,
    },
    /// Translucent one-pixel line blended without depth or picking.
    Beam([f32; 2], [f32; 2], Color, f32),
    /// Background point behind everything.
    Star([f32; 2], Color),
    /// Bitmap text glyph (see `glyphs`) at an integer pixel origin, each font pixel drawn as a
    /// `scale`-sized square over whatever is already drawn, without depth or picking.
    Glyph {
        origin: [f32; 2],
        scale: u32,
        bits: u128,
        color: Color,
    },
}

/// A finished frame's allocations, handed to the next frame so steady-state rendering does not
/// map and page-fault fresh megabytes every frame.
#[derive(Default)]
pub struct Buffers {
    items: Vec<Item>,
    pixels: Vec<u8>,
    depth: Vec<f32>,
    picks: Vec<u32>,
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub sky: Sky,
    pub items: Vec<Item>,
    pub pixels: Vec<u8>,
    depth: Vec<f32>,
    pub picks: Vec<u32>,
    pub identities: Vec<Identity>,
    /// Pixel position of the selected process, if it was drawn.
    pub anchor: Option<[f32; 2]>,
}

impl Frame {
    pub fn new(width: u32, height: u32, sky: Sky, buffers: Buffers) -> Self {
        let Buffers {
            mut items,
            pixels,
            depth,
            picks,
        } = buffers;
        items.clear();
        let mut frame = Self {
            width,
            height,
            sky,
            items,
            pixels,
            depth,
            picks,
            identities: Vec::new(),
            anchor: None,
        };
        if sky.backdrop == Backdrop::Space {
            for i in 0..(width * height / 1100).clamp(200, 4000) {
                let hash = i.wrapping_mul(2654435761);
                let x = (hash % width) as f32;
                let y = (hash.rotate_left(13) % height) as f32;
                frame.items.push(Item::Star([x, y], [37, 48, 71]));
            }
        }
        frame
    }

    pub fn release(self) -> Buffers {
        Buffers {
            items: self.items,
            pixels: self.pixels,
            depth: self.depth,
            picks: self.picks,
        }
    }

    /// Rasterizes the recorded items on the CPU into pixels, depth and picking buffers.
    pub fn rasterize(&mut self) {
        let n = (self.width * self.height) as usize;
        // Every pixel is painted by the sky, so only the length matters here.
        self.pixels.resize(n * 3, 0);
        self.depth.clear();
        self.depth.resize(n, f32::NEG_INFINITY);
        self.picks.clear();
        self.picks.resize(n, NONE);
        self.paint_sky();
        let items = std::mem::take(&mut self.items);
        for item in &items {
            match *item {
                Item::Triangle([a, b, c], color, pick) => self.triangle(a, b, c, color, pick),
                Item::Line(a, b, color) => self.segment(a, b, color),
                Item::Sphere {
                    center,
                    radius,
                    depth,
                    color,
                    pick,
                    selected,
                } => self.ball(center, radius, depth, color, pick, selected),
                Item::Glow {
                    center,
                    radius,
                    color,
                    strength,
                } => self.halo(center, radius, color, strength),
                Item::Beam(a, b, color, weight) => self.ray(a, b, color, weight),
                Item::Star([x, y], color) => {
                    self.pixel(x as i32, y as i32, BACKGROUND, color, NONE)
                }
                Item::Glyph {
                    origin,
                    scale,
                    bits,
                    color,
                } => self.glyph(origin, scale, bits, color),
            }
        }
        self.items = items;
    }

    fn glyph(&mut self, origin: [f32; 2], scale: u32, bits: u128, color: Color) {
        for row in 0..glyphs::HEIGHT {
            for column in 0..glyphs::WIDTH {
                if bits >> (row * glyphs::WIDTH + column) & 1 == 0 {
                    continue;
                }
                for dy in 0..scale {
                    let y = origin[1] as i64 + (row * scale + dy) as i64;
                    if !(0..self.height as i64).contains(&y) {
                        continue;
                    }
                    for dx in 0..scale {
                        let x = origin[0] as i64 + (column * scale + dx) as i64;
                        if (0..self.width as i64).contains(&x) {
                            let i = (y as usize * self.width as usize + x as usize) * 3;
                            self.pixels[i..i + 3].copy_from_slice(&color);
                        }
                    }
                }
            }
        }
    }

    fn paint_sky(&mut self) {
        let (w, h) = (self.width as usize, self.height as usize);
        let sky = self.sky;
        let columns: Vec<f32> = (0..w)
            .map(|x| sky.profile(x as f32 / w as f32, 0.0))
            .collect();
        for y in 0..h {
            let v = y as f32 / h as f32;
            let base = sky.base(v);
            let row = sky.profile(v, 1.9);
            let line = &mut self.pixels[y * w * 3..(y + 1) * w * 3];
            if sky.hazy() {
                for (pixel, column) in line.as_chunks_mut::<3>().0.iter_mut().zip(&columns) {
                    let density = row * column;
                    for k in 0..3 {
                        pixel[k] = (base[k] + sky.haze[k] * density).min(255.0) as u8;
                    }
                }
            } else {
                let color = base.map(|c| c as u8);
                for pixel in line.as_chunks_mut::<3>().0 {
                    *pixel = color;
                }
            }
        }
    }

    /// Nearest pickable pixel within `radius` of (x, y); terminal mouse input is imprecise.
    pub fn pick_near(&self, x: f32, y: f32, radius: f32) -> Option<Identity> {
        if self.picks.is_empty() {
            return None;
        }
        let reach = radius.ceil() as i32;
        let (cx, cy) = (x as i32, y as i32);
        let mut best = None;
        let mut best_distance = radius * radius;
        for py in (cy - reach).max(0)..=(cy + reach).min(self.height as i32 - 1) {
            for px in (cx - reach).max(0)..=(cx + reach).min(self.width as i32 - 1) {
                let pick = self.picks[py as usize * self.width as usize + px as usize];
                let distance = (px as f32 + 0.5 - x).powi(2) + (py as f32 + 0.5 - y).powi(2);
                if pick != NONE && distance <= best_distance {
                    best_distance = distance;
                    best = Some(pick);
                }
            }
        }
        best.and_then(|index| self.identities.get(index as usize).copied())
    }

    /// Frame pixel position of a world point, if it lands inside the frame.
    pub fn locate(&self, camera: &Camera, p: Point) -> Option<[f32; 2]> {
        let q = self.project(camera, p);
        ((0.0..self.width as f32).contains(&q[0]) && (0.0..self.height as f32).contains(&q[1]))
            .then_some([q[0], q[1]])
    }

    /// Two-pixel annotation line drawn over the finished frame, ignoring depth.
    pub fn stroke(&mut self, a: [f32; 2], b: [f32; 2], color: Color) {
        let steps = (a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .min(4000.0)
            .ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let x = (a[0] + (b[0] - a[0]) * t) as i32;
            let y = (a[1] + (b[1] - a[1]) * t) as i32;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1)] {
                self.blend(x + dx, y + dy, color, 1.0);
            }
        }
    }

    /// Annotation circle over the finished frame.
    pub fn circle(&mut self, center: [f32; 2], radius: f32, color: Color) {
        let point = |i: usize| {
            let angle = i as f32 / 40.0 * TAU;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        };
        for i in 0..40 {
            self.stroke(point(i), point(i + 1), color);
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: Color, weight: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 3;
        for (channel, &target) in self.pixels[i..i + 3].iter_mut().zip(&color) {
            *channel = (*channel as f32 + (target as f32 - *channel as f32) * weight) as u8;
        }
    }

    fn pixel(&mut self, x: i32, y: i32, depth: f32, color: Color, pick: u32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = y as usize * self.width as usize + x as usize;
        if depth < self.depth[i] {
            return;
        }
        self.depth[i] = depth;
        self.picks[i] = pick;
        self.pixels[i * 3..i * 3 + 3].copy_from_slice(&color);
    }

    fn triangle(&mut self, a: Point, b: Point, c: Point, color: Color, pick: u32) {
        let edge = |a: Point, b: Point, x: f32, y: f32| {
            (x - a[0]) * (b[1] - a[1]) - (y - a[1]) * (b[0] - a[0])
        };
        let area = edge(a, b, c[0], c[1]);
        if area.abs() < 0.001 {
            return;
        }
        let min_x = a[0].min(b[0]).min(c[0]).floor().max(0.0) as i32;
        let max_x = a[0].max(b[0]).max(c[0]).ceil().min(self.width as f32 - 1.0) as i32;
        let min_y = a[1].min(b[1]).min(c[1]).floor().max(0.0) as i32;
        let max_y = a[1]
            .max(b[1])
            .max(c[1])
            .ceil()
            .min(self.height as f32 - 1.0) as i32;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let w0 = edge(b, c, x as f32 + 0.5, y as f32 + 0.5) / area;
                let w1 = edge(c, a, x as f32 + 0.5, y as f32 + 0.5) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    self.pixel(x, y, w0 * a[2] + w1 * b[2] + w2 * c[2], color, pick);
                }
            }
        }
    }

    fn segment(&mut self, a: Point, b: Point, color: Color) {
        let steps = (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            self.pixel(
                (a[0] + (b[0] - a[0]) * t) as i32,
                (a[1] + (b[1] - a[1]) * t) as i32,
                a[2] + (b[2] - a[2]) * t + 0.02,
                color,
                NONE,
            );
        }
    }

    fn ball(&mut self, p: Point, r: f32, depth: f32, color: Color, pick: u32, selected: bool) {
        let outer = if selected { r + 3.0 } else { r + 1.0 };
        let x0 = (p[0] - outer).floor().max(0.0) as i32;
        let x1 = (p[0] + outer).ceil().min(self.width as f32 - 1.0) as i32;
        let y0 = (p[1] - outer).floor().max(0.0) as i32;
        let y1 = (p[1] + outer).ceil().min(self.height as f32 - 1.0) as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let nx = (x as f32 - p[0]) / r;
                let ny = (y as f32 - p[1]) / r;
                let d = nx * nx + ny * ny;
                if d <= 1.0 {
                    let nz = (1.0 - d).sqrt();
                    let light = (0.25 + (-nx * 0.4 - ny * 0.5 + nz * 0.65).max(0.0)).min(1.2);
                    self.pixel(x, y, p[2] + nz * depth, tint(color, light), pick);
                } else if selected && d <= (outer / r).powi(2) {
                    self.pixel(x, y, p[2], [237, 220, 154], pick);
                }
            }
        }
    }

    fn halo(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        let x0 = (p[0] - radius).floor() as i32;
        let x1 = (p[0] + radius).ceil() as i32;
        let y0 = (p[1] - radius).floor() as i32;
        let y1 = (p[1] + radius).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - p[0]).hypot(y as f32 + 0.5 - p[1]) / radius;
                if d < 1.0 {
                    self.blend(x, y, color, strength * (1.0 - d) * (1.0 - d));
                }
            }
        }
    }

    fn ray(&mut self, a: [f32; 2], b: [f32; 2], color: Color, weight: f32) {
        let steps = (a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .min(8000.0)
            .ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps.max(1) as f32;
            let x = (a[0] + (b[0] - a[0]) * t) as i32;
            let y = (a[1] + (b[1] - a[1]) * t) as i32;
            self.blend(x, y, color, weight);
        }
    }

    pub(crate) fn project(&self, camera: &Camera, p: Point) -> Point {
        let v = camera.view(p);
        [
            self.width as f32 * 0.5 + v[0] * camera.zoom,
            self.height as f32 * ORIGIN_Y + v[1] * camera.zoom,
            v[2],
        ]
    }

    pub(crate) fn facet(&mut self, camera: &Camera, points: [Point; 3], color: Color, pick: u32) {
        let p = points.map(|p| self.project(camera, p));
        self.items.push(Item::Triangle(p, color, pick));
    }

    pub(crate) fn quad(&mut self, camera: &Camera, points: [Point; 4], color: Color, pick: u32) {
        let p = points.map(|p| self.project(camera, p));
        self.items
            .push(Item::Triangle([p[0], p[1], p[2]], color, pick));
        self.items
            .push(Item::Triangle([p[0], p[2], p[3]], color, pick));
    }

    pub(crate) fn line(&mut self, camera: &Camera, a: Point, b: Point, color: Color) {
        let a = self.project(camera, a);
        let b = self.project(camera, b);
        // Skip unbounded work when the camera is far inside a large scene.
        if (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) <= 12000.0 {
            self.items.push(Item::Line(a, b, color));
        }
    }

    fn trace(&mut self, camera: &Camera, orbit: &Orbit, color: Color) {
        let mut last = orbit.point(0.0);
        for segment in 1..=72 {
            let next = orbit.point(segment as f32 / 72.0 * TAU);
            self.line(camera, last, next, color);
            last = next;
        }
    }

    pub(crate) fn glow(
        &mut self,
        camera: &Camera,
        position: Point,
        radius: f32,
        color: Color,
        strength: f32,
    ) {
        let p = self.project(camera, position);
        self.items.push(Item::Glow {
            center: [p[0], p[1]],
            radius,
            color,
            strength,
        });
    }

    pub(crate) fn beam(&mut self, camera: &Camera, a: Point, b: Point, color: Color, weight: f32) {
        let a = self.project(camera, a);
        let b = self.project(camera, b);
        if (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) <= 12000.0 {
            self.items
                .push(Item::Beam([a[0], a[1]], [b[0], b[1]], color, weight));
        }
    }

    pub(crate) fn sphere(
        &mut self,
        camera: &Camera,
        position: Point,
        radius: f32,
        color: Color,
        pick: u32,
        selected: bool,
    ) {
        let center = self.project(camera, position);
        self.items.push(Item::Sphere {
            center,
            radius: (radius * camera.zoom).max(MIN_BODY_PX),
            depth: radius,
            color,
            pick,
            selected,
        });
    }

    /// A sphere at its exact projected size, for large bodies with geometry drawn on their
    /// surface. `sphere` draws bodies at `radius * zoom` pixels, about 18% smaller than their
    /// projection, which the other views' spacing was tuned with.
    pub(crate) fn world_sphere(
        &mut self,
        camera: &Camera,
        position: Point,
        radius: f32,
        color: Color,
    ) {
        let center = self.project(camera, position);
        self.items.push(Item::Sphere {
            center,
            radius: radius * camera.zoom * SCALE,
            depth: radius,
            color,
            pick: NONE,
            selected: false,
        });
    }

    pub fn write_png(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(path)?),
            self.width,
            self.height,
        );
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&self.pixels)?;
        Ok(())
    }
}

/// Spawn animation progress, 0 to 1, easing out over GROW_SECONDS after a process appears.
fn growth(births: &HashMap<Identity, f32>, id: Identity, time: f32) -> f32 {
    let born = births.get(&id).copied().unwrap_or(f32::NEG_INFINITY);
    if time < born {
        return 1.0;
    }
    let t = ((time - born) / GROW_SECONDS).min(1.0);
    1.0 - (1.0 - t).powi(3)
}

/// What a view module draws with and reports back to the scene.
pub(crate) struct Stage<'a> {
    pub frame: &'a mut Frame,
    pub camera: &'a Camera,
    pub selected: Option<Identity>,
    pub time: f32,
    births: &'a HashMap<Identity, f32>,
    /// Where each drawn process is, for picking anchors, labels and the tour.
    pub positions: &'a mut HashMap<Identity, Point>,
    /// World-anchored text labels.
    pub places: &'a mut Vec<(Point, String)>,
    /// Points the camera fit must include; empty means fit the positions on the ground.
    pub bounds: &'a mut Vec<Point>,
    /// Extra lines for a process's inspector popup.
    pub notes: &'a mut HashMap<Identity, Vec<String>>,
}

impl Stage<'_> {
    pub fn growth(&self, id: Identity) -> f32 {
        growth(self.births, id, self.time)
    }
}

/// Points around a horizontal circle at two heights, for fitting the camera to a round view.
pub(crate) fn ring_bounds(center: [f32; 2], radius: f32, low: f32, high: f32) -> Vec<Point> {
    (0..24)
        .flat_map(|k| {
            let angle = k as f32 / 24.0 * TAU;
            let [x, y] = [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ];
            [[x, y, low], [x, y, high]]
        })
        .collect()
}

/// The longest process name a scene label shows, in terminal cells. Linux `comm` stops here, so
/// only the longer macOS names are ever cut.
pub const LABEL_NAME_CELLS: usize = 15;

/// A name without Apple's own `com.apple.` prefix, which says nothing on macOS. Other vendors'
/// reverse-DNS names stay whole, and so does a name that is nothing but the prefix.
pub fn without_apple_prefix(name: &str) -> &str {
    name.strip_prefix("com.apple.")
        .filter(|rest| !rest.is_empty())
        .unwrap_or(name)
}

/// A process name as a scene label: the Apple prefix dropped, then anything still longer than
/// `LABEL_NAME_CELLS` keeps its first 13 characters and ends in `..`, so a label is never wider
/// than 15 cells. The ellipsis is ASCII because `terminal::clean_text` turns every other character
/// into `?`. Hover tags, the inspector and search keep the full name.
pub fn label_name(name: &str) -> Cow<'_, str> {
    let name = without_apple_prefix(name);
    if name.chars().count() <= LABEL_NAME_CELLS {
        return Cow::Borrowed(name);
    }
    let mut cut: String = name.chars().take(LABEL_NAME_CELLS - 2).collect();
    cut.push_str("..");
    Cow::Owned(cut)
}

pub fn tint(color: Color, scale: f32) -> Color {
    color.map(|v| (v as f32 * scale).clamp(0.0, 255.0) as u8)
}

fn process_color(process: &Process, group: usize) -> Color {
    match process.state {
        'Z' => [213, 97, 126],
        'T' | 't' => [225, 161, 75],
        'D' => [222, 94, 73],
        _ => PALETTE[group % PALETTE.len()],
    }
}

/// Orbit colours by role: slate kernel, teal system, amber session, violet containers.
pub(crate) fn kind_color(process: &Process) -> Color {
    let base = match (process.state, process.kind) {
        ('Z', _) => return [213, 97, 126],
        ('T' | 't', _) => return [225, 161, 75],
        ('D', _) => return [222, 94, 73],
        (_, Kind::Kernel) => [104, 122, 156],
        (_, Kind::System) => [72, 183, 199],
        (_, Kind::Session) => [236, 178, 92],
        (_, Kind::Container) => [178, 136, 232],
    };
    let jitter = (process.id.pid.wrapping_mul(2_654_435_761) >> 24) as f32 / 255.0;
    tint(base, 0.86 + 0.24 * jitter)
}

#[derive(Clone, Copy)]
struct Plot {
    district: usize,
    slot: usize,
}

/// A Keplerian ellipse with its focus on the parent body.
#[derive(Clone, Copy)]
struct Orbit {
    center: Point,
    a: f32,
    e: f32,
    omega: f32,
}

impl Orbit {
    fn point(&self, eccentric: f32) -> Point {
        let (s, c) = eccentric.sin_cos();
        let x = self.a * (c - self.e);
        let y = self.a * (1.0 - self.e * self.e).sqrt() * s;
        let (ws, wc) = self.omega.sin_cos();
        [
            self.center[0] + x * wc - y * ws,
            self.center[1] + x * ws + y * wc,
            self.center[2],
        ]
    }

    fn at(&self, mean: f32) -> Point {
        self.point(eccentric_anomaly(mean, self.e))
    }
}

struct Path {
    orbit: Orbit,
    depth: usize,
    id: Identity,
    spoke: bool,
    mean: f32,
    position: Point,
    cpu: f32,
}

struct Body {
    index: usize,
    position: Point,
    grow: f32,
}

struct Tree<'a> {
    processes: &'a [&'a Process],
    children: Vec<Vec<usize>>,
    extents: Vec<f32>,
    /// Body radius from the memory of the whole subtree a body anchors.
    radii: Vec<f32>,
    /// Radius including thread rings, which neighbours must clear.
    reach: Vec<f32>,
    masses: Vec<f32>,
}

pub struct Scene {
    plots: HashMap<Identity, Plot>,
    groups: HashMap<String, Vec<usize>>,
    occupancy: Vec<HashMap<usize, Identity>>,
    resources: HashMap<Identity, (f32, f32)>,
    systems: HashMap<Identity, ([f32; 2], f32)>,
    /// Point the orbit systems revolve about and the systems it was computed for, the motion
    /// time they were last turned to, and each system's angular speed in radians per second.
    galaxy: [f32; 2],
    galaxy_members: Vec<Identity>,
    galaxy_time: Option<f32>,
    galaxy_speed: HashMap<Identity, f32>,
    /// Frames rendered, and the frame the galaxy last turned in: it only turns across
    /// consecutive frames, so switching to another view and back never spins it.
    frames: u64,
    galaxy_frame: u64,
    last_time: Option<f32>,
    births: HashMap<Identity, f32>,
    previous: HashMap<Identity, Point>,
    deaths: Vec<(Point, f32)>,
    /// Height of the tallest building in the last city frame, so fitting leaves it headroom.
    tallest: f32,
    /// Exits and births since the last frame, as disturbances for the ripple and flow media.
    splashes: Vec<Point>,
    newborn: Vec<Identity>,
    wave: Option<Wave>,
    flow: Option<Flow>,
    /// Ripple pebble clusters by cgroup, and each process's cluster and slot in it.
    clusters: HashMap<String, Cluster>,
    pebbles: HashMap<Identity, (String, usize)>,
    started: bool,
    /// Animate the processes that are already running when the first frame is drawn.
    pub intro: bool,
    /// Which socket links to draw, and which processes' links stand out.
    pub links: Links,
    pub highlight: Vec<Identity>,
    pub positions: HashMap<Identity, Point>,
    /// Orbit system roots: identity, bodies drawn, and reach from the star in world units.
    pub stars: Vec<(Identity, usize, f32)>,
    /// For orbit bodies with children: memory of the subtree they anchor and its process count.
    pub mass: HashMap<Identity, (u64, usize)>,
    pub places: Vec<(Point, String)>,
    bounds: Vec<Point>,
    pub notes: HashMap<Identity, Vec<String>>,
    cores: cores::Track,
    cells: cells::Dishes,
    strata: strata::Strata,
    globe: globe::Globe,
    reef: reef::Reef,
    pub coop: coop::Coop,
    pub matrix: matrix::Rain,
    /// Allocations recycled from the previous frame.
    pub spare: Buffers,
    pub visible: usize,
    pub collapsed: usize,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            plots: HashMap::new(),
            groups: HashMap::new(),
            occupancy: Vec::new(),
            resources: HashMap::new(),
            systems: HashMap::new(),
            galaxy: [0.0; 2],
            galaxy_members: Vec::new(),
            galaxy_time: None,
            galaxy_speed: HashMap::new(),
            frames: 0,
            galaxy_frame: 0,
            last_time: None,
            births: HashMap::new(),
            previous: HashMap::new(),
            deaths: Vec::new(),
            tallest: 0.0,
            splashes: Vec::new(),
            newborn: Vec::new(),
            wave: None,
            flow: None,
            clusters: HashMap::new(),
            pebbles: HashMap::new(),
            started: false,
            intro: false,
            links: Links::All,
            highlight: Vec::new(),
            positions: HashMap::new(),
            stars: Vec::new(),
            mass: HashMap::new(),
            places: Vec::new(),
            bounds: Vec::new(),
            notes: HashMap::new(),
            cores: cores::Track::default(),
            cells: cells::Dishes::default(),
            strata: strata::Strata::default(),
            globe: globe::Globe::default(),
            reef: reef::Reef::default(),
            coop: coop::Coop::default(),
            matrix: matrix::Rain::default(),
            spare: Buffers::default(),
            visible: 0,
            collapsed: 0,
        }
    }

    fn growth(&self, id: Identity, time: f32) -> f32 {
        growth(&self.births, id, time)
    }

    /// Whether the last view drawn gave explicit bounds; such views grow as data arrives.
    pub fn bounded(&self) -> bool {
        !self.bounds.is_empty()
    }

    /// Feeds a new sample to views that keep their own history, whichever view is shown.
    pub fn record(&mut self, snapshot: &Snapshot, time: f32) {
        self.strata.record(snapshot, time);
        self.coop.record(snapshot);
    }

    #[cfg(test)]
    fn strata_rows(&self) -> Vec<Identity> {
        self.strata.rows()
    }

    fn city_positions(&mut self, processes: &[&Process], snapshot: &Snapshot) {
        let alive: HashSet<_> = snapshot.processes.iter().map(|p| p.id).collect();
        self.plots.retain(|id, plot| {
            if alive.contains(id) {
                true
            } else {
                self.occupancy[plot.district].remove(&plot.slot);
                false
            }
        });
        self.positions.clear();
        for process in processes {
            if !self.plots.contains_key(&process.id) {
                let districts = self.groups.entry(process.group.clone()).or_default();
                let mut vacant = None;
                for &district in districts.iter() {
                    if let Some(slot) = (0..16).find(|s| !self.occupancy[district].contains_key(s))
                    {
                        vacant = Some(Plot { district, slot });
                        break;
                    }
                }
                let plot = vacant.unwrap_or_else(|| {
                    let district = self.occupancy.len();
                    self.occupancy.push(HashMap::new());
                    districts.push(district);
                    Plot { district, slot: 0 }
                });
                self.occupancy[plot.district].insert(plot.slot, process.id);
                self.plots.insert(process.id, plot);
            }
            let plot = self.plots[&process.id];
            let base = district_origin(plot.district);
            self.positions.insert(
                process.id,
                [
                    base[0] + (plot.slot % 4) as f32 * 4.0 + 2.0,
                    base[1] + (plot.slot / 4) as f32 * 4.0 + 2.0,
                    0.0,
                ],
            );
        }
    }

    pub fn fit(&self, camera: &mut Camera, width: u32, height: u32) {
        if self.positions.is_empty() && self.bounds.is_empty() {
            return;
        }
        let probe = Camera {
            center: [0.0, 0.0],
            zoom: 1.0,
            ..camera.clone()
        };
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        let points: Vec<Point> = if self.bounds.is_empty() {
            self.positions.values().map(|p| [p[0], p[1], 0.0]).collect()
        } else {
            self.bounds.clone()
        };
        for &p in &points {
            let q = probe.view(p);
            for i in 0..2 {
                min[i] = min[i].min(q[i]);
                max[i] = max[i].max(q[i]);
            }
        }
        camera.center = probe.ground((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5);
        if !self.bounds.is_empty() {
            camera.zoom =
                (width as f32 / (max[0] - min[0])).min(height as f32 / (max[1] - min[1])) * 0.9;
            // Frames are drawn around a point ORIGIN_Y down the screen; centre the bounds exactly.
            let lift = (ORIGIN_Y - 0.5) * height as f32 / camera.zoom;
            camera.center = probe.ground((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5 + lift);
            return;
        }
        // The vertical margin leaves headroom for the tallest building above the ground plane.
        let headroom = 24.0_f32.max(self.tallest * camera.pitch.cos() * SCALE + 8.0);
        camera.zoom = (width as f32 / (max[0] - min[0] + 12.0))
            .min(height as f32 / (max[1] - min[1] + headroom))
            * 0.8;
    }

    /// Records a frame; call `Frame::rasterize` (or a GPU backend) to turn it into pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        snapshot: &Snapshot,
        view: View,
        camera: &Camera,
        width: u32,
        height: u32,
        selected: Option<Identity>,
        time: f32,
        limit: usize,
        focus: Option<Identity>,
    ) -> Frame {
        self.frames += 1;
        let dt = self.last_time.map_or(0.0, |previous| time - previous);
        if !(0.0..=2.0).contains(&dt) {
            self.resources.clear();
        }
        self.last_time = Some(time);
        let alpha = 1.0 - (-dt.max(0.0) / 0.3).exp();
        let alive: HashSet<_> = snapshot.processes.iter().map(|p| p.id).collect();
        self.resources.retain(|id, _| alive.contains(id));
        self.births.retain(|id, _| alive.contains(id));
        let born = if self.started || self.intro {
            time
        } else {
            f32::NEG_INFINITY
        };
        let fresh = self.started;
        self.started = true;
        for p in &snapshot.processes {
            if let std::collections::hash_map::Entry::Vacant(entry) = self.births.entry(p.id) {
                entry.insert(born);
                if fresh {
                    self.newborn.push(p.id);
                }
            }
        }
        let visual: Vec<Process> = snapshot
            .processes
            .iter()
            .map(|p| {
                let memory = p.memory as f32;
                let current = self.resources.entry(p.id).or_insert((p.cpu, memory));
                current.0 += alpha * (p.cpu - current.0);
                current.1 += alpha * (memory - current.1);
                let mut p = p.clone();
                p.cpu = current.0;
                p.memory = current.1 as u64;
                p
            })
            .collect();
        let allowed = focus.map(|root| descendants(snapshot, root));
        let mut processes: Vec<&Process> = visual
            .iter()
            .filter(|p| allowed.as_ref().is_none_or(|set| set.contains(&p.id)))
            .collect();
        processes.sort_by_key(|p| p.id);
        if processes.len() > limit {
            // A stable PID order keeps resource changes from continuously evicting buildings.
            if let Some(id) = selected
                && let Some(index) = processes.iter().position(|p| p.id == id)
                && index >= limit
            {
                processes.swap(index, limit - 1);
            }
            processes.truncate(limit);
        }
        let [cpu, memory, io] = snapshot.pressure;
        let (c, m, i) = if view == View::Matrix {
            (0.0, 0.0, 0.0)
        } else {
            (bounded(cpu, 20.0), bounded(memory, 10.0), bounded(io, 20.0))
        };
        let sky = Sky {
            backdrop: view.backdrop(),
            haze: [
                c * 110.0 + m * 140.0 + i * 30.0,
                c * 70.0 + m * 20.0 + i * 50.0,
                c * 10.0 + m * 60.0 + i * 150.0,
            ],
            time,
        };
        let mut frame = Frame::new(width, height, sky, std::mem::take(&mut self.spare));
        frame.identities = processes.iter().map(|p| p.id).collect();
        self.visible = processes.len();
        self.collapsed = 0;
        self.stars.clear();
        self.mass.clear();
        self.places.clear();
        self.bounds.clear();
        self.notes.clear();
        if view != View::City {
            self.tallest = 0.0;
        }
        match view {
            View::City => {
                self.city_positions(&processes, snapshot);
                self.draw_city(&mut frame, &processes, camera, selected, time);
            }
            View::Orbit => self.draw_orbits(&mut frame, &processes, camera, selected, time),
            View::Ripple => {
                let stir = bounded(cpu + io, 20.0);
                self.draw_ripples(&mut frame, &processes, camera, selected, time, stir)
            }
            View::Flow => {
                let stir = bounded(cpu + memory + io, 25.0);
                self.draw_flow(
                    &mut frame, &processes, camera, selected, time, snapshot, stir,
                )
            }
            View::Matrix => {
                self.positions.clear();
                self.matrix.draw(&mut frame, time);
                self.visible = 0;
            }
            View::Cores | View::Cells | View::Strata | View::Globe | View::Reef | View::Coop => {
                self.positions.clear();
                let mut stage = Stage {
                    frame: &mut frame,
                    camera,
                    selected,
                    time,
                    births: &self.births,
                    positions: &mut self.positions,
                    places: &mut self.places,
                    bounds: &mut self.bounds,
                    notes: &mut self.notes,
                };
                let shown = match view {
                    View::Cores => self.cores.draw(&mut stage, &processes, snapshot),
                    View::Cells => self.cells.draw(&mut stage, &processes, snapshot),
                    View::Strata => self.strata.draw(&mut stage, &processes, snapshot),
                    View::Globe => self.globe.draw(&mut stage, &processes, snapshot),
                    View::Reef => self.reef.draw(&mut stage, &processes, snapshot),
                    View::Coop => self.coop.draw(&mut stage, &processes, snapshot),
                    _ => unreachable!(),
                };
                self.visible = shown;
                self.collapsed = processes.len().saturating_sub(shown);
            }
        }
        self.splashes.clear();
        self.newborn.clear();
        let lift = if view == View::City { 1.5 } else { 0.0 };
        // Only the city and orbit draw every link; elsewhere arcs remain for highlighted processes.
        let quiet = !matches!(view, View::City | View::Orbit | View::Ripple);
        self.draw_links(&mut frame, camera, snapshot, time, lift, quiet);
        for (id, &point) in &self.previous {
            if !alive.contains(id) {
                self.deaths.push((point, time));
                self.splashes.push(point);
            }
        }
        self.previous = self.positions.clone();
        self.deaths
            .retain(|&(_, died)| (0.0..FADE_SECONDS).contains(&(time - died)));
        for &(point, died) in &self.deaths {
            let age = (time - died) / FADE_SECONDS;
            let ring = Orbit {
                center: point,
                a: 0.6 + 3.4 * age,
                e: 0.0,
                omega: 0.0,
            };
            frame.trace(camera, &ring, tint([255, 196, 140], 1.0 - age));
            if age < 0.35 {
                let radius = (2.0 * camera.zoom).max(10.0);
                frame.glow(camera, point, radius, [255, 226, 186], 1.0 - age / 0.35);
            }
        }
        frame.anchor = selected
            .and_then(|id| self.positions.get(&id))
            .and_then(|&p| frame.locate(camera, p));
        frame
    }

    /// Socket links as arcs lifted above the plane with travelling pulses, and connections that
    /// leave the machine as beams rising off the body. Links touching highlighted processes glow.
    fn draw_links(
        &self,
        frame: &mut Frame,
        camera: &Camera,
        snapshot: &Snapshot,
        time: f32,
        lift: f32,
        quiet: bool,
    ) {
        if self.links == Links::Off {
            return;
        }
        let mode = if quiet { Links::Focused } else { self.links };
        let hot = |id: &Identity| self.highlight.contains(id);
        for &(a, b, count) in &snapshot.links {
            let (Some(&pa), Some(&pb)) = (self.positions.get(&a), self.positions.get(&b)) else {
                continue;
            };
            let glowing = hot(&a) || hot(&b);
            if mode == Links::Focused && !glowing {
                continue;
            }
            let (color, weight) = if glowing {
                (LINK_HOT, 0.8)
            } else {
                (LINK, (0.1 + 0.05 * (count as f32).ln()).min(0.3))
            };
            let span = (pa[0] - pb[0]).hypot(pa[1] - pb[1]);
            let start = [pa[0], pa[1], pa[2] + lift];
            let end = [pb[0], pb[1], pb[2] + lift];
            let control = [
                (pa[0] + pb[0]) * 0.5,
                (pa[1] + pb[1]) * 0.5,
                (pa[2] + pb[2]) * 0.5 + lift + span * 0.35,
            ];
            let curve = |t: f32| {
                let u = 1.0 - t;
                [0, 1, 2].map(|k| u * u * start[k] + 2.0 * u * t * control[k] + t * t * end[k])
            };
            let mut last = start;
            for step in 1..=20 {
                let next = curve(step as f32 / 20.0);
                frame.beam(camera, last, next, color, weight);
                last = next;
            }
            let phase = (a.pid.wrapping_mul(31) ^ b.pid.wrapping_mul(17)) % 100;
            let t = (time * 0.35 + phase as f32 / 100.0).fract();
            let strength = if glowing { 0.9 } else { 0.35 };
            frame.glow(camera, curve(t), 5.0, color, strength);
        }
        for (id, &count) in &snapshot.outside {
            let Some(&p) = self.positions.get(id) else {
                continue;
            };
            let glowing = hot(id);
            if mode == Links::Focused && !glowing {
                continue;
            }
            let height = 5.0 + 3.0 * (count as f32).ln_1p();
            let strength = if glowing { 0.9 } else { 0.45 };
            for step in 0..10 {
                let a = [p[0], p[1], p[2] + lift + height * step as f32 / 10.0];
                let b = [p[0], p[1], p[2] + lift + height * (step + 1) as f32 / 10.0];
                frame.beam(camera, a, b, OUTSIDE, strength * (1.0 - step as f32 / 10.0));
            }
        }
    }

    fn draw_city(
        &mut self,
        frame: &mut Frame,
        processes: &[&Process],
        camera: &Camera,
        selected: Option<Identity>,
        time: f32,
    ) {
        let districts: HashSet<_> = processes
            .iter()
            .map(|p| self.plots[&p.id].district)
            .collect();
        for district in districts {
            let [x, y] = district_origin(district);
            frame.quad(
                camera,
                [
                    [x - 1.0, y - 1.0, -0.12],
                    [x + 17.0, y - 1.0, -0.12],
                    [x + 17.0, y + 17.0, -0.12],
                    [x - 1.0, y + 17.0, -0.12],
                ],
                [18, 29, 43],
                NONE,
            );
            let color = tint(PALETTE[district % 8], 0.35);
            for i in 0..=4 {
                let offset = i as f32 * 4.0;
                frame.line(
                    camera,
                    [x + offset, y, 0.0],
                    [x + offset, y + 16.0, 0.0],
                    [29, 43, 57],
                );
                frame.line(
                    camera,
                    [x, y + offset, 0.0],
                    [x + 16.0, y + offset, 0.0],
                    [29, 43, 57],
                );
            }
            frame.line(
                camera,
                [x - 0.5, y - 0.5, 0.0],
                [x + 16.5, y - 0.5, 0.0],
                color,
            );
            frame.line(
                camera,
                [x - 0.5, y - 0.5, 0.0],
                [x - 0.5, y + 16.5, 0.0],
                color,
            );
        }
        let mut tallest = 0.0_f32;
        for (index, process) in processes.iter().enumerate() {
            let p = self.positions[&process.id];
            let plot = self.plots[&process.id];
            let grow = self.growth(process.id, time);
            let half = (0.42 + 1.05 * bounded(process.memory as f32 / 1048576.0, 220.0).sqrt())
                * (0.35 + 0.65 * grow);
            // Idle processes stay low blocks; one busy core is a ~23-unit tower and multi-core
            // work climbs towards 45, against 4-unit plots.
            let height = (1.0 + 44.0 * bounded(process.cpu, 100.0)) * grow.max(0.05);
            tallest = tallest.max(height);
            let shadow = height.min(14.0);
            let color = process_color(process, plot.district);
            let [x, y, _] = p;
            frame.quad(
                camera,
                [
                    [x - half, y - half, 0.01],
                    [x + half + shadow * 0.5, y - half + shadow * 0.3, 0.01],
                    [x + half + shadow * 0.5, y + half + shadow * 0.3, 0.01],
                    [x - half, y + half, 0.01],
                ],
                [10, 18, 29],
                NONE,
            );
            let corners = [
                [x - half, y - half, 0.05],
                [x + half, y - half, 0.05],
                [x + half, y + half, 0.05],
                [x - half, y + half, 0.05],
            ];
            let top = corners.map(|mut p| {
                p[2] = height;
                p
            });
            for side in 0..4 {
                let next = (side + 1) % 4;
                frame.quad(
                    camera,
                    [corners[side], corners[next], top[next], top[side]],
                    tint(color, [0.40, 0.66, 0.55, 0.30][side]),
                    index as u32,
                );
                if camera.zoom > 3.0 {
                    let levels = (height / 0.85) as usize;
                    for level in 0..levels {
                        for column in 0..3 {
                            let a = 0.15 + column as f32 * 0.26;
                            let b = a + 0.12;
                            let z = 0.35 + level as f32 * 0.85;
                            if z + 0.25 >= height {
                                continue;
                            }
                            let point = |t: f32, z: f32| {
                                [
                                    corners[side][0] + (corners[next][0] - corners[side][0]) * t,
                                    corners[side][1] + (corners[next][1] - corners[side][1]) * t,
                                    z,
                                ]
                            };
                            let lit = !(process.id.pid as usize + level * 7 + column * 11)
                                .is_multiple_of(5);
                            let window = if lit {
                                tint([241, 206, 136], 0.55 + 0.35 * bounded(process.cpu, 50.0))
                            } else {
                                [28, 40, 53]
                            };
                            let mut points = [
                                point(a, z),
                                point(b, z),
                                point(b, z + 0.25),
                                point(a, z + 0.25),
                            ];
                            // Offset windows outward so the depth buffer doesn't z-fight with the wall.
                            let normal =
                                [[0.0, -0.015], [0.015, 0.0], [0.0, 0.015], [-0.015, 0.0]][side];
                            for p in &mut points {
                                p[0] += normal[0];
                                p[1] += normal[1];
                            }
                            frame.quad(camera, points, window, index as u32);
                        }
                    }
                }
            }
            frame.quad(camera, top, tint(color, 0.95), index as u32);
            if process.cpu > 1.0 {
                let brightness = 0.65 + 0.25 * (time * 2.0 + process.id.pid as f32).sin();
                frame.sphere(
                    camera,
                    [x, y, height + 0.22],
                    0.12,
                    tint([255, 196, 104], brightness),
                    index as u32,
                    false,
                );
            }
            if process.gpu_memory > 0 {
                let beacon = [x - half * 0.5, y - half * 0.5, height + 0.3];
                frame.glow(camera, beacon, (1.4 * camera.zoom).max(8.0), NVIDIA, 0.7);
                frame.sphere(camera, beacon, 0.16, NVIDIA, index as u32, false);
            }
            if selected == Some(process.id) {
                for side in 0..4 {
                    let next = (side + 1) % 4;
                    frame.line(camera, top[side], top[next], [255, 235, 171]);
                    frame.line(camera, corners[side], top[side], [255, 235, 171]);
                }
            }
            if process.io_rate.is_some_and(|v| v > 1024.0) {
                let t = (time * 0.7 + process.id.pid as f32 * 0.13).fract();
                frame.sphere(
                    camera,
                    [x + half + 0.2, y - half + t * half * 2.0, 0.15],
                    0.12,
                    [105, 228, 216],
                    index as u32,
                    false,
                );
            }
        }
        self.tallest = tallest;
    }

    /// Lays out the process tree as nested Keplerian systems. `time` drives spawn growth and
    /// `motion` the orbital phase, so other views can reuse the layout with bodies held still.
    fn layout<'a>(
        &mut self,
        processes: &'a [&'a Process],
        time: f32,
        motion: f32,
    ) -> (Tree<'a>, Vec<Path>, Vec<Body>) {
        self.positions.clear();
        let by_pid: HashMap<u32, usize> = processes
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id.pid, i))
            .collect();
        let parents: Vec<Option<usize>> = processes
            .iter()
            .enumerate()
            .map(|(i, p)| by_pid.get(&p.parent).copied().filter(|&parent| parent != i))
            .collect();
        let mut child_counts = vec![0; processes.len()];
        for &parent in parents.iter().flatten() {
            child_counts[parent] += 1;
        }
        let mut children = vec![Vec::new(); processes.len()];
        let mut roots = Vec::new();
        for (i, parent) in parents.iter().enumerate() {
            match *parent {
                // Branches hanging off a top-level process (init, kthreadd, a focused root) become
                // their own systems; leaves stay in orbit so kernel threads form one belt.
                Some(parent) if child_counts[i] == 0 || parents[parent].is_some() => {
                    children[parent].push(i)
                }
                _ => roots.push(i),
            }
        }
        for list in &mut children {
            list.sort_by_key(|&child| (child_counts[child] == 0, processes[child].id));
        }
        let n = processes.len();
        let mut masses = vec![0.0; n];
        let mut members = vec![0; n];
        let mut weighed = HashSet::new();
        for &root in &roots {
            weigh(
                processes,
                &children,
                root,
                &mut masses,
                &mut members,
                &mut weighed,
            );
        }
        let radii: Vec<f32> = masses.iter().map(|&m| mass_radius(m)).collect();
        let reach: Vec<f32> = radii
            .iter()
            .zip(processes)
            .map(|(&r, p)| r * ring_scale(p.threads))
            .collect();
        for (i, kids) in children.iter().enumerate() {
            if !kids.is_empty() {
                self.mass
                    .insert(processes[i].id, (masses[i] as u64, members[i]));
            }
        }
        let mut extents = vec![0.0; n];
        let mut measured = HashSet::new();
        for &root in &roots {
            measure(
                &children,
                &reach,
                processes,
                root,
                0,
                &mut extents,
                &mut measured,
            );
        }
        let tree = Tree {
            processes,
            children,
            extents,
            radii,
            reach,
            masses,
        };
        let centers = self.place_systems(processes, &roots, &tree.extents, &tree.masses, motion);
        let mut visited = HashSet::new();
        let mut paths = Vec::new();
        let mut bodies = Vec::new();
        for (&root, center) in roots.iter().zip(centers) {
            let before = visited.len();
            let point = [center[0], center[1], 1.0];
            let grow = self.growth(processes[root].id, time);
            self.orbit_branch(
                &tree,
                root,
                point,
                point,
                grow,
                0,
                [time, motion],
                &mut visited,
                &mut paths,
                &mut bodies,
            );
            self.stars.push((
                processes[root].id,
                visited.len() - before,
                tree.extents[root],
            ));
        }
        self.collapsed = processes.len().saturating_sub(visited.len());
        self.visible = visited.len();
        (tree, paths, bodies)
    }

    fn draw_orbits(
        &mut self,
        frame: &mut Frame,
        processes: &[&Process],
        camera: &Camera,
        selected: Option<Identity>,
        time: f32,
    ) {
        let (tree, paths, bodies) = self.layout(processes, time, time);
        let mut drawn = HashSet::new();
        for path in &paths {
            let highlighted = selected == Some(path.id);
            let ring = (
                (path.orbit.center[0] * 8.0) as i32,
                (path.orbit.center[1] * 8.0) as i32,
                (path.orbit.a * 8.0) as i32,
            );
            let color = if highlighted {
                [163, 147, 99]
            } else {
                tint([64, 90, 122], 1.0 / (1.0 + path.depth as f32 * 0.3))
            };
            if highlighted || drawn.insert(ring) {
                frame.trace(camera, &path.orbit, color);
            }
            if path.spoke || highlighted {
                frame.line(camera, path.orbit.center, path.position, tint(color, 0.6));
            }
        }
        for path in paths.iter().filter(|path| path.cpu > 3.0) {
            let length = 2 + (14.0 * bounded(path.cpu, 40.0)) as usize;
            let step = (0.6 / path.orbit.a).min(0.25);
            let mut last = path.position;
            for k in 1..=length {
                let point = path.orbit.at(path.mean - k as f32 * step);
                let fade = 1.0 - k as f32 / (length + 1) as f32;
                frame.line(camera, last, point, tint(WARM, 0.2 + 0.8 * fade));
                last = point;
            }
        }
        for body in &bodies {
            let process = processes[body.index];
            let core = (tree.radii[body.index] * camera.zoom).max(MIN_BODY_PX);
            if process.cpu > 1.0 {
                let radius = core * (2.2 + 2.0 * bounded(process.cpu, 50.0));
                let strength = 0.3 + 0.6 * bounded(process.cpu, 40.0);
                frame.glow(camera, body.position, radius, WARM, strength);
            }
            if process.gpu_memory > 0 {
                let mib = process.gpu_memory as f32 / 1048576.0;
                let radius = core * (2.6 + 0.5 * (mib / 64.0).ln_1p());
                frame.glow(camera, body.position, radius, NVIDIA, 0.6);
            }
        }
        for body in &bodies {
            let process = processes[body.index];
            frame.sphere(
                camera,
                body.position,
                tree.radii[body.index] * body.grow,
                kind_color(process),
                body.index as u32,
                selected == Some(process.id),
            );
        }
        for body in &bodies {
            let process = processes[body.index];
            let radius = tree.radii[body.index] * body.grow;
            if process.gpu_memory > 0 {
                let halo = Orbit {
                    center: body.position,
                    a: radius * 1.9,
                    e: 0.0,
                    omega: 0.0,
                };
                frame.trace(camera, &halo, tint(NVIDIA, 0.8));
            }
            let count = rings(process.threads);
            if count == 0 {
                continue;
            }
            // Saturn-style rings in a plane tilted towards the viewer, so the front arc passes over
            // the body while the depth buffer hides the back arc behind it.
            let yaw = (process.id.pid.wrapping_mul(2_246_822_519) >> 20) as f32 / 4096.0 * TAU;
            let (ys, yc) = yaw.sin_cos();
            let (ts, tc) = 0.45_f32.sin_cos();
            let across = [yc, ys, 0.0];
            let up = [-ys * tc, yc * tc, ts];
            let color = kind_color(process);
            for k in 0..count {
                let r = radius * (1.45 + 0.28 * k as f32);
                let point = |angle: f32| {
                    let (s, c) = angle.sin_cos();
                    [0, 1, 2].map(|i| body.position[i] + r * (c * across[i] + s * up[i]))
                };
                let shade = tint(color, 1.25 - 0.15 * k as f32);
                let mut last = point(0.0);
                for segment in 1..=48 {
                    let next = point(segment as f32 / 48.0 * TAU);
                    frame.line(camera, last, next, shade);
                    last = next;
                }
            }
        }
    }

    /// The machine as a pond: each cgroup is a cluster of pebbles, and every busy process drives
    /// ripples at its own pitch, so a cluster's rings interfere with each other and spread over
    /// open water to the next. Buoys float on the surface sized by memory; every patch of water
    /// belongs to its nearest process; births drip and exits splash. Pressure raises a swell.
    #[allow(clippy::too_many_arguments)]
    fn draw_ripples(
        &mut self,
        frame: &mut Frame,
        processes: &[&Process],
        camera: &Camera,
        selected: Option<Identity>,
        time: f32,
        stir: f32,
    ) {
        let radii = self.scatter(processes);
        let bodies: Vec<Body> = processes
            .iter()
            .enumerate()
            .map(|(index, process)| Body {
                index,
                position: self.positions[&process.id],
                grow: self.growth(process.id, time),
            })
            .collect();
        let Some((grid, located)) = self.footprint(processes, &bodies, 150) else {
            return;
        };
        let mut wave = match self.wave.take() {
            Some(wave) if similar(&wave.grid, &grid) => wave,
            _ => Wave::new(grid, time),
        };
        let grid = wave.grid;
        for &point in &self.splashes {
            wave.drive([point[0], point[1]], -90.0);
        }
        for id in &self.newborn {
            if let Some(p) = self.positions.get(id) {
                wave.drive([p[0], p[1]], 30.0);
            }
        }
        let sources: Vec<([f32; 2], f32, f32, f32)> = bodies
            .iter()
            .zip(&located)
            .filter(|(body, _)| processes[body.index].cpu > 0.3)
            .map(|(body, &at)| {
                let process = processes[body.index];
                let hash = process.id.pid.wrapping_mul(2_654_435_761);
                let frequency = 0.35 + 0.9 * (hash >> 24) as f32 / 255.0;
                let phase = (hash >> 8 & 0xffff) as f32 / 65536.0 * TAU;
                (
                    at,
                    900.0 * bounded(process.cpu, 40.0),
                    TAU * frequency,
                    phase,
                )
            })
            .collect();
        wave.advance(time, |wave, now, dt| {
            for &(at, loudness, omega, phase) in &sources {
                wave.drive(at, loudness * (omega * now + phase).sin() * dt);
            }
        });
        let owners = territories(&grid, &bodies, &located, &radii);
        let swell = 0.01 + 0.12 * stir;
        let surface = |x: f32, y: f32, height: f32| {
            height * 0.09
                + swell
                    * ((x * 0.21 + y * 0.08 + time * 0.9).sin()
                        + 0.7 * (y * 0.17 - x * 0.11 - time * 0.7 + 1.3).sin()
                        + 0.5 * ((x + y) * 0.13 + time * 1.3 + 2.1).sin())
                    / 2.2
        };
        let (columns, rows) = (grid.columns, grid.rows);
        let heights: Vec<f32> = (0..grid.len())
            .map(|i| {
                let [x, y] = grid.world((i % columns) as f32, (i / columns) as f32);
                surface(x, y, wave.height[i])
            })
            .collect();
        let viewer = camera.viewer();
        // Light from the side of the view: calm water stays dark and only slopes catch glints.
        let light = normalize([viewer[1] * 0.75, -viewer[0] * 0.75, 0.65]);
        let half = normalize([
            light[0] + viewer[0],
            light[1] + viewer[1],
            light[2] + viewer[2],
        ]);
        // Lit per node from the height field's gradient and averaged over each facet, so the two
        // triangles of a cell shade alike. Highlights blend towards moonlight by at most
        // RIPPLE_SHINE and never add up to white.
        let shades: Vec<[f32; 3]> = (0..grid.len())
            .map(|i| {
                let (x, y) = (i % columns, i / columns);
                let gradient = |a: usize, b: usize, cells: usize| {
                    (heights[b] - heights[a]) / (cells.max(1) as f32 * grid.cell)
                };
                let (x0, x1) = (x.saturating_sub(1), (x + 1).min(columns - 1));
                let (y0, y1) = (y.saturating_sub(1), (y + 1).min(rows - 1));
                let normal = normalize([
                    -gradient(y * columns + x0, y * columns + x1, x1 - x0),
                    -gradient(y0 * columns + x, y1 * columns + x, y1 - y0),
                    1.0,
                ]);
                let diffuse = dot(normal, light).max(0.0);
                let glint = dot(normal, half).max(0.0).powi(48);
                let crest = (heights[i] * 1.8).clamp(-0.5, 1.0);
                let foam = (wave.velocity[i].abs() * 0.05).min(1.0);
                let tint = processes
                    .get(owners[i] as usize)
                    .map_or(RIPPLE_DEEP, |p| kind_color(p).map(f32::from));
                let shine = (glint * 0.75 + foam * 0.25).min(RIPPLE_SHINE);
                [0, 1, 2].map(|k| {
                    let water = (RIPPLE_DEEP[k] * 0.92 + tint[k] * 0.08) * (0.55 + 0.5 * diffuse)
                        + RIPPLE_CREST[k] * crest;
                    water + (RIPPLE_MOON[k] - water) * shine
                })
            })
            .collect();
        let corner = |node: usize| {
            let [x, y] = grid.world((node % columns) as f32, (node / columns) as f32);
            [x, y, heights[node]]
        };
        for y in 0..rows - 1 {
            for x in 0..columns - 1 {
                let a = y * columns + x;
                let (b, c, d) = (a + 1, a + columns + 1, a + columns);
                for nodes in [[a, b, c], [a, c, d]] {
                    let color = [0, 1, 2].map(|k| {
                        (nodes.iter().map(|&n| shades[n][k]).sum::<f32>() / 3.0).clamp(0.0, 255.0)
                            as u8
                    });
                    frame.facet(camera, nodes.map(corner), color, owners[a]);
                }
            }
        }
        for (body, &at) in bodies.iter().zip(&located) {
            let process = processes[body.index];
            let radius = radii[body.index] * body.grow;
            let z = grid.sample(&heights, at, mix) + radius * 0.35;
            let position = [at[0], at[1], z];
            self.positions.insert(process.id, position);
            self.glows(frame, camera, process, position, radii[body.index]);
            frame.sphere(
                camera,
                position,
                radius,
                kind_color(process),
                body.index as u32,
                selected == Some(process.id),
            );
        }
        self.wave = Some(wave);
    }

    /// Spacetime weather: memory bends a sheet into gravity wells, CPU spins whirlpools, sockets
    /// become two-lane rivers between the wells of processes that talk, and connections leaving
    /// the machine send sparks up off the sheet. Particles in each emitter's colour trace it all.
    #[allow(clippy::too_many_arguments)]
    fn draw_flow(
        &mut self,
        frame: &mut Frame,
        processes: &[&Process],
        camera: &Camera,
        selected: Option<Identity>,
        time: f32,
        snapshot: &Snapshot,
        stir: f32,
    ) {
        let (tree, _, bodies) = self.layout(processes, time, 0.0);
        let Some((grid, located)) = self.footprint(processes, &bodies, 140) else {
            return;
        };
        let mut flow = match self.flow.take() {
            Some(flow) if similar(&flow.grid, &grid) => flow,
            _ => Flow::new(grid, time),
        };
        let grid = flow.grid;
        let dt = (time - flow.time).clamp(0.0, 0.1);
        flow.time = time;
        if !(0.0..0.25).contains(&(time - flow.field_time)) {
            self.weather(
                &mut flow,
                processes,
                &bodies,
                &located,
                &tree.radii,
                snapshot,
            );
            flow.field_time = time;
        }
        let room = |flow: &Flow| flow.particles.len() < 6000;
        let mut background = dt * 400.0;
        while background > flow.random() && room(&flow) {
            background -= 1.0;
            let p = grid.world(
                flow.random() * (grid.columns - 1) as f32,
                flow.random() * (grid.rows - 1) as f32,
            );
            let life = 5.0 + 3.0 * flow.random();
            flow.spawn(p, [90, 130, 190], life, false, false, [0.0; 2]);
        }
        for (body, &at) in bodies.iter().zip(&located) {
            let process = processes[body.index];
            let mut expected = if process.cpu > 0.5 {
                dt * (1.0 + 30.0 * bounded(process.cpu, 40.0))
            } else {
                0.0
            };
            let reach = tree.radii[body.index] * 1.5 + 0.5;
            while expected > flow.random() && room(&flow) {
                expected -= 1.0;
                let angle = flow.random() * TAU;
                let distance = reach * flow.random().sqrt();
                let p = [
                    at[0] + distance * angle.cos(),
                    at[1] + distance * angle.sin(),
                ];
                let life = 4.0 + 3.0 * flow.random();
                flow.spawn(p, kind_color(process), life, true, false, [0.0; 2]);
            }
        }
        let place: HashMap<Identity, (usize, [f32; 2])> = bodies
            .iter()
            .zip(&located)
            .map(|(body, &at)| (processes[body.index].id, (body.index, at)))
            .collect();
        for &(a, b, count) in &snapshot.links {
            let (Some(&(ia, pa)), Some(&(ib, pb))) = (place.get(&a), place.get(&b)) else {
                continue;
            };
            let length = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
            if length < 0.5 {
                continue;
            }
            let direction = [(pb[0] - pa[0]) / length, (pb[1] - pa[1]) / length];
            let lane = 0.6 * (2.5 + 0.8 * (count as f32).ln_1p());
            let normal = [-direction[1] * lane, direction[0] * lane];
            let busy = processes[ia].cpu + processes[ib].cpu;
            // Traffic enters the lane that leads away from each end, so rivers carry it across.
            for (from, side, color) in [
                (pa, 1.0, kind_color(processes[ia])),
                (pb, -1.0, kind_color(processes[ib])),
            ] {
                let mut expected =
                    dt * (0.4 + 0.3 * count as f32) * (0.5 + 2.0 * bounded(busy, 30.0));
                while expected > flow.random() && room(&flow) {
                    expected -= 1.0;
                    let start = [from[0] + normal[0] * side, from[1] + normal[1] * side];
                    flow.spawn(start, color, length / 4.0 + 2.0, true, false, [0.0; 2]);
                }
            }
        }
        for (id, &count) in &snapshot.outside {
            if let Some(index) = processes.iter().position(|p| p.id == *id)
                && let Some(slot) = bodies.iter().position(|b| b.index == index)
            {
                let mut expected = dt * 2.0 * count as f32;
                while expected > flow.random() && room(&flow) {
                    expected -= 1.0;
                    flow.spawn(located[slot], OUTSIDE, 2.5, true, true, [0.0; 2]);
                }
            }
        }
        for point in self.splashes.clone() {
            for k in 0..40 {
                let angle = k as f32 / 40.0 * TAU;
                let kick = [9.0 * angle.cos(), 9.0 * angle.sin()];
                flow.spawn(
                    [point[0], point[1]],
                    [255, 226, 186],
                    1.6,
                    true,
                    false,
                    kick,
                );
            }
        }
        for id in &self.newborn {
            if let Some(p) = self.positions.get(id) {
                for k in 0..12 {
                    let angle = k as f32 / 12.0 * TAU;
                    let kick = [3.0 * angle.cos(), 3.0 * angle.sin()];
                    flow.spawn([p[0], p[1]], [220, 240, 255], 1.2, true, false, kick);
                }
            }
        }
        let turbulence = 0.4 + 6.0 * stir;
        flow.advance(dt, |q| {
            [
                turbulence * (q[1] * 0.31 + time * 0.8).sin(),
                turbulence * (q[0] * 0.29 - time * 0.6).cos(),
            ]
        });
        // The fabric brightens and turns violet with depth, so wells read even from above.
        let deepest = flow.depth.iter().fold(-1.0_f32, |a, &b| a.min(b));
        let fabric = |depth: f32| {
            let t = (depth / deepest).clamp(0.0, 1.0);
            [
                (28.0 + 70.0 * t) as u8,
                (42.0 + 40.0 * t) as u8,
                (72.0 + 120.0 * t) as u8,
            ]
        };
        let node = |x: usize, y: usize| {
            let [wx, wy] = grid.world(x as f32, y as f32);
            [wx, wy, flow.depth[y * grid.columns + x]]
        };
        for y in (0..grid.rows).step_by(3) {
            for x in 1..grid.columns {
                let (a, b) = (node(x - 1, y), node(x, y));
                frame.line(camera, a, b, fabric(b[2]));
            }
        }
        for x in (0..grid.columns).step_by(3) {
            for y in 1..grid.rows {
                let (a, b) = (node(x, y - 1), node(x, y));
                frame.line(camera, a, b, fabric(b[2]));
            }
        }
        for (body, &at) in bodies.iter().zip(&located) {
            let process = processes[body.index];
            let radius = tree.radii[body.index] * 0.7 * body.grow;
            let position = [at[0], at[1], flow.depth_at(at) + radius * 0.6];
            self.positions.insert(process.id, position);
            self.glows(frame, camera, process, position, radius);
            frame.sphere(
                camera,
                position,
                radius,
                kind_color(process),
                body.index as u32,
                selected == Some(process.id),
            );
        }
        for particle in &flow.particles {
            let fade = (particle.age / 0.3).min(1.0)
                * (1.0 - particle.age / particle.life).max(0.0).sqrt();
            let strength = if particle.bright { 0.9 } else { 0.6 } * fade;
            for k in 0..TRAIL - 1 {
                let (a, b) = (particle.trail[k], particle.trail[k + 1]);
                if a != b {
                    let weight = strength * (1.0 - k as f32 / TRAIL as f32);
                    frame.beam(camera, a, b, particle.color, weight);
                }
            }
            if particle.bright {
                frame.glow(camera, particle.trail[0], 3.0, particle.color, 0.6 * fade);
            }
        }
        self.flow = Some(flow);
    }

    /// Recomputes the flow's velocity field and well depths from the current processes.
    fn weather(
        &self,
        flow: &mut Flow,
        processes: &[&Process],
        bodies: &[Body],
        located: &[[f32; 2]],
        radii: &[f32],
        snapshot: &Snapshot,
    ) {
        let at: HashMap<Identity, [f32; 2]> = bodies
            .iter()
            .zip(located)
            .map(|(body, &p)| (processes[body.index].id, p))
            .collect();
        // Each solar system bends a bowl as wide as the system, deeper for more memory; heavy
        // processes add their own dimples. (centre, depth, width, inflow)
        let mut wells: Vec<([f32; 2], f32, f32, f32)> = self
            .stars
            .iter()
            .filter_map(|&(id, _, extent)| {
                let bytes = self.mass.get(&id).map_or_else(
                    || {
                        processes
                            .iter()
                            .find(|p| p.id == id)
                            .map_or(0, |p| p.memory)
                    },
                    |&(bytes, _)| bytes,
                );
                let depth = 2.0 + 14.0 * bounded(bytes as f32 / 1048576.0, 2048.0);
                Some((*at.get(&id)?, depth, (extent * 0.5).max(4.0), 0.0))
            })
            .collect();
        wells.extend(
            bodies
                .iter()
                .zip(located)
                .filter(|(body, _)| radii[body.index] > 0.9)
                .map(|(body, &p)| {
                    let r = radii[body.index];
                    (p, 0.4 * r, 1.0 + 2.0 * r, 0.6 * r * r * r)
                }),
        );
        let vortices: Vec<([f32; 2], f32, f32)> = bodies
            .iter()
            .zip(located)
            .filter(|(body, _)| processes[body.index].cpu > 0.5)
            .map(|(body, &at)| {
                let process = processes[body.index];
                let spin = if process.id.pid.is_multiple_of(2) {
                    1.0
                } else {
                    -1.0
                };
                let strength = (6.0 + 60.0 * bounded(process.cpu, 40.0)) * spin;
                (at, strength, (1.0 + 2.0 * radii[body.index]).powi(2))
            })
            .collect();
        let place: HashMap<Identity, ([f32; 2], f32)> = bodies
            .iter()
            .zip(located)
            .map(|(body, &at)| (processes[body.index].id, (at, processes[body.index].cpu)))
            .collect();
        let jets: Vec<Lane> = snapshot
            .links
            .iter()
            .filter_map(|(a, b, count)| {
                let (&(pa, ca), &(pb, cb)) = (place.get(a)?, place.get(b)?);
                let length = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
                (length > 0.5).then(|| {
                    let direction = [(pb[0] - pa[0]) / length, (pb[1] - pa[1]) / length];
                    let width = 2.5 + 0.8 * (*count as f32).ln_1p();
                    let speed = (8.0 + 3.0 * (*count as f32).ln_1p())
                        * (0.6 + 1.4 * bounded(ca + cb, 30.0));
                    Lane {
                        start: pa,
                        direction,
                        length,
                        width,
                        speed,
                    }
                })
            })
            .collect();
        let grid = flow.grid;
        flow.velocity.fill([0.0; 2]);
        flow.depth.fill(0.0);
        // Every influence fades with distance, so each one only visits the cells within its reach.
        let around = |center: [f32; 2], reach: f32, apply: &mut dyn FnMut([f32; 2], usize)| {
            let low = grid.locate([center[0] - reach, center[1] - reach]);
            let high = grid.locate([center[0] + reach, center[1] + reach]);
            let span = |a: f32, b: f32, limit: usize| {
                (a.floor().max(0.0) as usize)..((b.ceil() + 1.0).max(0.0) as usize).min(limit)
            };
            for j in span(low[1], high[1], grid.rows) {
                for i in span(low[0], high[0], grid.columns) {
                    apply(grid.world(i as f32, j as f32), j * grid.columns + i);
                }
            }
        };
        let (velocity, depth) = (&mut flow.velocity, &mut flow.depth);
        for &(c, deep, width, inflow) in &wells {
            around(c, width * 3.0, &mut |p, cell| {
                let d = [p[0] - c[0], p[1] - c[1]];
                let distance2 = d[0] * d[0] + d[1] * d[1];
                // Gaussian bowls stay local, so the sheet is flat between systems.
                depth[cell] -= deep * (-distance2 / (width * width)).exp();
                let r2 = distance2 + width * width;
                let pull = inflow / r2 / r2.sqrt();
                velocity[cell][0] -= d[0] * pull;
                velocity[cell][1] -= d[1] * pull;
            });
        }
        for &(c, strength, core) in &vortices {
            // Beyond this radius the swirl is below 0.1 units per second.
            let reach = (strength.abs() * 10.0).sqrt().max(core.sqrt() * 3.0);
            around(c, reach, &mut |p, cell| {
                let d = [p[0] - c[0], p[1] - c[1]];
                let swirl = strength / (d[0] * d[0] + d[1] * d[1] + core);
                velocity[cell][0] -= d[1] * swirl;
                velocity[cell][1] += d[0] * swirl;
            });
        }
        for &Lane {
            start: a,
            direction,
            length,
            width,
            speed,
        } in &jets
        {
            let middle = [
                a[0] + direction[0] * length * 0.5,
                a[1] + direction[1] * length * 0.5,
            ];
            around(middle, length * 0.5 + width * 2.0, &mut |p, cell| {
                let d = [p[0] - a[0], p[1] - a[1]];
                let along = d[0] * direction[0] + d[1] * direction[1];
                if !(0.0..length).contains(&along) {
                    return;
                }
                // Two lanes, one each way: the side of the line decides the direction.
                let side = direction[0] * d[1] - direction[1] * d[0];
                let lane = (side.abs() - width * 0.6) / (width * 0.5);
                let weight = (-lane * lane).exp() * (PI * along / length).sin().sqrt();
                let sign = if side > 0.0 { 1.0 } else { -1.0 };
                velocity[cell][0] += direction[0] * speed * weight * sign;
                velocity[cell][1] += direction[1] * speed * weight * sign;
            });
        }
        for v in velocity.iter_mut() {
            let speed = v[0].hypot(v[1]);
            if speed > 20.0 {
                *v = v.map(|c| c * 20.0 / speed);
            }
        }
        for d in depth.iter_mut() {
            *d = d.max(-20.0);
        }
    }

    /// The ground-plane grid covering the layout with a margin, and each body's ground position.
    fn footprint(
        &self,
        processes: &[&Process],
        bodies: &[Body],
        cells: usize,
    ) -> Option<(Grid, Vec<[f32; 2]>)> {
        let located: Vec<[f32; 2]> = bodies
            .iter()
            .map(|body| {
                let p = self.positions[&processes[body.index].id];
                [p[0], p[1]]
            })
            .collect();
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for p in &located {
            for k in 0..2 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        if located.is_empty() {
            return None;
        }
        let margin = 8.0 + 0.08 * (max[0] - min[0]).max(max[1] - min[1]);
        let grid = Grid::covering(
            [min[0] - margin, min[1] - margin],
            [max[0] + margin, max[1] + margin],
            cells,
        );
        Some((grid, located))
    }

    /// Ripple layout: places every process as a pebble in its cgroup's cluster and records the
    /// positions. A process keeps its slot while it lives; a cluster spreads out or moves only
    /// when it outgrows the water reserved for it. Returns each process's pebble radius.
    fn scatter(&mut self, processes: &[&Process]) -> Vec<f32> {
        let radii: Vec<f32> = processes
            .iter()
            .map(|p| mass_radius(p.memory as f32))
            .collect();
        let members: HashMap<Identity, (&str, f32, u64)> = processes
            .iter()
            .zip(&radii)
            .map(|(p, &r)| (p.id, (p.group.as_str(), r, p.memory)))
            .collect();
        self.pebbles.retain(|id, (group, slot)| {
            let stays = members.get(id).is_some_and(|m| m.0 == group);
            if !stays && let Some(cluster) = self.clusters.get_mut(group) {
                cluster.slots.remove(slot);
            }
            stays
        });
        self.clusters.retain(|_, cluster| !cluster.slots.is_empty());
        for process in processes {
            if !self.pebbles.contains_key(&process.id) {
                let cluster = self.clusters.entry(process.group.clone()).or_default();
                let slot = (0..=cluster.slots.len())
                    .find(|slot| !cluster.slots.contains_key(slot))
                    .expect("one of len + 1 slots is free");
                cluster.slots.insert(slot, process.id);
                self.pebbles
                    .insert(process.id, (process.group.clone(), slot));
            }
        }
        let mut pending = Vec::new();
        for (name, cluster) in &mut self.clusters {
            let largest = cluster
                .slots
                .values()
                .map(|id| members[id].1)
                .fold(0.0, f32::max);
            // Spiral neighbours sit at least ~1.6 spacings apart; jitter takes up to 0.2 of that.
            let needed = (2.0 * largest + PEBBLE_GAP) / 1.4;
            if needed > cluster.spacing {
                cluster.spacing = needed * 1.15;
            }
            let outermost = cluster.slots.keys().max().copied().unwrap_or(0);
            cluster.extent = cluster.spacing * (outermost as f32 + 1.0).sqrt() + largest;
            let reach = cluster.extent + POND_GAP;
            if reach > cluster.reserved {
                cluster.reserved = 0.0;
                pending.push((name.clone(), reach));
            }
        }
        pending.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (name, reach) in pending {
            let placed: Vec<_> = self
                .clusters
                .values()
                .filter(|c| c.reserved > 0.0)
                .map(|c| (c.center, c.reserved))
                .collect();
            let reserved = reach * 1.1;
            let center = vacant(&placed, reserved);
            if let Some(cluster) = self.clusters.get_mut(&name) {
                cluster.center = center;
                cluster.reserved = reserved;
            }
        }
        self.positions.clear();
        for process in processes {
            let (group, slot) = &self.pebbles[&process.id];
            let cluster = &self.clusters[group];
            let k = *slot as f32;
            let distance = cluster.spacing * (k + 0.5).sqrt();
            let angle = k * GOLDEN_ANGLE + spin(group);
            let hash = process.id.pid.wrapping_mul(2_654_435_761);
            let jitter = cluster.spacing * 0.1 * (hash & 0xffff) as f32 / 65536.0;
            let toss = (hash >> 16) as f32 / 65536.0 * TAU;
            self.positions.insert(
                process.id,
                [
                    cluster.center[0] + distance * angle.cos() + jitter * toss.cos(),
                    cluster.center[1] + distance * angle.sin() + jitter * toss.sin(),
                    0.0,
                ],
            );
        }
        for cluster in self.clusters.values() {
            if let Some((_, &anchor)) = cluster.slots.iter().min_by_key(|(slot, _)| **slot)
                && cluster.slots.len() > 1
            {
                let count = cluster.slots.len();
                let memory = cluster.slots.values().map(|id| members[id].2).sum();
                self.stars.push((anchor, count, cluster.extent));
                self.mass.insert(anchor, (memory, count));
            }
        }
        self.stars
            .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        radii
    }

    /// Warm CPU glow and green NVIDIA glow around a body, as in the orbit view.
    fn glows(&self, frame: &mut Frame, camera: &Camera, process: &Process, at: Point, radius: f32) {
        let core = (radius * camera.zoom).max(MIN_BODY_PX);
        if process.cpu > 1.0 {
            let glow = core * (2.2 + 2.0 * bounded(process.cpu, 50.0));
            frame.glow(
                camera,
                at,
                glow,
                WARM,
                0.3 + 0.6 * bounded(process.cpu, 40.0),
            );
        }
        if process.gpu_memory > 0 {
            let mib = process.gpu_memory as f32 / 1048576.0;
            frame.glow(
                camera,
                at,
                core * (2.6 + 0.5 * (mib / 64.0).ln_1p()),
                NVIDIA,
                0.6,
            );
        }
    }

    fn place_systems(
        &mut self,
        processes: &[&Process],
        roots: &[usize],
        extents: &[f32],
        masses: &[f32],
        motion: f32,
    ) -> Vec<[f32; 2]> {
        let needed: HashMap<Identity, f32> = roots
            .iter()
            .map(|&root| (processes[root].id, extents[root] + SYSTEM_GAP))
            .collect();
        let before = self.systems.len();
        self.systems
            .retain(|id, (_, reserved)| needed.get(id).is_some_and(|&need| need <= *reserved));
        let mut pending: Vec<_> = needed
            .iter()
            .filter(|(id, _)| !self.systems.contains_key(id))
            .map(|(&id, &need)| (id, need))
            .collect();
        if self.systems.len() != before || !pending.is_empty() {
            // A system left, arrived or was re-placed: the pivot has to be found again.
            self.galaxy_members.clear();
        }
        pending.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        for (id, need) in pending {
            // Headroom absorbs memory drift so systems are only re-placed when their structure grows.
            let reserved = need * 1.08;
            let placed: Vec<_> = self.systems.values().copied().collect();
            let center = vacant(&placed, reserved);
            self.systems.insert(id, (center, reserved));
        }
        let weights: HashMap<Identity, f32> = roots
            .iter()
            .map(|&root| (processes[root].id, masses[root]))
            .collect();
        self.revolve(&weights, motion);
        roots
            .iter()
            .map(|&root| self.systems[&processes[root].id].0)
            .collect()
    }

    /// Turns the systems about their memory-weighted barycenter by the motion time elapsed since
    /// the last call. The barycenter is taken when systems come or go, so the pivot stays put
    /// while they turn. Systems whose distances from it, give or take their reserved radius,
    /// overlap form a band that turns rigidly; bands occupy disjoint annuli, so whatever their
    /// speeds no two systems can collide. Each band turns at its own Kepler rate, outer bands
    /// slower, and phase accumulates, so a band changing speed never makes a system jump.
    fn revolve(&mut self, masses: &HashMap<Identity, f32>, motion: f32) {
        let dt = self.galaxy_time.map_or(0.0, |last| motion - last);
        self.galaxy_time = Some(motion);
        let continuous = self.galaxy_frame + 1 == self.frames;
        self.galaxy_frame = self.frames;
        let weight = |id: &Identity| masses.get(id).copied().unwrap_or(0.0).max(1.0);
        let mut members: Vec<Identity> = self.systems.keys().copied().collect();
        members.sort();
        if members != self.galaxy_members {
            let total: f32 = members.iter().map(weight).sum();
            let mut center = [0.0_f32; 2];
            for (id, (at, _)) in &self.systems {
                for k in 0..2 {
                    center[k] += at[k] * weight(id) / total.max(1.0);
                }
            }
            self.galaxy = center;
            self.galaxy_members = members;
        }
        let center = self.galaxy;
        let mut rings: Vec<(Identity, f32, f32)> = self
            .systems
            .iter()
            .map(|(&id, &(at, reserved))| {
                let distance = (at[0] - center[0]).hypot(at[1] - center[1]);
                (id, distance, reserved)
            })
            .collect();
        rings.sort_by(|a, b| (a.1 - a.2).total_cmp(&(b.1 - b.2)).then(a.0.cmp(&b.0)));
        let mut bands: Vec<(f32, Vec<(Identity, f32)>)> = Vec::new();
        for (id, distance, reserved) in rings {
            match bands.last_mut() {
                Some((outer, members)) if distance - reserved < *outer => {
                    *outer = outer.max(distance + reserved);
                    members.push((id, distance));
                }
                _ => bands.push((distance + reserved, vec![(id, distance)])),
            }
        }
        self.galaxy_speed.clear();
        let mut enclosed = 0.0;
        let mut inner = f32::INFINITY;
        for (_, members) in &bands {
            let mass: f32 = members.iter().map(|(id, _)| weight(id)).sum();
            enclosed += mass;
            let radius = members.iter().map(|(id, d)| d * weight(id)).sum::<f32>() / mass;
            // A heavy band outside a light one would otherwise outpace it; outer never turns faster.
            let speed = (TAU / galactic_period(radius, enclosed)).min(inner);
            inner = speed;
            for (id, _) in members {
                self.galaxy_speed.insert(*id, speed);
            }
        }
        // Turn only between consecutive frames and by at most MAX_TURN_STEP (above the 1 s frame
        // interval of --fps 1), so pauses, rewinds and view switches don't spin the galaxy.
        if !(continuous && dt > 0.0 && dt <= MAX_TURN_STEP) {
            return;
        }
        for (id, (at, _)) in &mut self.systems {
            let (s, c) = (self.galaxy_speed[id] * dt).sin_cos();
            let [x, y] = [at[0] - center[0], at[1] - center[1]];
            *at = [center[0] + x * c - y * s, center[1] + x * s + y * c];
        }
    }

    /// Places a body and its descendants. `target` is the settled position used for layout,
    /// picking and labels; `drawn` includes the spawn animation that flies bodies out of parents.
    #[allow(clippy::too_many_arguments)]
    fn orbit_branch(
        &mut self,
        tree: &Tree,
        index: usize,
        target: Point,
        drawn: Point,
        grow: f32,
        depth: usize,
        [time, motion]: [f32; 2],
        visited: &mut HashSet<usize>,
        paths: &mut Vec<Path>,
        bodies: &mut Vec<Body>,
    ) {
        if depth > MAX_DEPTH || !visited.insert(index) {
            return;
        }
        let process = tree.processes[index];
        self.positions.insert(process.id, target);
        bodies.push(Body {
            index,
            position: drawn,
            grow,
        });
        let kids = &tree.children[index];
        if depth == MAX_DEPTH || kids.is_empty() {
            return;
        }
        let (e, omega) = elements(process.id.pid);
        let sizes: Vec<f32> = kids.iter().map(|&child| tree.extents[child]).collect();
        let (slots, _) = shells(tree.reach[index], &sizes, e);
        for (&child, (a, phase)) in kids.iter().zip(slots) {
            let orbit = Orbit {
                center: target,
                a,
                e,
                omega,
            };
            let mean = phase + TAU * motion / period(a, tree.masses[index]);
            let mut settled = orbit.at(mean);
            settled[2] = target[2] + 0.2;
            let child_process = tree.processes[child];
            let child_grow = self.growth(child_process.id, time);
            let position = [
                drawn[0] + (settled[0] - target[0]) * child_grow,
                drawn[1] + (settled[1] - target[1]) * child_grow,
                drawn[2] + 0.2,
            ];
            paths.push(Path {
                orbit: Orbit {
                    center: drawn,
                    ..orbit
                },
                depth,
                id: child_process.id,
                spoke: depth + 1 < MAX_DEPTH && !tree.children[child].is_empty(),
                mean,
                position,
                cpu: child_process.cpu,
            });
            self.orbit_branch(
                tree,
                child,
                settled,
                position,
                child_grow,
                depth + 1,
                [time, motion],
                visited,
                paths,
                bodies,
            );
        }
    }
}

/// The first point on a golden-angle spiral out from the origin where a disc of `radius` clears
/// every placed (centre, radius) disc.
pub(crate) fn vacant(placed: &[([f32; 2], f32)], radius: f32) -> [f32; 2] {
    let step = (radius * 0.4).max(1.0);
    (0_u32..)
        .map(|k| {
            let distance = step * (k as f32).sqrt();
            let angle = k as f32 * GOLDEN_ANGLE;
            [distance * angle.cos(), distance * angle.sin()]
        })
        .find(|c| {
            placed
                .iter()
                .all(|(o, r)| (c[0] - o[0]).hypot(c[1] - o[1]) >= radius + r)
        })
        .expect("the spiral eventually clears every placed disc")
}

/// One cgroup's processes on the ripple pond: pebbles on a sunflower spiral around the centre,
/// slot k at sqrt(k + 1/2) spacings out and k golden angles round.
#[derive(Default)]
struct Cluster {
    center: [f32; 2],
    spacing: f32,
    /// Centre to the far edge of the outermost pebble.
    extent: f32,
    /// Radius of open water reserved around the centre; zero until placed.
    reserved: f32,
    slots: HashMap<usize, Identity>,
}

/// A stable angle per name, so clusters don't all share one spiral orientation.
pub(crate) fn spin(name: &str) -> f32 {
    let hash = name.bytes().fold(2_166_136_261_u32, |h, b| {
        (h ^ b as u32).wrapping_mul(16_777_619)
    });
    (hash >> 8) as f32 / 16_777_216.0 * TAU
}

/// A socket link as a two-lane river in the flow field.
#[derive(Clone, Copy)]
struct Lane {
    start: [f32; 2],
    direction: [f32; 2],
    length: f32,
    width: f32,
    speed: f32,
}

/// Media survive small layout changes; a different footprint starts a fresh simulation.
fn similar(a: &Grid, b: &Grid) -> bool {
    a.columns == b.columns
        && a.rows == b.rows
        && (a.cell / b.cell - 1.0).abs() < 0.05
        && (a.origin[0] - b.origin[0]).abs() < a.cell * 2.0
        && (a.origin[1] - b.origin[1]).abs() < a.cell * 2.0
}

/// Assigns every grid node to its nearest body by a multi-source breadth-first flood, larger
/// bodies claiming shared cells first. Returns body indices into the process list, or NONE.
fn territories(grid: &Grid, bodies: &[Body], located: &[[f32; 2]], radii: &[f32]) -> Vec<u32> {
    let mut owners = vec![NONE; grid.len()];
    let mut order: Vec<usize> = (0..bodies.len()).collect();
    order.sort_by(|&a, &b| radii[bodies[b].index].total_cmp(&radii[bodies[a].index]));
    let mut queue = std::collections::VecDeque::new();
    for slot in order {
        let [x, y] = grid.locate(located[slot]);
        let (x, y) = (x.round() as usize, y.round() as usize);
        if x < grid.columns && y < grid.rows && owners[y * grid.columns + x] == NONE {
            owners[y * grid.columns + x] = bodies[slot].index as u32;
            queue.push_back(y * grid.columns + x);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % grid.columns, i / grid.columns);
        let neighbours = [
            (x > 0).then(|| i - 1),
            (x + 1 < grid.columns).then(|| i + 1),
            (y > 0).then(|| i - grid.columns),
            (y + 1 < grid.rows).then(|| i + grid.columns),
        ];
        for j in neighbours.into_iter().flatten() {
            if owners[j] == NONE {
                owners[j] = owners[i];
                queue.push_back(j);
            }
        }
    }
    owners
}

pub(crate) fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn normalize(a: Point) -> Point {
    let length = dot(a, a).sqrt().max(1e-6);
    a.map(|v| v / length)
}

const MAX_DEPTH: usize = 3;
const ORBIT_GAP: f32 = 0.6;
const SYSTEM_GAP: f32 = 2.5;
pub(crate) const GOLDEN_ANGLE: f32 = 2.399_963;
/// Clearance between neighbouring ripple pebbles, and open water around each cluster.
const PEBBLE_GAP: f32 = 1.0;
const POND_GAP: f32 = 3.0;
const RIPPLE_DEEP: [f32; 3] = [10.0, 30.0, 46.0];
const RIPPLE_CREST: [f32; 3] = [14.0, 34.0, 40.0];
const RIPPLE_MOON: [f32; 3] = [150.0, 190.0, 210.0];
const RIPPLE_SHINE: f32 = 0.6;

/// Body radius from memory in bytes: volume proportional to memory, so a 1 GiB process is about
/// five times as wide as a 6 MiB one and idle kernel threads stay specks.
pub(crate) fn mass_radius(bytes: f32) -> f32 {
    (0.24 * (bytes / 1048576.0).max(0.0).cbrt()).clamp(0.25, 4.5)
}

/// Thread rings: none below 4 threads, then one more ring per fourfold increase, up to four.
fn rings(threads: u32) -> usize {
    match threads {
        0..=3 => 0,
        4..=15 => 1,
        16..=63 => 2,
        64..=255 => 3,
        _ => 4,
    }
}

fn ring_scale(threads: u32) -> f32 {
    match rings(threads) {
        0 => 1.0,
        count => 1.55 + 0.28 * (count - 1) as f32,
    }
}

/// Total memory and process count of each subtree, over every depth including collapsed ones.
fn weigh(
    processes: &[&Process],
    children: &[Vec<usize>],
    index: usize,
    masses: &mut [f32],
    members: &mut [usize],
    weighed: &mut HashSet<usize>,
) -> (f32, usize) {
    if !weighed.insert(index) {
        return (0.0, 0);
    }
    let mut total = (processes[index].memory as f32, 1);
    for &child in &children[index] {
        let (mass, count) = weigh(processes, children, child, masses, members, weighed);
        total.0 += mass;
        total.1 += count;
    }
    masses[index] = total.0;
    members[index] = total.1;
    total
}

fn arc(extent: f32) -> f32 {
    2.0 * extent + ORBIT_GAP
}

/// Eccentricity and periapsis direction shared by all of one parent's shells. Confocal ellipses
/// with the same shape are scaled copies of each other, so neighbouring shells never cross.
fn elements(pid: u32) -> (f32, f32) {
    let hash = pid.wrapping_mul(2_654_435_761);
    let e = 0.06 + 0.16 * (hash >> 24) as f32 / 255.0;
    let omega = TAU * ((hash >> 8) & 0xffff) as f32 / 65536.0;
    (e, omega)
}

/// Orbital period in seconds from Kepler's third law, T ~ sqrt(a^3 / M): wider orbits are slower
/// and heavier parents (more memory in their tree) pull their children round faster. Normalised
/// to 30 s at a = 3 around 512 MiB and clamped to a watchable range.
fn period(a: f32, parent_bytes: f32) -> f32 {
    let mass = (parent_bytes / (512.0 * 1048576.0)).max(1.0 / 512.0);
    (30.0 * (a / 3.0).powf(1.5) / mass.sqrt()).clamp(8.0, 600.0)
}

/// Longest motion step, in seconds, that the galaxy turns through between two frames.
const MAX_TURN_STEP: f32 = 2.0;

/// Seconds for a system to circle the galaxy once at `radius` world units with `enclosed` bytes
/// of systems inside its band: Kepler's T ~ sqrt(r^3 / M), normalised to four minutes at 30
/// units around 4 GiB and clamped so the galaxy always turns slowly next to its planets.
fn galactic_period(radius: f32, enclosed: f32) -> f32 {
    let mass = (enclosed / (4.0 * 1073741824.0)).max(1.0 / 4096.0);
    (240.0 * (radius.max(1.0) / 30.0).powf(1.5) / mass.sqrt()).clamp(120.0, 900.0)
}

/// Solves Kepler's equation M = E - e sin E for the eccentric anomaly E by Newton's method.
fn eccentric_anomaly(mean: f32, e: f32) -> f32 {
    let mean = mean.rem_euclid(TAU);
    let mut anomaly = mean;
    for _ in 0..6 {
        anomaly -= (anomaly - e * anomaly.sin() - mean) / (1.0 - e * anomaly.cos());
    }
    anomaly
}

/// Packs siblings into concentric confocal shells. Bodies on a shell share one ellipse and are
/// spaced evenly in mean anomaly, so they keep their order as Kepler motion bunches them near
/// apoapsis; capacity uses that slowest stretch. Returns (semi-major axis, starting mean anomaly)
/// per sibling and how far the system reaches from its centre.
fn shells(body: f32, sizes: &[f32], e: f32) -> (Vec<(f32, f32)>, f32) {
    let squeeze = ((1.0 - e) / (1.0 + e)).sqrt();
    let mut widest_after = vec![0.0_f32; sizes.len() + 1];
    for i in (0..sizes.len()).rev() {
        widest_after[i] = widest_after[i + 1].max(sizes[i]);
    }
    let mut slots = Vec::with_capacity(sizes.len());
    let mut a = (body + widest_after[0] + ORBIT_GAP + 1.0) / (1.0 - e);
    let mut reach = 0.0_f32;
    let mut start = 0;
    let mut shell = 0;
    while start < sizes.len() {
        let capacity = TAU * a * squeeze;
        let mut end = start;
        let mut used = 0.0;
        while end < sizes.len() && (end == start || used + arc(sizes[end]) <= capacity) {
            used += arc(sizes[end]);
            end += 1;
        }
        let slack = (capacity - used).max(0.0) / (end - start) as f32;
        let mut mean = shell as f32 * 0.5;
        let mut widest = 0.0_f32;
        for &size in &sizes[start..end] {
            let share = (arc(size) + slack) / (a * squeeze);
            slots.push((a, mean + share * 0.5));
            mean += share;
            widest = widest.max(size);
        }
        reach = reach.max(a * (1.0 + e) + widest);
        start = end;
        if start < sizes.len() {
            a += (widest + widest_after[start] + ORBIT_GAP) / (1.0 - e);
        }
        shell += 1;
    }
    (slots, reach)
}

fn measure(
    children: &[Vec<usize>],
    reach: &[f32],
    processes: &[&Process],
    index: usize,
    depth: usize,
    extents: &mut [f32],
    measured: &mut HashSet<usize>,
) -> f32 {
    if !measured.insert(index) {
        return 0.0;
    }
    let extent = if depth >= MAX_DEPTH || children[index].is_empty() {
        reach[index]
    } else {
        let sizes: Vec<f32> = children[index]
            .iter()
            .map(|&child| {
                measure(
                    children,
                    reach,
                    processes,
                    child,
                    depth + 1,
                    extents,
                    measured,
                )
            })
            .collect();
        shells(reach[index], &sizes, elements(processes[index].id.pid).0).1
    };
    extents[index] = extent;
    extent
}

fn district_origin(index: usize) -> [f32; 2] {
    // Grow square shells without relocating any previously allocated district.
    let shell = (index as f32).sqrt() as usize;
    let offset = index - shell * shell;
    let (x, y) = if offset <= shell {
        (shell, offset)
    } else {
        (2 * shell - offset, shell)
    };
    [x as f32 * 23.0, y as f32 * 23.0]
}

pub fn descendants(snapshot: &Snapshot, root: Identity) -> HashSet<Identity> {
    let mut result = HashSet::from([root]);
    let mut pids = HashSet::from([root.pid]);
    loop {
        let before = result.len();
        for p in &snapshot.processes {
            if pids.contains(&p.parent) {
                result.insert(p.id);
                pids.insert(p.id.pid);
            }
        }
        if result.len() == before {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;

    fn process(pid: u32, parent: u32) -> Process {
        Process {
            id: Identity {
                pid,
                start: pid as u64,
            },
            parent,
            name: format!("p{pid}"),
            command: String::new(),
            group: "g".into(),
            kind: Kind::System,
            state: 'S',
            cpu: 0.0,
            memory: 1 << 20,
            io_rate: None,
            read_rate: None,
            write_rate: None,
            written: None,
            priority: 20,
            nice: 0,
            threads: 1,
            gpu_memory: 0,
            core: 0,
            cpu_time: 0.0,
            cgroup: "/system.slice/g.service".into(),
            performance_share: None,
            waiting: None,
            files: crate::model::Measured::Pending,
            locks_held: crate::model::Measured::Pending,
            blocked_on: None,
        }
    }

    fn render(scene: &mut Scene, snapshot: &Snapshot, view: View, time: f32) -> Frame {
        let mut frame = scene.render(
            snapshot,
            view,
            &Camera::default(),
            320,
            180,
            None,
            time,
            512,
            None,
        );
        frame.rasterize();
        frame
    }

    fn blank(width: u32, height: u32) -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Dusk,
            haze: [0.0; 3],
            time: 0.0,
        };
        let mut frame = Frame::new(width, height, sky, Buffers::default());
        frame.rasterize();
        frame
    }

    #[test]
    fn default_camera_is_isometric_and_ground_inverts_the_projection() {
        let v = Camera::default().view([1.0, 0.0, 0.0]);
        assert!((v[0] - 0.866).abs() < 1e-3 && (v[1] - 0.5).abs() < 1e-3);
        assert!((Camera::default().view([0.0, 0.0, 1.0])[1] + 1.0).abs() < 1e-3);
        for (rotation, pitch) in [(0.0, ISOMETRIC), (1.3, 0.5), (-2.0, FRAC_PI_2)] {
            let camera = Camera {
                center: [3.0, -2.0],
                zoom: 7.0,
                rotation,
                pitch,
            };
            let q = camera.view([10.0, 4.0, 0.0]);
            let g = camera.ground(q[0] * camera.zoom, q[1] * camera.zoom);
            assert!(
                (g[0] - 7.0).abs() < 1e-3 && (g[1] - 6.0).abs() < 1e-3,
                "{g:?}"
            );
        }
    }

    #[test]
    fn kepler_solution_and_shells_keep_bodies_apart() {
        for &(mean, e) in &[(0.3_f32, 0.1_f32), (2.5, 0.22), (5.9, 0.06)] {
            let anomaly = eccentric_anomaly(mean, e);
            assert!((anomaly - e * anomaly.sin() - mean).abs() < 1e-4);
        }
        let orbit = Orbit {
            center: [0.0; 3],
            a: 10.0,
            e: 0.2,
            omega: 0.0,
        };
        assert!(
            (orbit.at(0.0)[0] - 8.0).abs() < 1e-3,
            "periapsis is a(1 - e)"
        );
        let (slots, reach) = shells(0.5, &[0.3; 200], 0.2);
        assert_eq!(slots.len(), 200);
        let mut axes: Vec<f32> = slots.iter().map(|s| s.0).collect();
        axes.dedup();
        assert!(axes.len() > 1, "200 siblings overflow into outer shells");
        assert!(axes[0] * 0.8 >= 0.5 + 0.3 + ORBIT_GAP + 1.0 - 1e-4);
        for pair in axes.windows(2) {
            assert!((pair[1] - pair[0]) * 0.8 >= 0.6 + ORBIT_GAP - 1e-4);
        }
        assert!(reach >= axes[axes.len() - 1] * 1.2);
        assert!(period(6.0, 8.0 * 1073741824.0) < period(6.0, 64.0 * 1048576.0));
    }

    #[test]
    fn sizes_span_memory_and_stars_carry_their_systems_mass() {
        let mib = |m: f32| m * 1048576.0;
        let small = mass_radius(mib(6.0));
        let large = mass_radius(mib(1024.0));
        assert!(large / small > 5.0, "{small} -> {large}");
        assert_eq!(mass_radius(0.0), 0.25);
        assert_eq!((rings(3), rings(4), rings(16), rings(1000)), (0, 1, 2, 4));
        let mut processes = vec![process(1, 0), process(10, 1)];
        processes[1].memory = 2 << 30;
        processes.push(process(11, 10));
        let snapshot = Snapshot {
            processes,
            ..Default::default()
        };
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, View::Orbit, 0.0);
        let id = snapshot.processes[1].id;
        assert_eq!(scene.mass[&id], ((2 << 30) + (1 << 20), 2));
    }

    #[test]
    fn spawned_bodies_grow_and_exits_leave_a_fading_flash() {
        let snapshot = demo(1.0, 32);
        let mut scene = Scene::new();
        scene.intro = true;
        render(&mut scene, &snapshot, View::Orbit, 10.0);
        let id = snapshot.processes[3].id;
        assert!(scene.growth(id, 10.0) < 0.01);
        assert!(scene.growth(id, 10.0 + GROW_SECONDS) > 0.99);
        let mut fewer = snapshot.clone();
        fewer.processes.remove(3);
        render(&mut scene, &fewer, View::Orbit, 10.5);
        assert_eq!(scene.deaths.len(), 1);
        render(&mut scene, &fewer, View::Orbit, 10.5 + FADE_SECONDS + 0.01);
        assert!(scene.deaths.is_empty());
        let mut quiet = Scene::new();
        render(&mut quiet, &snapshot, View::City, 10.0);
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
            let frame = scene.render(
                &snapshot,
                View::Orbit,
                &Camera::default(),
                320,
                180,
                None,
                3.0,
                512,
                None,
            );
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
    fn pressure_tints_the_sky() {
        let mut snapshot = demo(1.0, 16);
        snapshot.pressure = [0.0; 3];
        let calm = render(&mut Scene::new(), &snapshot, View::Orbit, 1.0);
        snapshot.pressure = [0.0, 60.0, 0.0];
        let stressed = render(&mut Scene::new(), &snapshot, View::Orbit, 1.0);
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
    fn ripples_and_flow_answer_to_activity() {
        let mut snapshot = demo(2.0, 64);
        for p in &mut snapshot.processes {
            p.cpu = 0.0;
        }
        let run = |snapshot: &Snapshot, view: View| {
            let mut scene = Scene::new();
            for k in 0..40 {
                render(&mut scene, snapshot, view, k as f32 * 0.05);
            }
            scene
        };
        let calm = run(&snapshot, View::Ripple);
        assert_eq!(calm.wave.as_ref().unwrap().energy(), 0.0);
        snapshot.processes[1].cpu = 80.0;
        let busy = run(&snapshot, View::Ripple);
        assert!(busy.wave.as_ref().unwrap().energy() > 1.0);
        assert_eq!(busy.positions.len(), busy.visible);
        let flow = run(&snapshot, View::Flow);
        let particles = &flow.flow.as_ref().unwrap().particles;
        assert!(
            particles.iter().any(|p| p.bright),
            "busy process and links emit"
        );
        assert!(
            particles.iter().any(|p| !p.bright),
            "background reveals the field"
        );
    }

    #[test]
    fn pebbles_cluster_by_cgroup_without_touching_and_stay_put() {
        let mut snapshot = demo(1.0, 64);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, View::Ripple, 1.0);
        assert_eq!(scene.clusters.len(), 4);
        let flat = |scene: &Scene, id: &Identity| {
            let p = scene.positions[id];
            [p[0], p[1]]
        };
        for (i, a) in snapshot.processes.iter().enumerate() {
            let at = flat(&scene, &a.id);
            let home = scene.clusters[&a.group].center;
            for (name, cluster) in &scene.clusters {
                let d = |c: [f32; 2]| (at[0] - c[0]).hypot(at[1] - c[1]);
                assert!(*name == a.group || d(home) < d(cluster.center));
            }
            for b in snapshot.processes[i + 1..]
                .iter()
                .filter(|b| b.group == a.group)
            {
                let gap = (at[0] - flat(&scene, &b.id)[0]).hypot(at[1] - flat(&scene, &b.id)[1]);
                let touching = mass_radius(a.memory as f32) + mass_radius(b.memory as f32);
                assert!(gap >= touching, "{} and {} overlap", a.name, b.name);
            }
        }
        let before: HashMap<Identity, [f32; 2]> = snapshot
            .processes
            .iter()
            .map(|p| (p.id, flat(&scene, &p.id)))
            .collect();
        snapshot.processes.remove(20);
        snapshot.processes[30].memory += snapshot.processes[30].memory / 20;
        snapshot.processes[31].cpu = 400.0;
        render(&mut scene, &snapshot, View::Ripple, 1.1);
        for p in &snapshot.processes {
            assert_eq!(flat(&scene, &p.id), before[&p.id], "{} moved", p.name);
        }
    }

    #[test]
    fn ripple_water_stays_dim_however_hard_it_is_stirred() {
        let mut snapshot = demo(2.0, 64);
        snapshot.pressure = [90.0; 3];
        for p in &mut snapshot.processes {
            p.cpu = 400.0;
        }
        let mut scene = Scene::new();
        let mut frame = render(&mut scene, &snapshot, View::Ripple, 0.0);
        for k in 1..80 {
            if k % 10 == 0 {
                scene.splashes.push([0.0; 3]);
            }
            frame = render(&mut scene, &snapshot, View::Ripple, k as f32 * 0.05);
        }
        assert!(scene.wave.as_ref().unwrap().energy() > 1.0);
        let brightest = frame
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(_, color, _) => color.iter().max().copied(),
                _ => None,
            })
            .max()
            .unwrap();
        assert!(brightest < 190, "water reached {brightest}");
    }

    #[test]
    fn every_view_renders_the_demo_and_an_empty_machine() {
        let snapshot = demo(30.0, 128);
        // The matrix draws the journal, not processes; its own tests cover it.
        for view in View::ALL.into_iter().filter(|&view| view != View::Matrix) {
            let mut scene = Scene::new();
            for k in 0..4 {
                render(&mut scene, &snapshot, view, 30.0 + k as f32 * 0.1);
            }
            assert!(scene.visible > 0, "{view:?} shows nothing");
            assert!(!scene.positions.is_empty(), "{view:?} has no positions");
            let mut camera = Camera::default();
            scene.fit(&mut camera, 320, 180);
            let mut frame = scene.render(&snapshot, view, &camera, 320, 180, None, 30.5, 512, None);
            frame.rasterize();
            let picks = frame.picks.iter().filter(|&&p| p != NONE).count();
            assert!(picks > 0, "{view:?} has nothing to click");
            assert!(
                frame
                    .picks
                    .iter()
                    .all(|&p| p == NONE || (p as usize) < frame.identities.len())
            );
            render(&mut Scene::new(), &Snapshot::default(), view, 0.0);
        }
    }

    #[test]
    fn marbles_hop_lanes_when_the_scheduler_moves_them() {
        let mut snapshot = demo(1.0, 16);
        for p in &mut snapshot.processes {
            p.cpu = 0.0;
            p.state = 'S';
        }
        snapshot.processes[3].cpu = 60.0;
        snapshot.processes[3].core = 0;
        let id = snapshot.processes[3].id;
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, View::Cores, 1.0);
        let resting = scene.positions[&id][2];
        snapshot.processes[3].core = 12;
        render(&mut scene, &snapshot, View::Cores, 1.1);
        render(&mut scene, &snapshot, View::Cores, 1.4);
        assert!(scene.positions[&id][2] > resting + 0.5, "airborne mid-hop");
        assert!(scene.notes[&id][0].contains("cpu12") && scene.notes[&id][0].contains("1 hop "));
        render(&mut scene, &snapshot, View::Cores, 2.0);
        let landed = scene.positions[&id];
        assert!(
            landed[0].hypot(landed[1]) > 20.0,
            "efficiency lanes are outside"
        );
        assert_eq!(scene.visible, 1, "idle processes stay off the track");
    }

    #[test]
    fn strata_ridges_follow_busy_processes_in_kind_order() {
        let mut scene = Scene::new();
        let mut snapshot = demo(0.0, 64);
        for step in 0..30 {
            snapshot.elapsed = step as f64;
            for (i, p) in snapshot.processes.iter_mut().enumerate() {
                p.cpu = if i % 4 == 0 { 30.0 + step as f32 } else { 0.0 };
            }
            // History builds up while another view is shown.
            scene.record(&snapshot, step as f32);
            render(&mut scene, &snapshot, View::City, step as f32);
        }
        render(&mut scene, &snapshot, View::Strata, 29.0);
        assert_eq!(scene.visible, 16);
        let first = snapshot.processes[0].id;
        assert!(
            scene.notes[&first][0].ends_with("over the last 29 s"),
            "{:?}",
            scene.notes[&first]
        );
        let rows: HashMap<Identity, f32> =
            scene.positions.iter().map(|(id, p)| (*id, p[1])).collect();
        snapshot.elapsed = 30.0;
        snapshot.processes[33].cpu = 50.0;
        render(&mut scene, &snapshot, View::Strata, 30.0);
        assert_eq!(scene.visible, 17, "a newly busy process earns a ridge");
        for (id, y) in &rows {
            assert_eq!(scene.positions[id][1], *y, "an existing ridge moved row");
        }
        let order: Vec<u8> = scene
            .strata_rows()
            .iter()
            .map(|id| {
                let kind = snapshot
                    .processes
                    .iter()
                    .find(|p| p.id == *id)
                    .unwrap()
                    .kind;
                [Kind::Kernel, Kind::System, Kind::Session, Kind::Container]
                    .iter()
                    .position(|&k| k == kind)
                    .unwrap() as u8
            })
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] <= pair[1]), "{order:?}");
    }

    #[test]
    fn dishes_and_globe_talkers_keep_their_places() {
        let mut snapshot = demo(10.0, 128);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, View::Cells, 10.0);
        let containers: Vec<(Identity, Point)> = snapshot
            .processes
            .iter()
            .filter(|p| p.kind == Kind::Container)
            .map(|p| (p.id, scene.positions[&p.id]))
            .collect();
        let session = snapshot
            .processes
            .iter()
            .find(|p| p.kind == Kind::Session)
            .unwrap()
            .cgroup
            .clone();
        snapshot.units.get_mut(&session).unwrap().memory *= 400;
        render(&mut scene, &snapshot, View::Cells, 10.1);
        render(&mut scene, &snapshot, View::Cells, 10.2);
        for (id, at) in &containers {
            assert_eq!(scene.positions[id], *at, "the container dish moved");
        }
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, View::Globe, 0.0);
        let mut talkers: Vec<Identity> = snapshot.remotes.iter().map(|r| r.id).collect();
        talkers.sort();
        let before: Vec<Point> = talkers[1..].iter().map(|id| scene.positions[id]).collect();
        snapshot.remotes.retain(|r| r.id != talkers[0]);
        render(&mut scene, &snapshot, View::Globe, 0.0);
        let after: Vec<Point> = talkers[1..].iter().map(|id| scene.positions[id]).collect();
        assert_eq!(
            before, after,
            "a process closing its connections reshuffled the others"
        );
    }

    #[test]
    fn globe_shows_only_processes_with_remote_connections() {
        let snapshot = demo(10.0, 128);
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, View::Globe, 10.0);
        let talkers: HashSet<Identity> = snapshot.remotes.iter().map(|r| r.id).collect();
        assert_eq!(scene.visible, talkers.len());
        assert!(scene.positions.keys().all(|id| talkers.contains(id)));
        let id = snapshot.remotes[0].id;
        assert!(scene.notes[&id][0].contains(&snapshot.remotes[0].address.to_string()));
    }

    #[test]
    fn city_does_not_relocate_when_resources_or_population_change() {
        let mut scene = Scene::new();
        let mut snapshot = demo(0.0, 32);
        render(&mut scene, &snapshot, View::City, 0.0);
        let id = snapshot.processes[10].id;
        let position = scene.positions[&id];
        snapshot.processes.remove(5);
        snapshot.processes[9].cpu = 1600.0;
        snapshot.processes[9].memory *= 100;
        render(&mut scene, &snapshot, View::City, 0.0);
        assert_eq!(scene.positions[&id], position);
    }

    #[test]
    fn depth_buffer_picks_the_frontmost_object() {
        let mut frame = blank(10, 10);
        let a = Identity { pid: 1, start: 1 };
        let b = Identity { pid: 2, start: 2 };
        frame.identities = vec![a, b];
        frame.pixel(5, 5, 4.0, [10, 20, 30], 1);
        frame.pixel(5, 5, 2.0, [30, 20, 10], 0);
        assert_eq!(frame.pick_near(5.5, 5.5, 0.5), Some(b));
        assert_eq!(frame.pick_near(20.5, 5.5, 0.5), None);
    }

    #[test]
    fn cell_sized_picking_selects_the_nearest_small_object() {
        let mut frame = blank(40, 40);
        let near = Identity { pid: 1, start: 1 };
        let far = Identity { pid: 2, start: 2 };
        frame.identities = vec![near, far];
        frame.pixel(24, 20, 0.0, [255; 3], 0);
        frame.pixel(10, 20, 0.0, [255; 3], 1);
        assert_eq!(frame.pick_near(20.5, 20.5, 9.0), Some(near));
        assert_eq!(frame.pick_near(14.5, 20.5, 9.0), Some(far));
        assert_eq!(frame.pick_near(20.5, 5.5, 9.0), None);
    }

    #[test]
    fn leaves_stay_in_orbit_and_systems_keep_clear_of_each_other() {
        let mut processes = vec![process(1, 0), process(2, 0)];
        processes.extend((10..60).map(|pid| process(pid, 2)));
        processes.extend((200..220).map(|pid| process(pid, 1)));
        for service in (100..160).step_by(10) {
            processes.push(process(service, 1));
            processes.extend((1..4).map(|k| process(service + k, service)));
        }
        let mut snapshot = Snapshot {
            processes,
            cores: 4,
            ..Default::default()
        };
        let mut scene = Scene::new();
        render(&mut scene, &snapshot, View::Orbit, 0.0);
        // init, kthreadd and the six services with children; kernel threads and leaf services orbit.
        assert_eq!(scene.systems.len(), 8);
        assert_eq!(scene.stars.len(), 8);
        assert_eq!(scene.visible, snapshot.processes.len());
        let systems: Vec<_> = scene.systems.values().copied().collect();
        for (i, (a, ra)) in systems.iter().enumerate() {
            for (b, rb) in &systems[i + 1..] {
                assert!((a[0] - b[0]).hypot(a[1] - b[1]) >= ra + rb - 1e-3);
            }
        }
        snapshot.processes[5].cpu = 800.0;
        let center = scene.galaxy;
        let reach = |placement: &([f32; 2], f32)| {
            (placement.0[0] - center[0]).hypot(placement.0[1] - center[1])
        };
        let before = scene.systems.clone();
        for step in 1..=240 {
            render(&mut scene, &snapshot, View::Orbit, step as f32 * 0.5);
        }
        assert_eq!(scene.systems.len(), 8, "turning never re-places a system");
        assert_eq!(
            scene.galaxy, center,
            "the pivot stays put while membership is stable"
        );
        let mut turned = 0;
        for (id, placement) in &scene.systems {
            assert!(
                (reach(placement) - reach(&before[id])).abs() < 0.01,
                "{id:?} left its orbit"
            );
            assert_eq!(placement.1, before[id].1);
            turned += usize::from(placement.0 != before[id].0);
        }
        assert!(turned >= 7, "only {turned} systems revolved");
        let service = snapshot
            .processes
            .iter()
            .find(|p| p.id.pid == 100)
            .unwrap()
            .id;
        let reserved = scene.systems[&service].1;
        snapshot
            .processes
            .extend((1000..1040).map(|pid| process(pid, 100)));
        render(&mut scene, &snapshot, View::Orbit, 121.0);
        assert!(
            scene.systems[&service].1 > reserved,
            "the grown system was re-placed"
        );
        assert_ne!(scene.galaxy, center, "re-placing a system moves the pivot");
        let systems: Vec<_> = scene.systems.values().copied().collect();
        for (i, (a, ra)) in systems.iter().enumerate() {
            for (b, rb) in &systems[i + 1..] {
                assert!((a[0] - b[0]).hypot(a[1] - b[1]) >= ra + rb - 1e-2);
            }
        }
    }

    #[test]
    fn overlapping_rings_turn_together_and_detached_systems_turn_slower() {
        let id = |pid: u32| Identity { pid, start: 1 };
        let mut scene = Scene::new();
        scene.systems = HashMap::from([
            (id(1), ([0.0, 0.0], 5.0)),
            (id(2), ([8.5, 0.0], 3.0)),
            (id(3), ([60.0, 0.0], 3.0)),
        ]);
        let masses = HashMap::from([(id(1), 4e9), (id(2), 1e9), (id(3), 1e8)]);
        scene.frames += 1;
        scene.revolve(&masses, 0.0);
        let speed = |scene: &Scene, pid| scene.galaxy_speed[&id(pid)];
        assert_eq!(
            speed(&scene, 1),
            speed(&scene, 2),
            "touching rings share a band"
        );
        assert!(
            speed(&scene, 3) < speed(&scene, 2),
            "outer bands turn slower"
        );
        assert!(TAU / speed(&scene, 3) <= 900.0 && TAU / speed(&scene, 1) >= 120.0);
        for step in 1..=2000 {
            scene.frames += 1;
            scene.revolve(&masses, step as f32 * 0.5);
        }
        let [(a, ra), (b, rb), (c, rc)] = [1, 2, 3].map(|pid| scene.systems[&id(pid)]);
        let gap = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
        assert!((gap(a, b) - 8.5).abs() < 0.01, "a band turns rigidly");
        assert!(gap(a, c) >= ra + rc && gap(b, c) >= rb + rc);
        assert!(c[1].abs() > 1.0, "the detached system moved round");
        scene.frames += 1;
        scene.revolve(&masses, 5000.0);
        assert_eq!(
            scene.systems[&id(3)].0,
            c,
            "a jump in time does not spin the galaxy"
        );
    }

    #[test]
    fn the_galaxy_turns_at_one_frame_per_second_but_not_across_view_switches() {
        let id = |pid: u32| Identity { pid, start: 1 };
        let mut scene = Scene::new();
        scene.systems = HashMap::from([(id(1), ([0.0, 0.0], 5.0)), (id(2), ([40.0, 0.0], 3.0))]);
        let masses = HashMap::from([(id(1), 4e9), (id(2), 1e8)]);
        let mut motion = 0.0;
        let mut turn = |scene: &mut Scene, frames: u64, seconds: f32| {
            scene.frames += frames;
            motion += seconds;
            scene.revolve(&masses, motion);
            scene.systems[&id(2)].0
        };
        turn(&mut scene, 1, 0.0);
        let start = turn(&mut scene, 1, 1.0);
        let next = turn(&mut scene, 1, 1.0);
        assert_ne!(
            start, next,
            "--fps 1 frames are a second apart and still turn it"
        );
        let away = turn(&mut scene, 3, 0.3);
        assert_eq!(
            away, next,
            "frames drawn in another view are skipped, however short"
        );
        // A light band close in and a heavy one further out, about a pivot that stays put.
        let mut scene = Scene::new();
        scene.systems = HashMap::from([(id(1), ([20.0, 0.0], 3.0)), (id(2), ([60.0, 0.0], 3.0))]);
        scene.galaxy_members = vec![id(1), id(2)];
        scene.revolve(&HashMap::from([(id(1), 1e6), (id(2), 64e9)]), 0.0);
        assert!(
            scene.galaxy_speed[&id(2)] <= scene.galaxy_speed[&id(1)],
            "outer bands never outpace inner ones"
        );
    }

    #[test]
    fn both_scenes_render_and_focus_preserves_descendants() {
        let snapshot = demo(1.0, 128);
        let branch = snapshot.processes[16].id;
        let set = descendants(&snapshot, branch);
        assert_eq!(set.len(), 16);
        for view in [View::City, View::Orbit] {
            let mut scene = Scene::new();
            let mut frame = scene.render(
                &snapshot,
                view,
                &Camera::default(),
                320,
                180,
                Some(branch),
                1.0,
                512,
                Some(branch),
            );
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
}
