//! The Horizon view: the OOM killer as a Schwarzschild black hole. A round black shadow with a
//! thin bright photon ring, and around it an accretion disk bent over the top of the shadow and
//! under its bottom, brighter and bluer on the side that approaches. Processes ride circular
//! orbits in the disk plane ranked by `oom_score`, the next victim on the innermost stable
//! circular orbit and the unkillable ones far out. When the kernel's OOM kill counter rises and a
//! process vanishes, the victim leaves the ISCO and plunges, stretched, reddened and dimmed, and
//! freezes and fades short of the horizon instead of crossing it, as a distant observer sees it.
//!
//! Lensing is done on vertices, not pixels: `lens` answers where on the screen the images of a
//! point are, `disk` cuts the disk, the sky, the shadow and the ring into triangles at the images
//! of their vertices, and `orbits` ranks, moves and retires the processes. Both rasterizers draw
//! the same triangles, spheres and glows.

mod disk;
mod lens;
mod orbits;

use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, TAU};

use self::disk::{Disk, Light, Screen, SkyField, layer};
use self::lens::{LensTable, Observer};
use self::orbits::{
    BAND, Body, Candidate, Ghost, KillSource, Kills, M_PER_SECOND, Orbits, RANKED, Sample,
};
use crate::model::{Identity, Kind, Measured, Process, Snapshot, bytes};
use crate::palette::{kind_color, mix, tint};
use crate::platform::Source;
use crate::render::{
    Camera, Color, Frame, ISOMETRIC, Item, LOWEST_PITCH, MIN_BODY_PX, NONE, SCALE, Stage,
    TourTarget, label_name,
};
use crate::simulation::{smoothing, sprout};
use crate::status::missing_note;
use crate::views::ViewModule;

/// The inclination at the lowest pitch, at the default isometric pitch (the mockup's) and at the
/// top-down pitch, in degrees from face-on.
const INCLINATION_LOWEST: f32 = 86.0;
const INCLINATION_ISOMETRIC: f32 = 80.0;
/// The next victim's ring and the colour a plunging body turns towards, and the colours of the
/// ISCO's and the unkillable band's guides.
const VICTIM: Color = [235, 60, 30];
const GUIDE_ISCO: Color = [168, 164, 204];
const GUIDE_BAND: Color = [100, 108, 140];
/// Dash and gap of the guides, in pixels at the mockup's height of 900.
const DASH_ISCO: (f32, f32) = (6.0, 4.5);
const DASH_BAND: (f32, f32) = (2.5, 5.5);
/// The secondary image of a body is drawn at this fraction of its radius: the lens shrinks it.
const SECONDARY_SIZE: f32 = 0.6;
/// How far outside the shadow a secondary image must be, in M, to be drawn as a sphere.
const RING_CLEARANCE: f32 = 1.2;
/// How slowly the memory pressure reading is followed, in seconds.
const PRESSURE_SECONDS: f32 = 0.6;
/// The most ranks that carry a label.
const LABELS: usize = 5;
const MEBIBYTE: f32 = 1_048_576.0;

/// The inclination of the observer, in radians from face-on, from the camera's pitch: a piecewise
/// linear map from the lowest pitch (86 degrees) through the default isometric pitch (80 degrees,
/// the mockup's) to top-down (0), a pure function of the camera.
fn inclination(pitch: f32) -> f32 {
    let degrees = if pitch <= ISOMETRIC {
        let t = ((pitch - LOWEST_PITCH) / (ISOMETRIC - LOWEST_PITCH)).clamp(0.0, 1.0);
        INCLINATION_LOWEST + (INCLINATION_ISOMETRIC - INCLINATION_LOWEST) * t
    } else {
        let t = ((pitch - ISOMETRIC) / (FRAC_PI_2 - ISOMETRIC)).clamp(0.0, 1.0);
        INCLINATION_ISOMETRIC * (1.0 - t)
    };
    degrees.to_radians()
}

/// What ranks the processes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Basis {
    /// The kernel's `oom_score`.
    #[default]
    Score,
    /// The memory footprint, where the platform has no score to read.
    Footprint,
}

/// The score a process is ranked by, and its `oom_score_adj`, if the platform read both: the
/// macOS jetsam band has no adjustment, so it is not an `oom_score`.
fn readable_score(process: &Process) -> Option<(i32, i16)> {
    match process.oom {
        Measured::Known(oom) => Some((oom.badness, oom.adjust?)),
        _ => None,
    }
}

/// Whether a process takes part: a kernel thread has no memory of its own to kill, and a zombie
/// none left.
fn takes_part(process: &Process) -> bool {
    process.kind != Kind::Kernel && process.state != 'Z'
}

/// The radius of a process's sphere in M: `0.12 cbrt(MiB)`, from 0.18 to 0.9.
fn body_radius(memory: u64) -> f32 {
    (0.12 * (memory as f32 / MEBIBYTE).cbrt()).clamp(0.18, 0.9)
}

/// The processes that may ride, what ranks them, and how many cannot be ranked.
struct Roster<'a> {
    candidates: Vec<Candidate<'a>>,
    basis: Basis,
    unranked: usize,
    /// Some process has a score that is not a Linux `oom_score` (a jetsam band).
    foreign: bool,
}

fn roster<'a>(processes: impl Iterator<Item = &'a Process>) -> Roster<'a> {
    let taking: Vec<&Process> = processes.filter(|process| takes_part(process)).collect();
    let basis = if taking
        .iter()
        .any(|process| readable_score(process).is_some())
    {
        Basis::Score
    } else {
        Basis::Footprint
    };
    let foreign = taking
        .iter()
        .any(|process| matches!(process.oom, Measured::Known(oom) if oom.adjust.is_none()));
    let mut unranked = 0;
    let candidates = taking
        .iter()
        .map(|process| {
            let (score, unkillable) = match basis {
                Basis::Score => match readable_score(process) {
                    Some((badness, adjust)) => (Some(badness), adjust <= -1000),
                    None => {
                        // A score not read yet is warming up, not missing.
                        unranked += usize::from(!matches!(process.oom, Measured::Pending));
                        (None, false)
                    }
                },
                Basis::Footprint => (
                    Some((process.memory >> 20).min(i32::MAX as u64) as i32),
                    false,
                ),
            };
            Candidate {
                id: process.id,
                name: &process.name,
                score,
                unkillable,
                color: kind_color(process),
                size: body_radius(process.memory),
            }
        })
        .collect();
    Roster {
        candidates,
        basis,
        unranked,
        foreign,
    }
}

