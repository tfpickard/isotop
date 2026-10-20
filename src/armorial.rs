//! The Roll of Arms: measured process ancestry as cadency on decorative house arms.
//! A projective compression of the nave gives perspective without changing the shared camera.
//! Every cloth has a reserved column and tier; neither resource ranking nor churn moves it.

use crate::model::{Identity, Kind, Process, Snapshot, bounded, top_n};
use crate::palette::tint;
use crate::render::field::{Field, Placement, Relief};
use crate::render::{Color, NONE, Point, Stage, Title, TourTarget, label_name};
use crate::simulation::{hash, hash_identity, smoothstep};
use crate::views::ViewModule;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};

const CAP: usize = 48;
const TEXELS: usize = 121;
const MESH: usize = 13;
const SALT: u64 = 0x6172_6d6f_7269_616c;
const GOLD: Color = [224, 177, 72];
const FOCAL: f32 = 22.0;
const WALL: f32 = 15.0;
const CLOTH: f32 = 13.0;
const TIERS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tincture {
    Or,
    Argent,
    Gules,
    Azure,
    Vert,
    Sable,
    Purpure,
}
impl Tincture {
    fn metal(self) -> bool {
        matches!(self, Self::Or | Self::Argent)
    }
    fn name(self) -> &'static str {
        match self {
            Self::Or => "or",
            Self::Argent => "argent",
            Self::Gules => "gules",
            Self::Azure => "azure",
            Self::Vert => "vert",
            Self::Sable => "sable",
            Self::Purpure => "purpure",
        }
    }
    fn color(self) -> Color {
        match self {
            Self::Or => [244, 197, 65],
            Self::Argent => [235, 235, 218],
            Self::Gules => [175, 27, 42],
            Self::Azure => [30, 66, 155],
            Self::Vert => [22, 100, 60],
            Self::Sable => [22, 21, 29],
            Self::Purpure => [114, 40, 132],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Division {
    Plain,
    Pale,
    Fess,
    Bend,
    Chevron,
    Quarterly,
    Saltire,
}
impl Division {
    fn second(self, u: f32, v: f32) -> bool {
        match self {
            Self::Plain => false,
            Self::Pale => u >= 0.5,
            Self::Fess => v >= 0.5,
            Self::Bend => v >= u,
            Self::Chevron => v > 0.35 + (u - 0.5).abs(),
            Self::Quarterly => (u >= 0.5) != (v >= 0.5),
            Self::Saltire => (u - 0.5).abs() > (v - 0.5).abs(),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Pale => "Per pale",
            Self::Fess => "Per fess",
            Self::Bend => "Per bend",
            Self::Chevron => "Per chevron",
            Self::Quarterly => "Quarterly",
            Self::Saltire => "Per saltire",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ordinary {
    None,
    Fess,
    Pale,
    Bend,
    Chevron,
    Cross,
    Saltire,
    Chief,
    Bordure,
}
impl Ordinary {
    fn covers(self, u: f32, v: f32) -> bool {
        let (x, y) = (u - 0.5, v - 0.5);
        match self {
            Self::None => false,
            Self::Fess => y.abs() < 0.12,
            Self::Pale => x.abs() < 0.12,
            Self::Bend => (y - x).abs() < 0.14,
            Self::Chevron => (v - 0.32 - x.abs()).abs() < 0.09,
            Self::Cross => x.abs() < 0.09 || y.abs() < 0.09,
            Self::Saltire => (y.abs() - x.abs()).abs() < 0.1,
            Self::Chief => v < 0.23,
            Self::Bordure => !(0.09..=0.91).contains(&u) || !(0.09..=0.91).contains(&v),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Fess => "fess",
            Self::Pale => "pale",
            Self::Bend => "bend",
            Self::Chevron => "chevron",
            Self::Cross => "cross",
            Self::Saltire => "saltire",
            Self::Chief => "chief",
            Self::Bordure => "bordure",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Charge {
    Roundel,
    Mullet,
    Lozenge,
    Billet,
    Cross,
}
impl Charge {
    fn covers(self, x: f32, y: f32) -> bool {
        match self {
            Self::Roundel => x.hypot(y) < 0.85,
            Self::Mullet => star(x, y, 5),
            Self::Lozenge => x.abs() + y.abs() < 1.0,
            Self::Billet => x.abs() < 0.48 && y.abs() < 0.9,
            Self::Cross => (x.abs() < 0.23 && y.abs() < 0.8) || (y.abs() < 0.23 && x.abs() < 0.8),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Roundel => "roundel",
            Self::Mullet => "mullet",
            Self::Lozenge => "lozenge",
            Self::Billet => "billet",
            Self::Cross => "cross couped",
        }
    }
    fn plural(self) -> &'static str {
        match self {
            Self::Roundel => "roundels",
            Self::Mullet => "mullets",
            Self::Lozenge => "lozenges",
            Self::Billet => "billets",
            Self::Cross => "crosses couped",
        }
    }
}

/// Ordinal kept even where the English nine-mark cycle repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Brisure(usize);
impl Brisure {
    fn name(self) -> &'static str {
        [
            "label of three points",
            "crescent",
            "mullet",
            "martlet",
            "annulet",
            "fleur-de-lis",
            "rose",
            "cross moline",
            "octofoil",
        ][(self.0 - 1) % 9]
    }
    fn covers(self, x: f32, y: f32) -> bool {
        let radius = x.hypot(y);
        match (self.0 - 1) % 9 {
            0 => {
                (x.abs() < 0.95 && (-0.5..=-0.3).contains(&y))
                    || (y > -0.5
                        && y < 0.65
                        && [-0.7, 0.0, 0.7]
                            .iter()
                            .any(|point| (x - point).abs() < 0.13))
            }
            1 => radius < 0.95 && x.hypot(y - 0.4) > 0.75,
            2 => star(x, y, 5),
            // Raised wing, body, head and tail, with the martlet's characteristic lack of feet.
            3 => {
                (x / 0.7).hypot(y / 0.32) < 1.0
                    || (x + 0.1).abs() + (y + 0.4).abs() < 0.5
                    || (x - 0.55).hypot(y + 0.25) < 0.22
                    || (x < -0.45 && x > -0.95 && y.abs() < (-x - 0.3) * 0.45)
            }
            4 => (0.6..0.9).contains(&radius),
            5 => {
                (x / 0.24).hypot((y + 0.35) / 0.6) < 1.0
                    || ((x.abs() - 0.4) / 0.28).hypot((y + 0.1) / 0.4) < 1.0
                    || (x.abs() < 0.5 && (y - 0.35).abs() < 0.1)
                    || (x.abs() < 0.18 && y > 0.25 && y < 0.85)
            }
            6 => radius < 0.65 + 0.22 * (5.0 * y.atan2(x)).cos(),
            7 => {
                (x.abs() < 0.18 && y.abs() < 0.8)
                    || (y.abs() < 0.18 && x.abs() < 0.8)
                    || (x.abs() - 0.67).hypot(y.abs() - 0.25) < 0.2
                    || (y.abs() - 0.67).hypot(x.abs() - 0.25) < 0.2
            }
            _ => radius < 0.72 + 0.18 * (8.0 * y.atan2(x)).cos(),
        }
    }
}
fn star(x: f32, y: f32, points: u32) -> bool {
    let phase = (y.atan2(x) + PI / 2.0).rem_euclid(TAU / points as f32) / (TAU / points as f32);
    x.hypot(y) < 1.0 - 0.58 * (1.0 - (2.0 * phase - 1.0).abs())
}

/// The generator's grammar, shared by painting and blazon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Arms {
    pigments: [Tincture; 2],
    division: Division,
    ordinary: Ordinary,
    charge: Charge,
    pale: bool,
}
fn arms(path: &str) -> Arms {
    let seed = hash::fnv1a_64(path);
    Arms {
        pigments: [
            [Tincture::Or, Tincture::Argent][(seed & 1) as usize],
            [
                Tincture::Gules,
                Tincture::Azure,
                Tincture::Vert,
                Tincture::Sable,
                Tincture::Purpure,
            ][((seed >> 1) % 5) as usize],
        ],
        division: [
            Division::Plain,
            Division::Pale,
            Division::Fess,
            Division::Bend,
            Division::Chevron,
            Division::Quarterly,
            Division::Saltire,
        ][((seed >> 5) % 7) as usize],
        ordinary: [
            Ordinary::None,
            Ordinary::Fess,
            Ordinary::Pale,
            Ordinary::Bend,
            Ordinary::Chevron,
            Ordinary::Cross,
            Ordinary::Saltire,
            Ordinary::Chief,
            Ordinary::Bordure,
        ][((seed >> 9) % 9) as usize],
        charge: [
            Charge::Roundel,
            Charge::Mullet,
            Charge::Lozenge,
            Charge::Billet,
            Charge::Cross,
        ][((seed >> 14) % 5) as usize],
        pale: seed & (1 << 20) != 0,
    }
}
impl Arms {
    fn charged(self, uv: [f32; 2], threads: u32) -> bool {
        let [u, v] = uv;
        let covers =
            |[x, y]: [f32; 2], size: f32| self.charge.covers((u - x) / size, (v - y) / size);
        match threads {
            0 | 1 => covers([0.5, 0.53], 0.17),
            2 if self.pale => [[0.5, 0.34], [0.5, 0.7]]
                .into_iter()
                .any(|at| covers(at, 0.14)),
            2 => [[0.3, 0.53], [0.7, 0.53]]
                .into_iter()
                .any(|at| covers(at, 0.14)),
            3..=7 => [[0.3, 0.37], [0.7, 0.37], [0.5, 0.72]]
                .into_iter()
                .any(|at| covers(at, 0.14)),
            _ => (0..4).any(|row| {
                (0..4).any(|column| {
                    covers(
                        [0.14 + column as f32 * 0.24, 0.17 + row as f32 * 0.23],
                        0.075,
                    )
                })
            }),
        }
    }
    /// Layer indices identify the field, ordinary, charge, then the two painted differences.
    fn paint(
        self,
        uv: [f32; 2],
        threads: u32,
        marks: &[Brisure],
        mut layer: impl FnMut(usize, Tincture),
    ) -> Tincture {
        let [u, v] = uv;
        let mut class = usize::from(self.division.second(u, v));
        layer(0, self.pigments[class]);
        for (index, covered) in [self.ordinary.covers(u, v), self.charged(uv, threads)]
            .into_iter()
            .enumerate()
        {
            if covered {
                class = usize::from(self.pigments[class].metal());
                layer(index + 1, self.pigments[class]);
            }
        }
        for (level, mark) in marks.iter().take(2).enumerate() {
            let size = if level == 0 { 0.19 } else { 0.052 };
            let centre = if level == 0 {
                [0.5, 0.19]
            } else {
                mark_centre(marks[0])
            };
            if mark.covers((u - centre[0]) / size, (v - centre[1]) / size)
                && (level == 0 || marks[0].covers((u - 0.5) / 0.19, (v - 0.19) / 0.19))
            {
                class = usize::from(self.pigments[class].metal());
                layer(level + 3, self.pigments[class]);
            }
        }
        self.pigments[class]
    }
    fn tincture_at(self, uv: [f32; 2], threads: u32, marks: &[Brisure]) -> Tincture {
        self.paint(uv, threads, marks, |_, _| {})
    }
    /// Name a solid tincture when it is solid; counterchanged only when both pigments occur.
    fn tincture_word(self, threads: u32, marks: &[Brisure], target: usize) -> &'static str {
        let mut seen = [false; 2];
        for row in 0..TEXELS {
            for column in 0..TEXELS {
                self.paint(
                    [
                        column as f32 / (TEXELS - 1) as f32,
                        row as f32 / (TEXELS - 1) as f32,
                    ],
                    threads,
                    marks,
                    |index, tincture| {
                        if index == target {
                            seen[usize::from(!tincture.metal())] = true;
                        }
                    },
                );
            }
        }
        match seen {
            [true, false] => self.pigments[0].name(),
            [false, true] => self.pigments[1].name(),
            _ => "counterchanged",
        }
    }
    fn blazon(self, threads: u32, marks: &[Brisure]) -> String {
        let [metal, color] = self.pigments.map(Tincture::name);
        let mut text = if self.division == Division::Plain {
            format!("{metal},")
        } else {
            format!("{} {metal} and {color},", self.division.name())
        };
        if self.ordinary != Ordinary::None {
            text.push_str(&format!(
                " a {} {},",
                self.ordinary.name(),
                self.tincture_word(threads, marks, 1)
            ));
        }
        let number = match threads {
            0 | 1 => "a",
            2 => "two",
            3..=7 => "three",
            _ => "semy of",
        };
        let arrangement = match threads {
            2 if self.pale => " in pale",
            2 => " in fess",
            3..=7 => " two and one",
            _ => "",
        };
        let charge = if threads >= 2 {
            self.charge.plural()
        } else {
            self.charge.name()
        };
        text.push_str(&format!(
            " {number} {charge}{arrangement} {}",
            self.tincture_word(threads, marks, 2)
        ));
        for (level, mark) in marks.iter().enumerate() {
            if level == 0 {
                text.push_str(&format!("; a {}", mark.name()));
            } else {
                text.push_str(&format!(
                    "; on the {}, a {}",
                    marks[level - 1].name(),
                    mark.name()
                ));
            }
            let tincture = if level < 2 {
                self.tincture_word(threads, marks, level + 3)
            } else {
                "counterchanged"
            };
            text.push_str(&format!(" {tincture} for difference"));
            if mark.0 >= 10 {
                text.push_str(&format!(" ({} child)", ordinal(mark.0)));
            }
        }
        text[..1].make_ascii_uppercase();
        text
    }
}
/// A point inside the painted mark, rather than the empty centre of a crescent or annulet.
fn mark_centre(mark: Brisure) -> [f32; 2] {
    match (mark.0 - 1) % 9 {
        0 => [0.5, 0.22],
        1 => [0.5, 0.05],
        4 => [0.5, 0.33],
        _ => [0.5, 0.19],
    }
}
fn ordinal(number: usize) -> String {
    let suffix = if (11..=13).contains(&(number % 100)) {
        "th"
    } else {
        match number % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{number}{suffix}")
}
fn house(process: &Process) -> &str {
    if process.cgroup.is_empty() {
        &process.group
    } else {
        &process.cgroup
    }
}

/// Birth order uses the entire measured sibling list. House founders start new armorial paths.
fn ancestry(snapshot: &Snapshot) -> HashMap<Identity, Vec<Brisure>> {
    let by_pid: HashMap<u32, &Process> = snapshot.processes.iter().map(|p| (p.id.pid, p)).collect();
    let mut children: BTreeMap<u32, Vec<&Process>> = BTreeMap::new();
    for process in &snapshot.processes {
        children.entry(process.parent).or_default().push(process);
    }
    let mut orders = HashMap::new();
    for siblings in children.values_mut() {
        siblings.sort_by(|a, b| {
            a.started
                .total_cmp(&b.started)
                .then(a.id.pid.cmp(&b.id.pid))
        });
        for (index, process) in siblings.iter().enumerate() {
            orders.insert(process.id, Brisure(index + 1));
        }
    }
    snapshot
        .processes
        .iter()
        .map(|process| {
            let mut path = Vec::new();
            let mut seen = HashSet::new();
            let mut current = process;
            while seen.insert(current.id) {
                let Some(parent) = by_pid.get(&current.parent).copied() else {
                    break;
                };
                if parent.id == current.id || house(parent) != house(process) {
                    break;
                }
                path.push(orders[&current.id]);
                current = parent;
            }
            path.reverse();
            (process.id, path)
        })
        .collect()
}

/// A founder followed by its descendants, before the next founder of the same house.
fn tree_order(snapshot: &Snapshot) -> Vec<&Process> {
    let by_pid: HashMap<u32, &Process> = snapshot.processes.iter().map(|p| (p.id.pid, p)).collect();
    let mut roots = Vec::new();
    let mut children: HashMap<Identity, Vec<&Process>> = HashMap::new();
    for process in &snapshot.processes {
        if let Some(parent) = by_pid
            .get(&process.parent)
            .filter(|p| p.id != process.id && house(p) == house(process))
        {
            children.entry(parent.id).or_default().push(process);
        } else {
            roots.push(process);
        }
    }
    let order = |a: &&Process, b: &&Process| {
        a.started
            .total_cmp(&b.started)
            .then(a.id.pid.cmp(&b.id.pid))
    };
    roots.sort_by(|a, b| house(a).cmp(house(b)).then_with(|| order(a, b)));
    for siblings in children.values_mut() {
        siblings.sort_by(order);
    }
    let mut stack: Vec<&Process> = roots.into_iter().rev().collect();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some(process) = stack.pop() {
        if !seen.insert(process.id) {
            continue;
        }
        result.push(process);
        if let Some(siblings) = children.get(&process.id) {
            stack.extend(siblings.iter().rev().copied());
        }
    }
    let mut remainder: Vec<&Process> = snapshot
        .processes
        .iter()
        .filter(|p| !seen.contains(&p.id))
        .collect();
    remainder.sort_by(|a, b| house(a).cmp(house(b)).then_with(|| order(a, b)));
    result.extend(remainder);
    result
}

/// Projectively compress the hall along its aisle. Points converge at one vanishing point.
fn world(across: f32, distance: f32, height: f32) -> Point {
    let scale = FOCAL / (FOCAL + distance);
    let along = -distance * scale;
    [
        (across * scale + along) * FRAC_1_SQRT_2,
        (along - across * scale) * FRAC_1_SQRT_2,
        height * scale,
    ]
}
/// Equal column addresses get generous near-field spacing; distant additions approach the window.
fn distance(column: f32) -> f32 {
    let scale = if column <= 8.0 {
        1.0 - 0.118 * column + 0.0035 * column * column
    } else {
        0.28 * 0.8_f32.powf(column - 8.0)
    };
    FOCAL * (scale.recip() - 1.0)
}
fn solid(stage: &mut Stage, low: Point, high: Point, color: Color) {
    let [a, d, z] = low;
    let [b, e, w] = high;
    for (corners, light) in [
        (
            [
                world(a, d, z),
                world(b, d, z),
                world(b, d, w),
                world(a, d, w),
            ],
            0.85,
        ),
        (
            [
                world(b, d, z),
                world(b, e, z),
                world(b, e, w),
                world(b, d, w),
            ],
            0.65,
        ),
        (
            [
                world(b, e, z),
                world(a, e, z),
                world(a, e, w),
                world(b, e, w),
            ],
            0.78,
        ),
        (
            [
                world(a, e, z),
                world(a, d, z),
                world(a, d, w),
                world(a, e, w),
            ],
            0.95,
        ),
        (
            [
                world(a, d, w),
                world(b, d, w),
                world(b, e, w),
                world(a, e, w),
            ],
            1.0,
        ),
    ] {
        stage
            .frame
            .quad(stage.camera, corners, tint(color, light), NONE);
    }
}

struct Bay {
    index: usize,
    columns: Vec<usize>,
}
struct Banner {
    process: Process,
    bay: usize,
    slot: usize,
    column: usize,
    born: f32,
    exited: Option<f32>,
    marks: Vec<Brisure>,
    field: Option<Field>,
}
impl Banner {
    fn side(&self) -> f32 {
        if self.bay.is_multiple_of(2) {
            -1.0
        } else {
            1.0
        }
    }
    fn top(&self) -> f32 {
        7.0 + (self.slot % TIERS) as f32 * 4.8
    }
    fn anchor(&self) -> Point {
        world(
            self.side() * CLOTH,
            distance(self.column as f32 + 0.5),
            self.top() - 1.5,
        )
    }
    fn texture(&mut self) {
        let field = self.field.get_or_insert_with(|| {
            Field::new(hash_identity(self.process.id, SALT), TEXELS, TEXELS)
        });
        let pattern = arms(house(&self.process));
        for row in 0..TEXELS {
            for column in 0..TEXELS {
                let (u, v) = (
                    column as f32 / (TEXELS - 1) as f32,
                    row as f32 / (TEXELS - 1) as f32,
                );
                // The board's diamond UVs are rotated; put a complete upright coat inside it.
                let (u, v) = if self.process.state == 'Z' {
                    let x = u - v;
                    let y = u + v - 1.0;
                    if x.abs() > 0.38 || y.abs() > 0.42 {
                        field.colors[row * TEXELS + column] = Tincture::Sable.color();
                        continue;
                    }
                    (0.5 + x / 0.76, 0.5 + y / 0.84)
                } else {
                    (u, v)
                };
                let tincture = pattern.tincture_at([u, v], self.process.threads, &self.marks);
                let color = tincture.color();
                field.colors[row * TEXELS + column] =
                    if self.process.nice > 0 && v > 0.82 && (u - 0.5).abs() < (v - 0.82) {
                        // A metal fimbriation separates the stain from any colour underneath.
                        if v > 0.85 && (u - 0.5).abs() < (v - 0.85) {
                            [125, 53, 26]
                        } else if !tincture.metal() {
                            pattern.pigments[0].color()
                        } else {
                            color
                        }
                    } else {
                        tint(color, 0.92 + 0.08 * (u * TAU * 2.0).cos())
                    };
            }
        }
        field.revision += 1;
    }
    /// Lowering also gathers the cloth, so an exit stays inside its own unoccluded reservation.
    fn corners(&self, time: f32) -> [Point; 4] {
        let size = (1.1 + 0.055 * (self.process.memory as f32 / 1_048_576.0).sqrt()).min(2.8) / 2.8;
        let exit = self
            .exited
            .map_or(0.0, |start| smoothstep((time - start).clamp(0.0, 1.0)));
        let growth = smoothstep((time - self.born).clamp(0.0, 1.0)).max(0.03) * (1.0 - exit);
        let height = if self.process.state == 'T' {
            size * 0.5
        } else {
            size * 3.6
        };
        let top = self.top() - height * 0.4 * exit;
        let half = 0.43 * size * (1.0 - exit);
        let centre = self.column as f32 + 0.5;
        let at = |offset: f32, z: f32| world(self.side() * CLOTH, distance(centre + offset), z);
        if self.process.state == 'Z' {
            [
                at(0.0, top),
                at(half, top - height * growth * 0.5),
                at(0.0, top - height * growth),
                at(-half, top - height * growth * 0.5),
            ]
        } else {
            [
                at(-half, top),
                at(half, top),
                at(half, top - height * growth),
                at(-half, top - height * growth),
            ]
        }
    }
}

#[derive(Default)]
pub struct Armorial {
    bays: BTreeMap<String, Bay>,
    /// Bays ever made, so a new bay's index is never one a banner still names.
    created: usize,
    banners: HashMap<Identity, Banner>,
    sample: Option<f64>,
    drawn: usize,
    lowering: usize,
    collapsed: usize,
    houses: usize,
}
impl Armorial {
    /// Seats new banners and repaints changed ones once per sample, then gives every admitted
    /// banner this frame's eased CPU and memory, which change between samples.
    fn refresh(&mut self, snapshot: &Snapshot, admitted: &[&Process], time: f32) {
        if self.sample.is_some_and(|sample| snapshot.elapsed < sample) {
            *self = Self::default();
        }
        self.banners
            .retain(|_, banner| banner.exited.is_none_or(|exit| time - exit < 1.0));
        self.reclaim();
        if self.sample != Some(snapshot.elapsed)
            || !admitted.iter().all(|p| self.banners.contains_key(&p.id))
        {
            self.seat(snapshot, admitted, time);
        }
        for process in admitted {
            if let Some(banner) = self.banners.get_mut(&process.id) {
                banner.process.cpu = process.cpu;
                banner.process.memory = process.memory;
            }
        }
    }
    fn seat(&mut self, snapshot: &Snapshot, admitted: &[&Process], time: f32) {
        let initial = self.sample.is_none();
        self.sample = Some(snapshot.elapsed);
        let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        let selected: HashSet<Identity> = admitted.iter().map(|p| p.id).collect();
        for (id, banner) in &mut self.banners {
            if !alive.contains(id) && banner.exited.is_none() {
                banner.exited = Some(time);
            }
            if alive.contains(id) && !selected.contains(id) {
                banner.field = None;
            }
        }
        let marks = ancestry(snapshot);
        // A process that changed house gives up its seat and is seated anew in its new bay below.
        for process in &snapshot.processes {
            if self
                .banners
                .get(&process.id)
                .is_some_and(|banner| house(&banner.process) != house(process))
            {
                self.banners.remove(&process.id);
            }
        }
        self.reclaim();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for process in admitted {
            *counts.entry(house(process)).or_default() += 1;
        }
        for process in tree_order(snapshot)
            .into_iter()
            .filter(|p| selected.contains(&p.id))
        {
            if !self.bays.contains_key(house(process)) {
                let side = usize::from(self.held(0) > self.held(1));
                let index = self.created * 2 + side;
                self.created += 1;
                let columns = self.vacant(side, counts[house(process)].div_ceil(TIERS));
                self.bays
                    .insert(house(process).to_owned(), Bay { index, columns });
            }
            if !self.banners.contains_key(&process.id) {
                let bay = &self.bays[house(process)];
                let slot = (0..)
                    .find(|slot| {
                        !self
                            .banners
                            .values()
                            .any(|b| b.bay == bay.index && b.slot == *slot)
                    })
                    .expect("a house has a vacant seat");
                if slot / TIERS == bay.columns.len() {
                    let column = self.vacant(bay.index % 2, 1);
                    let bay = self.bays.get_mut(house(process)).expect("reserved bay");
                    bay.columns.extend(column);
                }
                let bay = &self.bays[house(process)];
                self.banners.insert(
                    process.id,
                    Banner {
                        process: process.clone(),
                        bay: bay.index,
                        slot,
                        column: bay.columns[slot / TIERS],
                        born: time - if initial { 1.0 } else { 0.0 },
                        exited: None,
                        marks: marks[&process.id].clone(),
                        field: None,
                    },
                );
            }
        }
        for process in &snapshot.processes {
            if let Some(banner) = self.banners.get_mut(&process.id) {
                let changed = banner.field.is_none()
                    || banner.process.threads != process.threads
                    || banner.process.state != process.state
                    || banner.process.nice != process.nice
                    || house(&banner.process) != house(process)
                    || banner.marks != marks[&process.id];
                banner.process = process.clone();
                banner.marks = marks[&process.id].clone();
                banner.exited = None;
                if selected.contains(&process.id) && changed {
                    banner.texture();
                }
            }
        }
    }
    /// How many columns deep the hall must reach to hold every bay.
    fn extent(&self) -> usize {
        self.bays
            .values()
            .flat_map(|bay| &bay.columns)
            .max()
            .map_or(0, |column| column + 1)
    }
    /// The columns the bays on `side` hold.
    fn holding(&self, side: usize) -> impl Iterator<Item = usize> + '_ {
        self.bays
            .values()
            .filter(move |bay| bay.index % 2 == side)
            .flat_map(|bay| bay.columns.iter().copied())
    }
    fn held(&self, side: usize) -> usize {
        self.holding(side).count()
    }
    /// The `count` nearest columns on `side` that no bay holds, so a reclaimed bay's columns
    /// are taken again before the hall grows.
    fn vacant(&self, side: usize, count: usize) -> Vec<usize> {
        let held: HashSet<usize> = self.holding(side).collect();
        (0..)
            .filter(|column| !held.contains(column))
            .take(count)
            .collect()
    }
    /// Frees the bays that no banner, live or lowering, sits in; no other bay moves.
    fn reclaim(&mut self) {
        let occupied: HashSet<usize> = self.banners.values().map(|banner| banner.bay).collect();
        self.bays.retain(|_, bay| occupied.contains(&bay.index));
    }
    fn ghosts(&self, time: f32, focused: bool) -> Vec<Identity> {
        if focused {
            return Vec::new();
        }
        let mut ghosts: Vec<Identity> = self
            .banners
            .iter()
            .filter(|(_, b)| b.exited.is_some_and(|exit| time - exit < 1.0))
            .map(|(&id, _)| id)
            .collect();
        ghosts.sort_by_key(|id| {
            let b = &self.banners[id];
            (b.bay, b.slot)
        });
        ghosts.truncate(CAP);
        ghosts
    }
    fn hall(&self, stage: &mut Stage) {
        let columns = self.extent().max(6);
        let end = distance(columns as f32 + 0.6);
        for row in 0..columns * 3 {
            let near = distance(row as f32 / 3.0);
            let far = distance((row + 1) as f32 / 3.0);
            for column in -6_i32..6 {
                let left = column as f32 * 2.5;
                let color = if (row as i32 + column).rem_euclid(2) == 0 {
                    [58, 49, 44]
                } else {
                    [91, 76, 60]
                };
                stage.frame.quad(
                    stage.camera,
                    [
                        world(left, near, 0.0),
                        world(left + 2.5, near, 0.0),
                        world(left + 2.5, far, 0.0),
                        world(left, far, 0.0),
                    ],
                    color,
                    NONE,
                );
            }
        }
        // Continuous walls and a high barrel vault: no rib or beam crosses the cloth zone.
        for side in [-1.0, 1.0] {
            stage.frame.quad(
                stage.camera,
                [
                    world(side * WALL, 0.0, 0.0),
                    world(side * WALL, end, 0.0),
                    world(side * WALL, end, 14.0),
                    world(side * WALL, 0.0, 14.0),
                ],
                [57, 47, 44],
                NONE,
            );
            for column in 0..columns {
                let near = distance(column as f32);
                let far = distance(column as f32 + 1.0);
                let (a, b) = if side < 0.0 {
                    (-WALL, -WALL + 0.5)
                } else {
                    (WALL - 0.5, WALL)
                };
                solid(stage, [a, near, 0.0], [b, near + 0.2, 14.0], [128, 106, 78]);
                let (a, b) = if side < 0.0 {
                    (-WALL, -11.7)
                } else {
                    (11.7, WALL)
                };
                solid(
                    stage,
                    [a, near + 0.1, 1.9],
                    [b, far - 0.1, 2.4],
                    [83, 49, 31],
                );
                solid(
                    stage,
                    [a, near + 0.1, 0.0],
                    [b, far - 0.1, 0.45],
                    [58, 35, 25],
                );
                let mid = distance(column as f32 + 0.5);
                let candle = world(side * 11.5, mid, 2.8);
                let scale = FOCAL / (FOCAL + mid);
                stage.frame.hairline(
                    stage.camera,
                    world(side * 11.5, mid, 2.4),
                    candle,
                    [209, 178, 115],
                    1.5,
                );
                stage.frame.sphere(
                    stage.camera,
                    candle,
                    0.065 * scale,
                    [255, 222, 152],
                    NONE,
                    false,
                );
                stage
                    .frame
                    .glow(stage.camera, candle, 0.95 * scale, GOLD, 0.32);
                stage.frame.quad(
                    stage.camera,
                    [
                        world(side * 14.9, near, 12.8),
                        world(side * 14.9, far, 12.8),
                        world(side * 14.9, far, 13.8),
                        world(side * 14.9, near, 13.8),
                    ],
                    [89, 69, 54],
                    NONE,
                );
            }
        }
        for segment in 0..12 {
            let a = -PI / 2.0 + PI * segment as f32 / 12.0;
            let b = -PI / 2.0 + PI * (segment + 1) as f32 / 12.0;
            let color = tint([72, 58, 51], 0.78 + 0.22 * (a + 0.4).cos());
            stage.frame.quad(
                stage.camera,
                [
                    world(WALL * a.sin(), 0.0, 14.0 + 1.8 * a.cos()),
                    world(WALL * b.sin(), 0.0, 14.0 + 1.8 * b.cos()),
                    world(WALL * b.sin(), end, 14.0 + 1.8 * b.cos()),
                    world(WALL * a.sin(), end, 14.0 + 1.8 * a.cos()),
                ],
                color,
                NONE,
            );
        }
        stage.frame.quad(
            stage.camera,
            [
                world(-WALL, end, 0.0),
                world(WALL, end, 0.0),
                world(WALL, end, 14.0),
                world(-WALL, end, 14.0),
            ],
            [73, 59, 51],
            NONE,
        );
        self.window(stage, end - 0.2);
        stage.bounds.extend([
            world(-WALL, 0.0, 0.0),
            world(WALL, 0.0, 0.0),
            world(-WALL, 0.0, 14.0),
            world(WALL, 0.0, 14.0),
            world(0.0, 0.0, 15.8),
            world(0.0, end, 0.0),
        ]);
    }
    fn window(&self, stage: &mut Stage, end: f32) {
        let at = |x: f32, z: f32| world(x, end, z);
        stage.frame.quad(
            stage.camera,
            [at(-4.4, 1.4), at(4.4, 1.4), at(4.4, 11.3), at(-4.4, 11.3)],
            [200, 167, 109],
            NONE,
        );
        for row in 0..6 {
            for column in 0..4 {
                let x = -4.0 + column as f32 * 2.0;
                let z = 1.8 + row as f32 * 1.5;
                let color = [
                    [219, 159, 71],
                    [83, 120, 152],
                    [145, 67, 61],
                    [222, 199, 134],
                ][(row + column) % 4];
                stage.frame.quad(
                    stage.camera,
                    [
                        at(x, z),
                        at(x + 1.8, z),
                        at(x + 1.8, z + 1.25),
                        at(x, z + 1.25),
                    ],
                    color,
                    NONE,
                );
            }
        }
        for segment in 0..16 {
            let a = PI * segment as f32 / 16.0;
            let b = PI * (segment + 1) as f32 / 16.0;
            stage.frame.facet(
                stage.camera,
                [
                    at(0.0, 11.3),
                    at(4.4 * a.cos(), 11.3 + 4.0 * a.sin()),
                    at(4.4 * b.cos(), 11.3 + 4.0 * b.sin()),
                ],
                tint(GOLD, 0.75 + 0.15 * a.sin()),
                NONE,
            );
        }
        let centre = at(0.0, 8.0);
        let scale = FOCAL / (FOCAL + end);
        stage
            .frame
            .glow(stage.camera, centre, 7.0 * scale, [255, 194, 104], 0.22);
    }
    fn cloth(stage: &mut Stage, banner: &mut Banner, pick: u32) {
        if banner.field.is_none() {
            banner.texture();
        }
        let corners = banner.corners(stage.time);
        let process = &banner.process;
        let mid = distance(banner.column as f32 + 0.5);
        let scale = FOCAL / (FOCAL + mid);
        let field = banner.field.as_mut().expect("painted cloth");
        let cpu = bounded(process.cpu, 100.0);
        let heights = field
            .heights
            .get_or_insert_with(|| vec![0.0; TEXELS * TEXELS]);
        for row in 0..TEXELS {
            for column in 0..TEXELS {
                let u = column as f32 / (TEXELS - 1) as f32;
                let v = row as f32 / (TEXELS - 1) as f32;
                heights[row * TEXELS + column] = if process.state == 'Z' || process.state == 'T' {
                    0.0
                } else {
                    0.08 * scale * cpu * v * (u * TAU * 1.5 + stage.time * (0.6 + cpu * 1.4)).sin()
                };
            }
        }
        field
            .owners
            .get_or_insert_with(|| vec![pick; TEXELS * TEXELS])
            .fill(pick);
        field.revision += 1;
        stage.frame.field(
            stage.camera,
            Placement::Corners(corners),
            field,
            Relief::Displaced {
                scale: 1.0,
                mesh: MESH,
            },
        );
        let top = banner.top() + 0.15;
        stage.frame.hairline(
            stage.camera,
            world(banner.side() * WALL, mid, top),
            world(banner.side() * CLOTH, mid, top),
            GOLD,
            1.3,
        );
        stage
            .frame
            .hairline(stage.camera, corners[0], corners[1], GOLD, 1.3);
        helm(
            stage,
            world(banner.side() * CLOTH, mid, banner.top() + 0.55),
            process.kind,
            pick,
            scale,
        );
        if stage.highlight.contains(&process.id) {
            for edge in 0..4 {
                stage.frame.hairline(
                    stage.camera,
                    corners[edge],
                    corners[(edge + 1) % 4],
                    GOLD,
                    2.0,
                );
            }
        }
    }
    fn describe(stage: &mut Stage, banner: &Banner, snapshot: &Snapshot) {
        let process = &banner.process;
        let text = arms(house(process)).blazon(process.threads, &banner.marks);
        let mut line = String::new();
        for word in text.split_whitespace() {
            if line.len() + word.len() > 58 {
                stage.popup.push((process.id, std::mem::take(&mut line)));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            stage.popup.push((process.id, line));
        }
        if let Some(mark) = banner.marks.last() {
            let parent = snapshot
                .processes
                .iter()
                .find(|p| p.id.pid == process.parent)
                .map_or_else(
                    || format!("pid {}", process.parent),
                    |p| format!("{}-{}", label_name(&p.name), p.id.pid),
                );
            stage.popup.push((
                process.id,
                format!(
                    "cadency: {} son of {parent} ({})",
                    ordinal(mark.0),
                    mark.name()
                ),
            ));
        } else {
            stage.popup.push((
                process.id,
                "cadency: founder; no sampled parent in this house".into(),
            ));
        }
    }
}
/// The `budget` largest processes by memory, the selected one (if a candidate) taking the last
/// place when it would fall out, as the scene's own admission keeps it.
fn admit(candidates: &[Process], budget: usize, selected: Option<Identity>) -> Vec<&Process> {
    let mut admitted = top_n(candidates, budget, |p| p.memory as f64);
    if let Some(id) = selected
        && budget > 0
        && !admitted.iter().any(|p| p.id == id)
        && let Some(process) = candidates.iter().find(|p| p.id == id)
    {
        admitted.truncate(budget - 1);
        admitted.push(process);
    }
    admitted
}
fn helm(stage: &mut Stage, at: Point, kind: Kind, pick: u32, scale: f32) {
    let color = if kind == Kind::Kernel {
        GOLD
    } else {
        [180, 185, 192]
    };
    stage
        .frame
        .sphere(stage.camera, at, 0.23 * scale, color, pick, false);
    let across = |x: f32, z: f32| {
        [
            at[0] + x * scale * FRAC_1_SQRT_2 + 0.02 * scale,
            at[1] - x * scale * FRAC_1_SQRT_2 + 0.02 * scale,
            at[2] + z * scale,
        ]
    };
    match kind {
        Kind::Kernel | Kind::System => {
            for bar in -2..=2 {
                stage.frame.hairline(
                    stage.camera,
                    across(bar as f32 * 0.07, -0.14),
                    across(bar as f32 * 0.07, 0.14),
                    [35, 31, 31],
                    1.0,
                );
            }
        }
        Kind::Container => stage.frame.hairline(
            stage.camera,
            across(-0.19, 0.05),
            across(0.19, 0.05),
            [28, 26, 32],
            1.5,
        ),
        Kind::Session => stage.frame.hairline(
            stage.camera,
            across(0.02, 0.04),
            across(0.22, 0.04),
            [28, 26, 32],
            1.0,
        ),
    }
}
impl ViewModule for Armorial {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let budget = CAP.min(stage.limit);
        let candidates: Vec<Process> = processes.iter().map(|p| (*p).clone()).collect();
        let selected = stage
            .selected
            .filter(|id| candidates.iter().any(|p| p.id == *id));
        let admitted = admit(&candidates, budget, selected);
        self.refresh(snapshot, &admitted, stage.time);
        let mut ghosts = self.ghosts(stage.time, stage.focus.is_some());
        ghosts.truncate(budget.saturating_sub(usize::from(selected.is_some())));
        let chosen: HashSet<Identity> = admit(&candidates, budget - ghosts.len(), selected)
            .iter()
            .map(|p| p.id)
            .collect();
        let eligible: HashMap<Identity, u32> = processes
            .iter()
            .enumerate()
            .map(|(index, p)| (p.id, index as u32))
            .collect();
        self.hall(stage);
        for id in &ghosts {
            Self::cloth(
                stage,
                self.banners.get_mut(id).expect("lowering banner"),
                NONE,
            );
        }
        let mut ordered: Vec<Identity> = chosen.into_iter().collect();
        ordered.sort_by_key(|id| {
            let banner = &self.banners[id];
            (banner.bay, banner.slot)
        });
        let mut houses = BTreeMap::new();
        for id in &ordered {
            let banner = self.banners.get_mut(id).expect("reserved identity");
            let anchor = banner.anchor();
            stage.positions.insert(*id, anchor);
            stage.tour.push(TourTarget {
                id: *id,
                position: anchor,
                weight: banner.process.memory as f32,
            });
            Self::cloth(stage, banner, eligible[id]);
            houses.insert(house(&banner.process).to_owned(), banner.bay);
            if stage.selected == Some(*id) || stage.toured == Some(*id) {
                Self::describe(stage, banner, snapshot);
            }
        }
        for (name, index) in &houses {
            let bay = self
                .bays
                .values()
                .find(|bay| bay.index == *index)
                .expect("named bay");
            let side = if index.is_multiple_of(2) { -1.0 } else { 1.0 };
            stage.titles.push(Title {
                at: world(side * 11.3, distance(bay.columns[0] as f32 + 0.5), 1.0),
                reach: 0.4,
                text: label_name(
                    name.rsplit('/')
                        .find(|part| !part.is_empty())
                        .unwrap_or(name),
                )
                .into_owned(),
            });
        }
        stage
            .tour
            .sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
        self.drawn = ordered.len();
        self.lowering = ghosts.len();
        self.collapsed = processes.len().saturating_sub(self.drawn);
        self.houses = houses.len();
        self.drawn
    }
    fn legend(&self, _snapshot: &Snapshot) -> String {
        format!(
            " {} houses | {} banners ({} lowering) | {} collapsed | cadency: label 1st, crescent 2nd, mullet 3rd, martlet 4th, annulet 5th... | size = memory, sway = CPU, charges = threads, helm = kind",
            self.houses,
            self.drawn + self.lowering,
            self.lowering,
            self.collapsed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::process;
    use crate::render::{Camera, RenderInput, Scene};
    use crate::views::view;
    #[test]
    fn the_rule_of_tincture_holds_for_ten_thousand_seeds() {
        for seed in 0..10_000 {
            let pattern = arms(&format!("/house/{seed}"));
            assert_ne!(pattern.pigments[0].metal(), pattern.pigments[1].metal());
            for row in 0..21 {
                for column in 0..21 {
                    let mut previous: Option<Tincture> = None;
                    pattern.paint(
                        [column as f32 / 20.0, row as f32 / 20.0],
                        (seed % 12) as u32,
                        &[Brisure(seed % 18 + 1), Brisure(seed % 9 + 1)],
                        |_, next| {
                            if let Some(before) = previous {
                                assert_ne!(before.metal(), next.metal(), "seed {seed}");
                            }
                            previous = Some(next);
                        },
                    );
                }
            }
        }
    }
    #[test]
    fn the_second_child_bears_a_crescent() {
        let mut first = process(8, 1);
        first.started = 10.0;
        let mut second = process(7, 1);
        second.started = 11.0;
        let snapshot = Snapshot {
            processes: vec![process(1, 0), second.clone(), first],
            ..Snapshot::default()
        };
        assert_eq!(ancestry(&snapshot)[&second.id], vec![Brisure(2)]);
        assert_eq!(Brisure(2).name(), "crescent");
    }
    #[test]
    fn a_grandchilds_mark_sits_on_its_parents_mark() {
        let snapshot = Snapshot {
            processes: vec![
                process(1, 0),
                process(2, 1),
                process(3, 1),
                process(4, 3),
                process(5, 3),
            ],
            ..Snapshot::default()
        };
        let marks = ancestry(&snapshot)[&process(5, 3).id].clone();
        assert_eq!(marks, vec![Brisure(2), Brisure(2)]);
        let pattern = arms("test");
        let mut nested = 0;
        for row in 0..121 {
            for column in 0..121 {
                let uv = [column as f32 / 120.0, row as f32 / 120.0];
                if pattern.tincture_at(uv, 1, &marks) != pattern.tincture_at(uv, 1, &marks[..1]) {
                    assert!(marks[0].covers((uv[0] - 0.5) / 0.19, (uv[1] - 0.19) / 0.19));
                    nested += 1;
                }
            }
        }
        assert!(nested > 10, "the inner mark must be visible");
    }
    #[test]
    fn blazons_describe_hand_checked_cases() {
        let pattern = Arms {
            pigments: [Tincture::Or, Tincture::Azure],
            division: Division::Plain,
            ordinary: Ordinary::None,
            charge: Charge::Mullet,
            pale: false,
        };
        assert_eq!(pattern.blazon(1, &[]), "Or, a mullet azure");
        assert_eq!(
            Arms {
                division: Division::Bend,
                ..pattern
            }
            .blazon(3, &[Brisure(2)]),
            "Per bend or and azure, three mullets two and one counterchanged; a crescent azure for difference"
        );
        assert_eq!(pattern.blazon(8, &[]), "Or, semy of mullets azure");
        assert_eq!(
            pattern.blazon(1, &[Brisure(2), Brisure(2)]),
            "Or, a mullet azure; a crescent azure for difference; on the crescent, a crescent or for difference"
        );
        assert!(
            pattern
                .blazon(1, &[Brisure(10)])
                .ends_with("for difference (10th child)")
        );
        assert_eq!(ordinal(11), "11th");
        assert_eq!(Brisure(10).name(), "label of three points");
    }
    #[test]
    fn surviving_banners_keep_their_seats_when_resources_and_population_change() {
        let mut snapshot = crate::model::demo(12.0, 64);
        let mut module = Armorial::default();
        module.refresh(
            &snapshot,
            &top_n(&snapshot.processes, CAP, |p| p.memory as f64),
            12.0,
        );
        let before: HashMap<Identity, Point> = module
            .banners
            .iter()
            .map(|(&id, b)| (id, b.anchor()))
            .collect();
        snapshot.elapsed += 1.0;
        snapshot.processes.remove(0);
        for process in &mut snapshot.processes {
            process.memory *= 2;
            process.cpu *= 4.0;
        }
        snapshot.processes.push(process(99999, 1));
        module.refresh(
            &snapshot,
            &top_n(&snapshot.processes, CAP, |p| p.memory as f64),
            13.0,
        );
        for process in &snapshot.processes {
            if let (Some(anchor), Some(banner)) =
                (before.get(&process.id), module.banners.get(&process.id))
            {
                assert_eq!(*anchor, banner.anchor());
            }
        }
    }
    #[test]
    fn banners_take_the_eased_resources_of_every_frame_between_samples() {
        let snapshot = crate::model::demo(12.0, 64);
        let mut module = Armorial::default();
        module.refresh(
            &snapshot,
            &top_n(&snapshot.processes, CAP, |p| p.memory as f64),
            12.0,
        );
        let mut eased = snapshot.processes.clone();
        for process in &mut eased {
            process.memory = process.memory * 2 + 1;
            process.cpu += 7.0;
        }
        let admitted = top_n(&eased, CAP, |p| p.memory as f64);
        module.refresh(&snapshot, &admitted, 12.1);
        for process in admitted {
            let banner = &module.banners[&process.id];
            assert_eq!(banner.process.memory, process.memory);
            assert_eq!(banner.process.cpu, process.cpu);
        }
    }
    #[test]
    fn a_banner_moves_to_the_bay_of_its_new_house() {
        let house_process = |pid: u32, cgroup: &str| {
            let mut process = process(pid, 1);
            process.cgroup = cgroup.into();
            process
        };
        let mut snapshot = Snapshot {
            elapsed: 12.0,
            processes: vec![
                house_process(1, "/a"),
                house_process(2, "/a"),
                house_process(3, "/a"),
                house_process(4, "/b"),
            ],
            ..Snapshot::default()
        };
        let mut module = Armorial::default();
        module.refresh(
            &snapshot,
            &snapshot.processes.iter().collect::<Vec<_>>(),
            12.0,
        );
        snapshot.elapsed = 13.0;
        snapshot.processes[2].cgroup = "/b".into();
        module.refresh(
            &snapshot,
            &snapshot.processes.iter().collect::<Vec<_>>(),
            13.0,
        );
        let moved = &module.banners[&snapshot.processes[2].id];
        let destination = &module.bays["/b"];
        assert_eq!(moved.bay, destination.index);
        assert!(destination.columns.contains(&moved.column));
        let seats: HashSet<(usize, usize)> =
            module.banners.values().map(|b| (b.bay, b.slot)).collect();
        assert_eq!(seats.len(), module.banners.len());
    }
    #[test]
    fn the_bays_of_departed_houses_are_reclaimed_without_moving_survivors() {
        let house_process = |pid: u32, cgroup: String| {
            let mut process = process(pid, 1);
            process.cgroup = cgroup;
            process
        };
        let stay = house_process(2, "/stay".into());
        let mut module = Armorial::default();
        let mut anchor = None;
        for round in 0..50 {
            let snapshot = Snapshot {
                elapsed: 12.0 + 2.0 * round as f64,
                processes: vec![
                    stay.clone(),
                    house_process(100 + round, format!("/house/{round}")),
                ],
                ..Snapshot::default()
            };
            module.refresh(
                &snapshot,
                &snapshot.processes.iter().collect::<Vec<_>>(),
                snapshot.elapsed as f32,
            );
            let here = module.banners[&stay.id].anchor();
            assert_eq!(*anchor.get_or_insert(here), here);
        }
        // The survivor's house, the newest house and the one still lowering its banner.
        assert!(module.bays.len() <= 3, "{} bays", module.bays.len());
        assert!(module.extent() <= 3, "{} columns", module.extent());
    }
    #[test]
    fn the_same_house_path_generates_the_same_arms() {
        for seed in 0..100 {
            let path = format!("house-{seed}");
            assert_eq!(arms(&path), arms(&path));
        }
    }
    #[test]
    fn founders_are_followed_by_their_descendants_in_tree_order() {
        let snapshot = Snapshot {
            processes: vec![
                process(10, 0),
                process(1, 0),
                process(11, 10),
                process(2, 1),
                process(3, 2),
            ],
            ..Snapshot::default()
        };
        assert_eq!(
            tree_order(&snapshot)
                .iter()
                .map(|p| p.id.pid)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 10, 11]
        );
    }
    #[test]
    fn lowering_banners_share_the_budget_with_surviving_banners() {
        let mut snapshot = crate::model::demo(12.0, 96);
        let mut scene = Scene::new();
        let camera = Camera::default();
        scene.render(&RenderInput::small(
            &snapshot,
            view("armorial"),
            &camera,
            12.0,
        ));
        let departed: HashSet<Identity> = top_n(&snapshot.processes, 8, |p| p.memory as f64)
            .iter()
            .map(|p| p.id)
            .collect();
        snapshot.processes.retain(|p| !departed.contains(&p.id));
        snapshot.elapsed = 12.1;
        let frame = scene.render(&RenderInput::small(
            &snapshot,
            view("armorial"),
            &camera,
            12.1,
        ));
        assert_eq!(frame.fields.len(), CAP);
        assert_eq!(scene.visible, CAP - 8);
        let after = scene.render(&RenderInput::small(
            &snapshot,
            view("armorial"),
            &camera,
            13.2,
        ));
        assert_eq!(after.fields.len(), CAP);
        assert_eq!(scene.visible, CAP);
    }
    #[test]
    fn lowering_banners_also_respect_a_smaller_process_limit() {
        let mut snapshot = crate::model::demo(12.0, 64);
        let mut scene = Scene::new();
        let camera = Camera::default();
        let input = RenderInput {
            limit: 8,
            ..RenderInput::small(&snapshot, view("armorial"), &camera, 12.0)
        };
        scene.render(&input);
        let departed: HashSet<Identity> = scene.positions.keys().copied().collect();
        snapshot.processes.retain(|p| !departed.contains(&p.id));
        snapshot.elapsed = 12.1;
        let input = RenderInput {
            limit: 8,
            ..RenderInput::small(&snapshot, view("armorial"), &camera, 12.1)
        };
        let frame = scene.render(&input);
        assert_eq!(frame.fields.len(), 8);
        assert_eq!(scene.visible, 0);
    }

    #[test]
    fn the_selected_process_keeps_a_banner_while_lowering_banners_fill_the_budget() {
        let mut snapshot = crate::model::demo(12.0, 64);
        let mut scene = Scene::new();
        let camera = Camera::default();
        let selected = top_n(&snapshot.processes, snapshot.processes.len(), |p| {
            p.memory as f64
        })
        .last()
        .map(|p| p.id);
        let input = RenderInput {
            limit: 8,
            selected,
            ..RenderInput::small(&snapshot, view("armorial"), &camera, 12.0)
        };
        scene.render(&input);
        assert!(scene.positions.contains_key(&selected.unwrap()));
        let departed: HashSet<Identity> = scene
            .positions
            .keys()
            .copied()
            .filter(|id| Some(*id) != selected)
            .collect();
        snapshot.processes.retain(|p| !departed.contains(&p.id));
        snapshot.elapsed = 12.1;
        let input = RenderInput {
            limit: 8,
            selected,
            ..RenderInput::small(&snapshot, view("armorial"), &camera, 12.1)
        };
        let frame = scene.render(&input);
        assert!(scene.positions.contains_key(&selected.unwrap()));
        assert_eq!(frame.fields.len(), 8);
    }

    #[test]
    fn a_hatchment_preserves_the_difference_on_the_complete_arms() {
        let mut zombie = process(3, 1);
        zombie.state = 'Z';
        let snapshot = Snapshot {
            processes: vec![process(1, 0), process(2, 1), zombie.clone()],
            ..Snapshot::default()
        };
        let mut module = Armorial::default();
        module.refresh(
            &snapshot,
            &snapshot.processes.iter().collect::<Vec<_>>(),
            12.0,
        );
        let banner = &module.banners[&zombie.id];
        let field = banner.field.as_ref().unwrap();
        assert_eq!(field.colors[0], Tincture::Sable.color());
        let board_v = 40.0 / (TEXELS - 1) as f32;
        let local_v = 0.5 + (board_v * 2.0 - 1.0) / 0.84;
        let pattern = arms(house(&zombie));
        let painted = pattern.tincture_at([0.5, local_v], zombie.threads, &banner.marks);
        let plain = pattern.tincture_at([0.5, local_v], zombie.threads, &[]);
        assert_ne!(painted, plain);
        assert_eq!(field.colors[40 * TEXELS + 40], painted.color());
    }

    #[test]
    fn unfurling_and_lowering_follow_the_same_path_at_ten_and_sixty_frames_per_second() {
        fn at_rate(rate: u32) -> crate::render::Frame {
            let original = crate::model::demo(12.0, 64);
            let mut changed = original.clone();
            changed.elapsed = 12.5;
            let departed: HashSet<Identity> = top_n(&original.processes, 8, |p| p.memory as f64)
                .iter()
                .map(|p| p.id)
                .collect();
            changed.processes.retain(|p| !departed.contains(&p.id));
            let mut newborn = process(99999, original.processes[1].id.pid);
            newborn.memory = 1 << 32;
            newborn.started = 12.5;
            changed.processes.push(newborn);
            let mut scene = Scene::new();
            let camera = Camera {
                pitch: 0.35,
                ..Camera::default()
            };
            for tick in 0..rate {
                let time = 12.0 + tick as f32 / rate as f32;
                let snapshot = if tick * 2 < rate { &original } else { &changed };
                scene.render(&RenderInput::small(
                    snapshot,
                    view("armorial"),
                    &camera,
                    time,
                ));
            }
            let mut frame = scene.render(&RenderInput::small(
                &changed,
                view("armorial"),
                &camera,
                13.0,
            ));
            frame.rasterize();
            assert!(frame.picks.iter().any(|&pick| pick != NONE));
            frame
        }
        let slow = at_rate(10);
        let fast = at_rate(60);
        assert_eq!(slow.pixels, fast.pixels);
        assert_eq!(slow.picks, fast.picks);
    }

    #[test]
    fn every_banner_has_an_unoccluded_reservation_in_the_default_nave() {
        let snapshot = crate::model::demo(12.0, 192);
        let mut module = Armorial::default();
        module.refresh(
            &snapshot,
            &top_n(&snapshot.processes, CAP, |p| p.memory as f64),
            12.0,
        );
        let camera = Camera {
            pitch: 0.35,
            ..Camera::default()
        };
        let mut scene = Scene::new();
        let frame = scene.render(&RenderInput::small(
            &snapshot,
            view("armorial"),
            &camera,
            12.0,
        ));
        let rectangles: Vec<[f32; 4]> = module
            .banners
            .values()
            .map(|banner| {
                let corners = banner
                    .corners(12.0)
                    .map(|point| frame.project(&camera, point));
                [
                    corners.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
                    corners.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                    corners
                        .iter()
                        .map(|p| p[0])
                        .fold(f32::NEG_INFINITY, f32::max),
                    corners
                        .iter()
                        .map(|p| p[1])
                        .fold(f32::NEG_INFINITY, f32::max),
                ]
            })
            .collect();
        for (index, a) in rectangles.iter().enumerate() {
            for b in &rectangles[index + 1..] {
                assert!(
                    a[2] < b[0] || b[2] < a[0] || a[3] < b[1] || b[3] < a[1],
                    "overlapping reservations: {a:?} and {b:?}"
                );
            }
        }
    }
}
