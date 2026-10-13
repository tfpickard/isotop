//! What the plumbing view draws with: `Ink`, a thin handle on the frame and the camera, and the
//! colours of the boiler room. The things themselves (tanks, pipes, tubes, hoses, the room) are
//! in the modules below. Everything is triangles and the existing impostors, lit by the one
//! `shared::LIGHT` that `Frame::tube` bakes into its faces, so the CPU and the GPU draw it alike.

pub(super) mod pipes;
pub(super) mod scenery;
pub(super) mod tank;
pub(super) mod tubes;

use std::f32::consts::TAU;

use crate::palette::tint;
use crate::render::strokes::{Dash, Width, lit};
use crate::render::{Camera, Color, Frame, Item, NONE, Point};

/// Copper pipework.
pub const COPPER: Color = [232, 134, 78];
/// Brass: fittings, collars, rims, tubes.
pub const BRASS: Color = [236, 196, 100];
/// The iron of a tank.
pub const IRON: Color = [66, 108, 90];
/// The rubber of a hose.
pub const HOSE: Color = [74, 176, 82];
/// What a hose or a pipe is when its flow is not measured: neither green nor blue.
pub const DULL: Color = [118, 128, 124];
/// Glass.
pub const GLASS: Color = [196, 220, 228];
/// Water, and the bright core of a moving dash of it.
pub const WATER: Color = [52, 136, 206];
pub const WATER_HI: Color = [170, 228, 255];
/// Paint: the red of a valve wheel and of the gauge's red zone.
pub const RED: Color = [206, 52, 44];
/// A pneumatic capsule.
pub const CAPSULE: Color = [168, 84, 52];

/// The narrowest a band is drawn, in pixels.
const MIN_BAND: f32 = 1.5;

/// The world height of the pipes, the overhead Unix tubes and the hoses above the floor.
pub const PIPE_Z: f32 = 0.42;
pub const TUBE_Z: f32 = 3.5;
pub const HOSE_Z: f32 = 0.16;
/// The radius of a copper pipe, of an overhead tube and of a hose, in world units.
pub const PIPE_R: f32 = 0.17;
pub const TUBE_R: f32 = 0.2;
pub const HOSE_R: f32 = 0.15;

pub fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scale(a: Point, factor: f32) -> Point {
    a.map(|value| value * factor)
}

/// The frame and the camera, with what drawing against them needs.
pub struct Ink<'a> {
    pub frame: &'a mut Frame,
    pub camera: &'a Camera,
    /// The unit vector from the scene towards the viewer, and its horizontal part, which the
    /// gauge of a tank faces.
    pub toward: Point,
    pub facing: [f32; 2],
    /// Blended items (halos, mist), kept to be recorded after all geometry: a blended item
    /// covers whatever is already drawn, so it goes last.
    pub effects: Vec<Item>,
}

impl<'a> Ink<'a> {
    pub fn new(frame: &'a mut Frame, camera: &'a Camera) -> Self {
        let toward = camera.viewer();
        let flat = toward[0].hypot(toward[1]).max(1e-6);
        Ink {
            frame,
            camera,
            toward,
            facing: [toward[0] / flat, toward[1] / flat],
            effects: Vec::new(),
        }
    }

    /// Pixels in a world distance at the camera's zoom.
    pub fn pixels(&self, world: f32) -> f32 {
        world * self.camera.zoom * crate::render::SCALE
    }

    /// A flat-shaded prism of `sides` sides along `points`, `radii` at each point.
    pub fn prism(
        &mut self,
        points: &[Point],
        radii: &[f32],
        color: Color,
        pick: u32,
        sides: usize,
        caps: bool,
    ) {
        self.frame
            .tube(self.camera, points, radii, color, pick, sides, caps);
    }

    /// A round pipe of constant `radius` along `points`, closed at both ends.
    pub fn pipe(&mut self, points: &[Point], radius: f32, color: Color, pick: u32) {
        let radii = vec![radius; points.len()];
        self.prism(points, &radii, color, pick, 8, true);
    }

