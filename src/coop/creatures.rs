//! Drawing the creatures: chickens in the pose of their role, their chicks, and the foxes.

use std::f32::consts::TAU;

use crate::geometry::normalize_or_zero;
use crate::palette::tint;
use crate::render::{Camera, Color, Frame, NONE, Point, SCALE};

use super::draw::{offset, part};
use super::sim::{Chicken, Role};
use super::{FOX_SECONDS, Fox};

const COMB: Color = [216, 46, 46];
const BEAK: Color = [244, 168, 48];
const LEGS: Color = [232, 160, 64];
const PUPIL: Color = [18, 18, 22];
const MUD: Color = [90, 62, 40];
const SHADOW: Color = [40, 56, 36];
const CHICK: Color = [252, 220, 112];
const FOX: Color = [214, 108, 38];
const FOX_WHITE: Color = [240, 234, 226];
const FOX_DARK: Color = [52, 36, 30];
const GHOST: Color = [160, 160, 168];

/// A chicken as it is drawn this frame.
pub(super) struct Bird<'a> {
    pub(super) chicken: &'a Chicken,
    /// Where it stands on the ground, between its last two steps.
    pub(super) ground: [f32; 2],
    /// The body radius, grown in over the first moments of its process.
    pub(super) r: f32,
    pub(super) plumage: Color,
    pub(super) pick: u32,
    pub(super) selected: bool,
    pub(super) time: f32,
}

/// How a chicken stands and which way it faces, from its role.
struct Pose {
    forward: [f32; 2],
    phase: f32,
    perched: bool,
    walking: bool,
    zombie: bool,
    /// 1 for a chicken on its feet, -1 for a zombie with its feet up.
    up: f32,
    body: Point,
    /// The head's offset forward and up from the body, in body radii.
    head_forward: f32,
    head_up: f32,
    neck: bool,
}

impl Bird<'_> {
    fn pose(&self) -> Pose {
        let (chicken, ground, r, time) = (self.chicken, self.ground, self.r, self.time);
        let forward = [chicken.theta.cos(), chicken.theta.sin()];
        let phase = (chicken.id.pid % 97) as f32;
        let seated = chicken.role == Role::Roosting || chicken.perch > 0.0;
        let walking = chicken.role == Role::Foraging || chicken.walking();
        // Hopping up to the perch over the last unit of the walk.
        let lift = if seated {
            let away = (chicken.target[0] - ground[0]).hypot(chicken.target[1] - ground[1]);
            chicken.perch * (1.0 - away).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let perched = seated && lift > 0.0;
        let zombie = chicken.role == Role::Zombie;
        // Body height, then the head's offset forward and up, in body radii.
        let (height, head_forward, head_up, neck) = match chicken.role {
            Role::Zombie => (r * 0.8, 1.05, -0.3, false),
            Role::Frozen => (lift + r * 0.72, 0.85, 0.45, false),
            // Brooding: settled low on the straw pad, feathers fluffed out.
            Role::Brooding if perched => (lift + r * 0.4, 0.8, 0.4, false),
            _ if perched => (lift + r * 0.6, 0.7, 0.5, false),
            Role::Feeding if chicken.arrived => {
                // Pecking is decorative: the head bobs into the trough.
                let peck = (time * 7.0 + phase).sin().abs();
                (r * 1.3, 1.15, 0.25 - 0.45 * peck, true)
            }
            _ => (r * 1.3, 0.85, 0.95, true),
        };
        let up = if zombie { -1.0 } else { 1.0 };
        Pose {
            forward,
            phase,
            perched,
            walking,
            zombie,
            up,
            body: [ground[0], ground[1], height],
            head_forward,
            head_up,
            neck,
        }
    }
}

/// Draws one chicken in the pose of its role and returns its body centre: a plump body of
/// three spheres, neck, head with comb, beak, wattle and eyes, wings, a tail fan and legs.
pub(super) fn draw_chicken(frame: &mut Frame, camera: &Camera, bird: &Bird) -> Point {
    let pose = bird.pose();
    draw_shadow(frame, camera, bird, &pose);
    draw_mud(frame, camera, bird, &pose);
    draw_legs(frame, camera, bird, &pose);
    draw_torso(frame, camera, bird, &pose);
    draw_head(frame, camera, bird, &pose);
    pose.body
}

