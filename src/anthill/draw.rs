//! Drawing the anthill: mounds, piles, blankets, ants with their crumbs, and tunnels. Everything
//! is ordinary items (triangles, sphere impostors, puffs and pick discs), so the CPU and the GPU
//! draw it alike. Nothing here reads a process or a file: the view hands over positions, sizes and
//! colours, and what a thing's shape or colour jitter is made of carries no data.

use std::f32::consts::{PI, TAU};

use super::colony::Ant;
use super::flows::Way;
use crate::geometry::dot;
use crate::geometry::normalize;
use crate::palette::{mix, tint};
use crate::render::strokes::{Dash, Width};
use crate::render::{Camera, Color, Frame, Item, LIGHT, NONE, Point, SCALE};
use crate::simulation::{Rng, hash};

/// The soil of a mound before it is lit.
const SOIL: Color = [158, 118, 86];

/// The dark of an entrance, and the pebbles' pale stone.
const HOLE: Color = [14, 9, 7];
const STONE: Color = [226, 214, 190];

/// An ant is dark, and a crumb is pale when it was read, coral when it was written, and between
/// the two when the direction is not known.
const ANT: Color = [22, 15, 11];
pub const READ_CRUMB: Color = [255, 238, 196];
pub const WRITE_CRUMB: Color = [255, 118, 92];

/// The warm light round a pile.
const HALO: Color = [255, 236, 190];

/// How high above the ground the flat things lie, in world units, so that depth ties never decide.
const ANT_LIFT: f32 = 0.06;

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The colour of a crumb of a stream that went `way`.
pub fn crumb_color(way: Way) -> Color {
    match way {
        Way::Read => READ_CRUMB,
        Way::Write => WRITE_CRUMB,
        Way::Either => mix(READ_CRUMB, WRITE_CRUMB, 0.5),
    }
}

/// The radius, in pixels at a frame 1600 across with the slab fitted to it, of the crumb of an
/// ant that carries `bytes`: 0.55 + 0.27 log2(bytes / 64), clamped to 0.42 and 3.6.
pub fn crumb_px(bytes: f32) -> f32 {
    (0.55 + 0.27 * (bytes.max(1.0) / 64.0).log2()).clamp(0.42, 3.6)
}

/// A sphere impostor in world units at exactly its projected size, but never under a pixel: the
/// `Frame::sphere` floor of 3.5 pixels would swell every pebble and crumb.
pub fn ball(frame: &mut Frame, camera: &Camera, at: Point, radius: f32, color: Color, pick: u32) {
    let center = frame.project(camera, at);
    frame.items.push(Item::Sphere {
        center,
        radius: (radius * camera.zoom * SCALE).max(1.0),
        depth: radius,
        color,
        pick,
        selected: false,
    });
}

/// A soft round glow. A `Puff` rounds on both rasterizers, so many of them may stack.
fn puff(
    frame: &mut Frame,
    camera: &Camera,
    at: Point,
    radius_px: f32,
    color: Color,
    strength: f32,
) {
    let center = frame.project(camera, at);
    frame.items.push(Item::Puff {
        center: [center[0], center[1]],
        radius: radius_px,
        color,
        strength,
    });
}

/// A plane through `center`, spanned by the unit vectors `across` and `up`.
#[derive(Clone, Copy)]
struct Plane {
    center: Point,
    across: Point,
    up: Point,
}

/// A disc of `radius` lying in `plane`, as a fan of triangles.
fn disc(frame: &mut Frame, camera: &Camera, plane: Plane, radius: f32, color: Color, pick: u32) {
    const SIDES: usize = 8;
    let Plane { center, across, up } = plane;
    let rim = |k: usize| -> Point {
        let angle = k as f32 / SIDES as f32 * TAU;
        let (sin, cos) = angle.sin_cos();
        std::array::from_fn(|axis| center[axis] + radius * (cos * across[axis] + sin * up[axis]))
    };
    for k in 0..SIDES {
        frame.facet(camera, [center, rim(k), rim(k + 1)], color, pick);
    }
}