/// The kernel's count of OOM kills: `/proc/vmstat`'s, or where it was not read the sum of the
/// cgroups' own counts.
fn kill_counter(snapshot: &Snapshot) -> Option<Kills> {
    if let Some(count) = snapshot.oom_kills {
        return Some(Kills {
            count,
            source: KillSource::Vmstat,
        });
    }
    let counts: Vec<u64> = snapshot
        .units
        .values()
        .filter_map(|unit| unit.oom_kills)
        .collect();
    (!counts.is_empty()).then(|| Kills {
        count: counts.iter().sum(),
        source: KillSource::Cgroups,
    })
}

/// What the legend says that the last frame decided.
#[derive(Default)]
struct Facts {
    inclination: f32,
    basis: Basis,
    unranked: usize,
    foreign: bool,
    /// The next victim's name and what ranks it, as it is labelled.
    next: Option<(String, String)>,
}

#[derive(Default)]
pub struct Horizon {
    disk: Disk,
    orbits: Orbits,
    /// The memory pressure the disk shines at, eased over the samples.
    pressure: Option<f32>,
    /// The selection and the processes the last frame drew from (the slice `--limit` and a focus
    /// leave), so a sample taken while no frame is drawn ranks the same processes the frame does.
    selected: Option<Identity>,
    slice: Option<HashSet<Identity>>,
    facts: Facts,
    /// The processes the last frame drew (tests).
    #[cfg(test)]
    seated: Vec<Identity>,
}

/// The points the camera fit must include: the extremes of the disk's images, as ground points
/// that appear where the images do under `camera`.
fn envelope(camera: &Camera, extent: [f32; 2]) -> Vec<[f32; 3]> {
    let probe = Camera {
        center: [0.0, 0.0],
        zoom: 1.0,
        ..camera.clone()
    };
    let [across, up] = [extent[0] + 1.5, extent[1] + 1.5];
    [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
        .map(|(x, y)| {
            let [ground_x, ground_y] = probe.ground(x * across * SCALE, y * up * SCALE);
            [ground_x, ground_y, 0.0]
        })
        .to_vec()
}

/// The ground point that appears at the screen offset `offset` M from the hole, x right and y up:
/// where a lensed image stands in the world the camera looks at, for picking, labels and the tour.
fn ground_of(camera: &Camera, scale: f32, offset: [f32; 2]) -> [f32; 3] {
    let [x, y] = camera.ground(scale * offset[0], -scale * offset[1]);
    [x, y, 0.0]
}

/// What a frame draws with.
struct View<'a> {
    camera: &'a Camera,
    screen: Screen,
    observer: Observer,
    table: &'static LensTable,
}

impl View<'_> {
    /// The images of the point of the disk at `radius` and the world azimuth `azimuth`.
    fn images(&self, radius: f32, azimuth: f32) -> lens::Images {
        self.observer
            .disk_images(self.table, radius, azimuth + self.camera.rotation)
    }

    /// A sphere of `radius` M at the image `offset`, picking `pick`, at layer `depth`. Returns its
    /// centre on the frame.
    #[expect(clippy::too_many_arguments, reason = "one sphere item")]
    fn sphere(
        &self,
        frame: &mut Frame,
        offset: [f32; 2],
        radius: f32,
        depth: f32,
        color: Color,
        pick: u32,
        selected: bool,
    ) -> [f32; 2] {
        let center = self.screen.pixel(offset);
        frame.items.push(Item::Sphere {
            center: [center[0], center[1], depth],
            radius: (radius * self.screen.scale).max(MIN_BODY_PX),
            depth: 0.5,
            color,
            pick,
            selected,
        });
        center
    }

    /// Whether a secondary image is far enough out of the shadow to stand as a sphere: light that
    /// has gone most of the way round the hole piles up in a ring no thicker than
    /// `RING_CLEARANCE` M just outside the shadow, where every body's image is squeezed into the
    /// same sliver.
    fn clear_of_the_ring(&self, offset: [f32; 2]) -> bool {
        offset[0].hypot(offset[1]) > lens::SHADOW_RADIUS + RING_CLEARANCE
    }

    /// How much nearer to the observer than the hole a point of the disk plane is, in M, so the
    /// near side of the disk stands in front of the far side where bodies overlap.
    fn nearness(&self, radius: f32, azimuth: f32) -> f32 {
        -self.observer.inclination().sin() * radius * (azimuth + self.camera.rotation).sin()
    }

    /// Draws what is left of a vanished process: it fades where it stood, or, killed, eases to
    /// the ISCO and plunges as a radial streak that reddens and dims.
    fn ghost(&self, stage: &mut Stage, ghost: &Ghost) {
        let state = ghost.pose(stage.time);
        let images = self.images(state.radius, state.azimuth);
        let radius = ghost.size * (0.5 + 0.5 * state.light);
        let pixels = (radius * self.screen.scale).max(MIN_BODY_PX);
        if !state.plunging {
            let color = tint(ghost.color, state.light);
            let depth = layer::BODY + 0.1 * self.nearness(state.radius, state.azimuth);
            self.sphere(
                stage.frame,
                images.primary,
                radius,
                depth,
                color,
                NONE,
                false,
            );
            if self.clear_of_the_ring(images.secondary) {
                self.sphere(
                    stage.frame,
                    images.secondary,
                    radius * SECONDARY_SIZE,
                    layer::SECONDARY_BODY,
                    tint(color, 0.6),
                    NONE,
                    false,
                );
            }
            return;
        }
        let glowing = mix(ghost.color, VICTIM, 1.0 - state.light);
        let color = tint(glowing, state.light);
        let center = self.screen.pixel(images.primary);
        self.sphere(
            stage.frame,
            images.primary,
            radius,
            layer::PLUNGE,
            color,
            NONE,
            false,
        );
        if state.stretch > 1.02 {
            // A streak along the line from the hole, stretched both ways about the image.
            let (dx, dy) = (
                center[0] - self.screen.hole[0],
                center[1] - self.screen.hole[1],
            );
            let length = dx.hypot(dy);
            let (ux, uy) = if length > 1e-3 {
                (dx / length, dy / length)
            } else {
                (1.0, 0.0)
            };
            let along = pixels * state.stretch;
            let across = pixels * 0.9 / state.stretch.sqrt();
            let corner = |a: f32, b: f32| {
                [
                    center[0] + ux * a - uy * b,
                    center[1] + uy * a + ux * b,
                    layer::PLUNGE - 0.5,
                ]
            };
            let quad = [
                corner(along, across),
                corner(along, -across),
                corner(-along, -across),
                corner(-along, across),
            ];
            let shade = tint(glowing, 0.8 * state.light);
            for triangle in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
                stage
                    .frame
                    .items
                    .push(Item::Triangle(triangle, shade, NONE));
            }
        }
        stage.frame.items.push(Item::Glow {
            center,
            radius: (pixels * 3.0 * state.stretch.min(2.2)).max(10.0 * self.screen.unit()),
            color: glowing,
            strength: 0.8 * state.light,
        });
        if state.light > 0.3 {
            stage.places.push((
                ground_of(self.camera, self.screen.scale, images.primary),
                format!("{} killed", label_name(&ghost.name)),
            ));
        }
    }
}