/// A soft shadow grounds a bird that is not on its perch; decorative.
fn draw_shadow(frame: &mut Frame, camera: &Camera, bird: &Bird, pose: &Pose) {
    let (ground, r) = (bird.ground, bird.r);
    if !pose.perched && bird.pick != NONE {
        let shade = r * 1.05;
        for k in 0..6 {
            let (a, b) = (k as f32 / 6.0 * TAU, (k + 1) as f32 / 6.0 * TAU);
            frame.facet(
                camera,
                [
                    [ground[0], ground[1], 0.015],
                    [
                        ground[0] + shade * a.cos(),
                        ground[1] + shade * a.sin(),
                        0.015,
                    ],
                    [
                        ground[0] + shade * b.cos(),
                        ground[1] + shade * b.sin(),
                        0.015,
                    ],
                ],
                SHADOW,
                NONE,
            );
        }
    }
}

/// The mud puddle a stuck chicken stands in.
fn draw_mud(frame: &mut Frame, camera: &Camera, bird: &Bird, pose: &Pose) {
    let (ground, r) = (bird.ground, bird.r);
    if bird.chicken.role == Role::Stuck {
        let puddle = r * 1.8;
        for k in 0..10 {
            let (a, b) = (k as f32 / 10.0 * TAU, (k + 1) as f32 / 10.0 * TAU);
            let wobble = |angle: f32| puddle * (0.85 + 0.15 * (angle * 3.0 + pose.phase).sin());
            frame.facet(
                camera,
                [
                    [ground[0], ground[1], 0.02],
                    [
                        ground[0] + wobble(a) * a.cos(),
                        ground[1] + wobble(a) * 0.8 * a.sin(),
                        0.02,
                    ],
                    [
                        ground[0] + wobble(b) * b.cos(),
                        ground[1] + wobble(b) * 0.8 * b.sin(),
                        0.02,
                    ],
                ],
                if k % 2 == 0 { MUD } else { tint(MUD, 0.88) },
                NONE,
            );
        }
    }
}

/// Legs in the air for a zombie, striding for a walker and standing for the rest; none for a
/// chicken sitting on a perch or frozen.
fn draw_legs(frame: &mut Frame, camera: &Camera, bird: &Bird, pose: &Pose) {
    let (chicken, ground, r, time) = (bird.chicken, bird.ground, bird.r, bird.time);
    let (body, forward) = (pose.body, pose.forward);
    if pose.zombie {
        for side in [-0.3, 0.3] {
            let hip = offset(body, forward, -0.1 * r, side * r, 0.5 * r);
            let knee = offset(body, forward, -0.15 * r, side * 1.2 * r, 1.25 * r);
            let foot = offset(body, forward, 0.05 * r, side * 1.3 * r, 1.6 * r);
            frame.line(camera, hip, knee, LEGS);
            frame.line(camera, knee, foot, LEGS);
        }
    } else if !pose.perched && chicken.role != Role::Frozen {
        let stride = if pose.walking {
            (time * (4.0 + 3.0 * chicken.speed) + pose.phase).sin() * 0.35 * r
        } else {
            0.0
        };
        for (side, sign) in [(-0.3, 1.0), (0.3, -1.0)] {
            let hip = offset(body, forward, 0.0, side * r, -0.5 * r);
            let foot = [
                ground[0] + forward[0] * stride * sign - forward[1] * side * r,
                ground[1] + forward[1] * stride * sign + forward[0] * side * r,
                0.0,
            ];
            frame.line(camera, hip, foot, LEGS);
        }
    }
}

