use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use crate::history::History;
use crate::medium::Grid;
use crate::model::{Identity, Process, Snapshot, bounded};
use crate::palette::tint;
use crate::simulation::{growth, hash, smoothing};
use crate::views::systems::Orbit;
use crate::views::{LinkRule, REGISTRY, View, ViewInfo, ViewKey, ViewModule, ViewOptions};
use crate::{globe, glyphs};

pub mod field;
pub mod shared;
pub(crate) mod strokes;

pub type Color = [u8; 3];
pub type Point = [f32; 3];
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "first used by a view that draws a field")
)]
pub use field::{Field, Placement, Relief, Shading};
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "first used by a view that draws a field")
)]
pub use shared::LIGHT;
pub use shared::NONE;
/// The pick-buffer bit that marks an index into `Frame::things` rather than `Frame::identities`.
/// Process indices stay below it, and a thing index below `THING - 1`, so neither meets `NONE`.
pub const THING: u32 = 1 << 31;
/// Screen pixels per world unit at zoom 1: sqrt(3/2), the isometric foreshortening.
pub(crate) const SCALE: f32 = 1.224_745;
/// Elevation of a true isometric view, asin(1/sqrt(3)).
pub const ISOMETRIC: f32 = 0.615_480;
/// The lowest camera elevation allowed: the camera never looks from below the ground.
pub const LOWEST_PITCH: f32 = 0.35;
/// The camera centre sits slightly below the middle to leave headroom for buildings.
pub const ORIGIN_Y: f32 = 0.57;
/// Depth assigned to background stars, below every scene object.
pub const BACKGROUND: f32 = -10000.0;
pub(crate) const MIN_BODY_PX: f32 = 3.5;
/// The longest piece of a `Frame::hairline` one pair of triangles covers, in pixels.
const HAIRLINE_PIECE: f32 = 8.0;
const FADE_SECONDS: f32 = 1.2;
pub(crate) const WARM: Color = [255, 190, 110];
pub(crate) const NVIDIA: Color = [118, 214, 60];
const LINK: Color = [80, 175, 235];
const LINK_HOT: Color = [150, 232, 255];
pub(crate) const OUTSIDE: Color = [255, 110, 200];

/// The sky behind a view. The GPU shader receives the discriminant as `globals.sky.x`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    Dusk = shared::BACKDROP_DUSK as isize,
    Space = shared::BACKDROP_SPACE as isize,
    Sea = shared::BACKDROP_SEA as isize,
    Void = shared::BACKDROP_VOID as isize,
    /// A dusk sky in three stops, from deep blue through violet to an orange horizon.
    Twilight = shared::BACKDROP_TWILIGHT as isize,
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
    /// Globe only: how far the earth has been turned by hand about its axis, in radians.
    pub globe_turn: f32,
    /// Globe only: how far the earth leans about the screen-horizontal axis, in radians;
    /// positive brings the southern hemisphere up. Used clamped to the camera's pitch +- pi/2,
    /// because the pitch changes without touching it.
    pub globe_tilt: f32,
    /// Globe only: the earth holds still at `globe_turn` instead of turning with time.
    pub globe_held: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            zoom: 5.0,
            rotation: 0.0,
            pitch: ISOMETRIC,
            globe_turn: 0.0,
            globe_tilt: 0.0,
            globe_held: false,
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
    pub(crate) fn ground(&self, dx: f32, dy: f32) -> [f32; 2] {
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
        self.pitch = (self.pitch + delta).clamp(LOWEST_PITCH, FRAC_PI_2);
    }

    /// Unit vector from the scene towards the viewer, for specular highlights.
    pub(crate) fn viewer(&self) -> Point {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        let (ps, pc) = self.pitch.sin_cos();
        [s * pc, c * pc, ps]
    }

    /// Centres the view on `p` itself rather than the ground below it: a raised point appears
    /// above its ground point by its height over tan(pitch) along the view direction.
    pub fn look_at(&mut self, p: Point) {
        let (s, c) = (self.rotation + FRAC_PI_4).sin_cos();
        let along = p[2] / self.pitch.tan();
        self.center = [p[0] - along * s, p[1] - along * c];
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
        self.globe_turn += ((goal.globe_turn - self.globe_turn + PI).rem_euclid(TAU) - PI) * amount;
        self.globe_tilt += (goal.globe_tilt - self.globe_tilt) * amount;
        self.globe_held = goal.globe_held;
    }

    /// Globe only: freezes the earth where it is at `time`, folding the turn it has made with
    /// time into `globe_turn`, so nothing moves on screen. Does nothing if already held.
    pub fn hold(&mut self, time: f32) {
        if !self.globe_held {
            self.globe_turn = (self.globe_turn + globe::drift(time)).rem_euclid(TAU);
            self.globe_held = true;
        }
    }

    /// Globe only: lets a held earth turn with time again from where it stands at `time`.
    /// Does nothing unless held.
    pub fn release(&mut self, time: f32) {
        if self.globe_held {
            self.globe_turn = (self.globe_turn - globe::drift(time)).rem_euclid(TAU);
            self.globe_held = false;
        }
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
            Backdrop::Space => shared::SPACE_BASE,
            Backdrop::Dusk => {
                std::array::from_fn(|i| shared::DUSK_BASE[i] + y * shared::DUSK_SLOPE[i])
            }
            Backdrop::Sea => {
                std::array::from_fn(|i| shared::SEA_BASE[i] - y * shared::SEA_SLOPE[i])
            }
            Backdrop::Void => [0.0; 3],
            Backdrop::Twilight => {
                let (from, to, along) = if y < shared::TWILIGHT_KNEE {
                    (
                        shared::TWILIGHT_TOP,
                        shared::TWILIGHT_MIDDLE,
                        y / shared::TWILIGHT_KNEE,
                    )
                } else {
                    (
                        shared::TWILIGHT_MIDDLE,
                        shared::TWILIGHT_HORIZON,
                        (y - shared::TWILIGHT_KNEE) / (1.0 - shared::TWILIGHT_KNEE),
                    )
                };
                std::array::from_fn(|i| from[i] + (to[i] - from[i]) * along)
            }
        }
    }

    /// Haze density along one normalised screen axis; the field is the product of both axes.
    pub fn profile(&self, t: f32, phase: f32) -> f32 {
        0.5 + 0.5
            * (t * shared::HAZE_FREQUENCY_A + self.time * shared::HAZE_DRIFT_A + phase).sin()
            * (t * shared::HAZE_FREQUENCY_B - self.time * shared::HAZE_DRIFT_B
                + phase * shared::HAZE_PHASE_B)
                .cos()
    }

    pub fn hazy(&self) -> bool {
        self.haze.iter().any(|&c| c > 0.5)
    }
}

/// Something pickable that is not a process: an IRQ line, a file, a grave. `kind` is a short
/// lowercase noun that the inspector names it by; `key` belongs to the view that draws it and
/// stays the same from frame to frame (an IRQ number, a hash of device and inode, a serial).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Thing {
    pub kind: &'static str,
    pub key: u64,
}

/// What a pick names: a process, or a thing a view drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Process(Identity),
    Thing(Thing),
}