    /// A band along `points`, `width` world units wide but never under a pixel and a half, so it
    /// is always triangles (a thinner stroke would be one-pixel lines, which the two
    /// rasterizers draw differently), facing the screen.
    pub fn band(&mut self, points: &[Point], width: f32, color: Color, dash: Option<Dash>) {
        let width = if self.pixels(width) >= MIN_BAND {
            Width::World(width)
        } else {
            Width::Pixels(MIN_BAND)
        };
        self.frame
            .ribbon(self.camera, points, width, color, NONE, dash);
    }

    /// A ball of exactly `radius` world units, not pickable: a rivet, a drop.
    pub fn ball(&mut self, at: Point, radius: f32, color: Color) {
        if self.pixels(radius) >= 0.8 {
            self.frame.world_sphere(self.camera, at, radius, color);
        }
    }

    /// A ball that picks as `pick`, at least a few pixels wide: an elbow, a flange.
    pub fn knob(&mut self, at: Point, radius: f32, color: Color, pick: u32) {
        self.frame
            .sphere(self.camera, at, radius, color, pick, false);
    }

    /// A flat disc of `radius` in the plane through `center` spanned by the unit vectors `right`
    /// and `up`, as `steps` triangles of exactly `color` (the caller shades it).
    #[expect(
        clippy::too_many_arguments,
        reason = "a disc is a centre, a plane, a size, a look and a pick"
    )]
    pub fn disc(
        &mut self,
        center: Point,
        right: Point,
        up: Point,
        radius: f32,
        color: Color,
        pick: u32,
        steps: usize,
    ) {
        let rim = |k: usize| {
            let angle = TAU * k as f32 / steps as f32;
            add(
                center,
                add(
                    scale(right, radius * angle.cos()),
                    scale(up, radius * angle.sin()),
                ),
            )
        };
        for k in 0..steps {
            self.frame
                .facet(self.camera, [center, rim(k), rim(k + 1)], color, pick);
        }
    }

    /// An axis-aligned box, its top and four sides lit by `shared::LIGHT` as the faces of a tube
    /// are, all picking as `pick`.
    pub fn block(&mut self, low: Point, high: Point, color: Color, pick: u32) {
        let [x0, y0, z0] = low;
        let [x1, y1, z1] = high;
        let faces: [([Point; 4], Point); 5] = [
            (
                [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
                [0.0, 0.0, 1.0],
            ),
            (
                [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
                [0.0, -1.0, 0.0],
            ),
            (
                [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
                [0.0, 1.0, 0.0],
            ),
            (
                [[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]],
                [-1.0, 0.0, 0.0],
            ),
            (
                [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
                [1.0, 0.0, 0.0],
            ),
        ];
        for (corners, normal) in faces {
            self.frame
                .quad(self.camera, corners, lit(color, normal), pick);
        }
    }

    /// A quad of four world points, flat-coloured as `color` (already shaded by the caller).
    pub fn quad(&mut self, corners: [Point; 4], color: Color, pick: u32) {
        self.frame.quad(self.camera, corners, color, pick);
    }

    /// A soft halo at `at` of `radius` pixels, recorded with the other effects after the
    /// geometry. A `Puff` rounds as the GPU does, so any number of them agree on both
    /// rasterizers.
    pub fn halo(&mut self, at: Point, radius: f32, color: Color, strength: f32) {
        let p = self.frame.project(self.camera, at);
        self.effects.push(Item::Puff {
            center: [p[0], p[1]],
            radius,
            color,
            strength: strength.clamp(0.0, 1.0),
        });
    }

    /// Records the effects, last. Call once, after everything else.
    pub fn finish(&mut self) {
        let effects = std::mem::take(&mut self.effects);
        self.frame.items.extend(effects);
    }
}

/// `color` brightened for something the app highlights (selected, hovered or toured).
pub fn highlighted(color: Color, on: bool) -> Color {
    if on { tint(color, 1.28) } else { color }
}