/// The tail fan, the body of three spheres and the wings.
fn draw_torso(frame: &mut Frame, camera: &Camera, bird: &Bird, pose: &Pose) {
    let (r, plumage, pick) = (bird.r, bird.plumage, bird.pick);
    let (body, forward, up) = (pose.body, pose.forward, pose.up);
    let feathers = tint(plumage, 0.72);
    for side in [-0.3_f32, 0.0, 0.3] {
        frame.facet(
            camera,
            [
                offset(body, forward, -0.6 * r, (side - 0.25) * r, up * 0.2 * r),
                offset(body, forward, -0.6 * r, (side + 0.25) * r, up * 0.2 * r),
                offset(body, forward, -1.05 * r, side * 1.1 * r, up * 1.05 * r),
            ],
            feathers,
            pick,
        );
    }
    frame.sphere(camera, body, r, plumage, pick, bird.selected);
    part(
        frame,
        camera,
        offset(body, forward, -0.5 * r, 0.0, up * 0.15 * r),
        0.72 * r,
        tint(plumage, 0.92),
        pick,
    );
    part(
        frame,
        camera,
        offset(body, forward, 0.45 * r, 0.0, up * 0.05 * r),
        0.75 * r,
        tint(plumage, 1.05),
        pick,
    );
    for side in [-1.0_f32, 1.0] {
        frame.facet(
            camera,
            [
                offset(body, forward, 0.3 * r, side * 0.9 * r, up * 0.3 * r),
                offset(body, forward, -0.75 * r, side * 0.8 * r, up * 0.4 * r),
                offset(body, forward, -0.35 * r, side * 0.82 * r, -up * 0.2 * r),
            ],
            tint(plumage, 0.8),
            pick,
        );
    }
}

/// A chicken's head as the parts that sit on it are placed.
struct Head {
    at: Point,
    radius: f32,
}

/// The neck, the head and what is on it.
fn draw_head(frame: &mut Frame, camera: &Camera, bird: &Bird, pose: &Pose) {
    let (r, plumage, pick) = (bird.r, bird.plumage, bird.pick);
    let (body, forward, up) = (pose.body, pose.forward, pose.up);
    let head = Head {
        at: offset(
            body,
            forward,
            pose.head_forward * r,
            0.0,
            up * pose.head_up * r,
        ),
        radius: 0.45 * r,
    };
    if pose.neck {
        part(
            frame,
            camera,
            offset(body, forward, 0.65 * r, 0.0, (pose.head_up * 0.5 + 0.1) * r),
            0.38 * r,
            plumage,
            pick,
        );
    }
    part(
        frame,
        camera,
        head.at,
        head.radius,
        tint(plumage, 1.08),
        pick,
    );
    draw_comb(frame, camera, bird, pose, &head);
    draw_beak(frame, camera, pose, &head);
    draw_eyes(frame, camera, pose, &head);
}

/// The comb: three small spheres when it is large enough on screen, otherwise one triangle.
fn draw_comb(frame: &mut Frame, camera: &Camera, bird: &Bird, pose: &Pose, head: &Head) {
    let (forward, up) = (pose.forward, pose.up);
    let (head_radius, head) = (head.radius, head.at);
    let comb_radius = 0.18 * bird.r;
    if comb_radius * camera.zoom >= 1.2 {
        for (along, rise) in [(0.35, 0.8), (-0.05, 0.95), (-0.45, 0.75)] {
            part(
                frame,
                camera,
                offset(
                    head,
                    forward,
                    along * head_radius,
                    0.0,
                    up * rise * head_radius,
                ),
                comb_radius * if along.abs() < 0.1 { 1.0 } else { 0.8 },
                COMB,
                NONE,
            );
        }
    } else {
        frame.facet(
            camera,
            [
                offset(
                    head,
                    forward,
                    0.5 * head_radius,
                    0.0,
                    up * 0.5 * head_radius,
                ),
                offset(
                    head,
                    forward,
                    -0.7 * head_radius,
                    0.0,
                    up * 0.5 * head_radius,
                ),
                offset(
                    head,
                    forward,
                    -0.1 * head_radius,
                    0.0,
                    up * 1.5 * head_radius,
                ),
            ],
            COMB,
            NONE,
        );
    }
}