/// What a mound is drawn from.
pub struct Mound<'a> {
    pub at: [f32; 2],
    pub radius: f32,
    /// The process's index in the frame's identities: what a click on the mound picks.
    pub pick: u32,
    /// A hash of the process, which jitters the soil facet by facet and turns the mesh.
    pub seed: u64,
    pub highlight: bool,
    /// Its descriptors were read, so it has entrances and a crater that glows; a nest that was
    /// not read sleeps, dimmer, because nothing is known of where its ants would go.
    pub awake: bool,
    /// The colour of the nest's kind, which lights its entrances.
    pub glow: Color,
    /// The bearings of up to four entrances, one for each distinct destination, in radians.
    pub entrances: &'a [f32],
}

/// The height of a mound over its radius.
const SLOPE: f32 = 0.95;

/// A mound: a paraboloid of 24 segments, 12 where it is under 8 pixels across its radius, in
/// bands of flat-shaded facets, with a crater on top and an entrance on its slope for each of
/// its destinations that faces them.
pub fn mound(frame: &mut Frame, camera: &Camera, mound: &Mound) {
    let Mound {
        at,
        radius,
        pick,
        seed,
        highlight,
        awake,
        glow,
        entrances,
    } = *mound;
    let px = radius * camera.zoom * SCALE;
    let (segments, rings): (usize, &[f32]) = if px >= 8.0 {
        (24, &[1.0, 0.72, 0.46, 0.22])
    } else if px >= 3.0 {
        (12, &[1.0, 0.6, 0.28])
    } else {
        (12, &[1.0, 0.45])
    };
    let height = SLOPE * radius;
    let lift = |rho: f32| height * (1.0 - rho * rho);
    let turn = (seed & 0xffff) as f32 / 65536.0 * TAU;
    let vertex = |rho: f32, segment: usize| -> Point {
        let angle = turn + segment as f32 / segments as f32 * TAU;
        [
            at[0] + radius * rho * angle.cos(),
            at[1] + radius * rho * angle.sin(),
            lift(rho),
        ]
    };
    // The unit normal of the paraboloid at `rho` and `angle`: z = H (1 - rho^2) slopes by
    // 2 H rho / R against the distance from the middle.
    let normal = |rho: f32, angle: f32| -> Point {
        let tilt = 2.0 * height * rho / radius;
        normalize([tilt * angle.cos(), tilt * angle.sin(), 1.0])
    };
    let base = if highlight {
        mix(SOIL, [255, 236, 190], 0.3)
    } else if awake {
        mix(SOIL, glow, 0.14)
    } else {
        tint(SOIL, 0.72)
    };
    // A facet's brightness: ambient and the one light of the scene.
    let shade = |normal: Point| 0.30 + 0.85 * dot(normal, LIGHT).max(0.0);
    for (band, pair) in rings.windows(2).enumerate() {
        let middle = (pair[0] + pair[1]) * 0.5;
        // Fresh soil darkens toward the base.
        let fresh = 0.85 + 0.15 * (1.0 - middle);
        for segment in 0..segments {
            let angle = turn + (segment as f32 + 0.5) / segments as f32 * TAU;
            let grain = 0.90
                + 0.20
                    * ((hash::fnv1a_64_bytes((seed ^ (band * 64 + segment) as u64).to_le_bytes())
                        >> 40) as f32
                        / 16_777_216.0);
            let color = tint(base, fresh * grain * shade(normal(middle, angle)));
            frame.quad(
                camera,
                [
                    vertex(pair[0], segment),
                    vertex(pair[0], segment + 1),
                    vertex(pair[1], segment + 1),
                    vertex(pair[1], segment),
                ],
                color,
                pick,
            );
        }
    }
    // The top: a fan from the last ring to the apex.
    let top = rings[rings.len() - 1];
    let apex = [at[0], at[1], height];
    for segment in 0..segments {
        let angle = turn + (segment as f32 + 0.5) / segments as f32 * TAU;
        let color = tint(base, 0.95 * shade(normal(top * 0.5, angle)));
        frame.facet(
            camera,
            [vertex(top, segment), vertex(top, segment + 1), apex],
            color,
            pick,
        );
    }
    if px < 5.0 || !awake {
        return;
    }
    // The crater: dark, lit from inside in the colour of the nest's kind.
    let flat = |z: f32| Plane {
        center: [at[0], at[1], height + z],
        across: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
    };
    disc(frame, camera, flat(0.02), 0.2 * radius, HOLE, pick);
    disc(
        frame,
        camera,
        flat(0.03),
        0.13 * radius,
        tint(glow, 0.55),
        pick,
    );
    puff(
        frame,
        camera,
        flat(0.02).center,
        (px * 0.45).max(4.0),
        glow,
        0.45,
    );
    // An entrance on the slope for each destination, facing it.
    let rho = 0.62;
    for &bearing in entrances.iter().take(4) {
        let (sin, cos) = bearing.sin_cos();
        let point = [
            at[0] + radius * rho * cos,
            at[1] + radius * rho * sin,
            lift(rho),
        ];
        let out = normal(rho, bearing);
        let along = [-sin, cos, 0.0];
        let up = normalize(cross(out, along));
        let lifted = |by: f32| Plane {
            center: std::array::from_fn(|axis| point[axis] + by * out[axis]),
            across: along,
            up,
        };
        let size = (0.19 * radius).max(0.2);
        disc(frame, camera, lifted(0.04), size, HOLE, pick);
        disc(
            frame,
            camera,
            lifted(0.06),
            size * 0.62,
            tint(glow, 0.5),
            pick,
        );
        puff(
            frame,
            camera,
            point,
            (size * camera.zoom * SCALE * 1.6).max(3.0),
            glow,
            0.3,
        );
    }
}

