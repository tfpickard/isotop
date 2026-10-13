use crate::model::Identity;
use crate::simulation::hash;

use super::camera::Camera;
use super::{Color, NONE, Point, field, shared};

/// The pick-buffer bit that marks an index into `Frame::things` rather than `Frame::identities`.
/// Process indices stay below it, and a thing index below `THING - 1`, so neither meets `NONE`.
pub const THING: u32 = 1 << 31;
pub const BACKGROUND: f32 = -10000.0;

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
/// untrusted, like process text, and reaches the terminal only as `terminal::SafeText`.
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "first recorded by a view that draws a field")
    )]
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
    pub(super) depth: Vec<f32>,
    pub picks: Vec<u32>,
    pub identities: Vec<Identity>,
    /// The things this frame's thing picks index, in `thing_pick` order.
    pub things: Vec<Thing>,
    /// The fields this frame draws, in the order `Frame::field` recorded them: what an
    /// `Item::Field` indexes.
    pub fields: Vec<field::ShadedField>,
    /// The previous frame's fields, which `Frame::field` takes back when one is unchanged.
    pub(super) earlier_fields: Vec<field::ShadedField>,
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

/// A blank `width` by `height` dusk frame, rasterized, for tests.
#[cfg(test)]
pub(super) fn blank(width: u32, height: u32) -> Frame {
    let sky = Sky {
        backdrop: Backdrop::Dusk,
        haze: [0.0; 3],
        time: 0.0,
    };
    let mut frame = Frame::new(width, height, sky, Buffers::default());
    frame.rasterize();
    frame
}

/// A sky of `backdrop` with no haze, for tests.
#[cfg(test)]
pub(super) fn sky_of(backdrop: Backdrop) -> Sky {
    Sky {
        backdrop,
        haze: [0.0; 3],
        time: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