/// The beak, in two facets, and the wattle under it.
fn draw_beak(frame: &mut Frame, camera: &Camera, pose: &Pose, head: &Head) {
    let (forward, up) = (pose.forward, pose.up);
    let (head_radius, head) = (head.radius, head.at);
    let tip = offset(
        head,
        forward,
        1.8 * head_radius,
        0.0,
        -up * 0.15 * head_radius,
    );
    frame.facet(
        camera,
        [
            offset(
                head,
                forward,
                0.8 * head_radius,
                0.0,
                up * 0.25 * head_radius,
            ),
            offset(
                head,
                forward,
                0.8 * head_radius,
                0.0,
                -up * 0.35 * head_radius,
            ),
            tip,
        ],
        BEAK,
        NONE,
    );
    frame.facet(
        camera,
        [
            offset(head, forward, 0.8 * head_radius, -0.3 * head_radius, 0.0),
            offset(head, forward, 0.8 * head_radius, 0.3 * head_radius, 0.0),
            tip,
        ],
        tint(BEAK, 0.85),
        NONE,
    );
    part(
        frame,
        camera,
        offset(
            head,
            forward,
            0.75 * head_radius,
            0.0,
            -up * 0.75 * head_radius,
        ),
        0.25 * head_radius,
        COMB,
        NONE,
    );
}

/// The two eyes.
fn draw_eyes(frame: &mut Frame, camera: &Camera, pose: &Pose, head: &Head) {
    let (forward, up) = (pose.forward, pose.up);
    let (head_radius, head) = (head.radius, head.at);
    for side in [-0.6, 0.6] {
        part(
            frame,
            camera,
            offset(
                head,
                forward,
                0.55 * head_radius,
                side * head_radius,
                up * 0.25 * head_radius,
            ),
            0.16 * head_radius,
            PUPIL,
            NONE,
        );
    }
}

/// Chicks for threads: in a file behind a moving hen along its trail, tucked beside a still one.
pub(super) fn draw_chicks(
    frame: &mut Frame,
    camera: &Camera,
    chicken: &Chicken,
    ground: [f32; 2],
    body: Point,
    r: f32,
    time: f32,
) {
    if chicken.chicks == 0 {
        return;
    }
    let size = 0.2;
    let moving = chicken.role == Role::Foraging || chicken.walking();
    let forward = [chicken.theta.cos(), chicken.theta.sin()];
    let mut cursor = ground;
    let mut points = chicken.trail.iter();
    let mut travelled = 0.0;
    for k in 0..chicken.chicks {
        let p = if moving {
            let wanted = r + 0.45 + k as f32 * 0.6;
            let mut spot = None;
            while travelled < wanted {
                let Some(&next) = points.next() else {
                    break;
                };
                let step = (next[0] - cursor[0]).hypot(next[1] - cursor[1]);
                if travelled + step >= wanted && step > 1e-6 {
                    let t = (wanted - travelled) / step;
                    let at = [
                        cursor[0] + (next[0] - cursor[0]) * t,
                        cursor[1] + (next[1] - cursor[1]) * t,
                    ];
                    travelled = wanted;
                    cursor = at;
                    spot = Some(at);
                    break;
                }
                travelled += step;
                cursor = next;
            }
            let at = spot.unwrap_or_else(|| {
                // A short trail: the rest of the brood bunches up at its end.
                let angle = k as f32 * 2.4;
                [
                    cursor[0] + 0.15 * angle.cos(),
                    cursor[1] + 0.15 * angle.sin(),
                ]
            });
            let hop = 0.05 * (time * 9.0 + k as f32).sin().abs();
            [at[0], at[1], size + hop]
        } else {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let along = (k / 2) as f32;
            let base = [body[0], body[1], (body[2] - r * 0.45).max(size)];
            offset(
                base,
                forward,
                0.35 * r - along * 0.25,
                side * (0.75 * r + 0.15 + along * 0.12),
                0.0,
            )
        };
        let look = if moving { forward } else { [0.0, 1.0] };
        part(frame, camera, p, size, CHICK, NONE);
        part(
            frame,
            camera,
            offset(p, look, 0.15, 0.0, 0.14),
            0.11,
            tint(CHICK, 1.05),
            NONE,
        );
    }
}