/// What a pile is drawn from.
pub struct Heap {
    pub at: [f32; 2],
    pub radius: f32,
    /// What a click on it picks: the thing pick of its file.
    pub pick: u32,
    /// A hash of the file's path, which places each pebble.
    pub seed: u64,
    /// How much of itself it shows, from 0 when a file has just opened or is about to close to 1.
    pub presence: f32,
    pub highlight: bool,
}

/// The most pebbles a pile is made of.
pub const MOST_PEBBLES: usize = 24;

/// A pile: pebbles heaped in a paraboloid, at positions its path hashes to, in a soft light.
pub fn pile(frame: &mut Frame, camera: &Camera, heap: &Heap) {
    let Heap {
        at,
        radius,
        pick,
        seed,
        presence,
        highlight,
    } = *heap;
    if presence <= 0.0 {
        return;
    }
    let radius = radius * (0.4 + 0.6 * presence);
    let t = ((radius - 0.45) / 2.55).clamp(0.0, 1.0);
    let count = (6.0 + (MOST_PEBBLES - 6) as f32 * t).round() as usize;
    let stone = if highlight {
        mix(STONE, [255, 255, 235], 0.5)
    } else {
        STONE
    };
    let mut rng = Rng::new(seed);
    let peak = 0.75 * radius;
    let grain = (0.22 * radius).clamp(0.1, 0.5);
    for _ in 0..count {
        let rho = radius * rng.unit().sqrt();
        let angle = rng.unit() * TAU;
        let up = peak * (1.0 - rho / radius) * (0.75 + 0.25 * rng.unit());
        let shade = (0.82 + 0.23 * rng.unit()) * (0.35 + 0.65 * presence);
        let size = grain * (0.8 + 0.35 * rng.unit());
        ball(
            frame,
            camera,
            [
                at[0] + rho * angle.cos(),
                at[1] + rho * angle.sin(),
                up + size,
            ],
            size,
            tint(stone, shade),
            pick,
        );
    }
    let middle = [at[0], at[1], peak * 0.4];
    let px = radius * camera.zoom * SCALE;
    puff(frame, camera, middle, px * 0.9, HALO, 0.16 * presence);
    frame.pick_disc(camera, [at[0], at[1], peak * 0.5], px, pick);
}

/// How far across a blanket is, in world units.
pub const BLANKET: f32 = 5.4;