/// The inspector popup of a thing, built on demand by the view that drew it. Its text is
/// untrusted, like process text, and reaches the terminal only through `terminal::clean_text`.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub title: String,
    pub lines: Vec<String>,
    /// The world point the popup is placed beside.
    pub at: Point,
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
    /// A soft halo like `Glow`, blended with round-to-nearest, as the GPU's blend unit does, so
    /// that any number of them can stack and the two rasterizers still agree to a level (a
    /// `Glow` truncates on the CPU, and stacked ones drift apart). Strength is clamped to
    /// [0, 1]. For content whose look is the density of many splats, such as a flock.
    Puff {
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
    /// A disc that writes only the pick buffer, so blended or splatted content stays pickable:
    /// no colour and no depth, depth-tested at `depth` as a sphere is. `pick` names a process or
    /// a thing. Record it after the geometry it lies over: it writes no depth, so geometry
    /// recorded later covers it wherever that geometry passes the depth test.
    Pick {
        center: [f32; 2],
        radius: f32,
        depth: f32,
        pick: u32,
    },
    /// Depth-tested triangle whose colour and pick are sampled from field `field` (an index into
    /// `Frame::fields`) at `uv`, in node units, interpolated linearly across the triangle. It
    /// writes depth, the sampled colour and the owner of the nearest node (see `field`).
    Field {
        points: [Point; 3],
        uv: [[f32; 2]; 3],
        field: u32,
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
    /// The fields the last frame drew, kept with their shaded colours so a field that has not
    /// changed is not shaded again.
    fields: Vec<field::ShadedField>,
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
    /// The things this frame's thing picks index, in `thing_pick` order.
    pub things: Vec<Thing>,
    /// The fields this frame draws, in the order `Frame::field` recorded them: what an
    /// `Item::Field` indexes.
    pub fields: Vec<field::ShadedField>,
    /// The previous frame's fields, which `Frame::field` takes back when one is unchanged.
    earlier_fields: Vec<field::ShadedField>,
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
            fields,
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
            things: Vec::new(),
            fields: Vec::new(),
            earlier_fields: fields,
            anchor: None,
        };
        if sky.backdrop == Backdrop::Space {
            for i in 0..(width * height / 1100).clamp(200, 4000) {
                let hash = hash::knuth(i);
                let x = (hash % width) as f32;
                let y = (hash.rotate_left(13) % height) as f32;
                frame.items.push(Item::Star([x, y], [37, 48, 71]));
            }
        }
        frame
    }

    /// Depth per pixel after a CPU rasterization; larger is nearer.
    #[cfg(test)]
    pub fn depth(&self) -> &[f32] {
        &self.depth
    }

    pub fn release(self) -> Buffers {
        Buffers {
            items: self.items,
            pixels: self.pixels,
            depth: self.depth,
            picks: self.picks,
            fields: self.fields,
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
        let fields = std::mem::take(&mut self.fields);
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
                Item::Puff {
                    center,
                    radius,
                    color,
                    strength,
                } => self.puff(center, radius, color, strength.clamp(0.0, 1.0)),
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
                Item::Pick {
                    center,
                    radius,
                    depth,
                    pick,
                } => self.splat(center, radius, depth, pick),
                Item::Field { points, uv, field } => {
                    self.field_triangle(points, uv, &fields[field as usize]);
                }
            }
        }
        self.items = items;
        self.fields = fields;
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

    /// The pick value for `thing` in this frame. Call it once per thing per frame and give
    /// every item of that thing the value it returns.
    pub fn thing_pick(&mut self, thing: Thing) -> u32 {
        debug_assert!(
            self.things.len() < (THING - 1) as usize,
            "thing indices stay clear of NONE"
        );
        self.things.push(thing);
        (self.things.len() - 1) as u32 | THING
    }

    /// What a pick-buffer value names, if anything.
    pub fn target(&self, pick: u32) -> Option<Target> {
        if pick == NONE {
            None
        } else if pick & THING != 0 {
            let index = (pick & !THING) as usize;
            self.things.get(index).copied().map(Target::Thing)
        } else {
            self.identities
                .get(pick as usize)
                .copied()
                .map(Target::Process)
        }
    }

    /// Nearest process pixel within `radius` of (x, y); terminal mouse input is imprecise.
    /// Things are passed over: `target_near` finds them.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the app picks through target_near since B9; kept for process-only callers"
        )
    )]
    pub fn pick_near(&self, x: f32, y: f32, radius: f32) -> Option<Identity> {
        self.nearest(x, y, radius, |pick| pick & THING == 0)
            .and_then(|index| self.identities.get(index as usize).copied())
    }

    /// The process or thing at the nearest pickable pixel within `radius` of (x, y), by the
    /// same search and tie rule as `pick_near`.
    pub fn target_near(&self, x: f32, y: f32, radius: f32) -> Option<Target> {
        self.nearest(x, y, radius, |pick| pick != NONE)
            .and_then(|pick| self.target(pick))
    }

    /// The pick value of the nearest pixel within `radius` of (x, y) whose pick `accept`s; of
    /// pixels equally near, the last in row order.
    fn nearest(&self, x: f32, y: f32, radius: f32, accept: impl Fn(u32) -> bool) -> Option<u32> {
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
                if accept(pick) && distance <= best_distance {
                    best_distance = distance;
                    best = Some(pick);
                }
            }
        }
        best
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

    /// `blend` rounded to the nearest level, as the GPU's blend unit rounds.
    fn blend_rounded(&mut self, x: i32, y: i32, color: Color, weight: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 3;
        for (channel, &target) in self.pixels[i..i + 3].iter_mut().zip(&color) {
            *channel = (*channel as f32 + (target as f32 - *channel as f32) * weight + 0.5) as u8;
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
                a[2] + (b[2] - a[2]) * t + shared::LINE_DEPTH_BIAS,
                color,
                NONE,
            );
        }
    }

    /// Writes `pick` over a disc where `depth` is at least as near as what is drawn, as a
    /// sphere's surface tests, leaving colour and depth alone. Samples at integer pixel
    /// coordinates like `ball`.
    fn splat(&mut self, center: [f32; 2], radius: f32, depth: f32, pick: u32) {
        let x0 = (center[0] - radius).floor().max(0.0) as i32;
        let x1 = (center[0] + radius).ceil().min(self.width as f32 - 1.0) as i32;
        let y0 = (center[1] - radius).floor().max(0.0) as i32;
        let y1 = (center[1] + radius).ceil().min(self.height as f32 - 1.0) as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let nx = (x as f32 - center[0]) / radius;
                let ny = (y as f32 - center[1]) / radius;
                let i = y as usize * self.width as usize + x as usize;
                if nx * nx + ny * ny <= 1.0 && depth >= self.depth[i] {
                    self.picks[i] = pick;
                }
            }
        }
    }

    fn ball(&mut self, p: Point, r: f32, depth: f32, color: Color, pick: u32, selected: bool) {
        let outer = if selected {
            r + shared::SELECTION_RING_WIDTH
        } else {
            r + shared::SPHERE_EDGE
        };
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
                    let light = (shared::SPHERE_AMBIENT
                        + (-nx * shared::SPHERE_LIGHT_X - ny * shared::SPHERE_LIGHT_Y
                            + nz * shared::SPHERE_LIGHT_Z)
                            .max(0.0))
                    .min(shared::SPHERE_LIGHT_MAX);
                    self.pixel(x, y, p[2] + nz * depth, tint(color, light), pick);
                } else if selected && d <= (outer / r).powi(2) {
                    self.pixel(x, y, p[2], shared::SELECTION_COLOR.map(|c| c as u8), pick);
                }
            }
        }
    }

    fn halo(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        self.disc(p, radius, color, strength, Self::blend);
    }

    fn puff(&mut self, p: [f32; 2], radius: f32, color: Color, strength: f32) {
        self.disc(p, radius, color, strength, Self::blend_rounded);
    }

    /// Blends `color` over the pixels within `radius` of `p` with weight `strength` times the
    /// square of one minus the distance in radii, by `blend`.
    fn disc(
        &mut self,
        p: [f32; 2],
        radius: f32,
        color: Color,
        strength: f32,
        blend: fn(&mut Self, i32, i32, Color, f32),
    ) {
        let x0 = (p[0] - radius).floor() as i32;
        let x1 = (p[0] + radius).ceil() as i32;
        let y0 = (p[1] - radius).floor() as i32;
        let y1 = (p[1] + radius).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - p[0]).hypot(y as f32 + 0.5 - p[1]) / radius;
                if d < 1.0 {
                    blend(self, x, y, color, strength * (1.0 - d) * (1.0 - d));
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

    pub(crate) fn trace(&mut self, camera: &Camera, orbit: &Orbit, color: Color) {
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

    /// A segment as a ribbon `width` pixels wide, in pieces of at most `HAIRLINE_PIECE` pixels so
    /// no triangle scans a large box. Like a line it is depth-tested and leaves nothing pickable
    /// where it is drawn; unlike a one-pixel line or beam, its triangles rasterize alike on the
    /// CPU and the GPU.
    pub(crate) fn hairline(
        &mut self,
        camera: &Camera,
        a: Point,
        b: Point,
        color: Color,
        width: f32,
    ) {
        let a = self.project(camera, a);
        let b = self.project(camera, b);
        self.hairline_between(a, b, color, width);
    }

    /// `hairline` between two points already in frame space (x, y, depth), for a view that draws
    /// in screen space, such as a lensed image, and sets the depth of its layers itself.
    pub(crate) fn hairline_between(&mut self, a: Point, b: Point, color: Color, width: f32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = dx.hypot(dy);
        if !(1e-3..=12000.0).contains(&length) {
            return;
        }
        let half = width * 0.5;
        let (nx, ny) = (-dy / length * half, dx / length * half);
        let pieces = (length / HAIRLINE_PIECE).ceil() as usize;
        let at = |t: f32| [a[0] + dx * t, a[1] + dy * t, a[2] + (b[2] - a[2]) * t];
        for piece in 0..pieces {
            let p = at(piece as f32 / pieces as f32);
            let q = at((piece + 1) as f32 / pieces as f32);
            let corners = [
                [p[0] + nx, p[1] + ny, p[2]],
                [q[0] + nx, q[1] + ny, q[2]],
                [q[0] - nx, q[1] - ny, q[2]],
                [p[0] - nx, p[1] - ny, p[2]],
            ];
            self.items.push(Item::Triangle(
                [corners[0], corners[1], corners[2]],
                color,
                NONE,
            ));
            self.items.push(Item::Triangle(
                [corners[0], corners[2], corners[3]],
                color,
                NONE,
            ));
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

    /// A pick-only disc of `radius` pixels at the projection of `position`, depth-tested there:
    /// it keeps splatted or translucent content under it pickable.
    pub(crate) fn pick_disc(&mut self, camera: &Camera, position: Point, radius: f32, pick: u32) {
        let center = self.project(camera, position);
        self.items.push(Item::Pick {
            center: [center[0], center[1]],
            radius,
            depth: center[2],
            pick,
        });
    }

    /// The silhouette of a sphere `sphere` would draw, as a ring of lines facing the viewer with
    /// a fainter ring inside it: a body drawn hollow. Lines draw alike in both rasterizers and,
    /// like every line, leave nothing pickable.
    pub(crate) fn outline(&mut self, camera: &Camera, position: Point, radius: f32, color: Color) {
        let center = self.project(camera, position);
        let outer = (radius * camera.zoom).max(MIN_BODY_PX);
        for (scale, shade) in [(1.0, 1.0), (0.6, 0.45)] {
            let point = |k: usize| {
                let angle = k as f32 / 48.0 * TAU;
                [
                    center[0] + outer * scale * angle.cos(),
                    center[1] + outer * scale * angle.sin(),
                    center[2],
                ]
            };
            for k in 0..48 {
                self.items
                    .push(Item::Line(point(k), point(k + 1), tint(color, shade)));
            }
        }
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

/// Everything a frame depends on besides the scene's own state: what `Scene::render` draws.
#[derive(Clone, Copy)]
pub struct RenderInput<'a> {
    /// The sample to draw: the newest, or the one rewound to while paused.
    pub snapshot: &'a Snapshot,
    pub view: View,
    pub camera: &'a Camera,
    /// The frame's size in pixels.
    pub width: u32,
    pub height: u32,
    pub selected: Option<Identity>,
    /// The selected thing, or else the hovered one, for the view to highlight.
    pub thing: Option<Thing>,
    /// Seconds on the animation clock, or the shown sample's `elapsed` while paused.
    pub time: f32,
    /// The most processes drawn; the rest of the PID order is left out, the selection kept.
    pub limit: usize,
    /// The root of the subtree to draw alone, if one is focused.
    pub focus: Option<Identity>,
    /// Whether the app is paused, so `Stage::dt` is 0 whatever `time` does.
    pub paused: bool,
    /// The process the tour is visiting, if a tour runs.
    pub toured: Option<Identity>,
}

#[cfg(test)]
impl<'a> RenderInput<'a> {
    /// A 320 by 180 frame of `view` at `time`, running, with nothing selected or focused and at
    /// most 512 processes, as most tests draw.
    pub fn small(snapshot: &'a Snapshot, view: View, camera: &'a Camera, time: f32) -> Self {
        RenderInput {
            snapshot,
            view,
            camera,
            width: 320,
            height: 180,
            selected: None,
            thing: None,
            time,
            limit: 512,
            focus: None,
            paused: false,
            toured: None,
        }
    }
}

/// How many arrivals of processes absent from the shown sample the scene remembers, oldest
/// forgotten first.
const ARRIVALS_KEPT: usize = 4096;

/// The longest step `Stage::dt` reports, in seconds. A longer gap (a stall, a slow frame) counts
/// as this much, so a simulation never leaps.
pub(crate) const MAX_DT: f32 = 0.5;

/// The one step policy behind `Stage::dt`: the time of the last frame that moved forward.
#[derive(Default)]
struct FrameClock {
    last: Option<f32>,
}

impl FrameClock {
    /// The step for a frame at `time`. Paused frames leave the clock where it was: the app's
    /// animation clock stands still while paused, so the first frame after it steps one frame.
    /// A frame whose time went back restarts the count from its own time.
    fn step(&mut self, time: f32, paused: bool) -> f32 {
        if paused {
            return 0.0;
        }
        let dt = match self.last {
            Some(last) if time > last => (time - last).min(MAX_DT),
            _ => 0.0,
        };
        self.last = Some(time);
        dt
    }
}

/// A process a view asks the tour (`g` and the idle tour) to visit, besides the busiest and the
/// largest processes it always picks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TourTarget {
    pub id: Identity,
    /// Where the tour centres the camera and points its callout.
    pub position: Point,
    /// The tour visits heavier targets first, as Orbit's stars by the processes they hold.
    pub weight: f32,
}

/// The name of a system a view draws, which the app labels in bright text just under the system's
/// outer edge (or at `at` when that edge is far below it): a `Title` for each system worth a
/// label, in the order the labels are offered a place.
#[derive(Clone, Debug, PartialEq)]
pub struct Title {
    /// The system's centre, which the label is placed from.
    pub at: Point,
    /// How far the system reaches from `at` in world units.
    pub reach: f32,
    /// The label, already cut to fit (`label_name`).
    pub text: String,
}

/// Which side of its point a `Tag` is written on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no view writes a tag to the left of its point; the app moves one that runs off the screen"
    )
)]
pub enum Side {
    Right,
    Left,
}