/// The fox: in through the hedge to its victim, then away with a grey ghost in its mouth.
pub(super) fn draw_fox(frame: &mut Frame, camera: &Camera, fox: &Fox, time: f32) {
    let u = (time - fox.start) / FOX_SECONDS;
    if !(0.0..1.0).contains(&u) {
        return;
    }
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let lerp =
        |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let (at, heading, carrying) = if u < 0.42 {
        (
            lerp(fox.entry, fox.prey, ease(u / 0.42)),
            normalize_or_zero([fox.prey[0] - fox.entry[0], fox.prey[1] - fox.entry[1]]),
            false,
        )
    } else if u < 0.55 {
        (
            fox.prey,
            normalize_or_zero([fox.prey[0] - fox.entry[0], fox.prey[1] - fox.entry[1]]),
            true,
        )
    } else {
        (
            lerp(fox.prey, fox.exit, ease((u - 0.55) / 0.45)),
            normalize_or_zero([fox.exit[0] - fox.prey[0], fox.exit[1] - fox.prey[1]]),
            true,
        )
    };
    let forward = if heading == [0.0, 0.0] {
        [1.0, 0.0]
    } else {
        heading
    };
    let running = !(0.42..0.55).contains(&u);
    let size = 1.4;
    let gallop = if running { (time * 14.0).sin() } else { 0.0 };
    let body = [at[0], at[1], size * 0.95 + 0.08 * gallop.abs()];
    for (along, side) in [(0.55, -0.25), (0.55, 0.25), (-0.55, -0.25), (-0.55, 0.25)] {
        let swing = gallop * 0.3 * if along > 0.0 { 1.0 } else { -1.0 };
        let hip = offset(body, forward, along * size, side * size, -0.3 * size);
        let foot = offset(
            [at[0], at[1], 0.0],
            forward,
            (along + swing) * size,
            side * size,
            0.0,
        );
        frame.line(camera, hip, foot, FOX_DARK);
    }
    let mut tail = offset(body, forward, -0.95 * size, 0.0, 0.05 * size);
    for (k, girth) in [0.24_f32, 0.34, 0.4, 0.36, 0.26].into_iter().enumerate() {
        let color = if k == 4 { FOX_WHITE } else { FOX };
        frame.world_sphere(camera, tail, girth * size / SCALE, color);
        let sway = 0.06 * gallop * size * k as f32;
        tail = offset(
            tail,
            forward,
            -0.3 * size,
            sway,
            (0.1 + 0.03 * k as f32) * size,
        );
    }
    for (along, girth) in [(-0.5_f32, 0.48_f32), (0.0, 0.52), (0.5, 0.55)] {
        frame.world_sphere(
            camera,
            offset(body, forward, along * size, 0.0, 0.0),
            girth * size / SCALE,
            FOX,
        );
    }
    frame.world_sphere(
        camera,
        offset(body, forward, 0.8 * size, 0.0, -0.12 * size),
        0.32 * size / SCALE,
        FOX_WHITE,
    );
    let head = offset(body, forward, 1.2 * size, 0.0, 0.45 * size);
    frame.world_sphere(camera, head, 0.4 * size / SCALE, FOX);
    let snout = offset(head, forward, 0.85 * size, 0.0, -0.15 * size);
    for side in [-0.22, 0.22] {
        frame.facet(
            camera,
            [
                offset(head, forward, 0.2 * size, side * size, 0.08 * size),
                offset(head, forward, 0.2 * size, 0.0, -0.28 * size),
                snout,
            ],
            tint(FOX, 0.9),
            NONE,
        );
        frame.facet(
            camera,
            [
                offset(head, forward, 0.2 * size, side * size, 0.08 * size),
                offset(head, forward, 0.2 * size, 0.0, 0.2 * size),
                snout,
            ],
            FOX,
            NONE,
        );
        frame.facet(
            camera,
            [
                offset(head, forward, -0.12 * size, side * 0.8 * size, 0.25 * size),
                offset(head, forward, 0.12 * size, side * 1.5 * size, 0.25 * size),
                offset(head, forward, 0.0, side * 1.3 * size, 0.85 * size),
            ],
            FOX_DARK,
            NONE,
        );
    }
    frame.world_sphere(camera, snout, 0.08 * size / SCALE, FOX_DARK);
    if carrying && let Some(radius) = fox.ghost {
        let mouth = offset(head, forward, 0.7 * size + radius * 0.8, 0.0, -0.35 * size);
        frame.world_sphere(camera, mouth, radius / SCALE, GHOST);
    }
}