/// A gingham blanket lying at `at`: nine checks by nine, turned a little, with its shadow.
pub fn blanket(frame: &mut Frame, camera: &Camera, at: [f32; 2], pick: u32, highlight: bool) {
    const CHECKS: usize = 9;
    let turn: f32 = 18.0 * PI / 180.0;
    let (sin, cos) = turn.sin_cos();
    let place = |a: f32, b: f32, z: f32| -> Point {
        let (x, y) = (a * BLANKET, b * BLANKET);
        [at[0] + x * cos - y * sin, at[1] + x * sin + y * cos, z]
    };
    let shadow = |a: f32, b: f32| -> Point {
        let mut point = place(a, b, 0.02);
        point[0] += 0.4;
        point[1] += 0.3;
        point
    };
    frame.quad(
        camera,
        [
            shadow(-0.5, -0.5),
            shadow(0.5, -0.5),
            shadow(0.5, 0.5),
            shadow(-0.5, 0.5),
        ],
        [52, 40, 31],
        NONE,
    );
    let (cream, red) = ([214, 200, 176], [168, 36, 40]);
    let boost = if highlight { 1.3 } else { 1.0 };
    for i in 0..CHECKS {
        for j in 0..CHECKS {
            let mixed = ((i % 2) + (j % 2)) as f32 / 2.0;
            let weave = 0.86 + 0.1 * (((i * 7 + j * 3) % 5) as f32 / 4.0);
            let color = tint(mix(cream, red, mixed), 0.74 * weave * boost);
            let [a0, a1] = [i, i + 1].map(|k| k as f32 / CHECKS as f32 - 0.5);
            let [b0, b1] = [j, j + 1].map(|k| k as f32 / CHECKS as f32 - 0.5);
            frame.quad(
                camera,
                [
                    place(a0, b0, 0.04),
                    place(a1, b0, 0.04),
                    place(a1, b1, 0.04),
                    place(a0, b1, 0.04),
                ],
                color,
                pick,
            );
        }
    }
}

/// The ants and their crumbs, `alpha` of the way from the colony's last step to its next.
/// `sprite` is how much larger than at a slab 64 across the little things are drawn, so that they
/// keep their size on screen as the slab grows; `unsized_bytes` is the crumb of a stream whose
/// size is unknown.
pub fn ants(frame: &mut Frame, camera: &Camera, ants: &[Ant], alpha: f32, sprite: f32) {
    let pixels_per_unit = camera.zoom * SCALE;
    // An ant is 0.32 long and 0.14 wide at a slab 64 across, and never under 3 pixels long.
    let length = (0.32 * sprite).max(3.0 / pixels_per_unit);
    let width = length * 0.45;
    // An unsized crumb has the size a call of half a kibibyte would have.
    let unsized_radius = crumb_px(512.0) * 0.001 * 64.0 * sprite;
    for ant in ants {
        let [x, y] = ant.at(alpha);
        let heading = ant.facing(alpha);
        let (sin, cos) = heading.sin_cos();
        let (ahead, side) = ([cos, sin], [-sin, cos]);
        let corner = |along: f32, across: f32| -> Point {
            [
                x + ahead[0] * along + side[0] * across,
                y + ahead[1] * along + side[1] * across,
                ANT_LIFT,
            ]
        };
        frame.quad(
            camera,
            [
                corner(length * 0.5, width * 0.5),
                corner(length * 0.5, -width * 0.5),
                corner(-length * 0.5, -width * 0.5),
                corner(-length * 0.5, width * 0.5),
            ],
            ANT,
            NONE,
        );
        if !ant.carrying() {
            continue;
        }
        // A crumb sized by its bytes per call at 1000 slab units to the pixel.
        let radius = ant.route.crumb.map_or(unsized_radius, |bytes| {
            crumb_px(bytes) * 0.001 * 64.0 * sprite
        });
        let reach = length * 0.5 + radius * 0.8;
        ball(
            frame,
            camera,
            [
                x + ahead[0] * reach,
                y + ahead[1] * reach,
                ANT_LIFT + radius,
            ],
            radius,
            crumb_color(ant.route.way),
            NONE,
        );
    }
}

/// How a tunnel's dashes are cut.
#[derive(Clone, Copy, Debug)]
pub struct Cut {
    pub on: f32,
    pub off: f32,
    pub width: f32,
}

/// How one lane of a tunnel is laid and dashed.
#[derive(Clone, Copy, Debug)]
pub struct Lane {
    /// How far to the side of the path it lies, in world units.
    pub offset: f32,
    pub cut: Cut,
    /// How far the dashes have moved along the path, in world units, forward or back.
    pub travelled: f32,
    pub forward: bool,
    pub color: Color,
}