/// A short text written beside a world point instead of centred on it, as a `places` entry is:
/// the name of a star next to the star. `side` is where the text goes if it fits; the app writes
/// it on the other side when that one runs off the screen, and leaves it out when neither fits
/// or another label is in the way. Its text is untrusted like process text and reaches the
/// terminal only through `terminal::clean_text`.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub at: Point,
    pub side: Side,
    pub text: String,
}

/// What a view draws with, and its only way to talk back to the app. A view reads only `Stage`,
/// the process slice and `&Snapshot`, and talks back only through `Stage` outputs. What the
/// platform could not measure is `snapshot.missing`; `Stage` does not copy it.
pub(crate) struct Stage<'a> {
    pub frame: &'a mut Frame,
    pub camera: &'a Camera,
    pub selected: Option<Identity>,
    /// The selected thing, or the hovered one if none is selected, so the view can highlight
    /// it. Never Some while a process is selected.
    pub thing: Option<Thing>,
    /// The process the tour is visiting, if a tour runs; the tour clears the selection.
    pub toured: Option<Identity>,
    /// The root of the focused subtree, if one is focused: the process slice holds only that
    /// subtree, so a view leaves out what it draws besides the processes (exited ones).
    pub focus: Option<Identity>,
    /// Which links the `c` key shows. A view whose row says `LinkRule::DrawnByView` draws its
    /// own links and follows it: all of them, only those of the `highlight`, or none.
    pub links: Links,
    /// The processes whose links stand out: the selected, the hovered and the toured.
    pub highlight: &'a [Identity],
    /// `--limit`, the most processes a frame draws. The slice is already cut to it; a view that
    /// draws more than the slice (exited processes) keeps within it.
    pub limit: usize,
    pub time: f32,
    /// Frames the scene has rendered, this one included, whichever view was shown. A view that
    /// must tell a frame that follows its own last frame from one after a gap (another view was
    /// shown in between) compares it with the number it last drew at: the galaxy of Orbit
    /// turns only across consecutive frames (Flow's is held still).
    pub frames: u64,
    /// Seconds to advance a simulation by this frame, by the one policy for new code: 0 on the
    /// first frame, while paused, and when `time` went back (a rewind or a headless replay);
    /// otherwise the time since the last frame that moved forward, at most `MAX_DT`. The scene
    /// computes it once per frame whichever view is shown, so returning to a view never leaps.
    /// New views step with it, or with S2 (#23)'s `FixedStep` fed from `time`. The views that
    /// predate it keep their own clamps, because changing them changes pixels (A33 decides).
    pub dt: f32,
    /// Every process's CPU and memory over the last samples, as of this frame. The scene records
    /// each sample whichever view is shown; read the past with `History::window` and `series`,
    /// from the shown snapshot's `elapsed`.
    pub history: &'a History,
    births: &'a HashMap<Identity, f32>,
    arrivals: &'a HashMap<Identity, f32>,
    /// CPU and memory as eased for every sampled process, drawn or not.
    resources: &'a HashMap<Identity, (f32, f32)>,
    /// Where processes that left were last drawn, as the scene found out about them in its
    /// previous frame: the ground for a splash.
    pub splashes: &'a [Point],
    /// The processes that appeared in this frame's sample after the scene's first frame.
    pub newborn: &'a [Identity],
    /// Where each drawn process is, for picking anchors, labels and the tour.
    pub positions: &'a mut HashMap<Identity, Point>,
    /// World-anchored text labels.
    pub places: &'a mut Vec<(Point, String)>,
    /// Points the camera fit must include; empty means fit the positions on the ground.
    pub bounds: &'a mut Vec<Point>,
    /// Extra lines for a process's inspector popup, after its sockets.
    pub notes: &'a mut HashMap<Identity, Vec<String>>,
    /// Braille sparkline lines for a process's inspector popup, right after its notes. Only
    /// generated Braille Patterns (U+2800 to U+28FF) and spaces: the terminal draws them through
    /// `terminal::clean_sparkline`, which turns everything else into `?`, so text from the system
    /// never belongs here.
    pub sparklines: &'a mut HashMap<Identity, Vec<String>>,
    /// Processes the tour should visit; a hub is passed over, since it has no callout.
    pub tour: &'a mut Vec<TourTarget>,
    /// Lines for a process's inspector popup, right after its own figures and in the order
    /// pushed: what the view knows about it beyond the sample, such as the memory of the
    /// subtree it anchors.
    pub popup: &'a mut Vec<(Identity, String)>,
    /// Lines for the tour's callout of a process, after its own figures and in the order pushed:
    /// what the view makes of the visit, such as a frame it cannot enter.
    pub callout: &'a mut Vec<(Identity, String)>,
    /// The names of the systems the view draws, labelled under their outer edges after
    /// `places`, in the order pushed. Each system's centre (`Title::at`) is kept in the camera
    /// fit with the `positions`, when the view gave no `bounds`.
    pub titles: &'a mut Vec<Title>,
    /// Texts written beside a point, before `places` and `titles` are offered their cells: the
    /// labels a view wants read before its other text.
    pub tags: &'a mut Vec<Tag>,
    /// How far above the ground plane the view's content rises, in world units, for the camera
    /// fit to leave room for it when the view gave no `bounds`. Start at 0 each frame. Default:
    /// nothing rises.
    pub headroom: &'a mut f32,
}