impl ViewModule for Horizon {
    #[expect(clippy::too_many_lines, reason = "the drawing reads in frame order")]
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let camera = stage.camera;
        let observer = Observer::new(inclination(camera.pitch));
        let scale = camera.zoom * SCALE;
        let hole = stage.frame.project(camera, [0.0; 3]);
        let screen = Screen {
            hole: [hole[0], hole[1]],
            scale,
            width: stage.frame.width as f32,
            height: stage.frame.height as f32,
        };
        let view = View {
            camera,
            screen,
            observer,
            table: LensTable::shared(),
        };
        self.disk.prepare(observer, scale);
        stage.bounds.extend(envelope(camera, self.disk.extent()));

        // The orbits: who rides where, and the ghosts of whoever vanished.
        self.selected = stage.selected;
        self.slice = Some(processes.iter().map(|process| process.id).collect());
        let Roster {
            candidates,
            basis,
            unranked,
            foreign,
        } = roster(processes.iter().copied());
        let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        self.orbits.observe(&Sample {
            elapsed: snapshot.elapsed,
            time: stage.time,
            candidates: &candidates,
            alive: &alive,
            selected: stage.selected,
            kills: kill_counter(snapshot),
        });
        self.orbits.advance(stage.dt, stage.time);

        let reading = snapshot.pressure[1].clamp(0.0, 100.0);
        let pressure = self.pressure.get_or_insert(reading);
        *pressure += (reading - *pressure) * smoothing(stage.dt, PRESSURE_SECONDS);
        let light = Light {
            gain: 0.45 + 0.9 * *pressure / 100.0,
            time: stage.time * M_PER_SECOND,
            rotation: camera.rotation,
            sky: SkyField::new(&stage.frame.sky, stage.frame.width, stage.frame.height),
        };
        let unit = screen.unit();

        // Back to front: the sky, the secondary image, the shadow and its ring, the primary image.
        self.disk.stars(stage.frame, &screen);
        self.disk.draw_secondary(stage.frame, &screen, &light);
        disk::draw_shadow(stage.frame, &screen);
        disk::draw_ring(stage.frame, &screen, observer.inclination());
        self.disk.draw_primary(stage.frame, &screen, &light);
        let guides = [
            (lens::ISCO, DASH_ISCO, 1.4, GUIDE_ISCO),
            (0.5 * (BAND.0 + BAND.1), DASH_BAND, 1.2, GUIDE_BAND),
        ];
        for (radius, dash, width, color) in guides {
            let points = disk::orbit_image(view.table, observer, radius);
            let dash = (dash.0 * unit, dash.1 * unit);
            disk::draw_dashed(stage.frame, &screen, &points, dash, width * unit, color);
        }
        disk::draw_glare(stage.frame, &screen, view.table, observer, &light);

