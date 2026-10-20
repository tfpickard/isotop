//! The Light List: processes as lighthouses identified by their time characteristic.
//! Allard's law gives range from memory-derived candela; I/O pressure sets visibility.
//! Towers keep their seats, while a pure waveform drives the lantern, lens and reflection.

use crate::layout::Seats;
use crate::model::{Identity, Kind, Process, Snapshot, top_n};
use crate::palette::{mix, tint};
use crate::platform::Source;
use crate::render::field::{Field, Placement, Relief};
use crate::render::{
    Camera, Color, Frame, Item, ORIGIN_Y, Point, SCALE, Stage, Title, TourTarget, label_name,
};
use crate::simulation::hash_identity;
use crate::status::missing_note;
use crate::views::ViewModule;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::f32::consts::{FRAC_1_SQRT_2, TAU};

const CAP: usize = 64;
const SALT: u64 = 0x6c69_6768_7473;
const PERIODS: [f64; 11] = [2.5, 3.0, 4.0, 5.0, 6.0, 7.5, 10.0, 12.0, 15.0, 20.0, 30.0];
const MILE: f64 = 1852.0;
const THRESHOLD: f64 = 2e-7;
const SEA: Color = [5, 15, 26];
const HAZE: Color = [42, 60, 77];
const REMEMBER: f64 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Extinguished,
    Fixed,
    Occulting,
    Flashing,
    Quick,
    VeryQuick,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Characteristic {
    class: Class,
    count: u32,
    period: f64,
    letter: char,
    color: Color,
    nominal: f64,
    focal: f32,
}