impl Stage<'_> {
    pub fn growth(&self, id: Identity) -> f32 {
        growth(self.births, id, self.time)
    }

    /// Seconds since the scene first saw the process, or None for one that was there on the
    /// scene's first frame (it did not start while anyone watched) and while `time` is before
    /// that moment (a rewind). `growth` is an easing fraction that the app's intro also
    /// animates for processes already running; this is the true age of one that arrived.
    pub fn age(&self, id: Identity) -> Option<f32> {
        let arrived = *self.arrivals.get(&id)?;
        (self.time >= arrived).then_some(self.time - arrived)
    }

    /// A sampled process's CPU and memory as the scene eases them, whether it is drawn or not.
    /// A drawn process carries these very figures, so a view that sizes or ranks by processes it
    /// hides measures them as it measures the ones it shows.
    pub fn eased(&self, process: &Process) -> (f32, u64) {
        self.resources
            .get(&process.id)
            .map_or((process.cpu, process.memory), |&(cpu, memory)| {
                (cpu, memory as u64)
            })
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

/// What persists between frames and belongs to the frame, not to any one view: the smoothing of
/// the sampled processes, their births and deaths, the links and highlights, the outputs of the
/// last frame the app reads back, the spare buffers, and the module of each view in its row's
/// slot. A view keeps its own state in its module and reaches the scene only through `Stage`.
/// Every field says in its doc comment why it is frame-wide (the lint below insists), and
/// `scene_has_no_view_specific_fields` fails to compile until a new field is listed in it, which
/// is the review: add one only if every view and the app share it.
#[deny(clippy::missing_docs_in_private_items)]
pub struct Scene {
    /// Each process's CPU and memory as eased towards the samples. Frame-wide: the easing
    /// belongs to the sample, so every view reads the same figures (`Stage::resources`) and a
    /// view that is opened later finds them already settled.
    resources: HashMap<Identity, (f32, f32)>,
    /// Frames rendered, whichever view was shown: `Stage::frames`.
    frames: u64,
    /// The time of the previous frame, to tell a rewind or a gap from a step and drop `resources`.
    last_time: Option<f32>,
    /// The step every `Stage` gets as `dt`.
    clock: FrameClock,
    /// When each process was first seen, for the growth every view eases new bodies in with.
    births: HashMap<Identity, f32>,
    /// When each process that arrived after the first frame was first seen: `Stage::age`.
    arrivals: HashMap<Identity, f32>,
    /// Where the previous frame drew each process, so the exit flash and the splash of the media
    /// know where a process that left stood, whichever view is shown.
    previous: HashMap<Identity, Point>,
    /// The exit flashes still fading: the point and the time of a death. Frame-wide with
    /// `previous`, drawn by the scene over any view whose row has the flash.
    deaths: Vec<(Point, f32)>,
    /// The last frame's `Stage::headroom`, so fitting leaves it room above the ground.
    headroom: f32,
    /// Where processes left since the last frame, as disturbances for the media of any view
    /// (`Stage::splashes`).
    splashes: Vec<Point>,
    /// The processes that arrived since the last frame (`Stage::newborn`).
    newborn: Vec<Identity>,
    /// Whether a frame has been drawn: processes present at the first one did not start while
    /// anyone watched.
    started: bool,
    /// Animate the processes that are already running when the first frame is drawn.
    pub intro: bool,
    /// Which socket links to draw: the `c` key, global to every view.
    pub links: Links,
    /// The processes whose links stand out: selected, hovered or toured, set by the app for
    /// whichever view is shown.
    pub highlight: Vec<Identity>,
    /// The last frame's `Stage::positions`: where each drawn process is, which picking, labels,
    /// the tour, the links and the exit flash all read.
    pub positions: HashMap<Identity, Point>,
    /// The last frame's `Stage::places`.
    pub places: Vec<(Point, String)>,
    /// The last frame's `Stage::titles`.
    pub titles: Vec<Title>,
    /// The last frame's `Stage::tags`.
    pub tags: Vec<Tag>,
    /// The last frame's `Stage::bounds`, which `fit` frames the camera to.
    bounds: Vec<Point>,
    /// The rotation and pitch of the camera the last frame was drawn with, when the shown row is
    /// a `billboard`: `fit` measures `bounds` under them.
    billboard: Option<(f32, f32)>,
    /// The last frame's `Stage::notes`: popup lines after the sockets.
    pub notes: HashMap<Identity, Vec<String>>,
    /// The last frame's `Stage::sparklines`.
    pub sparklines: HashMap<Identity, Vec<String>>,
    /// The last frame's `Stage::tour`: the stops the tour and the Space visits make.
    pub tour: Vec<TourTarget>,
    /// The last frame's `Stage::popup`: lines the inspector popup carries for the shown view.
    pub popup: Vec<(Identity, String)>,
    /// The last frame's `Stage::callout`: lines of the tour's callout.
    pub callout: Vec<(Identity, String)>,
    /// Every process's CPU and memory over the last samples, recorded whichever view is shown.
    pub(crate) history: History,
    /// The views this scene draws, one row each: `REGISTRY`, unless a test brings its own.
    rows: &'static [ViewInfo],
    /// The module of each view, in its row's slot, made by the row's `build` when the scene is
    /// created. Every module is built up front, not on first show, because `record` feeds every
    /// sample to every view and a view built late would miss the samples before it opened.
    modules: Vec<Box<dyn ViewModule>>,
    /// Allocations recycled from the previous frame.
    pub spare: Buffers,
    /// How many processes the shown view drew in the last frame: the status line's count.
    pub visible: usize,
    /// How many of the processes admitted to the last frame the shown view left out.
    pub collapsed: usize,
}

impl Scene {
    pub fn new() -> Self {
        Self::with_rows(REGISTRY)
    }

    /// A scene that draws the views of `rows`, building each module view's state.
    pub(crate) fn with_rows(rows: &'static [ViewInfo]) -> Self {
        Self {
            resources: HashMap::new(),
            frames: 0,
            last_time: None,
            clock: FrameClock::default(),
            births: HashMap::new(),
            arrivals: HashMap::new(),
            previous: HashMap::new(),
            deaths: Vec::new(),
            headroom: 0.0,
            splashes: Vec::new(),
            newborn: Vec::new(),
            started: false,
            intro: false,
            links: Links::All,
            highlight: Vec::new(),
            positions: HashMap::new(),
            places: Vec::new(),
            titles: Vec::new(),
            tags: Vec::new(),
            bounds: Vec::new(),
            billboard: None,
            notes: HashMap::new(),
            sparklines: HashMap::new(),
            tour: Vec::new(),
            popup: Vec::new(),
            callout: Vec::new(),
            history: History::default(),
            rows,
            modules: rows.iter().map(|row| (row.build)()).collect(),
            spare: Buffers::default(),
            visible: 0,
            collapsed: 0,
        }
    }

    /// The row of `view` in the table this scene draws from.
    pub fn row(&self, view: View) -> &'static ViewInfo {
        &self.rows[view.index()]
    }

    /// The third status line for `view` while no process is inspected.
    pub fn legend(&self, view: View, snapshot: &Snapshot) -> String {
        self.modules[view.index()].legend(snapshot)
    }

    /// The line `view` shows in place of the inspector, if it draws no processes to inspect.
    pub fn inspector_line(&self, view: View) -> Option<String> {
        self.modules[view.index()].inspector_line()
    }

    /// The inspector card of `thing`, which `view`, the shown view, drew: built now from the
    /// module's own state, or None once it no longer shows the thing.
    pub fn card(&self, view: View, thing: Thing, snapshot: &Snapshot) -> Option<Card> {
        self.modules[view.index()].card(thing, snapshot)
    }

    /// Hands every module the command-line settings, once, before the first frame.
    pub fn configure(&mut self, options: &ViewOptions) {
        for module in &mut self.modules {
            module.configure(options);
        }
    }

    /// Offers a key to `view`, the shown view; see `ViewKey` for what the result means.
    pub fn key(&mut self, view: View, key: ViewKey) -> bool {
        self.modules[view.index()].key(key)
    }

    /// The state of the module view `view` draws, if it is a `T` and lets its tests see it
    /// (`ViewModule::state`).
    #[cfg(test)]
    pub fn module<T: 'static>(&self, view: View) -> Option<&T> {
        self.modules[view.index()].state()?.downcast_ref()
    }

    /// Where the view harness looks for `view`'s processes after a frame: the module's own
    /// anchors. None checks `positions`.
    #[cfg(test)]
    pub fn anchors(&self, view: View) -> Option<HashMap<Identity, Point>> {
        self.modules[view.index()].anchors()
    }

    /// Notes a splash at `at` as an exit would, for the next frame, for a test that stirs the
    /// water.
    #[cfg(test)]
    pub(crate) fn splash(&mut self, at: Point) {
        self.splashes.push(at);
    }

    /// The easing fraction `Stage::growth` hands a view, for the tests of the births.
    #[cfg(test)]
    fn growth(&self, id: Identity, time: f32) -> f32 {
        growth(&self.births, id, time)
    }

    /// Whether the last view drawn gave explicit bounds; such views grow as data arrives.
    pub fn bounded(&self) -> bool {
        !self.bounds.is_empty()
    }

    /// Feeds a new sample to every view's record hook in registry order, whichever view is
    /// shown, so views that keep their own history have it when they open.
    pub fn record(&mut self, snapshot: &Snapshot, time: f32) {
        self.history.record(snapshot, time);
        for module in &mut self.modules {
            module.record(snapshot, time);
        }
    }

    pub fn fit(&self, camera: &mut Camera, width: u32, height: u32) {
        if self.positions.is_empty() && self.titles.is_empty() && self.bounds.is_empty() {
            return;
        }
        let probe = Camera {
            center: [0.0, 0.0],
            zoom: 1.0,
            ..camera.clone()
        };
        // A billboard's bounds hold for the camera that drew them, which may not yet have turned
        // to the goal; its extent on the screen is the same under every rotation and pitch.
        let measure = match self.billboard {
            Some((rotation, pitch)) if !self.bounds.is_empty() => Camera {
                rotation,
                pitch,
                ..probe.clone()
            },
            _ => probe.clone(),
        };
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        let points: Vec<Point> = if self.bounds.is_empty() {
            // The drawn processes, and the systems' centres a hub stands at, which are no process.
            let centres = self.titles.iter().map(|title| title.at);
            self.positions
                .values()
                .copied()
                .chain(centres)
                .map(|p| [p[0], p[1], 0.0])
                .collect()
        } else {
            self.bounds.clone()
        };
        for &p in &points {
            let q = measure.view(p);
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
        // The vertical margin leaves room for the tallest thing above the ground plane.
        let headroom = 24.0_f32.max(self.headroom * camera.pitch.cos() * SCALE + 8.0);
        camera.zoom = (width as f32 / (max[0] - min[0] + 12.0))
            .min(height as f32 / (max[1] - min[1] + headroom))
            * 0.8;
    }

    /// Records a frame; call `Frame::rasterize` (or a GPU backend) to turn it into pixels.
    pub fn render(&mut self, input: &RenderInput) -> Frame {
        let RenderInput {
            snapshot,
            view,
            camera,
            width,
            height,
            selected,
            thing: _,
            time,
            limit,
            focus,
            paused,
            toured: _,
        } = *input;
        self.frames += 1;
        // The sample is already in when the main loop recorded it; headless renders and tests
        // that never call `record` fill the history here.
        self.history.record(snapshot, time);
        let dt = self.clock.step(time, paused);
        let alive: HashSet<_> = snapshot.processes.iter().map(|p| p.id).collect();
        let visual = self.ease(snapshot, &alive, time);
        let processes = admitted(&visual, snapshot, selected, limit, focus);
        let row = self.row(view);
        let [cpu, memory, io] = snapshot.pressure;
        let (c, m, i) = if row.haze {
            (bounded(cpu, 20.0), bounded(memory, 10.0), bounded(io, 20.0))
        } else {
            (0.0, 0.0, 0.0)
        };
        let sky = Sky {
            backdrop: row.backdrop,
            haze: [
                c * 110.0 + m * 140.0 + i * 30.0,
                c * 70.0 + m * 20.0 + i * 50.0,
                c * 10.0 + m * 60.0 + i * 150.0,
            ],
            time,
        };
        let mut frame = Frame::new(width, height, sky, std::mem::take(&mut self.spare));
        frame.identities = processes.iter().map(|p| p.id).collect();
        frame.things.clear();
        debug_assert!(
            frame.identities.len() <= THING as usize,
            "process indices stay below the thing bit"
        );
        self.visible = processes.len();
        self.collapsed = 0;
        self.places.clear();
        self.titles.clear();
        self.tags.clear();
        self.bounds.clear();
        self.billboard = row.billboard.then_some((camera.rotation, camera.pitch));
        self.notes.clear();
        self.sparklines.clear();
        self.tour.clear();
        self.popup.clear();
        self.callout.clear();
        // A view that stands things on the ground reports their height again every frame it is
        // shown.
        self.headroom = 0.0;
        self.draw_view(&mut frame, input, &processes, dt);
        let lift = row.link_lift;
        self.splashes.clear();
        self.newborn.clear();
        match row.links {
            LinkRule::All => self.draw_links(&mut frame, camera, snapshot, time, lift, false, row),
            LinkRule::Quiet => self.draw_links(&mut frame, camera, snapshot, time, lift, true, row),
            LinkRule::DrawnByView => {}
        }
        self.flash_exits(&mut frame, camera, &alive, time, row.flash);
        frame.anchor = selected
            .and_then(|id| self.positions.get(&id))
            .and_then(|&p| frame.locate(camera, p));
        frame
    }

    /// Forgets the arrivals of processes that are gone, except the latest `ARRIVALS_KEPT`: a
    /// rewind to before a process started also finds it absent, and jumping forward again must
    /// give it back the age it had, not a new one.
    fn keep_arrivals(&mut self, alive: &HashSet<Identity>) {
        let absent =
            self.arrivals.len() - self.arrivals.keys().filter(|id| alive.contains(id)).count();
        let Some(excess) = absent
            .checked_sub(ARRIVALS_KEPT)
            .filter(|&excess| excess > 0)
        else {
            return;
        };
        let mut gone: Vec<(f32, Identity)> = self
            .arrivals
            .iter()
            .filter(|(id, _)| !alive.contains(id))
            .map(|(&id, &arrived)| (arrived, id))
            .collect();
        gone.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, id) in gone.into_iter().take(excess) {
            self.arrivals.remove(&id);
        }
    }

    /// Takes in the frame's sample: notes births, and returns its processes with CPU and memory
    /// eased towards the sampled values.
    fn ease(&mut self, snapshot: &Snapshot, alive: &HashSet<Identity>, time: f32) -> Vec<Process> {
        let dt = self.last_time.map_or(0.0, |previous| time - previous);
        if !(0.0..=2.0).contains(&dt) {
            self.resources.clear();
        }
        self.last_time = Some(time);
        let alpha = smoothing(dt.max(0.0), 0.3);
        self.resources.retain(|id, _| alive.contains(id));
        self.births.retain(|id, _| alive.contains(id));
        self.keep_arrivals(alive);
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
                    self.arrivals.entry(p.id).or_insert(time);
                }
            }
        }
        snapshot
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
            .collect()
    }

    /// Draws the shown view.
    fn draw_view(
        &mut self,
        frame: &mut Frame,
        input: &RenderInput,
        processes: &[&Process],
        dt: f32,
    ) {
        let RenderInput {
            snapshot,
            view,
            camera,
            selected,
            thing,
            time,
            limit,
            focus,
            toured,
            ..
        } = *input;
        self.positions.clear();
        let mut stage = Stage {
            frame,
            camera,
            selected,
            thing,
            toured,
            focus,
            links: self.links,
            highlight: &self.highlight,
            limit,
            time,
            frames: self.frames,
            dt,
            history: &self.history,
            births: &self.births,
            arrivals: &self.arrivals,
            resources: &self.resources,
            splashes: &self.splashes,
            newborn: &self.newborn,
            positions: &mut self.positions,
            places: &mut self.places,
            bounds: &mut self.bounds,
            notes: &mut self.notes,
            sparklines: &mut self.sparklines,
            tour: &mut self.tour,
            popup: &mut self.popup,
            callout: &mut self.callout,
            titles: &mut self.titles,
            tags: &mut self.tags,
            headroom: &mut self.headroom,
        };
        let shown = self.modules[view.index()].draw(&mut stage, processes, snapshot);
        self.visible = shown;
        self.collapsed = processes.len().saturating_sub(shown);
    }

    /// Marks where processes that left were last drawn and, where the row has the flash, draws
    /// an expanding ring and glow there for `FADE_SECONDS`.
    fn flash_exits(
        &mut self,
        frame: &mut Frame,
        camera: &Camera,
        alive: &HashSet<Identity>,
        time: f32,
        flash: bool,
    ) {
        for (id, &point) in &self.previous {
            if !alive.contains(id) {
                self.deaths.push((point, time));
                self.splashes.push(point);
            }
        }
        self.previous = self.positions.clone();
        self.deaths
            .retain(|&(_, died)| (0.0..FADE_SECONDS).contains(&(time - died)));
        let flashes: &[(Point, f32)] = if flash { &self.deaths } else { &[] };
        for &(point, died) in flashes {
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
    }

    /// Socket links as arcs lifted above the plane with travelling pulses, and connections that
    /// leave the machine as beams rising off the body. Links touching highlighted processes glow.
    #[expect(
        clippy::too_many_arguments,
        reason = "the frame's link inputs and the row"
    )]
    fn draw_links(
        &self,
        frame: &mut Frame,
        camera: &Camera,
        snapshot: &Snapshot,
        time: f32,
        lift: f32,
        quiet: bool,
        row: &ViewInfo,
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
                if row.link_strokes {
                    frame.hairline(camera, last, next, tint(color, 0.35 + 0.65 * weight), 1.1);
                } else {
                    frame.beam(camera, last, next, color, weight);
                }
                last = next;
            }
            let phase = (a.pid.wrapping_mul(31) ^ b.pid.wrapping_mul(17)) % 100;
            let t = (time * 0.35 + phase as f32 / 100.0).fract();
            let strength = if glowing { 0.9 } else { 0.35 };
            frame.glow(camera, curve(t), 5.0, color, strength);
        }
        // Overlapping beams blend in drawing order, and a `HashMap` iterates in a different order
        // every run.
        let mut outside: Vec<_> = snapshot.outside.iter().collect();
        outside.sort_unstable_by_key(|&(id, _)| *id);
        for (id, &count) in outside {
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
                let weight = strength * (1.0 - step as f32 / 10.0);
                if row.link_strokes {
                    frame.hairline(camera, a, b, tint(OUTSIDE, 0.35 + 0.65 * weight), 1.1);
                } else {
                    frame.beam(camera, a, b, OUTSIDE, weight);
                }
            }
        }
    }
}