/// One lane of a tunnel along `path` (world points). A ribbon is only triangles, so it draws alike
/// on the CPU and the GPU.
pub fn lane(frame: &mut Frame, camera: &Camera, path: &[Point], lane: Lane) {
    let Lane {
        offset,
        cut,
        travelled,
        forward,
        color,
    } = lane;
    if path.len() < 2 {
        return;
    }
    let points: Vec<Point> = path
        .iter()
        .enumerate()
        .map(|(index, &at)| {
            let (before, after) = (
                path[index.saturating_sub(1)],
                path[(index + 1).min(path.len() - 1)],
            );
            let (dx, dy) = (after[0] - before[0], after[1] - before[1]);
            let size = dx.hypot(dy).max(1e-6);
            [
                at[0] - dy / size * offset,
                at[1] + dx / size * offset,
                at[2],
            ]
        })
        .collect();
    let period = cut.on + cut.off;
    let slid = travelled.rem_euclid(period);
    // The pattern starts at `-phase` along the path, so a smaller phase moves it forward.
    let phase = if forward { -slid } else { slid };
    frame.ribbon(
        camera,
        &points,
        Width::World(cut.width),
        color,
        NONE,
        Some(Dash {
            on: cut.on,
            off: cut.off,
            phase,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Backdrop, Buffers, Sky};

    fn blank() -> Frame {
        Frame::new(
            320,
            180,
            Sky {
                backdrop: Backdrop::Dusk,
                haze: [0.0; 3],
                time: 0.0,
            },
            Buffers::default(),
        )
    }

    fn triangles(frame: &Frame) -> usize {
        frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Triangle(..)))
            .count()
    }

    fn big(zoom: f32) -> Camera {
        Camera {
            zoom,
            ..Camera::default()
        }
    }

    fn a_mound<'a>(entrances: &'a [f32]) -> Mound<'a> {
        Mound {
            at: [0.0, 0.0],
            radius: 2.0,
            pick: 3,
            seed: 11,
            highlight: false,
            awake: true,
            glow: [72, 183, 199],
            entrances,
        }
    }

    #[test]
    fn crumb_size_follows_bytes_per_syscall_on_the_prototypes_curve() {
        assert!((crumb_px(64.0) - 0.55).abs() < 1e-5);
        assert!((crumb_px(128.0 * 1024.0) - (0.55 + 0.27 * 11.0)).abs() < 1e-4);
        assert_eq!(crumb_px(1.0), 0.42, "clamped below");
        assert_eq!(crumb_px(1.0e12), 3.6, "and above");
        let sizes: Vec<f32> = [64.0, 512.0, 4096.0, 32_768.0, 262_144.0]
            .into_iter()
            .map(crumb_px)
            .collect();
        assert!(sizes.windows(2).all(|pair| pair[1] > pair[0]), "{sizes:?}");
    }

    #[test]
    fn a_mound_has_twenty_four_segments_and_twelve_when_small() {
        let near = big(8.0);
        let mut frame = blank();
        mound(&mut frame, &near, &a_mound(&[]));
        // Four rings: three bands of 24 quads and a fan of 24, craters aside.
        assert!(
            triangles(&frame) >= 3 * 24 * 2 + 24,
            "{}",
            triangles(&frame)
        );
        let far = big(1.0);
        let mut small = blank();
        mound(&mut small, &far, &a_mound(&[]));
        let tiny = triangles(&small);
        assert!(tiny <= 2 * 12 * 2 + 12 + 24, "{tiny}");
        assert!(tiny < triangles(&frame) / 2);
    }

    #[test]
    fn a_mound_picks_its_process_everywhere_and_grows_an_entrance_for_each_destination() {
        let camera = big(8.0);
        let mut none = blank();
        mound(&mut none, &camera, &a_mound(&[]));
        let mut four = blank();
        mound(&mut four, &camera, &a_mound(&[0.0, 1.0, 2.0, 3.0, 4.0]));
        assert!(
            triangles(&four) - triangles(&none) >= 4 * 2 * 8,
            "four entrances, never five"
        );
        let picks: Vec<u32> = four
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Triangle(_, _, pick) => Some(*pick),
                _ => None,
            })
            .collect();
        assert!(picks.iter().all(|&pick| pick == 3));
    }

    #[test]
    fn a_pile_is_at_most_twenty_four_pebbles_and_picks_its_thing() {
        let camera = big(8.0);
        let mut frame = blank();
        let heap = Heap {
            at: [0.0, 0.0],
            radius: 3.0,
            pick: 77,
            seed: 5,
            presence: 1.0,
            highlight: false,
        };
        pile(&mut frame, &camera, &heap);
        let balls = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Sphere { pick: 77, .. }))
            .count();
        assert_eq!(balls, MOST_PEBBLES);
        assert!(
            frame
                .items
                .iter()
                .any(|item| matches!(item, Item::Pick { pick: 77, .. }))
        );
        let mut small = blank();
        pile(
            &mut small,
            &camera,
            &Heap {
                radius: 0.45,
                ..heap
            },
        );
        let few = small
            .items
            .iter()
            .filter(|item| matches!(item, Item::Sphere { .. }))
            .count();
        assert_eq!(few, 6);
        let mut gone = blank();
        pile(
            &mut gone,
            &camera,
            &Heap {
                presence: 0.0,
                ..heap
            },
        );
        assert!(gone.items.is_empty(), "a file that closed leaves nothing");
    }

    #[test]
    fn a_pile_is_the_same_pile_every_frame() {
        let camera = big(8.0);
        let heap = Heap {
            at: [4.0, -3.0],
            radius: 1.5,
            pick: 9,
            seed: 123,
            presence: 1.0,
            highlight: false,
        };
        let (mut one, mut two) = (blank(), blank());
        pile(&mut one, &camera, &heap);
        pile(&mut two, &camera, &heap);
        assert_eq!(format!("{:?}", one.items), format!("{:?}", two.items));
    }

    #[test]
    fn a_blanket_is_eighty_one_checks_that_pick_it() {
        let camera = big(8.0);
        let mut frame = blank();
        blanket(&mut frame, &camera, [0.0, 0.0], 5, false);
        let checks = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Triangle(_, _, 5)))
            .count();
        assert_eq!(checks, 81 * 2);
    }

    #[test]
    fn ants_are_two_triangles_that_pick_nothing_and_a_crumb_only_when_carrying() {
        use super::super::colony::{Colony, Route, STEP, Sending};
        use super::super::flows::Goal;
        use crate::model::Identity;
        let id = Identity { pid: 1, start: 1 };
        let route = |way| {
            Route::new(
                id,
                Goal::File(1),
                way,
                Some(4096.0),
                ([0.0, 0.0], 2.0),
                ([30.0, 0.0], 1.0),
                [200, 160, 60],
            )
        };
        for (way, outbound_carrying) in [(Way::Read, false), (Way::Write, true)] {
            let mut colony = Colony::new(64.0);
            colony.send([Sending {
                route: route(way),
                per_second: 4.0,
            }]);
            for _ in 0..20 {
                colony.step(STEP);
            }
            let count = colony.ants.len();
            assert!(count >= 3);
            let mut frame = blank();
            ants(&mut frame, &big(8.0), &colony.ants, 0.5, 1.0);
            assert_eq!(triangles(&frame), count * 2);
            assert!(frame.items.iter().all(|item| match item {
                Item::Triangle(_, _, pick) => *pick == NONE,
                Item::Sphere { pick, .. } => *pick == NONE,
                _ => true,
            }));
            let crumbs = frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Sphere { .. }))
                .count();
            assert_eq!(crumbs > 0, outbound_carrying, "{way:?}");
        }
    }

    #[test]
    fn a_crumb_is_pale_for_a_read_coral_for_a_write_and_between_for_either() {
        assert_eq!(crumb_color(Way::Read), [255, 238, 196]);
        assert_eq!(crumb_color(Way::Write), [255, 118, 92]);
        let either = crumb_color(Way::Either);
        assert!(either[1] > 118 && either[1] < 238 && either[2] > 92 && either[2] < 196);
    }

    #[test]
    fn dashes_slide_forward_or_back_with_the_distance_they_have_travelled() {
        let camera = big(8.0);
        let path: Vec<Point> = (0..20).map(|k| [k as f32 * 2.0, 0.0, 0.14]).collect();
        let cut = Cut {
            on: 1.0,
            off: 1.0,
            width: 0.2,
        };
        let draw = |travelled: f32, forward: bool| {
            let mut frame = blank();
            let lay = Lane {
                offset: 0.0,
                cut,
                travelled,
                forward,
                color: [200, 200, 200],
            };
            lane(&mut frame, &camera, &path, lay);
            format!("{:?}", frame.items)
        };
        assert_eq!(
            draw(0.0, true),
            draw(2.0, true),
            "a whole period is the same pattern"
        );
        assert_ne!(draw(0.0, true), draw(0.5, true));
        assert_ne!(draw(0.5, true), draw(0.5, false), "the two ways differ");
        assert_eq!(draw(0.5, true), draw(2.5, true));
    }
}