/// Smallest published period at least as long as the desired period and the group.
fn quantize_period(desired: f64, count: u32) -> f64 {
    let minimum = 0.5 * f64::from(count) + f64::from(count - 1) + 2.0;
    PERIODS
        .into_iter()
        .find(|&period| period >= desired.max(minimum))
        .unwrap_or(30.0)
}
/// Allard's law: I T^(d / 1852) / d^2 = 2e-7 lux, returning nautical miles.
fn allard(intensity: f64, transmissivity: f64) -> f64 {
    let mut low = 0.0;
    // Ignoring attenuation gives an upper bound, including for transparent air.
    let mut high = (intensity / THRESHOLD).sqrt();
    for _ in 0..64 {
        let distance = (low + high) * 0.5;
        let illuminance = intensity * transmissivity.powf(distance / MILE) / distance.powi(2);
        if illuminance > THRESHOLD {
            low = distance;
        } else {
            high = distance;
        }
    }
    (low + high) * 0.5 / MILE
}
/// Log-linear candela: 1 MiB is 10 cd and 10 GiB is a million cd.
fn candela(memory: u64) -> f64 {
    let decades = (memory.max(1 << 20) as f64 / 1_048_576.0).log10() / 10240.0_f64.log10();
    10.0_f64.powf(1.0 + 5.0 * decades.clamp(0.0, 1.0))
}
fn characteristic(process: &Process, mean_cpu: f32) -> Characteristic {
    // CPU is percent of one core, not a fractional core.
    let class = match process.state {
        'Z' => Class::Extinguished,
        'T' | 't' => Class::Fixed,
        'D' => Class::Occulting,
        _ if process.cpu >= 200.0 => Class::VeryQuick,
        'R' => Class::Quick,
        _ if process.cpu >= 80.0 => Class::Quick,
        _ => Class::Flashing,
    };
    let count = (process.threads.max(1) as f64).log2().round() as u32 + 1;
    let count = count.clamp(1, 6);
    let period = match class {
        Class::Quick => 1.0,
        Class::VeryQuick => 0.5,
        Class::Fixed | Class::Extinguished => 0.0,
        _ => quantize_period(30.0 / (1.0 + f64::from(mean_cpu.max(0.0)) / 15.0), count),
    };
    let (letter, color) = match process.kind {
        Kind::System => ('W', [245, 239, 207]),
        Kind::Session => ('R', [255, 70, 65]),
        Kind::Container => ('G', [65, 244, 135]),
        Kind::Kernel => ('Y', [255, 211, 64]),
    };
    Characteristic {
        class,
        count,
        period,
        letter,
        color,
        nominal: allard(candela(process.memory), 0.74),
        focal: (8.0 + 12.0 * process.cpu_time.max(0.0).ln_1p() / 10.0_f32.ln()).min(80.0),
    }
}
impl Characteristic {
    fn notation(self) -> String {
        let prefix = match self.class {
            Class::Extinguished => return "extinguished".into(),
            Class::Fixed => "F".into(),
            Class::Occulting => format!("Oc({})", self.count),
            Class::Flashing => format!("Fl({})", self.count),
            Class::Quick => "Q".into(),
            Class::VeryQuick => "VQ".into(),
        };
        if self.class == Class::Fixed {
            format!("{prefix}{}", self.letter)
        } else {
            format!("{prefix}{}.{:.1}s", self.letter, self.period)
        }
    }
    fn full(self) -> String {
        format!(
            "{} {:.0}m {:.0}M",
            self.notation(),
            self.focal,
            self.nominal
        )
    }
    fn words(self, luminous: f64) -> String {
        let (class, rhythm) = match self.class {
            Class::Extinguished => ("extinguished", "no light".into()),
            Class::Fixed => ("fixed", "continuously lit".into()),
            Class::Occulting => (
                "group occulting",
                format!("{} eclipses every {} s", self.count, self.period),
            ),
            Class::Flashing => (
                "group flashing",
                format!("{} every {} s", self.count, self.period),
            ),
            Class::Quick => ("quick", "60 flashes/min".into()),
            Class::VeryQuick => ("very quick", "120 flashes/min".into()),
        };
        let color = match self.letter {
            'R' => "red",
            'G' => "green",
            'Y' => "yellow",
            _ => "white",
        };
        format!(
            "{class}, {rhythm}, {color}, nominal range {:.1} M, luminous range {luminous:.1} M in fog",
            self.nominal
        )
    }
}
fn phase(id: Identity) -> f64 {
    (hash_identity(id, SALT) >> 11) as f64 / ((1_u64 << 53) as f64)
}
/// Pure intensity fraction. Group flashes last 0.5 s, separated by 1 s darkness.
fn waveform(time: f64, light: Characteristic, phase: f64) -> f32 {
    match light.class {
        Class::Extinguished => 0.0,
        Class::Fixed => 1.0,
        Class::Quick | Class::VeryQuick => {
            if (time / light.period + phase).rem_euclid(1.0) < 0.3 {
                1.0
            } else {
                0.0
            }
        }
        Class::Flashing | Class::Occulting => {
            let local = (time + phase * light.period).rem_euclid(light.period);
            let in_group = local < f64::from(light.count) * 1.5 - 1.0;
            let pulse = in_group && local.rem_euclid(1.5) < 0.5;
            if if light.class == Class::Occulting {
                !pulse
            } else {
                pulse
            } {
                1.0
            } else {
                0.0
            }
        }
    }
}
fn visibility(snapshot: &Snapshot) -> f64 {
    if snapshot.missing.contains(Source::IoPressure) {
        20.0
    } else {
        20.0 / (1.0 + f64::from(snapshot.pressure[2].max(0.0)) / 5.0)
    }
}
fn luminous_range(process: &Process, snapshot: &Snapshot) -> f64 {
    let transmission = if snapshot.missing.contains(Source::IoPressure) {
        0.74
    } else {
        0.05_f64.powf(1.0 / visibility(snapshot))
    };
    allard(candela(process.memory), transmission)
}
fn fog(snapshot: &Snapshot) -> f32 {
    if snapshot.missing.contains(Source::IoPressure) {
        0.0
    } else {
        let pressure = snapshot.pressure[2].max(0.0);
        pressure / (pressure + 15.0)
    }
}
fn house(process: &Process) -> &str {
    process
        .cgroup
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(&process.group)
}
/// Across the coast and towards the default viewer, in world coordinates.
fn world(across: f32, along: f32, height: f32) -> Point {
    [
        (across + along) * FRAC_1_SQRT_2,
        (along - across) * FRAC_1_SQRT_2,
        height,
    ]
}
fn headland(index: usize) -> (f32, f32) {
    let column = if index == 0 {
        0.0
    } else if index % 2 == 1 {
        -(index.div_ceil(2) as f32)
    } else {
        (index / 2) as f32
    };
    let across = column * 13.0;
    (across, -8.0 - 0.002 * across * across)
}
struct Lamp<'a> {
    process: &'a Process,
    pick: u32,
    position: Point,
    light: Characteristic,
    range: f64,
    strength: f32,
}
/// A lens turns once per characteristic period. Uneven lens spacing leaves the group eclipse.
fn beam_angle(time: f64, light: Characteristic, phase: f64, beam: u32) -> f32 {
    let midpoint = if light.class == Class::Flashing {
        0.25 + 1.5 * f64::from(beam)
    } else {
        light.period * 0.15
    };
    (std::f64::consts::TAU * ((time / light.period + phase) - midpoint / light.period)) as f32
}
/// Soft Gaussian wedges in the shared colour field, composed once for both backends.
fn sweep(field: &mut Field, size: [f32; 2], origin: Point, end: Point, color: Color, density: f32) {
    let delta = [end[0] - origin[0], end[1] - origin[1]];
    let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
    let margin = size[1] * 0.08;
    let scale = [
        (field.columns - 1) as f32 / size[0],
        (field.rows - 1) as f32 / size[1],
    ];
    let left = ((origin[0].min(end[0]) - margin) * scale[0]).max(0.0) as usize;
    let right =
        ((origin[0].max(end[0]) + margin) * scale[0]).min((field.columns - 1) as f32) as usize;
    let top = ((origin[1].min(end[1]) - margin) * scale[1]).max(0.0) as usize;
    let bottom =
        ((origin[1].max(end[1]) + margin) * scale[1]).min((field.rows - 1) as f32) as usize;
    for row in top..=bottom {
        for column in left..=right {
            let offset = [
                column as f32 / scale[0] - origin[0],
                row as f32 / scale[1] - origin[1],
            ];
            let t = (offset[0] * delta[0] + offset[1] * delta[1]) / length_squared;
            if !(0.0..1.0).contains(&t) {
                continue;
            }
            let distance_squared =
                (offset[0] - t * delta[0]).powi(2) + (offset[1] - t * delta[1]).powi(2);
            let spread = (2.0 + t * 18.0) * size[1] / 900.0;
            let scatter = (0.12 + density * 0.24)
                * (1.0 - t).powf(0.7)
                * (-distance_squared / (2.0 * spread * spread)).exp();
            let index = row * field.columns + column;
            field.colors[index] = mix(field.colors[index], color, scatter);
        }
    }
}
/// `back` is the depth of the screen-space atmosphere, just behind the coast.
fn night(
    stage: &mut Stage,
    field: &mut Field,
    coast: [f32; 2],
    back: f32,
    lamps: &[Lamp<'_>],
    density: f32,
) {
    let [reach, far] = coast;
    let width = stage.frame.width as f32;
    let height = stage.frame.height as f32;
    let columns = (stage.frame.width as usize / 4 + 1).clamp(2, 512);
    let rows = (stage.frame.height as usize / 4 + 1).clamp(2, 512);
    field.columns = columns;
    field.rows = rows;
    field.colors.resize(columns * rows, SEA);
    for row in 0..rows {
        let y = height * row as f32 / (rows - 1) as f32;
        for column in 0..columns {
            let x = width * column as f32 / (columns - 1) as f32;
            let ground = stage.camera.ground(x - width * 0.5, y - height * ORIGIN_Y);
            let wx = ground[0] + stage.camera.center[0];
            let wy = ground[1] + stage.camera.center[1];
            let across = (wx - wy) * FRAC_1_SQRT_2;
            let along = (wx + wy) * FRAC_1_SQRT_2;
            let mut color = if along < far - 22.0 {
                mix(
                    [2, 5, 14],
                    [22, 43, 66],
                    (y / (height * 0.4)).min(1.0).powi(2),
                )
            } else {
                let swell = ((along * 2.4 + across * 0.25 - stage.time * 0.7).sin()
                    * (across * 0.31 + along * 0.7).cos()
                    * 0.5
                    + 0.5)
                    .powi(3);
                mix(SEA, [27, 49, 66], swell * (0.18 + 0.4 * y / height))
            };
            let ridge_height =
                2.0 + 2.7 * (across * 0.11).sin().abs() + 0.7 * (across * 0.37).sin();
            let ridge = far - 11.0 - ridge_height / stage.camera.pitch.tan();
            let shore = -5.5 - 0.002 * across * across + 1.5 * (across * 0.42).sin();
            if across.abs() < reach && along > ridge && along < shore {
                color = mix(
                    [7, 16, 22],
                    [17, 29, 35],
                    ((along - ridge) / (shore - ridge)).clamp(0.0, 1.0) * 0.55,
                );
                let rim = (-(along - ridge).powi(2) / 2.0).exp();
                color = mix(color, [34, 51, 62], rim * 0.4);
            }
            let bank = (-(along - far + 5.0).powi(2) / 100.0).exp() * (0.08 + density * 0.3);
            field.colors[row * columns + column] = mix(color, HAZE, bank);
        }
    }
    let viewer = stage.camera.viewer();
    for lamp in lamps {
        if !matches!(
            lamp.light.class,
            Class::Flashing | Class::Quick | Class::VeryQuick
        ) {
            continue;
        }
        let lantern = [
            lamp.position[0],
            lamp.position[1],
            lamp.position[2] + lamp.light.focal / 5.0 + 0.5,
        ];
        let origin = stage.frame.project(stage.camera, lantern);
        let count = if lamp.light.class == Class::Flashing {
            lamp.light.count
        } else {
            1
        };
        for beam in 0..count {
            let angle = viewer[1].atan2(viewer[0])
                + beam_angle(
                    f64::from(stage.time),
                    lamp.light,
                    phase(lamp.process.id),
                    beam,
                );
            let length = 8.0 + lamp.range as f32 * 2.8;
            let end = stage.frame.project(
                stage.camera,
                [
                    lantern[0] + length * angle.cos(),
                    lantern[1] + length * angle.sin(),
                    lantern[2] + length * 0.025,
                ],
            );
            sweep(
                field,
                [width, height],
                origin,
                end,
                lamp.light.color,
                density,
            );
        }
    }
    field.revision = stage.frames;
    let first = stage.frame.items.len();
    stage.frame.field(
        stage.camera,
        Placement::Corners([
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]),
        field,
        Relief::Flat,
    );
    // The atmosphere is a screen-space panel behind all world geometry. Preserve the
    // shared field's node coordinates so its bilinear sampling remains backend-identical.
    for item in &mut stage.frame.items[first..] {
        if let Item::Field { points, uv, .. } = item {
            for (point, node) in points.iter_mut().zip(uv.iter()) {
                *point = [
                    node[0] / (columns - 1) as f32 * width,
                    node[1] / (rows - 1) as f32 * height,
                    back,
                ];
            }
        }
    }
    // Irrational strides make a reproducible star field without a view-local RNG.
    for star in 0..260 {
        let x = (star as f32 * 0.618_034).fract() * width;
        let y = (star as f32 * 0.414_213_57 + 0.13).fract() * height * 0.26;
        stage.frame.items.push(Item::Puff {
            center: [x, y],
            radius: (1.3 + (star % 7) as f32 * 0.15) * height / 900.0,
            color: [170, 194, 218],
            strength: 0.6,
        });
    }
}
#[derive(Clone)]
struct Notice {
    at: f64,
    text: String,
}
struct Exit {
    at: f64,
    position: Point,
    light: Characteristic,
}
/// Where every sampled process stands: its headland and its seat on it.
#[derive(Clone, Default)]
struct Seating {
    seats: Seats,
    houses: BTreeMap<String, usize>,
}
/// Every process is seated, including processes hidden by memory admission, focus or limit.
#[derive(Default)]
pub struct Lights {
    placement: Seating,
    seen: HashMap<Identity, (Process, Point)>,
    born: HashMap<Identity, f64>,
    exits: VecDeque<Exit>,
    notices: VecDeque<Notice>,
    last: Option<f64>,
    drawn: usize,
    collapsed: usize,
    weather: Option<Field>,
}
impl Seating {
    fn seat(&mut self, snapshot: &Snapshot) {
        self.seats.assign(
            snapshot
                .processes
                .iter()
                .map(|process| (process.id, house(process))),
        );
        let names: BTreeSet<&str> = snapshot.processes.iter().map(house).collect();
        self.houses.retain(|name, _| names.contains(name.as_str()));
        let mut occupied: BTreeSet<usize> = self.houses.values().copied().collect();
        for name in names {
            if !self.houses.contains_key(name) {
                let mut index = 0;
                while occupied.contains(&index) {
                    index += 1;
                }
                occupied.insert(index);
                self.houses.insert(name.to_owned(), index);
            }
        }
    }
    fn position(&self, process: &Process) -> Point {
        let (across, along) = headland(self.houses[house(process)]);
        let seat = self
            .seats
            .seat(process.id)
            .expect("every sampled process is seated");
        world(
            across + (seat % 4) as f32 * 2.3 - 3.45,
            along - (seat / 4) as f32 * 2.8,
            0.8,
        )
    }
}
impl Lights {
    /// The newest exits still fading out at the stage's time, at most eight, none while focused.
    fn fading<'a>(&'a self, stage: &Stage) -> impl Iterator<Item = &'a Exit> + use<'a> {
        let time = f64::from(stage.time);
        let shown = if stage.focus.is_none() {
            8.min(CAP).min(stage.limit)
        } else {
            0
        };
        self.exits
            .iter()
            .rev()
            .filter(move |exit| {
                exit.light.class != Class::Extinguished && (0.0..1.5).contains(&(time - exit.at))
            })
            .take(shown)
    }
    fn notice(&mut self, process: &Process, at: f64, event: &str) {
        self.notices.push_back(Notice {
            at,
            text: format!(
                "NM: {} {} {event}",
                label_name(&process.name),
                characteristic(process, process.cpu).notation()
            ),
        });
    }
}
fn coast(
    stage: &mut Stage,
    snapshot: &Snapshot,
    lamps: &[Lamp<'_>],
    placement: &Seating,
    weather: &mut Option<Field>,
) -> f32 {
    let live: BTreeSet<&str> = snapshot.processes.iter().map(house).collect();
    let width = live
        .iter()
        .map(|name| headland(placement.houses[*name]).0.abs() + 8.0)
        .fold(22.0, f32::max);
    let far = live
        .iter()
        .map(|name| {
            headland(placement.houses[*name]).1
                - placement.seats.span(name).div_ceil(4) as f32 * 2.8
                - 3.0
        })
        .fold(-18.0, f32::min);
    // Reserve foreground rather than empty sky; admission never changes this envelope.
    stage.bounds.extend([
        world(-width, far, 18.0),
        world(width, far, 18.0),
        world(-width, 95.0, -0.1),
        world(width, 95.0, -0.1),
    ]);
    // Screen-space layers sit just outside the depths of the coast's box, because the GPU
    // normalizes depth over every vertex: a far-off layer would crush the scene's depths.
    let depths: Vec<f32> = [-width, width]
        .into_iter()
        .flat_map(|across| [far, 95.0].map(move |along| (across, along)))
        .flat_map(|(across, along)| [-0.1, 18.0].map(move |height| world(across, along, height)))
        .map(|corner| stage.frame.project(stage.camera, corner)[2])
        .collect();
    let back = depths.iter().copied().fold(f32::INFINITY, f32::min) - 1.0;
    let front = depths.iter().copied().fold(f32::NEG_INFINITY, f32::max) + 1.0;
    let field = weather.get_or_insert_with(|| Field::new(SALT, 2, 2));
    night(stage, field, [width, far], back, lamps, fog(snapshot));
    for name in live {
        let (across, along) = headland(placement.houses[name]);
        let depth = placement.seats.span(name).div_ceil(4).max(2) as f32 * 2.8;
        let shore = |u: f32| world(u, -5.5 - 0.002 * u * u + 1.5 * (u * 0.42).sin(), 0.10);
        for side in 0..8 {
            let a = across - 6.5 + side as f32 * 1.625;
            stage
                .frame
                .hairline(stage.camera, shore(a), shore(a + 1.625), [34, 51, 62], 0.8);
        }
        stage.titles.push(Title {
            at: world(across, along - depth * 0.5, 0.0),
            reach: 5.5,
            text: label_name(name).into_owned(),
        });
    }
    front
}
/// An eight-sided tapered, banded tower and its pickable lantern room.
fn tower(
    frame: &mut Frame,
    camera: &Camera,
    process: &Process,
    position: Point,
    light: Characteristic,
    pick: u32,
) {
    let height = light.focal / 5.0;
    let at = |angle: f32, radius: f32, z: f32| {
        [
            position[0] + radius * angle.cos(),
            position[1] + radius * angle.sin(),
            position[2] + z,
        ]
    };
    for band in 0..4 {
        let low = height * band as f32 / 4.0;
        let high = height * (band + 1) as f32 / 4.0;
        let stripe = match (process.kind, band % 2) {
            (Kind::Session, 1) => [151, 53, 49],
            (Kind::Container | Kind::Kernel, 1) => [26, 32, 39],
            _ => [159, 171, 170],
        };
        for side in 0..8 {
            let a = side as f32 * TAU / 8.0;
            let b = (side + 1) as f32 * TAU / 8.0;
            let shade = 0.38 + 0.38 * (a - 0.4).cos().max(0.0);
            let radius = |z: f32| 0.66 - 0.25 * z / height;
            frame.quad(
                camera,
                [
                    at(a, radius(low), low),
                    at(b, radius(low), low),
                    at(b, radius(high), high),
                    at(a, radius(high), high),
                ],
                tint(stripe, shade),
                pick,
            );
        }
    }
    // The lantern room is open glazing between the gallery floor and the roof, so the coloured
    // lamp drawn at its centre shows through the posts.
    for side in 0..8 {
        let a = side as f32 * TAU / 8.0;
        let b = (side + 1) as f32 * TAU / 8.0;
        frame.facet(
            camera,
            [
                at(a, 0.74, height),
                at(b, 0.74, height),
                at(0.0, 0.0, height),
            ],
            [46, 54, 64],
            pick,
        );
        frame.facet(
            camera,
            [
                at(a, 0.74, height + 0.95),
                at(b, 0.74, height + 0.95),
                at(0.0, 0.0, height + 1.4),
            ],
            [48, 61, 72],
            pick,
        );
        frame.hairline(
            camera,
            at(a, 0.52, height),
            at(a, 0.52, height + 0.95),
            [75, 90, 100],
            0.8,
        );
    }
}
/// The atmosphere's colour at a frame point, from its nearest node.
fn water(frame: &Frame, point: Point) -> Color {
    let field = &frame.fields[0];
    let column = (point[0] / frame.width as f32 * (field.columns - 1) as f32)
        .round()
        .clamp(0.0, (field.columns - 1) as f32) as usize;
    let row = (point[1] / frame.height as f32 * (field.rows - 1) as f32)
        .round()
        .clamp(0.0, (field.rows - 1) as f32) as usize;
    let texel = field.texels[row * field.columns + column].to_le_bytes();
    [texel[0], texel[1], texel[2]]
}
/// `front` is the depth of the star core, just before the coast.
fn shine(
    stage: &mut Stage,
    position: Point,
    light: Characteristic,
    strength: f32,
    range: f64,
    front: f32,
) {
    if light.class == Class::Extinguished {
        return;
    }
    let lantern = [
        position[0],
        position[1],
        position[2] + light.focal / 5.0 + 0.5,
    ];
    let at = stage.frame.project(stage.camera, lantern);
    let scale = stage.frame.height as f32 / 900.0;
    let radius = (22.0 + range as f32 * 3.0).min(115.0) * scale;
    if strength > 0.0 {
        for (size, weight) in [(radius, 0.55), (radius * 0.34, 0.65), (4.0 * scale, 1.0)] {
            stage.frame.items.push(Item::Puff {
                center: [at[0], at[1]],
                radius: size,
                color: light.color,
                strength: (strength * weight).min(1.0),
            });
        }
        // The star core is geometry, not a backend-dependent one-pixel beam. Opaque, it fades
        // with the light by mixing towards the atmosphere behind it.
        let core = mix(
            water(stage.frame, at),
            mix(light.color, [255, 255, 241], 0.6),
            strength.min(1.0),
        );
        stage.frame.hairline_between(
            [at[0] - 8.0 * scale, at[1], front],
            [at[0] + 8.0 * scale, at[1], front],
            core,
            scale,
        );
        stage.frame.hairline_between(
            [at[0], at[1] - 5.0 * scale, front],
            [at[0], at[1] + 5.0 * scale, front],
            core,
            scale,
        );
    }
    let viewer = stage.camera.viewer();
    let length = viewer[0].hypot(viewer[1]);
    let direction = [viewer[0] / length, viewer[1] / length];
    // Glitter is decorative, but its brightness follows the observed flash. Mix against
    // local water illumination, so a dim reflection cannot cut black stripes into a beam.
    for step in 0..52 {
        let t = step as f32 / 52.0;
        let distance = 5.0 + t * (12.0 + range as f32 * 0.8);
        let center = [
            position[0] + distance * direction[0],
            position[1] + distance * direction[1],
            0.02,
        ];
        let shimmer = (step as f32 * 2.39 + stage.time * 1.8 + position[0]).sin() * 0.5 + 0.5;
        let half = (0.12 + t * 1.1) * (0.25 + shimmer * 0.75);
        let water = water(stage.frame, stage.frame.project(stage.camera, center));
        stage.frame.hairline(
            stage.camera,
            [
                center[0] + half * direction[1],
                center[1] - half * direction[0],
                center[2],
            ],
            [
                center[0] - half * direction[1],
                center[1] + half * direction[0],
                center[2],
            ],
            mix(
                water,
                light.color,
                (0.025 + 0.45 * strength) * (1.0 - t) * (0.4 + shimmer * 0.6),
            ),
            (stage.camera.zoom * SCALE * 0.04).clamp(0.8, 2.0),
        );
    }
}
impl ViewModule for Lights {
    fn record(&mut self, snapshot: &Snapshot, _time: f32) {
        if self.last.is_some_and(|last| snapshot.elapsed <= last) {
            return;
        }
        let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        let mut departed: Vec<Identity> = self
            .seen
            .keys()
            .filter(|id| !alive.contains(id))
            .copied()
            .collect();
        departed.sort();
        for id in departed {
            let (process, position) = self.seen.remove(&id).expect("departed identity was seen");
            self.exits.push_back(Exit {
                at: snapshot.elapsed,
                position,
                light: characteristic(&process, process.cpu),
            });
            self.notice(&process, snapshot.elapsed, "extinguished");
        }
        self.placement.seat(snapshot);
        let mut processes: Vec<&Process> = snapshot.processes.iter().collect();
        processes.sort_by_key(|p| p.id);
        for process in processes {
            if self.last.is_some() && !self.seen.contains_key(&process.id) {
                self.born.insert(process.id, snapshot.elapsed);
                self.notice(process, snapshot.elapsed, "established");
            }
            self.seen.insert(
                process.id,
                (process.clone(), self.placement.position(process)),
            );
        }
        self.born.retain(|id, _| alive.contains(id));
        self.exits
            .retain(|exit| exit.at >= snapshot.elapsed - REMEMBER);
        self.notices
            .retain(|notice| notice.at >= snapshot.elapsed - REMEMBER);
        while self.exits.len() > 1024 {
            self.exits.pop_front();
        }
        while self.notices.len() > 1024 {
            self.notices.pop_front();
        }
        self.last = Some(snapshot.elapsed);
    }
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let rewound: Seating;
        let placement = if self.last.is_some_and(|last| snapshot.elapsed < last) {
            // A rewound sample is seated on a copy, so the seats and headlands kept for the
            // newest sample are not released and handed out again when time resumes.
            let mut copy = self.placement.clone();
            copy.seat(snapshot);
            rewound = copy;
            &rewound
        } else {
            self.placement.seat(snapshot);
            &self.placement
        };
        let time = f64::from(stage.time);
        let budget = CAP.min(stage.limit) - self.fading(stage).count();
        let candidates: Vec<Process> = processes.iter().map(|p| (*p).clone()).collect();
        let mut admitted: HashSet<Identity> = top_n(&candidates, budget, |p| p.memory as f64)
            .into_iter()
            .map(|p| p.id)
            .collect();
        // As the scene's own admission does, the selection keeps its light: it takes the slot
        // of the smallest admitted light rather than going dark.
        if let Some(id) = stage.selected
            && budget > 0
            && !admitted.contains(&id)
            && processes.iter().any(|p| p.id == id)
        {
            admitted = top_n(&candidates, budget - 1, |p| p.memory as f64)
                .into_iter()
                .map(|p| p.id)
                .chain([id])
                .collect();
        }
        let lamps: Vec<Lamp<'_>> = processes
            .iter()
            .enumerate()
            .filter(|(_, process)| admitted.contains(&process.id))
            .map(|(index, process)| {
                let cpus: Vec<f32> = stage
                    .history
                    .window(snapshot.elapsed, 30.0)
                    .filter_map(|sample| sample.cpu(process.id))
                    .collect();
                let mean = if cpus.is_empty() {
                    process.cpu
                } else {
                    cpus.iter().sum::<f32>() / cpus.len() as f32
                };
                let light = characteristic(process, mean);
                let age = self.born.get(&process.id).map(|at| time - at);
                let established = age
                    .filter(|age| (0.0..1.5).contains(age))
                    .map_or(1.0, |age| {
                        1.0 + (std::f64::consts::PI * age / 1.5).sin() as f32 * 0.6
                    });
                Lamp {
                    process,
                    pick: index as u32,
                    position: placement.position(process),
                    light,
                    range: luminous_range(process, snapshot),
                    strength: waveform(time, light, phase(process.id)) * established,
                }
            })
            .collect();
        let front = coast(stage, snapshot, &lamps, placement, &mut self.weather);
        for lamp in &lamps {
            let Lamp {
                process,
                position,
                light,
                range,
                strength,
                pick,
            } = *lamp;
            let lantern = [
                position[0],
                position[1],
                position[2] + light.focal / 5.0 + 0.5,
            ];
            tower(stage.frame, stage.camera, process, position, light, pick);
            let interior = if light.class == Class::Extinguished {
                [12, 17, 22]
            } else {
                mix([46, 30, 16], light.color, strength.min(1.0))
            };
            stage.frame.sphere(
                stage.camera,
                lantern,
                0.26,
                interior,
                pick,
                stage.highlight.contains(&process.id),
            );
            shine(stage, position, light, strength, range, front);
            stage.positions.insert(process.id, position);
            stage.tour.push(TourTarget {
                id: process.id,
                position: lantern,
                weight: light.nominal as f32,
            });
            if stage.selected == Some(process.id) || stage.toured == Some(process.id) {
                stage.popup.push((process.id, light.full()));
                stage.popup.push((process.id, light.words(range)));
                let notices = self
                    .notices
                    .iter()
                    .rev()
                    .filter(|notice| notice.at <= snapshot.elapsed)
                    .take(3)
                    .map(|notice| notice.text.clone())
                    .collect();
                stage.notes.insert(process.id, notices);
                stage.callout.push((process.id, light.full()));
            }
        }
        let fading: Vec<&Exit> = self.fading(stage).collect();
        for exit in fading {
            shine(
                stage,
                exit.position,
                exit.light,
                (1.0 - (time - exit.at) / 1.5) as f32,
                exit.light.nominal,
                front,
            );
        }
        stage
            .tour
            .sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
        self.drawn = lamps.len();
        self.collapsed = snapshot.processes.len().saturating_sub(self.drawn);
        self.drawn
    }
    fn legend(&self, snapshot: &Snapshot) -> String {
        let missing = missing_note(snapshot, Source::IoPressure, "clear weather, nominal range")
            .unwrap_or_default();
        let groups = missing_note(snapshot, Source::Cgroups, "headlands from process groups")
            .unwrap_or_default();
        let notices: String = self
            .notices
            .iter()
            .rev()
            .filter(|notice| notice.at <= snapshot.elapsed)
            .take(3)
            .map(|notice| format!(" | {}", notice.text))
            .collect();
        format!(
            " {} lights, {} collapsed | fog visibility {:.1} M{missing}{groups} | Fl(n) = threads | period = CPU | colour = kind | range = memory (Allard) | height = CPU time{notices}",
            self.drawn,
            self.collapsed,
            visibility(snapshot)
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::process;
    use crate::render::{Backdrop, RenderInput, Scene, hiding};
    use crate::views::{View, ViewInfo, view};
    static ROWS: &[ViewInfo] = &[ViewInfo::new("lights", "LIGHTS", Backdrop::Void, || {
        Box::new(Lights::default())
    })];
    #[test]
    fn allard_solves_a_hand_computed_ten_mile_light() {
        // At 10 M: d=18520 m, .74^10=.0492399039735587;
        // I=2e-7 * 342990400 / .0492399039735587 = 1393.140002763 cd.
        assert!((allard(1393.140002763, 0.74) - 10.0).abs() < 1e-8);
        assert!((candela(1 << 20) - 10.0).abs() < 1e-8);
        assert!((candela(10 << 30) - 1e6).abs() < 1e-8);
    }
    #[test]
    fn states_take_precedence_over_cpu_when_assigning_characteristics() {
        for (state, cpu, class) in [
            ('Z', 400.0, Class::Extinguished),
            ('T', 400.0, Class::Fixed),
            ('D', 400.0, Class::Occulting),
            ('R', 0.0, Class::Quick),
            ('R', 200.0, Class::VeryQuick),
            ('S', 80.0, Class::Quick),
            ('S', 79.0, Class::Flashing),
        ] {
            let process = Process {
                state,
                cpu,
                ..process(1, 0)
            };
            assert_eq!(characteristic(&process, cpu).class, class);
        }
    }
    #[test]
    fn a_group_has_exactly_its_thread_coded_number_of_flashes_per_period() {
        for count in 1..=6 {
            let light = characteristic(
                &Process {
                    threads: 1 << (count - 1),
                    state: 'S',
                    cpu: 0.0,
                    ..process(1, 0)
                },
                0.0,
            );
            assert_eq!(light.count, count);
            let samples = (light.period * 100.0) as usize;
            let mut previous = waveform(-0.01, light, 0.0);
            let mut flashes = 0;
            for step in 0..samples {
                let intensity = waveform(step as f64 / 100.0, light, 0.0);
                flashes += usize::from(intensity > previous);
                previous = intensity;
            }
            assert_eq!(flashes, count as usize);
            assert_eq!(waveform(light.period - 2.0, light, 0.0), 0.0);
        }
    }
    #[test]
    fn rotating_lenses_face_the_viewer_at_each_flash_midpoint() {
        for (state, cpu) in [('S', 0.0), ('R', 90.0), ('R', 250.0)] {
            let light = characteristic(
                &Process {
                    state,
                    cpu,
                    threads: 32,
                    ..process(1, 0)
                },
                cpu,
            );
            let count = if light.class == Class::Flashing {
                light.count
            } else {
                1
            };
            for beam in 0..count {
                let midpoint = if light.class == Class::Flashing {
                    0.25 + 1.5 * f64::from(beam)
                } else {
                    light.period * 0.15
                };
                let time = midpoint - 0.17 * light.period;
                assert!(beam_angle(time, light, 0.17, beam).abs() < 1e-5);
                assert_eq!(waveform(time, light, 0.17), 1.0);
                assert!(
                    (beam_angle(time + light.period * 0.25, light, 0.17, beam) - TAU * 0.25).abs()
                        < 1e-5
                );
            }
        }
    }
    #[test]
    fn periods_use_published_values_and_leave_room_for_the_whole_group() {
        assert_eq!(quantize_period(2.6, 1), 3.0);
        assert_eq!(quantize_period(7.0, 2), 7.5);
        assert_eq!(quantize_period(2.5, 6), 10.0);
        for count in 1..=6 {
            for desired in [0.0, 3.1, 6.1, 10.1, 29.0] {
                let period = quantize_period(desired, count);
                assert!(PERIODS.contains(&period));
                assert!(period >= f64::from(count) * 1.5 + 1.0);
            }
        }
    }
    #[test]
    fn lights_keep_their_seats_when_cpu_memory_and_population_change() {
        let mut snapshot = crate::model::demo(12.0, 192);
        let mut lights = Lights::default();
        lights.placement.seat(&snapshot);
        let before: HashMap<_, _> = snapshot
            .processes
            .iter()
            .map(|p| (p.id, lights.placement.position(p)))
            .collect();
        for process in &mut snapshot.processes {
            process.cpu *= 4.0;
            process.memory *= 2;
        }
        snapshot.processes.remove(3);
        snapshot.processes.push(process(9999, 1));
        lights.placement.seat(&snapshot);
        for process in &snapshot.processes {
            if let Some(position) = before.get(&process.id) {
                assert_eq!(*position, lights.placement.position(process));
            }
        }
        // These shared checks compare only identities visible before and after hiding.
        // Use a fixture within the cap; the seat checks above include hidden lights.
        let visible = crate::model::demo(12.0, CAP);
        hiding::focus_and_clear(view("lights"), &visible);
        hiding::limit_and_lift(view("lights"), &visible);
        hiding::bounds_ignore_hiding(view("lights"), &visible);
        // The full sampled envelope also stays fixed when admission hides most lights.
        let mut scene = Scene::new();
        let view = view("lights");
        hiding::place(&mut scene, &snapshot, view, 12.0, 4096, None, None);
        let mut camera = Camera::default();
        scene.fit(&mut camera, 160, 90);
        let fit = camera.clone();
        let root = snapshot.processes[16].id;
        hiding::place(&mut scene, &snapshot, view, 12.0, 4096, None, Some(root));
        scene.fit(&mut camera, 160, 90);
        assert!(camera == fit);
        hiding::place(&mut scene, &snapshot, view, 12.0, 32, None, None);
        scene.fit(&mut camera, 160, 90);
        assert!(camera == fit);
    }
    #[test]
    fn the_waveform_and_render_are_deterministic_at_any_frame_rate() {
        let snapshot = crate::model::demo(12.0, 64);
        let camera = Camera::default();
        let render = || {
            let mut scene = Scene::with_rows(ROWS);
            scene.record(&snapshot, 12.0);
            let mut frame =
                scene.render(&RenderInput::small(&snapshot, View::at(0), &camera, 12.0));
            frame.rasterize();
            (frame.pixels, frame.picks)
        };
        assert_eq!(render(), render());
        let light = characteristic(&snapshot.processes[3], 0.0);
        for second in 0..30 {
            assert_eq!(
                waveform(f64::from(second * 10) / 10.0, light, 0.17),
                waveform(f64::from(second * 60) / 60.0, light, 0.17)
            );
        }
    }
    #[test]
    fn fog_reduces_range_and_unreadable_pressure_uses_nominal_range() {
        let process = Process {
            memory: 1 << 30,
            ..process(1, 0)
        };
        let mut snapshot = Snapshot::default();
        let clear = luminous_range(&process, &snapshot);
        snapshot.pressure[2] = 80.0;
        assert!(luminous_range(&process, &snapshot) < clear);
        snapshot.missing.insert(Source::IoPressure);
        assert_eq!(
            luminous_range(&process, &snapshot),
            characteristic(&process, 0.0).nominal
        );
    }
    #[test]
    fn sample_diff_records_establishment_and_extinction_once_per_identity() {
        let mut lights = Lights::default();
        let mut snapshot = Snapshot {
            elapsed: 1.0,
            processes: vec![process(1, 0)],
            ..Snapshot::default()
        };
        lights.record(&snapshot, 1.0);
        assert!(lights.notices.is_empty());
        snapshot.elapsed = 2.0;
        snapshot.processes.push(process(2, 1));
        lights.record(&snapshot, 2.0);
        lights.record(&snapshot, 2.0);
        assert_eq!(lights.notices.len(), 1);
        let old = lights.placement.position(&snapshot.processes[1]);
        snapshot.elapsed = 3.0;
        snapshot.processes.pop();
        lights.record(&snapshot, 3.0);
        assert_eq!(lights.notices.len(), 2);
        assert_eq!(lights.exits[0].position, old);
    }
    #[test]
    fn the_selected_process_keeps_a_light_when_memory_admission_is_full() {
        let snapshot = crate::model::demo(12.0, 192);
        let smallest = snapshot
            .processes
            .iter()
            .min_by_key(|p| (p.memory, p.id))
            .expect("the demo has processes")
            .id;
        let mut scene = Scene::new();
        let view = view("lights");
        let unselected = hiding::place(&mut scene, &snapshot, view, 12.0, 4096, None, None);
        assert!(!unselected.contains_key(&smallest));
        let mut scene = Scene::new();
        let selected = hiding::place(
            &mut scene,
            &snapshot,
            view,
            12.0,
            4096,
            Some(smallest),
            None,
        );
        assert!(selected.contains_key(&smallest));
        assert_eq!(selected.len(), CAP);
    }
    #[test]
    fn drawing_a_rewound_sample_leaves_the_seats_of_the_newest_alone() {
        let sample = |elapsed: f64, pids: &[u32]| Snapshot {
            elapsed,
            processes: pids.iter().map(|&pid| process(pid, 1)).collect(),
            ..Snapshot::default()
        };
        let older = sample(1.0, &[10, 30]);
        let newest = sample(2.0, &[30, 40]);
        let mut scene = Scene::with_rows(ROWS);
        scene.record(&older, 1.0);
        scene.record(&newest, 2.0);
        let view = View::at(0);
        let before = hiding::place(&mut scene, &newest, view, 2.0, 4096, None, None);
        hiding::place(&mut scene, &older, view, 1.0, 4096, None, None);
        // A newcomer with a smaller identity than 40 would take 40's seat had the rewound
        // frame given that seat back to 10.
        let next = sample(3.0, &[20, 30, 40]);
        scene.record(&next, 3.0);
        let after = hiding::place(&mut scene, &next, view, 3.0, 4096, None, None);
        for (id, position) in &before {
            assert_eq!(after[id], *position);
        }
    }
    #[test]
    fn screen_space_layers_keep_their_depths_within_the_coast() {
        let snapshot = crate::model::demo(12.0, 192);
        let mut scene = Scene::with_rows(ROWS);
        scene.record(&snapshot, 12.0);
        let camera = Camera::default();
        let frame = scene.render(&RenderInput::small(&snapshot, View::at(0), &camera, 12.0));
        // The depths the GPU normalizes over, as `Gpu::render` tracks them.
        let mut depths = Vec::new();
        for item in &frame.items {
            match *item {
                Item::Triangle(points, ..) => depths.extend(points.map(|p| p[2])),
                Item::Field { points, .. } => depths.extend(points.map(|p| p[2])),
                Item::Sphere { center, depth, .. } => {
                    depths.extend([center[2] - depth, center[2] + depth]);
                }
                _ => {}
            }
        }
        let low = depths.iter().copied().fold(f32::INFINITY, f32::min);
        let high = depths.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        // The coast reaches 95 units towards the viewer and a few dozen away from it.
        assert!(high - low < 200.0, "depths span {low} to {high}");
    }
    #[test]
    fn the_backdrop_adds_no_stars_that_only_the_gpu_would_show() {
        // The CPU draws backdrop stars behind the atmosphere and the GPU in front of it.
        let snapshot = crate::model::demo(12.0, 64);
        let mut scene = Scene::new();
        scene.record(&snapshot, 12.0);
        let camera = Camera::default();
        let frame = scene.render(&RenderInput::small(
            &snapshot,
            view("lights"),
            &camera,
            12.0,
        ));
        assert!(
            !frame
                .items
                .iter()
                .any(|item| matches!(item, Item::Star(..)))
        );
    }
    #[test]
    fn no_opaque_housing_encloses_a_coloured_lantern() {
        let snapshot = crate::model::demo(12.0, 64);
        let mut scene = Scene::with_rows(ROWS);
        scene.record(&snapshot, 12.0);
        // Close enough that no sphere is held at the smallest body size.
        let camera = Camera {
            zoom: 40.0,
            ..Camera::default()
        };
        let frame = scene.render(&RenderInput::small(&snapshot, View::at(0), &camera, 12.0));
        let spheres: Vec<(Point, f32)> = frame
            .items
            .iter()
            .filter_map(|item| match *item {
                Item::Sphere { center, radius, .. } => Some((center, radius)),
                _ => None,
            })
            .collect();
        assert!(!spheres.is_empty());
        for &(center, radius) in &spheres {
            assert!(!spheres.iter().any(|&(other, around)| {
                let gap = (other[0] - center[0]).hypot(other[1] - center[1]);
                around > radius && gap + radius <= around
            }));
        }
    }
    #[test]
    fn the_star_core_fades_with_an_extinguished_light() {
        let lit = process(1, 0);
        let core = mix(characteristic(&lit, 0.0).color, [255, 255, 241], 0.6);
        let mut scene = Scene::with_rows(ROWS);
        scene.record(
            &Snapshot {
                elapsed: 1.0,
                processes: vec![lit],
                ..Snapshot::default()
            },
            1.0,
        );
        let gone = Snapshot {
            elapsed: 2.0,
            ..Snapshot::default()
        };
        scene.record(&gone, 2.0);
        let camera = Camera::default();
        // Halfway through the 1.5 s fade.
        let frame = scene.render(&RenderInput::small(&gone, View::at(0), &camera, 2.75));
        assert!(
            frame
                .items
                .iter()
                .any(|item| matches!(item, Item::Puff { color, .. } if *color == characteristic(&process(1, 0), 0.0).color)),
            "the exit is drawn"
        );
        assert!(
            !frame
                .items
                .iter()
                .any(|item| matches!(item, Item::Triangle(_, color, _) if *color == core))
        );
    }
}