        // The bodies, in identity order so a frame is repeatable.
        let index: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(index, process)| (process.id, index))
            .collect();
        let mut riding: Vec<(&Process, &Body)> = self
            .orbits
            .bodies()
            .iter()
            .filter_map(|(id, body)| Some((*processes.get(*index.get(id)?)?, body)))
            .collect();
        riding.sort_by_key(|(process, _)| process.id);
        let ranked = self.orbits.ranking.len();
        let victim = self.orbits.next_victim();
        let mut labelled: Vec<(usize, [f32; 3], String)> = Vec::new();
        let mut unkillable_label = None;
        let mut tour = Vec::new();
        self.facts.next = None;
        for &(process, body) in &riding {
            let pick = index[&process.id] as u32;
            let radius = body.size * sprout(stage.growth(process.id));
            let images = view.images(body.radius, body.phase);
            let depth = layer::BODY + 0.1 * view.nearness(body.radius, body.phase);
            let center = view.sphere(
                stage.frame,
                images.primary,
                radius,
                depth,
                body.color,
                pick,
                stage.selected == Some(process.id),
            );
            if view.clear_of_the_ring(images.secondary) {
                view.sphere(
                    stage.frame,
                    images.secondary,
                    radius * SECONDARY_SIZE,
                    layer::SECONDARY_BODY,
                    tint(body.color, 0.6),
                    pick,
                    false,
                );
            }
            if Some(process.id) == victim {
                // The next victim wears a ring.
                let ring = (radius * scale).max(MIN_BODY_PX) + 4.5 * unit;
                let corner = |k: usize| {
                    let angle = TAU * k as f32 / 28.0;
                    [
                        center[0] + ring * angle.cos(),
                        center[1] + ring * angle.sin(),
                        layer::BODY + 5.0,
                    ]
                };
                for k in 0..28 {
                    stage
                        .frame
                        .hairline_between(corner(k), corner(k + 1), VICTIM, 1.4 * unit);
                }
            }
            let position = ground_of(camera, scale, images.primary);
            stage.positions.insert(process.id, position);
            let score = readable_score(process);
            let caption = match (basis, score) {
                (Basis::Score, Some((badness, _))) => badness.to_string(),
                _ => bytes(process.memory),
            };
            let name = label_name(&process.name);
            match body.rank {
                Some(rank) => {
                    if rank < LABELS {
                        labelled.push((rank, position, format!("{name} {caption}")));
                    }
                    if rank == 0 {
                        self.facts.next = Some((name.to_string(), caption));
                    }
                }
                None if unkillable_label.is_none() => {
                    unkillable_label = Some((position, format!("{name} unkillable")));
                }
                None => {}
            }
            tour.push(TourTarget {
                id: process.id,
                position,
                weight: body
                    .rank
                    .map_or(0.5, |rank| (RANKED - rank.min(RANKED)) as f32),
            });
            stage.popup.push((
                process.id,
                match (basis, score) {
                    (Basis::Score, Some((badness, adjust))) => {
                        format!("oom_score {badness} | oom_score_adj {adjust}")
                    }
                    _ => format!("memory footprint {}", bytes(process.memory)),
                },
            ));
            stage.popup.push((
                process.id,
                match body.rank {
                    Some(rank) => format!("rank {} of {ranked}", rank + 1),
                    None => "unkillable: the kernel never picks it".to_owned(),
                },
            ));
            let period = lens::period(body.radius);
            stage.popup.push((
                process.id,
                format!(
                    "orbit r = {:.2} M | period {period:.0} M ({:.1} s)",
                    body.radius,
                    period / M_PER_SECOND
                ),
            ));
        }
        for ghost in self.orbits.ghosts() {
            if !ghost.gone(stage.time) {
                view.ghost(stage, ghost);
            }
        }
        labelled.sort_by_key(|(rank, ..)| *rank);
        stage.places.extend(
            labelled
                .into_iter()
                .map(|(_, position, text)| (position, text)),
        );
        stage.places.extend(unkillable_label);
        tour.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
        stage.tour.extend(tour);

        self.facts.inclination = observer.inclination();
        self.facts.basis = basis;
        self.facts.unranked = unranked;
        self.facts.foreign = foreign;
        #[cfg(test)]
        {
            self.seated = riding.iter().map(|(process, _)| process.id).collect();
        }
        riding.len()
    }

    fn record(&mut self, snapshot: &Snapshot, time: f32) {
        let slice = self.slice.as_ref();
        let Roster { candidates, .. } = roster(
            snapshot
                .processes
                .iter()
                .filter(|process| slice.is_none_or(|slice| slice.contains(&process.id))),
        );
        let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        self.orbits.observe(&Sample {
            elapsed: snapshot.elapsed,
            time,
            candidates: &candidates,
            alive: &alive,
            selected: self.selected,
            kills: kill_counter(snapshot),
        });
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let mut text = match &self.facts.next {
            Some((name, caption)) => format!(" next victim {name} {caption}"),
            None => " no process ranked".to_owned(),
        };
        let counter = kill_counter(snapshot);
        let notes = [
            missing_note(snapshot, Source::Oom, "ranked by memory footprint"),
            missing_note(
                snapshot,
                Source::OomKills,
                if counter.is_some() {
                    "kills counted from cgroups"
                } else {
                    "exits not classified"
                },
            ),
            missing_note(
                snapshot,
                Source::MemoryPressure,
                "disk at its base brightness",
            ),
            (self.facts.basis == Basis::Footprint && self.facts.foreign)
                .then(|| " | no linux oom score: ranked by memory footprint".to_owned()),
            (self.facts.unranked > 0).then(|| format!(" | {} unranked", self.facts.unranked)),
        ];
        text.extend(notes.into_iter().flatten());
        if let Some(Kills { count, source }) = counter {
            text.push_str(&format!(" | oom_kill {count} ({})", source.name()));
        }
        text.push_str(&format!(
            " | orbit radius = oom_score rank (ISCO 6M = next victim) | disk brightness = memory pressure | colour = Doppler + gravitational redshift | far orbit = oom_score_adj -1000 | i = {:.0} deg",
            self.facts.inclination.to_degrees()
        ));
        text
    }

    /// The seat each drawn process keeps, from its identity alone: where it starts going round
    /// on a circle of 15 M. A process's radius is its rank, which a change of the population
    /// moves by design (and eases), so the harness checks the seats; the ranks and their
    /// easing are tested in `orbits`.
    #[cfg(test)]
    fn anchors(&self) -> Option<HashMap<Identity, [f32; 3]>> {
        Some(
            self.seated
                .iter()
                .map(|&id| {
                    let angle = orbits::initial_phase(id);
                    (id, [15.0 * angle.cos(), 15.0 * angle.sin(), 0.0])
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo_snapshot;
    use crate::harness::{churned, rescaled};
    use crate::model::testing::process;
    use crate::model::{DemoOptions, Oom, demo, demo_with};
    use crate::render::{RenderInput, Scene};
    use crate::views::{REGISTRY, view};

    fn camera() -> Camera {
        Camera {
            zoom: 2.2,
            ..Camera::default()
        }
    }

    /// A frame of the demo at `time` as `--output` draws it: the row's replay of the seconds
    /// before it, then the sample of `time` itself, drawn by a scene of its own.
    fn demo_frame(time: f64) -> (Scene, Frame) {
        let horizon = view("horizon");
        let row = horizon.info();
        let mut scene = Scene::new();
        for back in (1..=8).rev() {
            let at = time - f64::from(back);
            scene.record(&demo_snapshot(row, at - 1.0, at, 192), at as f32);
        }
        let snapshot = demo_snapshot(row, time - 1.0, time, 192);
        let frame = scene.render(&RenderInput::small(
            &snapshot,
            horizon,
            &camera(),
            time as f32,
        ));
        (scene, frame)
    }

    fn legend(scene: &Scene, snapshot: &Snapshot) -> String {
        scene.legend(view("horizon"), snapshot)
    }

    /// The spheres at `depth` (within half a unit of it).
    fn spheres_at(frame: &Frame, depth: f32) -> usize {
        frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Sphere { center, .. } if (center[2] - depth).abs() < 0.5))
            .count()
    }

    #[test]
    fn inclination_follows_the_pitch_piecewise_linearly() {
        let degrees = |pitch: f32| inclination(pitch).to_degrees();
        assert!((degrees(LOWEST_PITCH) - 86.0).abs() < 1e-4);
        assert!((degrees(ISOMETRIC) - 80.0).abs() < 1e-4, "the default");
        assert!(degrees(FRAC_PI_2).abs() < 1e-4);
        assert!((degrees(0.5 * (LOWEST_PITCH + ISOMETRIC)) - 83.0).abs() < 1e-3);
        assert!((degrees(0.5 * (ISOMETRIC + FRAC_PI_2)) - 40.0).abs() < 1e-3);
        // Monotone, and held at the ends.
        assert!(degrees(0.1) == degrees(LOWEST_PITCH));
        assert!(
            (0..100)
                .all(|k| degrees(0.35 + k as f32 * 0.0122)
                    >= degrees(0.35 + (k + 1) as f32 * 0.0122))
        );
    }

    #[test]
    fn the_next_oom_victim_is_named_and_the_legend_says_what_it_maps() {
        let (scene, _) = demo_frame(12.0);
        let snapshot = demo_snapshot(view("horizon").info(), 11.0, 12.0, 192);
        let text = legend(&scene, &snapshot);
        assert!(
            text.starts_with(" next victim worker-134 950 | oom_kill 0 (vmstat) | orbit radius"),
            "{text}"
        );
        for part in [
            "orbit radius = oom_score rank (ISCO 6M = next victim)",
            "disk brightness = memory pressure",
            "colour = Doppler + gravitational redshift",
            "far orbit = oom_score_adj -1000",
        ] {
            assert!(text.contains(part), "{part}");
        }
        assert!(text.ends_with("i = 80 deg"), "{text}");
        // It rides the ISCO: its label is the first, and the legend names it.
        assert!(scene.places[0].1.starts_with("worker-134 950"));
    }

    #[test]
    fn the_legend_names_what_the_platform_could_not_read() {
        let horizon = view("horizon");
        let row = horizon.info();
        let (scene, _) = demo_frame(12.0);
        let mut snapshot = demo_snapshot(row, 11.0, 12.0, 192);
        // No vmstat counter, but the cgroups count: they stand in.
        snapshot.oom_kills = None;
        snapshot.missing = Source::OomKills.into();
        let text = legend(&scene, &snapshot);
        assert!(
            text.contains(" | no oom kills: kills counted from cgroups | oom_kill 0 (cgroups) | "),
            "{text}"
        );
        // No counter at all: exits are not classified.
        for unit in snapshot.units.values_mut() {
            unit.oom_kills = None;
        }
        let text = legend(&scene, &snapshot);
        assert!(
            text.contains(" | no oom kills: exits not classified | orbit radius"),
            "{text}"
        );
        assert!(!text.contains("oom_kill 0"), "{text}");
        // No memory pressure: the disk keeps its base brightness.
        snapshot.missing.insert(Source::MemoryPressure);
        assert!(
            legend(&scene, &snapshot).contains("no memory pressure: disk at its base brightness")
        );
    }

    #[test]
    fn without_oom_scores_the_view_ranks_by_memory_footprint_and_says_so() {
        // The demo without its scores: every `oom` is pending, as on macOS without root.
        let mut snapshot = demo_with(12.0, 192, DemoOptions::NONE);
        snapshot.missing = Source::Oom.into();
        let mut scene = Scene::new();
        let frame = scene.render(&RenderInput::small(
            &snapshot,
            view("horizon"),
            &camera(),
            12.0,
        ));
        let text = legend(&scene, &snapshot);
        assert!(
            text.contains(" | no oom score: ranked by memory footprint"),
            "{text}"
        );
        let largest = snapshot
            .processes
            .iter()
            .filter(|p| p.kind != Kind::Kernel && p.state != 'Z')
            .max_by_key(|p| p.memory)
            .unwrap();
        assert!(
            text.starts_with(&format!(
                " next victim {} {}",
                largest.name,
                bytes(largest.memory)
            )),
            "{text}"
        );
        assert!(scene.visible > 0);
        assert!(
            frame
                .items
                .iter()
                .any(|item| matches!(item, Item::Sphere { pick, .. } if *pick != NONE))
        );
        // There is no unkillable band and no plunge: nobody is `unkillable`.
        assert!(
            scene
                .places
                .iter()
                .all(|(_, text)| !text.ends_with("unkillable"))
        );
        // A jetsam band is not an oom score either: only root reads it, and it has no adjustment.
        for process in &mut snapshot.processes {
            process.oom = Measured::Known(Oom {
                badness: -(process.memory as i32 >> 20),
                adjust: None,
                exempt: false,
            });
        }
        snapshot.missing = crate::platform::Sources::default();
        scene.render(&RenderInput::small(
            &snapshot,
            view("horizon"),
            &camera(),
            12.5,
        ));
        assert!(
            legend(&scene, &snapshot).contains(" | no linux oom score: ranked by memory footprint")
        );
    }

    #[test]
    fn scores_not_read_yet_are_warming_up_not_missing_or_unranked() {
        // The first sample after the view opens: no score has arrived, none is missing.
        let snapshot = demo_with(12.0, 192, DemoOptions::NONE);
        let mut scene = Scene::new();
        scene.render(&RenderInput::small(
            &snapshot,
            view("horizon"),
            &camera(),
            12.0,
        ));
        let text = legend(&scene, &snapshot);
        assert!(
            !text.contains(" | no ") && !text.contains("unranked"),
            "{text}"
        );
        assert!(
            scene.visible > 0 && !scene.positions.is_empty(),
            "it ranks by footprint meanwhile"
        );
    }

    #[test]
    fn processes_without_a_readable_score_are_left_out_and_counted() {
        let mut snapshot = demo_snapshot(view("horizon").info(), 11.0, 12.0, 192);
        let unreadable = snapshot
            .processes
            .iter()
            .filter(|p| p.kind != Kind::Kernel && p.state != 'Z')
            .take(7)
            .map(|p| p.id)
            .collect::<Vec<_>>();
        for process in &mut snapshot.processes {
            if unreadable.contains(&process.id) {
                process.oom = Measured::Unreadable;
            }
        }
        let mut scene = Scene::new();
        scene.render(&RenderInput::small(
            &snapshot,
            view("horizon"),
            &camera(),
            12.0,
        ));
        assert!(legend(&scene, &snapshot).contains(" | 7 unranked | "));
        assert!(
            unreadable
                .iter()
                .all(|id| !scene.positions.contains_key(id))
        );
    }

    #[test]
    fn the_demo_drives_the_kill_cycle_and_the_victim_plunges_then_the_next_one_starts() {
        let streaks = |frame: &Frame| {
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Triangle(corners, _, pick) if *pick == NONE && (corners[0][2] - (layer::PLUNGE - 0.5)).abs() < 0.01))
                .count()
        };
        // Before the kill, and the next victim is worker-134, scoring 950.
        let before = demo_frame(19.5).1;
        assert_eq!(spheres_at(&before, layer::PLUNGE), 0);
        // A moment after it exits at 20 s: the ghost eases to the ISCO, still whole.
        let (scene, easing) = demo_frame(20.5);
        assert_eq!(spheres_at(&easing, layer::PLUNGE), 1);
        assert_eq!(streaks(&easing), 0);
        assert!(
            !scene.positions.keys().any(|id| id.pid == 1134),
            "its process is gone"
        );
        // Mid-plunge: a streak, stretched, with a glow.
        let (_, plunging) = demo_frame(23.5);
        assert_eq!(spheres_at(&plunging, layer::PLUNGE), 1);
        assert_eq!(streaks(&plunging), 2, "two triangles");
        // Nearly frozen at 24.8 s, and gone about 6.7 s after the exit, as the light falls by e
        // every 0.4 s of a distant observer's time.
        assert_eq!(spheres_at(&demo_frame(24.8).1, layer::PLUNGE), 1);
        assert_eq!(spheres_at(&demo_frame(27.5).1, layer::PLUNGE), 0);
        // At 26 s a new worker-134 starts, scores 950 again and is the next victim.
        let (scene, _) = demo_frame(27.5);
        let snapshot = demo_snapshot(view("horizon").info(), 26.5, 27.5, 192);
        assert!(
            legend(&scene, &snapshot)
                .starts_with(" next victim worker-134 950 | oom_kill 2 (vmstat)")
        );
        let revived = scene
            .positions
            .keys()
            .find(|id| id.pid == 1134)
            .expect("it is back");
        assert_ne!(revived.start, 135, "with a new start");
    }

    #[test]
    fn an_exit_the_kernel_did_not_count_fades_where_the_process_stood() {
        let horizon = view("horizon");
        let mut scene = Scene::new();
        let sample = demo_snapshot(horizon.info(), 11.0, 12.0, 192);
        scene.render(&RenderInput::small(&sample, horizon, &camera(), 12.0));
        // The second-ranked process exits, with no new OOM kill: it fades, and does not plunge.
        let leaver = scene
            .places
            .get(1)
            .map(|(_, text)| text.split(' ').next().unwrap().to_owned())
            .unwrap();
        let mut gone = demo_snapshot(horizon.info(), 12.0, 13.0, 192);
        gone.processes.retain(|p| p.name != leaver);
        let frame = scene.render(&RenderInput::small(&gone, horizon, &camera(), 13.0));
        assert_eq!(spheres_at(&frame, layer::PLUNGE), 0);
        let faded = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Sphere { pick, center, .. } if *pick == NONE && (center[2] - layer::BODY).abs() < 3.5))
            .count();
        assert_eq!(faded, 1, "one ghost, fading");
        assert!(
            frame
                .items
                .iter()
                .all(|item| !matches!(item, Item::Line(..))),
            "no exit ring"
        );
    }

    #[test]
    fn other_views_still_draw_exit_rings() {
        let (before, after) = (demo(12.0, 64), {
            let mut after = demo(12.0, 64);
            after.processes.remove(20);
            after
        });
        let lines = |frame: &Frame| {
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Line(..)))
                .count()
        };
        for name in ["city", "ripple"] {
            let mut scene = Scene::new();
            let still = scene.render(&RenderInput::small(&before, view(name), &camera(), 1.0));
            let again = scene.render(&RenderInput::small(&before, view(name), &camera(), 1.2));
            let left = scene.render(&RenderInput::small(&after, view(name), &camera(), 1.4));
            assert_eq!(lines(&again), lines(&still), "{name}: nothing exits");
            assert!(
                lines(&left) > lines(&again),
                "{name} draws a ring where a process left"
            );
        }
        // The horizon row switches the scene's ring off: nothing but its own ghosts.
        assert!(!view("horizon").info().flash);
        assert!(
            REGISTRY.iter().take(11).all(|row| row.flash),
            "the eleven original views keep their ring"
        );
    }

    #[test]
    fn the_demo_kill_cycle_only_runs_when_horizon_opts_in() {
        for row in REGISTRY {
            let asks = row.demo.oom_cycle;
            assert_eq!(asks, row.name == "horizon", "{}", row.name);
        }
        let options = view("horizon").info().demo;
        assert!(options.kernel_counters && options.oom_and_memory_detail);
        // The victim, worker-134, is in every other row's demo at 20.5 s and not in horizon's.
        let present = |options: DemoOptions| {
            demo_with(20.5, 192, options)
                .processes
                .iter()
                .any(|process| process.id.pid == 1134)
        };
        assert!(!present(options));
        for row in REGISTRY.iter().filter(|row| row.name != "horizon") {
            assert!(present(row.demo), "{}", row.name);
        }
        assert!(
            demo(20.5, 192)
                .processes
                .iter()
                .any(|process| process.id.pid == 1134)
        );
    }

    #[test]
    fn spheres_stand_where_the_positions_unproject_to_and_pick_their_process() {
        let (scene, frame) = demo_frame(12.0);
        assert!(scene.positions.len() > 30);
        for (id, position) in &scene.positions {
            let pick = frame
                .identities
                .iter()
                .position(|other| other == id)
                .unwrap() as u32;
            let centre = frame
                .items
                .iter()
                .find_map(|item| match *item {
                    Item::Sphere {
                        center,
                        pick: owner,
                        ..
                    } if owner == pick && center[2] >= layer::BODY - 3.5 => Some(center),
                    _ => None,
                })
                .expect("a sphere on the primary image");
            let projected = frame.project(&camera(), *position);
            assert!((projected[0] - centre[0]).abs() < 0.01, "{id:?}");
            assert!((projected[1] - centre[1]).abs() < 0.01, "{id:?}");
        }
        // Each that is not piled into the ring round the shadow also has a fainter sphere on its
        // secondary image, which picks the same process.
        let hole = frame.project(&camera(), [0.0; 3]);
        let scale = camera().zoom * SCALE;
        let secondary: Vec<[f32; 3]> = frame
            .items
            .iter()
            .filter_map(|item| match *item {
                Item::Sphere { center, pick, .. }
                    if pick != NONE && (center[2] - layer::SECONDARY_BODY).abs() < 0.5 =>
                {
                    Some(center)
                }
                _ => None,
            })
            .collect();
        assert!(!secondary.is_empty() && secondary.len() < scene.positions.len());
        for center in secondary {
            let distance = (center[0] - hole[0]).hypot(center[1] - hole[1]) / scale;
            assert!(distance > lens::SHADOW_RADIUS + RING_CLEARANCE - 1e-3);
        }
    }

    #[test]
    fn the_inspector_gives_score_rank_and_orbit() {
        let (scene, _) = demo_frame(12.0);
        let victim = *scene.positions.keys().find(|id| id.pid == 1134).unwrap();
        let lines: Vec<&str> = scene
            .popup
            .iter()
            .filter(|(id, _)| *id == victim)
            .map(|(_, line)| line.as_str())
            .collect();
        assert_eq!(lines.len(), 3);
        assert!(
            lines[0].starts_with("oom_score 950 | oom_score_adj 9"),
            "{lines:?}"
        );
        assert!(lines[1].starts_with("rank 1 of "), "{lines:?}");
        assert_eq!(lines[2], "orbit r = 6.00 M | period 92 M (9.2 s)");
        // An unkillable process says so, and rides far out.
        let unkillable = scene
            .popup
            .iter()
            .find(|(_, line)| line.contains("oom_score_adj -1000"))
            .map(|(id, _)| *id)
            .unwrap();
        let far: Vec<&str> = scene
            .popup
            .iter()
            .filter(|(id, _)| *id == unkillable)
            .map(|(_, line)| line.as_str())
            .collect();
        assert!(far[1].starts_with("unkillable"), "{far:?}");
        let radius: f32 = far[2]["orbit r = ".len()..]
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!((25.7..=27.3).contains(&radius), "{far:?}");
    }

    #[test]
    fn the_top_five_ranks_and_the_first_unkillable_are_labelled() {
        let (scene, _) = demo_frame(12.0);
        let labels: Vec<&str> = scene.places.iter().map(|(_, text)| text.as_str()).collect();
        assert_eq!(labels.len(), 6, "{labels:?}");
        let scores: Vec<i32> = labels[..5]
            .iter()
            .map(|text| text.rsplit(' ').next().unwrap().parse().unwrap())
            .collect();
        assert_eq!(scores[0], 950);
        assert!(
            scores.windows(2).all(|pair| pair[0] >= pair[1] - 5),
            "{scores:?}"
        );
        assert!(labels[5].ends_with(" unkillable"), "{labels:?}");
    }

    #[test]
    fn the_tour_visits_the_next_victim_first_then_the_other_ranks_then_the_unkillable() {
        let (scene, _) = demo_frame(12.0);
        assert_eq!(scene.tour.len(), 28 + 3);
        assert_eq!(scene.tour[0].id.pid, 1134);
        assert!(
            scene.tour[..28]
                .windows(2)
                .all(|pair| pair[0].weight > pair[1].weight)
        );
        assert!(scene.tour[28..].iter().all(|target| target.weight < 1.0));
        for target in &scene.tour {
            assert_eq!(scene.positions.get(&target.id), Some(&target.position));
        }
    }

    #[test]
    fn the_camera_fit_frames_the_disk_whatever_is_hidden() {
        let horizon = view("horizon");
        let sample = demo_snapshot(horizon.info(), 11.0, 12.0, 192);
        let fitted = |input: RenderInput| {
            let mut scene = Scene::new();
            scene.render(&input);
            assert!(scene.bounded());
            let mut camera = input.camera.clone();
            scene.fit(&mut camera, 1920, 1080);
            camera
        };
        let whole = fitted(RenderInput::small(
            &sample,
            horizon,
            &Camera::default(),
            12.0,
        ));
        let limited = fitted(RenderInput {
            limit: 10,
            ..RenderInput::small(&sample, horizon, &Camera::default(), 12.0)
        });
        let focused = fitted(RenderInput {
            focus: Some(sample.processes[16].id),
            ..RenderInput::small(&sample, horizon, &Camera::default(), 12.0)
        });
        assert!(
            limited == whole && focused == whole,
            "the fit does not follow what is hidden"
        );
        // The disk fills most of the width at the default inclination, about 27 px to the M.
        let pixels_per_m = whole.zoom * SCALE;
        assert!((20.0..34.0).contains(&pixels_per_m), "{pixels_per_m}");
        // Tilted face-on, the disk is as tall as it is wide, so the fit zooms out.
        let face_on = fitted(RenderInput::small(
            &sample,
            horizon,
            &Camera {
                pitch: FRAC_PI_2,
                ..Camera::default()
            },
            12.0,
        ));
        assert!(
            face_on.zoom < whole.zoom,
            "{} vs {}",
            face_on.zoom,
            whole.zoom
        );
    }

    #[test]
    fn tilting_the_camera_changes_the_inclination_and_so_the_picture() {
        let horizon = view("horizon");
        let sample = demo_snapshot(horizon.info(), 11.0, 12.0, 192);
        let render = |pitch: f32| {
            let mut scene = Scene::new();
            let camera = Camera { pitch, ..camera() };
            let frame = scene.render(&RenderInput::small(&sample, horizon, &camera, 12.0));
            (legend(&scene, &sample), frame, scene)
        };
        let (edge_on, ..) = render(LOWEST_PITCH);
        let (face_on, frame, scene) = render(FRAC_PI_2);
        assert!(edge_on.ends_with("i = 86 deg") && face_on.ends_with("i = 0 deg"));
        // Face-on the disk is round: ranked bodies of every azimuth stand at their orbit radius
        // from the hole, which here is the centre of the frame's ground.
        let centre = frame.project(
            &Camera {
                pitch: FRAC_PI_2,
                ..camera()
            },
            [0.0; 3],
        );
        assert!(
            frame
                .items
                .iter()
                .any(|item| matches!(item, Item::Sphere { .. }))
        );
        let seen = scene.positions.len();
        assert!(seen > 30 && centre[0] > 0.0);
    }

    #[test]
    fn the_orbits_are_the_same_at_ten_and_sixty_frames_per_second() {
        let horizon = view("horizon");
        let sample = demo_snapshot(horizon.info(), 11.0, 12.0, 192);
        let run = |fps: u32| {
            let mut scene = Scene::new();
            scene.record(&sample, 12.0);
            for step in 0..=3 * fps {
                let time = 12.0 + step as f32 / fps as f32;
                let frame = scene.render(&RenderInput::small(&sample, horizon, &camera(), time));
                scene.spare = frame.release();
            }
            scene.positions.clone()
        };
        let (slow, fast) = (run(10), run(60));
        assert_eq!(slow.len(), fast.len());
        for (id, place) in &slow {
            let other = fast[id];
            assert!(
                (place[0] - other[0]).hypot(place[1] - other[1]) < 0.02,
                "{id:?}"
            );
        }
        // And they moved: three seconds at 10 M a second is a third of a turn at the ISCO.
        let (_, first) = (0, {
            let mut scene = Scene::new();
            scene.record(&sample, 12.0);
            scene.render(&RenderInput::small(&sample, horizon, &camera(), 12.0));
            scene.positions.clone()
        });
        assert!(slow.iter().any(|(id, place)| {
            let start = first[id];
            (place[0] - start[0]).hypot(place[1] - start[1]) > 1.0
        }));
    }

    #[test]
    fn a_rescaled_or_churned_machine_keeps_its_ranks_and_its_orbits() {
        let horizon = view("horizon");
        let sample = demo_snapshot(horizon.info(), 11.0, 12.0, 192);
        let positions = |sample: &Snapshot| {
            let mut scene = Scene::new();
            scene.record(sample, 12.0);
            scene.render(&RenderInput::small(sample, horizon, &camera(), 12.0));
            scene.positions.clone()
        };
        let base = positions(&sample);
        // CPU and memory are not what ranks: nothing moves.
        let rescaled = positions(&rescaled(&sample));
        assert_eq!(base.len(), rescaled.len());
        for (id, place) in &base {
            assert_eq!(rescaled[id], *place, "{id:?}");
        }
        // Processes that exit and start change the ranks of those they pass, no one else's.
        let (churned, gone) = churned(&sample);
        let after = positions(&churned);
        let mut stayed = 0;
        for (id, place) in &base {
            if gone.contains(id) || !after.contains_key(id) {
                continue;
            }
            stayed += usize::from((place[0] - after[id][0]).hypot(place[1] - after[id][1]) < 0.01);
        }
        assert!(
            stayed > 3,
            "the top ranks outside the churn keep their places: {stayed}"
        );
    }

    #[test]
    fn a_process_the_slice_hides_is_neither_drawn_nor_an_exit() {
        let horizon = view("horizon");
        let sample = demo_snapshot(horizon.info(), 11.0, 12.0, 192);
        let mut scene = Scene::new();
        scene.render(&RenderInput::small(&sample, horizon, &camera(), 12.0));
        let shown = scene.positions.len();
        // A tight limit leaves most of the processes out of the slice but in the snapshot.
        let frame = scene.render(&RenderInput {
            limit: 10,
            ..RenderInput::small(&sample, horizon, &camera(), 12.5)
        });
        assert!(scene.positions.len() < shown);
        assert_eq!(spheres_at(&frame, layer::PLUNGE), 0, "no one plunged");
    }

    #[test]
    fn an_empty_machine_and_a_lone_process_draw() {
        let horizon = view("horizon");
        let mut scene = Scene::new();
        let frame = scene.render(&RenderInput::small(
            &Snapshot::default(),
            horizon,
            &camera(),
            1.0,
        ));
        assert!(
            frame.items.len() > 100,
            "the sky, the shadow and the ring remain"
        );
        assert_eq!(scene.visible, 0);
        assert!(legend(&scene, &Snapshot::default()).starts_with(" no process ranked"));
        let lone = Snapshot {
            processes: vec![process(7, 1)],
            ..Snapshot::default()
        };
        let frame = scene.render(&RenderInput::small(&lone, horizon, &camera(), 2.0));
        assert!(frame.picks.is_empty() || scene.positions.len() == 1);
        assert_eq!(scene.positions.len(), 1);
    }
}