/// Media survive small layout changes; a different footprint starts a fresh simulation.
pub(crate) fn similar(a: &Grid, b: &Grid) -> bool {
    a.columns == b.columns
        && a.rows == b.rows
        && (a.cell / b.cell - 1.0).abs() < 0.05
        && (a.origin[0] - b.origin[0]).abs() < a.cell * 2.0
        && (a.origin[1] - b.origin[1]).abs() < a.cell * 2.0
}

/// The ground-plane grid covering points with a margin and about `cells` cells along the longer
/// side, or None for no points: the water of Ripple and the sheet of Flow.
pub(crate) fn cover(located: &[[f32; 2]], cells: usize) -> Option<Grid> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for p in located {
        for k in 0..2 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    if located.is_empty() {
        return None;
    }
    let margin = 8.0 + 0.08 * (max[0] - min[0]).max(max[1] - min[1]);
    Some(Grid::covering(
        [min[0] - margin, min[1] - margin],
        [max[0] + margin, max[1] + margin],
        cells,
    ))
}

/// Warm CPU glow and green NVIDIA glow around a body, as in the orbit view.
pub(crate) fn glows(frame: &mut Frame, camera: &Camera, process: &Process, at: Point, radius: f32) {
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
    if let Some(gpu) = process.gpu_memory {
        let mib = gpu.bytes() as f32 / 1048576.0;
        frame.glow(
            camera,
            at,
            core * (2.6 + 0.5 * (mib / 64.0).ln_1p()),
            NVIDIA,
            0.6,
        );
    }
}

/// Body radius from memory in bytes: volume proportional to memory, so a 1 GiB process is about
/// five times as wide as a 6 MiB one and idle kernel threads stay specks.
pub(crate) fn mass_radius(bytes: f32) -> f32 {
    (0.24 * (bytes / 1048576.0).max(0.0).cbrt()).clamp(0.25, 4.5)
}

/// The processes a frame draws, in PID order: the focused subtree, or every process, cut to
/// `limit` with the selection kept.
fn admitted<'a>(
    visual: &'a [Process],
    snapshot: &Snapshot,
    selected: Option<Identity>,
    limit: usize,
    focus: Option<Identity>,
) -> Vec<&'a Process> {
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
    processes
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

/// Shared checks that a view keeps its layout while focus or the process limit hides processes.
#[cfg(test)]
pub(crate) mod hiding {
    use super::*;

    /// Records one small frame of `view` and returns where each drawn process stands.
    pub fn place(
        scene: &mut Scene,
        snapshot: &Snapshot,
        view: View,
        time: f32,
        limit: usize,
        selected: Option<Identity>,
        focus: Option<Identity>,
    ) -> HashMap<Identity, Point> {
        let frame = scene.render(&RenderInput {
            width: 160,
            height: 90,
            selected,
            limit,
            focus,
            ..RenderInput::small(snapshot, view, &Camera::default(), time)
        });
        scene.spare = frame.release();
        scene.positions.clone()
    }

    /// `view` after five seconds of frames, so whatever moves has left the spot it began at;
    /// returns the scene, the time of its last frame and where everyone stands. The families
    /// of sixteen of `model::demo` arrive one by one, the last first, so the layout has a
    /// history, as a live one does: placing everything again at once, largest first, would
    /// not land where it is.
    pub fn settled(view: View, sample: &Snapshot) -> (Scene, f32, HashMap<Identity, Point>) {
        let family = |process: &Process| process.id.pid.saturating_sub(1000) as usize / 16;
        let families = sample
            .processes
            .iter()
            .map(family)
            .max()
            .map_or(0, |f| f + 1);
        let mut scene = Scene::new();
        let mut time = 30.0;
        let mut places = HashMap::new();
        for step in 0..50 {
            let arrived = (1 + step * families / 45).min(families);
            let mut partial = sample.clone();
            partial
                .processes
                .retain(|process| family(process) >= families - arrived);
            // Each frame is a sample of its own, ending at the sample itself.
            partial.elapsed = sample.elapsed - (49 - step) as f64 * 0.1;
            places = place(&mut scene, &partial, view, time, 4096, None, None);
            time += 0.1;
        }
        (scene, time - 0.1, places)
    }

    /// Asserts that every process drawn now stands where it stood in `before`.
    pub fn in_place(now: &HashMap<Identity, Point>, before: &HashMap<Identity, Point>) {
        for (id, point) in now {
            assert_eq!(before.get(id), Some(point), "{id:?}");
        }
    }

    /// Focuses one family of sixteen of `model::demo` and clears the focus again, repeating the
    /// last frame's time so nothing steps and anything that moves was moved by the change of
    /// focus. The family is the first with a drawn member after init's, which holds everyone.
    pub fn focus_and_clear(view: View, sample: &Snapshot) {
        focus_and_clear_by(view, sample, |point| point);
    }

    /// As `focus_and_clear`, comparing places by `by`: the part of a position that is layout,
    /// for views whose medium also lifts what floats on it.
    pub fn focus_and_clear_by(view: View, sample: &Snapshot, by: fn(Point) -> Point) {
        let measure = |places: HashMap<Identity, Point>| -> HashMap<Identity, Point> {
            places
                .into_iter()
                .map(|(id, point)| (id, by(point)))
                .collect()
        };
        let (mut scene, time, before) = settled(view, sample);
        let before = measure(before);
        let first = before
            .keys()
            .filter(|id| id.pid >= 1016)
            .min()
            .expect("a process outside init's family is drawn");
        let root = sample.processes[(first.pid as usize - 1000) / 16 * 16].id;
        let focused = measure(place(
            &mut scene,
            sample,
            view,
            time,
            4096,
            None,
            Some(root),
        ));
        assert!(!focused.is_empty() && focused.len() < before.len());
        in_place(&focused, &before);
        let cleared = measure(place(&mut scene, sample, view, time, 4096, None, None));
        assert_eq!(cleared, before);
    }

    /// Cuts the population in half with the limit, swaps the last process drawn before it
    /// in across it as a selection, and lifts the limit again, at one time as above.
    pub fn limit_and_lift(view: View, sample: &Snapshot) {
        limit_and_lift_by(view, sample, |point| point);
    }

    /// As `limit_and_lift`, comparing places by `by` as `focus_and_clear_by` does.
    pub fn limit_and_lift_by(view: View, sample: &Snapshot, by: fn(Point) -> Point) {
        let measure = |places: HashMap<Identity, Point>| -> HashMap<Identity, Point> {
            places
                .into_iter()
                .map(|(id, point)| (id, by(point)))
                .collect()
        };
        let (mut scene, time, before) = settled(view, sample);
        let before = measure(before);
        let half = sample.processes.len() / 2;
        let limited = measure(place(&mut scene, sample, view, time, half, None, None));
        assert!(!limited.is_empty() && limited.len() < before.len());
        in_place(&limited, &before);
        let selected = *before.keys().max().expect("something is drawn");
        assert!(!limited.contains_key(&selected));
        let swapped = measure(place(
            &mut scene,
            sample,
            view,
            time,
            half,
            Some(selected),
            None,
        ));
        assert!(swapped.contains_key(&selected));
        in_place(&swapped, &before);
        let lifted = measure(place(&mut scene, sample, view, time, 4096, None, None));
        assert_eq!(lifted, before);
    }

    /// Asserts that the envelope `view` gives the camera fit does not depend on what is drawn:
    /// hiding all but one family, or half the population, leaves it as the whole population
    /// had it, at one time as above. Only for views whose envelope is fixed by their layout.
    pub fn bounds_ignore_hiding(view: View, sample: &Snapshot) {
        let (mut scene, time, before) = settled(view, sample);
        let whole = scene.bounds.clone();
        assert!(!whole.is_empty(), "the view gives explicit bounds");
        let root = sample.processes[16].id;
        assert!(before.contains_key(&root));
        place(&mut scene, sample, view, time, 4096, None, Some(root));
        assert_eq!(scene.bounds, whole, "focused");
        place(
            &mut scene,
            sample,
            view,
            time,
            sample.processes.len() / 2,
            None,
            None,
        );
        assert_eq!(scene.bounds, whole, "limited");
    }

    /// Cuts the population in half with the limit and lifts it again, comparing places by
    /// `by`. A selection swapped in across the limit without its parent is drawn as a system
    /// of its own, so it has no place to keep and is left out.
    pub fn limit_and_lift_without_selection(view: View, by: fn(Point) -> Point) {
        let sample = crate::model::demo(30.0, 160);
        let measure = |places: HashMap<Identity, Point>| -> HashMap<Identity, Point> {
            places
                .into_iter()
                .map(|(id, point)| (id, by(point)))
                .collect()
        };
        let (mut scene, time, before) = settled(view, &sample);
        let before = measure(before);
        let limited = measure(place(&mut scene, &sample, view, time, 80, None, None));
        assert!(!limited.is_empty() && limited.len() < before.len());
        in_place(&limited, &before);
        let lifted = measure(place(&mut scene, &sample, view, time, 4096, None, None));
        assert_eq!(lifted, before);
    }

    /// A position on the ground plane, without what a medium lifts it by.
    pub fn ground([x, y, _]: Point) -> Point {
        [x, y, 0.0]
    }
}

#[cfg(test)]
mod tests {
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
    fn approach_turns_the_globe_the_short_way_leans_it_linearly_and_follows_the_hold() {
        let mut camera = Camera {
            globe_turn: 0.1,
            globe_tilt: 1.0,
            ..Camera::default()
        };
        let goal = Camera {
            globe_turn: TAU - 0.1,
            globe_tilt: 0.0,
            globe_held: true,
            ..Camera::default()
        };
        camera.approach(&goal, 0.5);
        assert!(camera.globe_turn.abs() < 1e-5, "{}", camera.globe_turn);
        assert!((camera.globe_tilt - 0.5).abs() < 1e-6);
        assert!(camera.globe_held);
        camera.approach(&Camera::default(), 1.0);
        assert!(!camera.globe_held && camera.globe_tilt == 0.0);
    }

    #[test]
    fn holding_and_releasing_change_nothing_unless_the_state_changes() {
        let mut camera = Camera::default();
        camera.release(40.0);
        assert!(
            camera == Camera::default(),
            "releasing a free earth is a no-op"
        );
        camera.hold(40.0);
        let held = camera.clone();
        assert!(held.globe_held);
        camera.hold(90.0);
        assert!(camera == held, "holding again must not fold the turn twice");
        camera.release(40.0);
        assert!(
            (camera.globe_turn.rem_euclid(TAU)).abs() < 1e-4
                || (camera.globe_turn - TAU).abs() < 1e-4
        );
        assert!(!camera.globe_held);
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
                ..Camera::default()
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

    fn sky_of(backdrop: Backdrop) -> Sky {
        Sky {
            backdrop,
            haze: [0.0; 3],
            time: 0.0,
        }
    }

    #[test]
    fn twilight_does_not_change_existing_backdrops() {
        let discriminants = [
            Backdrop::Dusk,
            Backdrop::Space,
            Backdrop::Sea,
            Backdrop::Void,
            Backdrop::Twilight,
        ]
        .map(|backdrop| backdrop as u32);
        assert_eq!(discriminants, [0, 1, 2, 3, 4], "new backdrops come last");
        for y in [0.0_f32, 0.25, 0.5, 0.9, 0.999] {
            assert_eq!(
                sky_of(Backdrop::Dusk).base(y),
                [8.0 + y * 6.0, 13.0 + y * 7.0, 25.0 + y * 8.0]
            );
            assert_eq!(sky_of(Backdrop::Space).base(y), [5.0, 8.0, 18.0]);
            assert_eq!(
                sky_of(Backdrop::Sea).base(y),
                [24.0 - y * 18.0, 72.0 - y * 50.0, 96.0 - y * 58.0]
            );
            assert_eq!(sky_of(Backdrop::Void).base(y), [0.0; 3]);
        }
        // The rows a rasterized frame paints are the same formulas, truncated to bytes. Space is
        // left out: its stars are drawn over it.
        for (backdrop, first, last) in [
            (Backdrop::Dusk, [8, 13, 25], [13, 19, 32]),
            (Backdrop::Sea, [24, 72, 96], [6, 22, 38]),
            (Backdrop::Void, [0; 3], [0; 3]),
        ] {
            let mut frame = Frame::new(4, 100, sky_of(backdrop), Buffers::default());
            frame.rasterize();
            let row = |y: usize| frame.pixels[y * 12..y * 12 + 3].to_vec();
            assert_eq!(row(0), first, "{backdrop:?} top");
            assert_eq!(row(99), last, "{backdrop:?} bottom");
        }
    }

    #[test]
    fn twilight_runs_three_stops_down_the_frame() {
        let mut frame = Frame::new(4, 100, sky_of(Backdrop::Twilight), Buffers::default());
        frame.rasterize();
        let row = |y: usize| -> [u8; 3] { frame.pixels[y * 12..y * 12 + 3].try_into().unwrap() };
        assert_eq!(row(0), [28, 34, 70], "the top stop");
        assert_eq!(
            row(60),
            [120, 92, 138],
            "the middle stop, 60% of the way down"
        );
        let bottom = row(99);
        assert!(
            bottom[0] > 230 && (148..=150).contains(&bottom[1]) && (104..=108).contains(&bottom[2]),
            "just short of the horizon stop at the last row: {bottom:?}"
        );
        for y in 1..100 {
            let (above, here) = (row(y - 1), row(y));
            assert!(
                here[0] >= above[0],
                "red only rises down the frame at row {y}"
            );
            if y <= 60 {
                assert!(here[2] >= above[2], "blue rises towards the middle stop");
            } else {
                assert!(here[2] <= above[2], "blue falls towards the horizon stop");
            }
        }
        // The pressure haze adds on top of the gradient, as it does on every backdrop.
        let mut hazy = sky_of(Backdrop::Twilight);
        hazy.haze = [60.0, 0.0, 0.0];
        let mut tinted = Frame::new(4, 100, hazy, Buffers::default());
        tinted.rasterize();
        assert!(
            tinted
                .pixels
                .iter()
                .step_by(3)
                .map(|&r| u64::from(r))
                .sum::<u64>()
                > frame
                    .pixels
                    .iter()
                    .step_by(3)
                    .map(|&r| u64::from(r))
                    .sum::<u64>()
        );
    }

    #[test]
    fn stacked_puffs_round_to_the_nearest_level_where_glows_truncate() {
        // Over black, a centred half-strength disc of 255 blends to 127.5, then 191.5, then 223.5.
        let mut puffs = Frame::new(4, 4, sky_of(Backdrop::Void), Buffers::default());
        let mut glows = Frame::new(4, 4, sky_of(Backdrop::Void), Buffers::default());
        puffs.pixels = vec![0; 48];
        glows.pixels = vec![0; 48];
        let (mut rounded, mut truncated) = ([0u8; 3], [0u8; 3]);
        for k in 0..3 {
            puffs.puff([1.5, 1.5], 2.0, [255; 3], 0.5);
            glows.halo([1.5, 1.5], 2.0, [255; 3], 0.5);
            rounded[k] = puffs.pixels[(4 + 1) * 3];
            truncated[k] = glows.pixels[(4 + 1) * 3];
        }
        assert_eq!(
            rounded,
            [128, 192, 224],
            "a puff rounds to the nearest level"
        );
        assert_eq!(
            truncated,
            [127, 191, 223],
            "a glow truncates, as it always has"
        );
        // A strength above one is a full blend, never an overshoot.
        let mut frame = Frame::new(4, 4, sky_of(Backdrop::Void), Buffers::default());
        frame.items = vec![Item::Puff {
            center: [1.5, 1.5],
            radius: 2.0,
            color: [90; 3],
            strength: 3.0,
        }];
        frame.rasterize();
        assert_eq!(frame.pixels[(4 + 1) * 3], 90);
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
    fn the_scene_hands_every_module_the_command_line_settings() {
        let snapshot = demo(10.0, 8);
        let (probe, quiet) = (View::at(0), View::at(1));
        let mut scene = Scene::with_rows(testing::ROWS);
        scene.configure(&ViewOptions { hr_diagram: true });
        for view in [probe, quiet] {
            assert_eq!(
                scene.legend(view, &snapshot),
                " probe: 0 draws | 0 records | keys [] | ViewOptions { hr_diagram: true }",
                "a module takes the settings whether or not it is the shown view"
            );
        }
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

    /// A scene over `testing::GUIDED`, and a function that draws its `view` at `time`, paused or
    /// not.
    fn guided() -> (Scene, impl Fn(&mut Scene, View, f32, bool)) {
        let snapshot = demo(10.0, 8);
        let draw = move |scene: &mut Scene, view: View, time: f32, paused: bool| {
            scene.render(&RenderInput {
                paused,
                ..RenderInput::small(&snapshot, view, &Camera::default(), time)
            });
        };
        (Scene::with_rows(testing::GUIDED), draw)
    }

    #[test]
    fn stage_dt_is_zero_on_the_first_frame_when_paused_and_after_rewind() {
        let (mut scene, draw) = guided();
        let guide = View::at(0);
        draw(&mut scene, guide, 10.0, false);
        draw(&mut scene, guide, 10.1, false);
        // Paused, the time is the shown sample's, which moves either way; the clock stands still.
        draw(&mut scene, guide, 10.4, true);
        draw(&mut scene, guide, 9.0, true);
        draw(&mut scene, guide, 10.2, false);
        // A rewind, or a headless replay, restarts the count from the earlier time.
        draw(&mut scene, guide, 8.0, false);
        draw(&mut scene, guide, 8.25, false);
        assert_eq!(
            scene.legend(guide, &Snapshot::default()),
            " guide: dt [0.00 0.10 0.00 0.00 0.10 0.00 0.25]"
        );
    }

    #[test]
    fn the_stage_counts_frames_and_hands_over_the_splashes_and_the_newborn() {
        let (mut scene, draw) = guided();
        let (guide, probe) = (View::at(0), View::at(1));
        draw(&mut scene, guide, 10.0, false);
        draw(&mut scene, probe, 10.1, false);
        draw(&mut scene, guide, 10.2, false);
        let sample = demo(10.0, 8);
        let mut fewer = sample.clone();
        let gone = fewer.processes.remove(3);
        let render = |scene: &mut Scene, snapshot: &Snapshot, time: f32| {
            scene.render(&RenderInput::small(
                snapshot,
                guide,
                &Camera::default(),
                time,
            ));
        };
        render(&mut scene, &fewer, 10.3);
        // The exit is noticed after the frame it left, so the next frame has its splash.
        render(&mut scene, &fewer, 10.4);
        let mut more = fewer.clone();
        let mut arrival = more.processes[0].clone();
        arrival.id.pid += 10_000;
        more.processes.push(arrival);
        render(&mut scene, &more, 10.5);
        let seen = scene
            .module::<testing::Guide>(guide)
            .expect("the guide's state");
        assert_eq!(seen.frames, [1, 3, 4, 5, 6], "the probe's frame is counted");
        assert_eq!(
            seen.disturbances,
            [(0, 0), (0, 0), (0, 0), (1, 0), (0, 1)],
            "{gone:?} splashed once and one process arrived"
        );
    }

    #[test]
    fn stage_dt_is_capped() {
        let (mut scene, draw) = guided();
        let guide = View::at(0);
        for time in [10.0, 13.0, 13.2, 100.0] {
            draw(&mut scene, guide, time, false);
        }
        assert_eq!(
            scene.legend(guide, &Snapshot::default()),
            format!(" guide: dt [0.00 {MAX_DT:.2} 0.20 {MAX_DT:.2}]")
        );
        assert!((MAX_DT - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn a_view_shown_again_steps_one_frame_not_the_time_it_was_hidden() {
        let (mut scene, draw) = guided();
        let (guide, probe) = (View::at(0), View::at(1));
        draw(&mut scene, guide, 20.0, false);
        for frame in 1..=40 {
            draw(&mut scene, probe, 20.0 + frame as f32 * 0.1, false);
        }
        draw(&mut scene, guide, 24.1, false);
        assert_eq!(
            scene.legend(guide, &Snapshot::default()),
            " guide: dt [0.00 0.10]"
        );
    }

    #[test]
    fn a_billboard_is_framed_as_the_camera_that_drew_it_saw_it() {
        let snapshot = demo(10.0, 16);
        // Drawn under the default camera, while the goal has already turned and tilted.
        let goal = Camera {
            rotation: FRAC_PI_2,
            pitch: FRAC_PI_2,
            ..Camera::default()
        };
        let zoom = |view: View| {
            let mut scene = Scene::with_rows(testing::PLANES);
            render(&mut scene, &snapshot, view, 10.0);
            let mut fitted = goal.clone();
            scene.fit(&mut fitted, 1200, 600);
            (fitted.zoom, fitted.center)
        };
        // 600 by 300 pixels fill 0.9 of a 1200 by 600 frame at zoom 1.8, whatever the pitch.
        let (zoom_billboard, center) = zoom(View::at(0));
        assert!((zoom_billboard - 1.8).abs() < 1e-3, "{zoom_billboard}");
        // Centred, as the goal's rotation and pitch see the point ORIGIN_Y down the frame.
        let lifted = Camera {
            zoom: 1.0,
            ..goal.clone()
        }
        .ground(0.0, (ORIGIN_Y - 0.5) * 600.0 / zoom_billboard);
        assert!(
            (center[0] - lifted[0]).abs() < 1e-3 && (center[1] - lifted[1]).abs() < 1e-3,
            "{center:?} against {lifted:?}"
        );
        // An ordinary view's bounds are world points, framed as the goal will see them.
        let (zoom_ordinary, _) = zoom(View::at(1));
        assert!(
            (zoom_ordinary - 1.8).abs() > 0.1,
            "the bounds of an ordinary view are not corrected: {zoom_ordinary}"
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
            tags: _,
            bounds: _,
            billboard: _,
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
    fn a_screen_space_hairline_draws_the_same_ribbon_as_a_world_one() {
        let camera = Camera::default();
        let (a, b) = ([0.0, 0.0, 0.0], [4.0, 1.0, 0.0]);
        let mut world = blank(100, 100);
        world.hairline(&camera, a, b, [200, 100, 50], 2.0);
        let mut screen = blank(100, 100);
        let (a, b) = (screen.project(&camera, a), screen.project(&camera, b));
        screen.hairline_between(a, b, [200, 100, 50], 2.0);
        assert!(!world.items.is_empty());
        assert_eq!(format!("{:?}", world.items), format!("{:?}", screen.items));
    }

    #[test]
    fn the_ground_offset_of_a_screen_offset_projects_back_to_it() {
        let camera = Camera {
            rotation: 0.7,
            pitch: 0.9,
            zoom: 3.0,
            ..Camera::default()
        };
        let frame = blank(200, 120);
        let origin = frame.project(&camera, [camera.center[0], camera.center[1], 0.0]);
        let [x, y] = camera.ground(40.0, -25.0);
        let back = frame.project(&camera, [camera.center[0] + x, camera.center[1] + y, 0.0]);
        assert!((back[0] - origin[0] - 40.0).abs() < 1e-3);
        assert!((back[1] - origin[1] + 25.0).abs() < 1e-3);
    }

    /// A sphere item with `pick` at pixel (x, y), at depth 0.
    fn dot_item(x: f32, y: f32, pick: u32) -> Item {
        Item::Sphere {
            center: [x, y, 0.0],
            radius: 1.0,
            depth: 1.0,
            color: [255; 3],
            pick,
            selected: false,
        }
    }

    #[test]
    fn things_and_processes_share_the_pick_buffer_without_colliding() {
        let mut frame = blank(30, 10);
        let process = Identity { pid: 1, start: 1 };
        frame.identities = vec![process];
        let lamp = Thing {
            kind: "irq",
            key: 9,
        };
        let grave = Thing {
            kind: "grave",
            key: 9,
        };
        let lamp_pick = frame.thing_pick(lamp);
        let grave_pick = frame.thing_pick(grave);
        assert_eq!((lamp_pick, grave_pick), (THING, THING | 1));
        assert!(![lamp_pick, grave_pick].contains(&NONE));
        frame.items = vec![
            dot_item(5.0, 5.0, 0),
            dot_item(15.0, 5.0, lamp_pick),
            dot_item(25.0, 5.0, grave_pick),
        ];
        frame.rasterize();
        let at = |x: usize| frame.target(frame.picks[5 * 30 + x]);
        assert_eq!(at(5), Some(Target::Process(process)));
        assert_eq!(
            at(15),
            Some(Target::Thing(lamp)),
            "thing 0 is not process 0"
        );
        assert_eq!(
            at(25),
            Some(Target::Thing(grave)),
            "the same key of another kind"
        );
        assert_eq!(at(10), None);
        assert_eq!(frame.target(THING | 2), None, "past the frame's things");
        assert_eq!(frame.target(1), None, "past the frame's identities");
    }

    #[test]
    fn pick_near_still_returns_only_processes() {
        let mut frame = blank(40, 20);
        let process = Identity { pid: 1, start: 1 };
        frame.identities = vec![process];
        let file = Thing {
            kind: "file",
            key: 3,
        };
        let thing = frame.thing_pick(file);
        frame.pixel(10, 10, 0.0, [255; 3], 0);
        frame.pixel(20, 10, 0.0, [255; 3], thing);
        assert_eq!(
            frame.pick_near(20.5, 10.5, 12.0),
            Some(process),
            "the nearer thing is passed over"
        );
        assert_eq!(frame.pick_near(20.5, 10.5, 3.0), None);
        assert_eq!(
            frame.target_near(20.5, 10.5, 3.0),
            Some(Target::Thing(file))
        );
    }

    #[test]
    fn target_near_finds_the_nearest_thing_or_process() {
        let mut frame = blank(40, 40);
        let process = Identity { pid: 1, start: 1 };
        frame.identities = vec![process];
        let grave = Thing {
            kind: "grave",
            key: 7,
        };
        let thing = frame.thing_pick(grave);
        frame.pixel(24, 20, 0.0, [255; 3], 0);
        frame.pixel(10, 20, 0.0, [255; 3], thing);
        assert_eq!(
            frame.target_near(20.5, 20.5, 9.0),
            Some(Target::Process(process))
        );
        assert_eq!(
            frame.target_near(14.5, 20.5, 9.0),
            Some(Target::Thing(grave))
        );
        assert_eq!(frame.target_near(20.5, 5.5, 9.0), None);
        frame.pixel(16, 20, 0.0, [255; 3], thing);
        assert_eq!(
            frame.target_near(20.5, 20.5, 9.0),
            Some(Target::Process(process)),
            "of two equally near, the later in row order wins, as in pick_near"
        );
    }

    #[test]
    fn a_pick_splat_writes_picks_but_no_pixels_or_depth() {
        let plain = blank(30, 30);
        let mut frame = blank(30, 30);
        let process = Identity { pid: 4, start: 4 };
        frame.identities = vec![process];
        let grave = Thing {
            kind: "grave",
            key: 1,
        };
        let thing = frame.thing_pick(grave);
        frame.items = vec![
            Item::Pick {
                center: [10.0, 15.0],
                radius: 5.0,
                depth: 1.0,
                pick: thing,
            },
            Item::Pick {
                center: [25.0, 15.0],
                radius: 2.0,
                depth: 1.0,
                pick: 0,
            },
        ];
        frame.rasterize();
        assert_eq!(frame.pixels, plain.pixels, "no colour");
        assert!(
            frame
                .depth()
                .iter()
                .all(|&depth| depth == f32::NEG_INFINITY),
            "no depth"
        );
        // The integer points within 5 of the centre: Gauss's circle count N(5).
        assert_eq!(
            frame.picks.iter().filter(|&&pick| pick == thing).count(),
            81
        );
        assert_eq!(frame.picks[15 * 30 + 16], NONE, "outside the disc");
        assert_eq!(
            frame.target_near(10.5, 15.5, 1.0),
            Some(Target::Thing(grave))
        );
        assert_eq!(
            frame.target_near(25.5, 15.5, 1.0),
            Some(Target::Process(process)),
            "a splat picks processes too"
        );
    }

    #[test]
    fn a_pick_splat_behind_a_triangle_is_not_picked() {
        let mut frame = blank(30, 30);
        let wall = Identity { pid: 1, start: 1 };
        frame.identities = vec![wall];
        let behind = frame.thing_pick(Thing {
            kind: "grave",
            key: 1,
        });
        let front = frame.thing_pick(Thing {
            kind: "grave",
            key: 2,
        });
        let splat = |center: [f32; 2], radius: f32, depth: f32, pick: u32| Item::Pick {
            center,
            radius,
            depth,
            pick,
        };
        // The triangle covers the pixels with x + y below 30, at depth 5.
        frame.items = vec![
            Item::Triangle(
                [[0.0, 0.0, 5.0], [30.0, 0.0, 5.0], [0.0, 30.0, 5.0]],
                [90; 3],
                0,
            ),
            splat([8.0, 8.0], 4.0, 2.0, behind),
            splat([8.0, 8.0], 2.0, 9.0, front),
            splat([24.0, 24.0], 3.0, 2.0, behind),
        ];
        frame.rasterize();
        let pick = |x: usize, y: usize| frame.picks[y * 30 + x];
        assert_eq!(
            pick(8, 11),
            0,
            "behind the triangle, the triangle keeps the pick"
        );
        assert_eq!(pick(8, 8), front, "in front of it, the splat takes it");
        assert_eq!(
            pick(24, 24),
            behind,
            "with nothing in front, the splat takes it"
        );
        assert_eq!(&frame.pixels[(8 * 30 + 8) * 3..][..3], &[90; 3]);
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
}
