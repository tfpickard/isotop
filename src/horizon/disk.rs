//! What the horizon view draws of the sky and the disk, as lensed triangles: a mesh of the disk
//! in the (radius, azimuth) plane whose vertices are carried to their images on the screen, once
//! for the primary image and once, coarser, for the secondary image; the shadow, the photon
//! ring and the stars. Nothing is traced per pixel, and both rasterizers draw the same triangles.
//!
//! The vertices are lensed again only when the inclination or the scale changes. Every frame
//! recolours the facets: the intensity and colour temperature that depend only on where a facet
//! is are fixed with the mesh (`Facet::base`), and the gain from memory pressure and the
//! turbulence that rotates with the disk change by the frame.

use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

use super::lens::{self, DISK_INNER, DISK_OUTER, LensTable, Observer, SHADOW_RADIUS, STAR_SHELL};
use crate::blackbody;
use crate::render::{Color, Frame, Item, NONE, Point, Sky};
use crate::simulation::Rng;

/// Depth of each layer, nearer is larger. The layers are far enough apart that the rasterizers
/// need not agree on ties.
pub mod layer {
    pub const SECONDARY_DISK: f32 = 100.0;
    pub const SECONDARY_BODY: f32 = 110.0;
    pub const SHADOW: f32 = 120.0;
    pub const RING: f32 = 130.0;
    pub const PRIMARY_DISK: f32 = 140.0;
    pub const GUIDE: f32 = 150.0;
    pub const BODY: f32 = 160.0;
    pub const PLUNGE: f32 = 175.0;
}

/// How big a facet is on the screen at most, in pixels along a cell's side, and the most
/// triangles the disk may have in both images.
const FACET_PIXELS: f32 = 4.0;
const TRIANGLE_CAP: usize = 120_000;
/// The secondary image has half the density of facets: its sides are this much longer.
const SECONDARY_COARSENING: f32 = std::f32::consts::SQRT_2;
/// Samples of the pre-pass that measures how fast the image moves across the disk.
const COARSE_RADII: usize = 96;
const COARSE_AZIMUTHS: usize = 720;
/// The fewest rows and columns of cells.
const FEWEST_ROWS: usize = 12;
const FEWEST_COLUMNS: usize = 96;
/// Segments of the shadow's fan and of the photon ring.
const SHADOW_SEGMENTS: usize = 96;
const RING_SEGMENTS: usize = 160;
/// The photon ring's centre as an impact parameter above the shadow's edge, its width, and the
/// width it keeps whatever the scale, in pixels.
const RING_OFFSET: f32 = 0.07;
const RING_WIDTH: f32 = 0.10;
const RING_THINNEST: f32 = 1.5;
/// The stars on the shell behind the hole.
const STARS: usize = 1500;
const BRIGHT_STARS: usize = 40;
/// Glares along the inner image of the disk, at most.
const GLARES: usize = 24;

/// The exposure and exponent of the tone curve `1 - exp(-0.36 I^1.3)`.
const EXPOSURE: f32 = 0.36;
const TOE: f32 = 1.3;
/// Entries of the tone table per unit of sqrt(I), and its size.
const TONE_PER_ROOT: f32 = 400.0;
const TONE_ENTRIES: usize = 2048;

/// The colour of a patch of intensity `I` in linear light: `255 srgb(1 - exp(-0.36 I^1.3))`.
pub fn tone(intensity: f32) -> u8 {
    static TABLE: OnceLock<Vec<u8>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..TONE_ENTRIES)
            .map(|entry| {
                let root = entry as f32 / TONE_PER_ROOT;
                let linear = 1.0 - (-EXPOSURE * (root * root).powf(TOE)).exp();
                let encoded = if linear <= 0.003_130_8 {
                    12.92 * linear
                } else {
                    1.055 * linear.powf(1.0 / 2.4) - 0.055
                };
                (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect()
    });
    let entry = (intensity.max(0.0).sqrt() * TONE_PER_ROOT) as usize;
    table[entry.min(TONE_ENTRIES - 1)]
}

/// Where the hole is on the frame and how many pixels one M is.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    /// The frame position of the centre of the shadow.
    pub hole: [f32; 2],
    /// Pixels per M.
    pub scale: f32,
    pub width: f32,
    pub height: f32,
}

impl Screen {
    /// The frame position of a screen offset in M from the hole, x right and y up.
    pub fn pixel(&self, offset: [f32; 2]) -> [f32; 2] {
        [
            self.hole[0] + self.scale * offset[0],
            self.hole[1] - self.scale * offset[1],
        ]
    }

    /// A pixel size at the 900-pixel height the mockup was drawn at.
    pub fn unit(&self) -> f32 {
        self.height / 900.0
    }

    fn sees(&self, points: &[[f32; 2]]) -> bool {
        let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
        for point in points {
            for k in 0..2 {
                low[k] = low[k].min(point[k]);
                high[k] = high[k].max(point[k]);
            }
        }
        high[0] >= 0.0 && low[0] < self.width && high[1] >= 0.0 && low[1] < self.height
    }
}

/// What changes the colour of the disk from frame to frame.
pub struct Light {
    /// `0.45 + 0.9 p / 100` for a memory pressure of `p` percent.
    pub gain: f32,
    /// Coordinate time in M.
    pub time: f32,
    /// The azimuth the camera's rotation adds.
    pub rotation: f32,
    /// The sky the disk glows against.
    pub sky: SkyField,
}

/// The sky of a frame, sampled every `SKY_STEP` pixels: a facet adds its light to it, as a glowing
/// disk adds to what is behind it, so the faint outer edge fades into the haze instead of
/// ending in dark blocks.
pub struct SkyField {
    base: Vec<[f32; 3]>,
    columns: Vec<f32>,
    rows: Vec<f32>,
    haze: [f32; 3],
}

const SKY_STEP: f32 = 8.0;

impl SkyField {
    pub fn new(sky: &Sky, width: u32, height: u32) -> SkyField {
        let (width, height) = (width as f32, height as f32);
        let columns = (0..=(width / SKY_STEP) as usize + 1)
            .map(|k| sky.profile(k as f32 * SKY_STEP / width, 0.0))
            .collect();
        let samples = (0..=(height / SKY_STEP) as usize + 1).map(|k| k as f32 * SKY_STEP / height);
        SkyField {
            base: samples.clone().map(|v| sky.base(v)).collect(),
            columns,
            rows: samples.map(|v| sky.profile(v, 1.9)).collect(),
            haze: if sky.hazy() { sky.haze } else { [0.0; 3] },
        }
    }

    /// The sky colour at a frame position.
    fn at(&self, point: [f32; 2]) -> [f32; 3] {
        let column = ((point[0] / SKY_STEP).max(0.0) as usize).min(self.columns.len() - 1);
        let row = ((point[1] / SKY_STEP).max(0.0) as usize).min(self.rows.len() - 1);
        let density = self.columns[column] * self.rows[row];
        std::array::from_fn(|k| self.base[row][k] + self.haze[k] * density)
    }
}

// ---------------------------------------------------------------------------------------------
// Turbulence
// ---------------------------------------------------------------------------------------------

/// The emissivity of the disk in the frame that goes round with it: smooth noise in streaks
/// long in azimuth and short in radius, with a fine ring structure, normalized to a mean of 1.
/// It is the texture, not data.
struct Turbulence {
    values: Vec<f32>,
}

const TURBULENCE_RADIAL: usize = 256;
const TURBULENCE_AZIMUTHAL: usize = 1024;
/// Octaves: the lattice cell in M and in radians, and the weight.
const OCTAVES: [(f32, f32, f32); 4] = [
    (0.14, 0.10, 0.55),
    (0.40, 0.32, 0.80),
    (1.20, 1.00, 0.60),
    (3.20, 2.40, 0.35),
];
const TURBULENCE_SEED: u64 = 0x7475_7262;

impl Turbulence {
    fn shared() -> &'static Turbulence {
        static TURBULENCE: OnceLock<Turbulence> = OnceLock::new();
        TURBULENCE.get_or_init(Turbulence::build)
    }

    /// Smooth value noise in [-1, 1] on a lattice of `cells` columns (wrapping) and rows.
    fn noise(seed: u64, row: f32, column: f32, columns: usize) -> f32 {
        let (row0, column0) = (row.floor(), column.floor());
        let (fraction_row, fraction_column) = (row - row0, column - column0);
        let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
        let corner = |dr: i64, dc: i64| {
            let r = row0 as i64 + dr;
            let c = (column0 as i64 + dc).rem_euclid(columns as i64);
            let hash = Rng::new(seed ^ ((r as u64) << 32 | c as u64)).next_u64();
            (hash >> 40) as f32 / 8_388_608.0 - 1.0
        };
        let (wr, wc) = (smooth(fraction_row), smooth(fraction_column));
        let near = corner(0, 0) * (1.0 - wc) + corner(0, 1) * wc;
        let far = corner(1, 0) * (1.0 - wc) + corner(1, 1) * wc;
        near * (1.0 - wr) + far * wr
    }

    fn build() -> Turbulence {
        let span = DISK_OUTER - DISK_INNER;
        let mut totals = Vec::with_capacity(TURBULENCE_RADIAL * TURBULENCE_AZIMUTHAL);
        for i in 0..TURBULENCE_RADIAL {
            let radius = span * (i as f32 + 0.5) / TURBULENCE_RADIAL as f32;
            for j in 0..TURBULENCE_AZIMUTHAL {
                let azimuth = TAU * (j as f32 + 0.5) / TURBULENCE_AZIMUTHAL as f32;
                let total: f32 = OCTAVES
                    .iter()
                    .enumerate()
                    .map(|(k, &(cell_radius, cell_azimuth, weight))| {
                        let columns = (TAU / cell_azimuth).round() as usize;
                        weight
                            * Self::noise(
                                TURBULENCE_SEED + k as u64,
                                radius / cell_radius,
                                azimuth / TAU * columns as f32,
                                columns,
                            )
                    })
                    .sum();
                totals.push(total);
            }
        }
        let mean = totals.iter().sum::<f32>() / totals.len() as f32;
        let deviation =
            (totals.iter().map(|t| (t - mean).powi(2)).sum::<f32>() / totals.len() as f32).sqrt();
        let mut values: Vec<f32> = totals
            .iter()
            .enumerate()
            .map(|(index, total)| {
                let radius = DISK_INNER
                    + span * ((index / TURBULENCE_AZIMUTHAL) as f32 + 0.5)
                        / TURBULENCE_RADIAL as f32;
                let rings = 0.35 * (TAU * radius / 0.55 + 3.0 * (radius * 0.7).sin()).sin();
                (0.55 * (total - mean) / deviation + 0.25 * rings).exp()
            })
            .collect();
        let average = values.iter().sum::<f32>() / values.len() as f32;
        values.iter_mut().for_each(|value| *value /= average);
        Turbulence { values }
    }

    /// The emissivity at `radius` and `azimuth` in the co-rotating frame, interpolated.
    fn at(&self, radius: f32, azimuth: f32) -> f32 {
        let row = ((radius - DISK_INNER) / (DISK_OUTER - DISK_INNER) * TURBULENCE_RADIAL as f32
            - 0.5)
            .clamp(0.0, TURBULENCE_RADIAL as f32 - 1.001);
        let column = azimuth.rem_euclid(TAU) / TAU * TURBULENCE_AZIMUTHAL as f32 - 0.5;
        let column = if column < 0.0 {
            column + TURBULENCE_AZIMUTHAL as f32
        } else {
            column
        };
        let (i, j) = (row as usize, column as usize);
        let (fraction_row, fraction_column) = (row - i as f32, column - j as f32);
        let at =
            |i: usize, j: usize| self.values[i * TURBULENCE_AZIMUTHAL + j % TURBULENCE_AZIMUTHAL];
        let near = at(i, j) * (1.0 - fraction_column) + at(i, j + 1) * fraction_column;
        let far = at(i + 1, j) * (1.0 - fraction_column) + at(i + 1, j + 1) * fraction_column;
        near * (1.0 - fraction_row) + far * fraction_row
    }
}

// ---------------------------------------------------------------------------------------------
// The mesh
// ---------------------------------------------------------------------------------------------

/// One triangle of a sheet: its corners, and what is fixed about its colour.
struct Facet {
    corners: [u32; 3],
    /// Linear colour at unit gain and unit turbulence: `F g^4` times the blackbody colour of
    /// `7200 K (F^(1/4) g)^1.55`, `g` being the inverse of one plus the redshift.
    base: [f32; 3],
    /// Where the facet's centre is on the disk, and the angular velocity there.
    radius: f32,
    azimuth: f32,
    spin: f32,
}

/// The disk seen as one of its two images.
struct Mesh {
    /// Vertices as offsets from the hole, in M, x right and y up.
    offsets: Vec<[f32; 2]>,
    facets: Vec<Facet>,
}

/// The pre-pass for one image of the disk: how fast it moves across the screen along each row
/// and column of a coarse grid, which sets how finely the real mesh must be cut.
struct Sheet {
    secondary: bool,
    radii: Vec<f32>,
    azimuths: Vec<f32>,
    /// Pixels per unit of radius, for each coarse row, and per radian, for each coarse column.
    need_radius: Vec<f32>,
    need_azimuth: Vec<f32>,
    /// The largest |x| and |y| an image of the disk reaches, in M.
    extent: [f32; 2],
}

fn linspace(from: f32, to: f32, count: usize) -> Vec<f32> {
    (0..=count)
        .map(|k| from + (to - from) * k as f32 / count as f32)
        .collect()
}

impl Sheet {
    fn image(
        table: &LensTable,
        observer: Observer,
        secondary: bool,
        radius: f32,
        azimuth: f32,
    ) -> [f32; 2] {
        let images = observer.disk_images(table, radius, azimuth);
        if secondary {
            images.secondary
        } else {
            images.primary
        }
    }

    fn measure(table: &LensTable, observer: Observer, secondary: bool, scale: f32) -> Sheet {
        let radii = linspace(DISK_INNER, DISK_OUTER, COARSE_RADII);
        let azimuths = linspace(0.0, TAU, COARSE_AZIMUTHS);
        let grid: Vec<[f32; 2]> = radii
            .iter()
            .flat_map(|&radius| {
                azimuths[..COARSE_AZIMUTHS]
                    .iter()
                    .map(move |&azimuth| Self::image(table, observer, secondary, radius, azimuth))
            })
            .collect();
        let at = |i: usize, j: usize| grid[i * COARSE_AZIMUTHS + j % COARSE_AZIMUTHS];
        let distance = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        let (step_radius, step_azimuth) = (radii[1] - radii[0], azimuths[1] - azimuths[0]);
        let need_azimuth = (0..COARSE_AZIMUTHS)
            .map(|j| {
                (0..=COARSE_RADII)
                    .map(|i| distance(at(i, j), at(i, j + 1)))
                    .fold(0.0, f32::max)
                    * scale
                    / step_azimuth
            })
            .collect();
        let need_radius = (0..COARSE_RADII)
            .map(|i| {
                (0..COARSE_AZIMUTHS)
                    .map(|j| distance(at(i, j), at(i + 1, j)))
                    .fold(0.0, f32::max)
                    * scale
                    / step_radius
            })
            .collect();
        let extent = grid.iter().fold([0.0_f32; 2], |extent, point| {
            [extent[0].max(point[0].abs()), extent[1].max(point[1].abs())]
        });
        Sheet {
            secondary,
            radii,
            azimuths,
            need_radius,
            need_azimuth,
            extent,
        }
    }

    /// The cell edges that keep a cell's side to `target` pixels or less: along the radius, and
    /// round the azimuth.
    fn edges(&self, target: f32) -> (Vec<f32>, Vec<f32>) {
        (
            spaced(&self.radii, &self.need_radius, target, FEWEST_ROWS, false),
            spaced(
                &self.azimuths,
                &self.need_azimuth,
                target,
                FEWEST_COLUMNS,
                true,
            ),
        )
    }

    /// How many triangles the cells with `target` pixels to a side make.
    fn triangles(&self, target: f32) -> usize {
        let (rows, columns) = self.edges(target);
        2 * (rows.len() - 1) * (columns.len() - 1)
    }

    /// Cuts the mesh at `target` pixels to a cell and lenses its vertices.
    fn mesh(&self, table: &LensTable, observer: Observer, target: f32) -> Mesh {
        let (rows, columns) = self.edges(target);
        let (row_count, column_count) = (rows.len() - 1, columns.len() - 1);
        let mut offsets = Vec::with_capacity((row_count + 1) * column_count);
        for &radius in &rows {
            for &azimuth in &columns[..column_count] {
                offsets.push(Self::image(
                    table,
                    observer,
                    self.secondary,
                    radius,
                    azimuth,
                ));
            }
        }
        let vertex = |i: usize, j: usize| (i * column_count + j % column_count) as u32;
        let inclination = observer.inclination();
        let mut facets = Vec::with_capacity(2 * row_count * column_count);
        for i in 0..row_count {
            for j in 0..column_count {
                let (radius0, radius1) = (rows[i], rows[i + 1]);
                let (azimuth0, azimuth1) = (columns[j], columns[j + 1]);
                let (v00, v10, v11, v01) = (
                    vertex(i, j),
                    vertex(i + 1, j),
                    vertex(i + 1, j + 1),
                    vertex(i, j + 1),
                );
                // The primary image colours each triangle at its own centre; the secondary,
                // whose cells are coarser and sheared, colours both at the centre of the cell, so
                // its triangles do not alternate.
                let middle = (0.5 * (radius0 + radius1), 0.5 * (azimuth0 + azimuth1));
                for (corners, (radius, azimuth)) in [
                    (
                        [v00, v10, v11],
                        (
                            (radius0 + 2.0 * radius1) / 3.0,
                            (2.0 * azimuth0 + azimuth1) / 3.0,
                        ),
                    ),
                    (
                        [v00, v11, v01],
                        (
                            (2.0 * radius0 + radius1) / 3.0,
                            (azimuth0 + 2.0 * azimuth1) / 3.0,
                        ),
                    ),
                ] {
                    let (radius, azimuth) = if self.secondary {
                        middle
                    } else {
                        (radius, azimuth)
                    };
                    let across = corners
                        .iter()
                        .map(|&corner| offsets[corner as usize][0])
                        .sum::<f32>()
                        / 3.0;
                    let emission = lens::disk_emission(radius, across, inclination);
                    let colour = blackbody::linear(emission.kelvin);
                    facets.push(Facet {
                        corners,
                        base: colour.map(|channel| channel * emission.intensity),
                        radius,
                        azimuth,
                        spin: lens::angular_velocity(radius),
                    });
                }
            }
        }
        Mesh { offsets, facets }
    }
}

/// A grid of edges from the first node to the last whose cells keep `need * width <= target`:
/// the density of cells is `need / target` per unit, made smooth by a maximum over nine
/// neighbours and a Gaussian of 3 samples, and integrated; the edges split the total evenly.
/// `need[k]` holds for the interval from `nodes[k]` to `nodes[k + 1]`.
fn spaced(nodes: &[f32], need: &[f32], target: f32, fewest: usize, periodic: bool) -> Vec<f32> {
    let count = need.len();
    let at = |k: isize| -> usize {
        if periodic {
            k.rem_euclid(count as isize) as usize
        } else {
            k.clamp(0, count as isize - 1) as usize
        }
    };
    let widest: Vec<f32> = (0..count as isize)
        .map(|k| (k - 4..=k + 4).map(|n| need[at(n)]).fold(0.0, f32::max))
        .collect();
    let weights: Vec<f32> = (-9..=9)
        .map(|n: i32| (-0.5 * (n as f32 / 3.0).powi(2)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    let density: Vec<f32> = (0..count as isize)
        .map(|k| {
            let blurred: f32 = (-9..=9)
                .zip(&weights)
                .map(|(n, weight)| weight * widest[at(k + n as isize)])
                .sum();
            (blurred / total / target).max(1e-3)
        })
        .collect();
    let mut cumulative = vec![0.0];
    for (k, density) in density.iter().enumerate() {
        cumulative.push(cumulative[k] + density * (nodes[k + 1] - nodes[k]));
    }
    let cells = (*cumulative.last().expect("one entry") as usize).max(fewest);
    let whole = *cumulative.last().expect("one entry");
    let mut at_node = 0;
    (0..=cells)
        .map(|edge| {
            let wanted = whole * edge as f32 / cells as f32;
            while at_node + 1 < count && cumulative[at_node + 1] < wanted {
                at_node += 1;
            }
            let (low, high) = (cumulative[at_node], cumulative[at_node + 1]);
            let fraction = if high > low {
                ((wanted - low) / (high - low)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            nodes[at_node] + (nodes[at_node + 1] - nodes[at_node]) * fraction
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Stars
// ---------------------------------------------------------------------------------------------

/// The images of one star of the shell behind the hole.
struct StarImage {
    offset: [f32; 2],
    color: Color,
    /// Brightness of the image, which glows when it is among the brightest.
    amplitude: f32,
}

const STAR_SEED: u64 = 0x0073_7461_7273;

/// Both images of every star on the shell, with their brightness magnified by how much the lens
/// stretches the star's neighbourhood, as `horizon.py`'s `star_layer`: a star `beta` from the
/// line of sight on the shell at the angle `theta` on the sky.
fn lensed_stars(table: &LensTable, observer: Observer) -> Vec<StarImage> {
    let mut rng = Rng::new(STAR_SEED);
    let inclination = observer.inclination();
    let (sin, cos) = inclination.sin_cos();
    // The observer's direction, and the screen's right and up, in the hole's frame.
    let (toward, right, up) = ([0.0, -sin, cos], [1.0, 0.0, 0.0], [0.0, cos, sin]);
    let place = |beta: f32, theta: f32| -> [f32; 3] {
        let behind = (1.0 - (beta / STAR_SHELL).powi(2)).max(0.0).sqrt();
        let spread = beta / STAR_SHELL;
        std::array::from_fn(|k| {
            STAR_SHELL
                * (-behind * toward[k] + spread * (theta.cos() * right[k] + theta.sin() * up[k]))
        })
    };
    let mut images = Vec::with_capacity(2 * STARS);
    for _ in 0..STARS {
        let beta = STAR_SHELL * rng.unit().sqrt() * 0.999;
        let theta = rng.unit() * TAU;
        let flux = (1.0 / (1.0 - rng.unit()).powf(1.0 / 1.6)).clamp(1.0, 40.0) * 0.2;
        let kelvin = (6500.0_f32.ln() + 0.45 * rng.normal()).exp();
        let step = 0.02;
        let base = observer.images(table, place(beta, theta));
        let radial = observer.images(table, place(beta + step, theta));
        let tangent = observer.images(table, place(beta, theta + step / beta.max(1e-3)));
        for (first, second, third) in [
            (base.primary, radial.primary, tangent.primary),
            (base.secondary, radial.secondary, tangent.secondary),
        ] {
            let stretch = |other: [f32; 2]| (other[0] - first[0]).hypot(other[1] - first[1]) / step;
            let amplitude = flux * stretch(third).min(4.0) * stretch(second).clamp(0.25, 3.0);
            if amplitude < 0.03 {
                continue;
            }
            let brightness = 0.1 + 0.9 * (amplitude / (amplitude + 0.9)).powf(0.9);
            images.push(StarImage {
                offset: first,
                color: blackbody::color(kelvin, brightness),
                amplitude,
            });
        }
    }
    images
}

// ---------------------------------------------------------------------------------------------
// The disk
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
struct Key {
    inclination: u32,
    scale: u32,
}

/// The lensed sky and disk for one inclination and scale.
#[derive(Default)]
pub struct Disk {
    key: Option<Key>,
    primary: Option<Mesh>,
    secondary: Option<Mesh>,
    stars: Vec<StarImage>,
    /// The brightest stars, as indices into `stars`.
    bright: Vec<usize>,
    extent: [f32; 2],
    /// Frame positions of the vertices of the mesh being drawn.
    scratch: Vec<[f32; 2]>,
}

impl Disk {
    /// Lenses the vertices for `observer` at `scale` pixels per M, if either changed.
    pub fn prepare(&mut self, observer: Observer, scale: f32) {
        let key = Key {
            inclination: observer.inclination().to_bits(),
            scale: scale.to_bits(),
        };
        if self.key == Some(key) {
            return;
        }
        let table = LensTable::shared();
        let first = Sheet::measure(table, observer, false, scale);
        let second = Sheet::measure(table, observer, true, scale);
        let mut target = FACET_PIXELS;
        while first.triangles(target) + second.triangles(target * SECONDARY_COARSENING)
            > TRIANGLE_CAP
            && target < 200.0
        {
            target *= 1.1;
        }
        self.primary = Some(first.mesh(table, observer, target));
        self.secondary = Some(second.mesh(table, observer, target * SECONDARY_COARSENING));
        self.extent = [
            first.extent[0].max(second.extent[0]).max(SHADOW_RADIUS),
            first.extent[1].max(second.extent[1]).max(SHADOW_RADIUS),
        ];
        self.stars = lensed_stars(table, observer);
        let mut order: Vec<usize> = (0..self.stars.len()).collect();
        order.sort_by(|&a, &b| self.stars[b].amplitude.total_cmp(&self.stars[a].amplitude));
        order.truncate(BRIGHT_STARS);
        self.bright = order;
        self.key = Some(key);
    }

    /// The largest |x| and |y| any image of the disk reaches, in M: what the camera fit frames.
    pub fn extent(&self) -> [f32; 2] {
        self.extent
    }

    /// How many triangles the two images of the disk are cut into.
    #[cfg(test)]
    pub fn triangles(&self) -> usize {
        self.primary
            .iter()
            .chain(&self.secondary)
            .map(|mesh| mesh.facets.len())
            .sum()
    }

    /// The stars, first in the display list so that everything else covers them, then the glows of
    /// the brightest. Positions are whole pixels, which both rasterizers plot alike.
    pub fn stars(&self, frame: &mut Frame, screen: &Screen) {
        for star in &self.stars {
            let [x, y] = screen.pixel(star.offset);
            let (x, y) = (x.floor(), y.floor());
            if x >= 0.0 && y >= 0.0 && x < screen.width && y < screen.height {
                frame.items.push(Item::Star([x, y], star.color));
            }
        }
        for &index in &self.bright {
            let star = &self.stars[index];
            let [x, y] = screen.pixel(star.offset);
            let brightness = (star.amplitude / (star.amplitude + 1.5)).clamp(0.0, 1.0);
            if x > -20.0 && y > -20.0 && x < screen.width + 20.0 && y < screen.height + 20.0 {
                frame.items.push(Item::Glow {
                    center: [x.floor() + 0.5, y.floor() + 0.5],
                    radius: (3.0 + 4.0 * brightness) * screen.unit(),
                    color: star.color,
                    strength: 0.4 * brightness,
                });
            }
        }
    }

    /// The secondary image of the disk, at the farthest layer.
    pub fn draw_secondary(&mut self, frame: &mut Frame, screen: &Screen, light: &Light) {
        if let Some(mesh) = self.secondary.as_ref() {
            Self::draw_mesh(
                mesh,
                &mut self.scratch,
                frame,
                screen,
                light,
                layer::SECONDARY_DISK,
            );
        }
    }

    /// The primary image of the disk.
    pub fn draw_primary(&mut self, frame: &mut Frame, screen: &Screen, light: &Light) {
        if let Some(mesh) = self.primary.as_ref() {
            Self::draw_mesh(
                mesh,
                &mut self.scratch,
                frame,
                screen,
                light,
                layer::PRIMARY_DISK,
            );
        }
    }

    fn draw_mesh(
        mesh: &Mesh,
        scratch: &mut Vec<[f32; 2]>,
        frame: &mut Frame,
        screen: &Screen,
        light: &Light,
        depth: f32,
    ) {
        scratch.clear();
        scratch.extend(mesh.offsets.iter().map(|&offset| screen.pixel(offset)));
        let turbulence = Turbulence::shared();
        for facet in &mesh.facets {
            let corners = facet.corners.map(|corner| scratch[corner as usize]);
            if !screen.sees(&corners) {
                continue;
            }
            let swirl = facet.azimuth + light.rotation - facet.spin * light.time;
            let gain = light.gain * turbulence.at(facet.radius, swirl);
            let glow = facet.base.map(|channel| tone(gain * channel));
            if glow.iter().all(|&channel| channel < 2) {
                continue;
            }
            let [a, b, c] = corners;
            let sky = light
                .sky
                .at([(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0]);
            let color = std::array::from_fn(|k| (f32::from(glow[k]) + sky[k]).min(255.0) as u8);
            frame.items.push(Item::Triangle(
                [
                    [a[0], a[1], depth],
                    [b[0], b[1], depth],
                    [c[0], c[1], depth],
                ],
                color,
                NONE,
            ));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shadow, ring, glare and guides
// ---------------------------------------------------------------------------------------------

fn push_quad(frame: &mut Frame, quad: [Point; 4], color: Color) {
    frame
        .items
        .push(Item::Triangle([quad[0], quad[1], quad[2]], color, NONE));
    frame
        .items
        .push(Item::Triangle([quad[0], quad[2], quad[3]], color, NONE));
}

/// The black disc of radius `3 sqrt 3` M that no photon from behind the hole crosses, as a fan.
pub fn draw_shadow(frame: &mut Frame, screen: &Screen) {
    // The corners lie on a circle a little larger than the shadow, so the polygon's flats are
    // never inside it.
    let radius = SHADOW_RADIUS * screen.scale / (PI / SHADOW_SEGMENTS as f32).cos();
    let center = [screen.hole[0], screen.hole[1], layer::SHADOW];
    let corner = |k: usize| {
        let angle = TAU * k as f32 / SHADOW_SEGMENTS as f32;
        [
            screen.hole[0] + radius * angle.cos(),
            screen.hole[1] + radius * angle.sin(),
            layer::SHADOW,
        ]
    };
    for k in 0..SHADOW_SEGMENTS {
        frame.items.push(Item::Triangle(
            [center, corner(k), corner(k + 1)],
            [0, 0, 0],
            NONE,
        ));
    }
}

/// The photon ring: light that has gone round the hole, a thin ring just outside the shadow. It
/// is drawn, not traced: its radius and width are fixed, and its colour follows the Doppler and
/// gravitational shift of matter at 7 M, brighter and bluer where the disk approaches.
pub fn draw_ring(frame: &mut Frame, screen: &Screen, inclination: f32) {
    let middle = (SHADOW_RADIUS + RING_OFFSET) * screen.scale;
    let half = (0.5 * RING_WIDTH * screen.scale).max(0.5 * RING_THINNEST);
    let (inner, outer) = (middle - half, middle + half);
    let corner = |radius: f32, k: usize| {
        let angle = TAU * k as f32 / RING_SEGMENTS as f32;
        [
            screen.hole[0] + radius * angle.cos(),
            screen.hole[1] + radius * angle.sin(),
            layer::RING,
        ]
    };
    for k in 0..RING_SEGMENTS {
        let angle = TAU * (k as f32 + 0.5) / RING_SEGMENTS as f32;
        let shift = 1.0
            / lens::one_plus_z(
                7.0,
                (SHADOW_RADIUS + RING_OFFSET) * angle.cos(),
                inclination,
            );
        let kelvin = 7200.0 * (0.95 * shift).powf(1.55);
        let intensity = 2.4 * shift.powi(4);
        let color = blackbody::linear(kelvin).map(|channel| tone(channel * intensity));
        push_quad(
            frame,
            [
                corner(inner, k),
                corner(outer, k),
                corner(outer, k + 1),
                corner(inner, k + 1),
            ],
            color,
        );
    }
}

/// The lensed primary image of the circle of `radius` in the disk plane, as offsets in M.
pub fn orbit_image(table: &LensTable, observer: Observer, radius: f32) -> Vec<[f32; 2]> {
    (0..=720)
        .map(|k| {
            observer
                .disk_images(table, radius, TAU * k as f32 / 720.0)
                .primary
        })
        .collect()
}

/// Strokes `points` (offsets in M) as dashes of `on` pixels every `on + off` pixels, `width`
/// pixels wide, as ribbons of triangles that both rasterizers fill alike. A dash that turns a
/// corner is drawn piece by piece.
pub fn draw_dashed(
    frame: &mut Frame,
    screen: &Screen,
    points: &[[f32; 2]],
    (on, off): (f32, f32),
    width: f32,
    color: Color,
) {
    let pixels: Vec<[f32; 2]> = points.iter().map(|&point| screen.pixel(point)).collect();
    // Where each vertex stands along the path, in pixels.
    let mut along = vec![0.0_f64];
    for pair in pixels.windows(2) {
        let last = *along.last().expect("one entry");
        along.push(last + f64::from((pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1])));
    }
    let total = *along.last().expect("one entry");
    let at = |position: f64| -> [f32; 3] {
        let segment = along
            .partition_point(|&reached| reached <= position)
            .clamp(1, pixels.len() - 1);
        let (from, to) = (along[segment - 1], along[segment]);
        let fraction = if to > from {
            ((position - from) / (to - from)) as f32
        } else {
            0.0
        };
        let (a, b) = (pixels[segment - 1], pixels[segment]);
        [
            a[0] + (b[0] - a[0]) * fraction,
            a[1] + (b[1] - a[1]) * fraction,
            layer::GUIDE,
        ]
    };
    if pixels.len() < 2 || total <= 0.0 || on <= 0.0 {
        return;
    }
    let period = f64::from(on + off);
    let dashes = (total / period).ceil() as usize;
    for dash in 0..dashes {
        let (start, end) = (
            dash as f64 * period,
            (dash as f64 * period + f64::from(on)).min(total),
        );
        // The corners of the path inside the dash split it into pieces.
        let mut from = start;
        let first = along.partition_point(|&reached| reached <= start);
        let last = along.partition_point(|&reached| reached < end);
        for &corner in &along[first..last.max(first)] {
            frame.hairline_between(at(from), at(corner), color, width);
            from = corner;
        }
        frame.hairline_between(at(from), at(end), color, width);
    }
}

/// Glows along the inner image of the disk, as many as fit without touching, at most `GLARES`,
/// where the disk is brightest. Like the texture and the stars they are not data.
pub fn draw_glare(
    frame: &mut Frame,
    screen: &Screen,
    table: &LensTable,
    observer: Observer,
    light: &Light,
) {
    let radius = 0.03 * screen.height;
    let inclination = observer.inclination();
    let points: Vec<[f32; 2]> = (0..=720)
        .map(|k| {
            observer
                .disk_images(table, DISK_INNER + 0.6, TAU * k as f32 / 720.0)
                .primary
        })
        .collect();
    let mut placed: Vec<[f32; 2]> = Vec::new();
    // The brightest places first, so the crowd of glows sits where the light is.
    let mut order: Vec<(f32, usize)> = (0..720)
        .map(|k| {
            let emission = lens::disk_emission(DISK_INNER + 0.6, points[k][0], inclination);
            (emission.intensity, k)
        })
        .collect();
    order.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    for (intensity, k) in order {
        if placed.len() >= GLARES {
            break;
        }
        let at = screen.pixel(points[k]);
        if placed
            .iter()
            .any(|other| (other[0] - at[0]).hypot(other[1] - at[1]) < 2.0 * radius)
        {
            continue;
        }
        placed.push(at);
        let warm = blackbody::linear(
            lens::disk_emission(DISK_INNER + 0.6, points[k][0], inclination).kelvin,
        );
        let strength = (0.5 * light.gain * intensity).clamp(0.0, 1.0).sqrt() * 0.35;
        if strength < 0.02
            || at[0] < -radius
            || at[1] < -radius
            || at[0] > screen.width + radius
            || at[1] > screen.height + radius
        {
            continue;
        }
        frame.items.push(Item::Glow {
            center: at,
            radius,
            color: warm.map(|channel| tone(channel * 2.0)),
            strength,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Backdrop, Buffers, Sky};

    fn blank(width: u32, height: u32) -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Void,
            haze: [0.0; 3],
            time: 0.0,
        };
        Frame::new(width, height, sky, Buffers::default())
    }

    fn disk_at(inclination_degrees: f32, scale: f32) -> Disk {
        let mut disk = Disk::default();
        disk.prepare(Observer::new(inclination_degrees.to_radians()), scale);
        disk
    }

    #[test]
    fn the_tone_curve_is_the_exposure_curve_in_srgb() {
        assert_eq!(tone(0.0), 0);
        // I = 1: 1 - exp(-0.36) = 0.3023 linear, which is 0.5897 in sRGB.
        let expected =
            (255.0 * (1.055 * (1.0 - (-0.36_f32).exp()).powf(1.0 / 2.4) - 0.055)).round();
        assert!((f32::from(tone(1.0)) - expected).abs() <= 1.0);
        assert_eq!(tone(1000.0), tone(100.0), "it saturates");
        assert!(tone(8.0) > 240);
        assert!((0..60).all(|k| tone(k as f32 * 0.05) <= tone(k as f32 * 0.05 + 0.05)));
        // Darker than 2/255 is where facets are skipped: that is below an intensity of 0.008.
        assert!(tone(0.005) < 2 && tone(0.02) >= 2);
    }

    #[test]
    fn the_turbulence_has_a_mean_of_one_and_goes_round_with_the_disk() {
        let turbulence = Turbulence::shared();
        let mean = turbulence.values.iter().sum::<f32>() / turbulence.values.len() as f32;
        assert!((mean - 1.0).abs() < 1e-3);
        assert!(
            turbulence
                .values
                .iter()
                .all(|&value| value > 0.0 && value < 40.0)
        );
        // Periodic in azimuth.
        let (a, b) = (turbulence.at(12.0, 0.001), turbulence.at(12.0, TAU + 0.001));
        assert!((a - b).abs() < 1e-5);
        // Streaks: neighbouring azimuths along a ring agree more than neighbouring radii.
        let mut along = 0.0;
        let mut across = 0.0;
        for k in 0..400 {
            let (r, a) = (8.0 + k as f32 * 0.05, k as f32 * 0.37);
            along += (turbulence.at(r, a) - turbulence.at(r, a + 0.05)).abs();
            across += (turbulence.at(r, a) - turbulence.at(r + 0.05 * 3.0, a)).abs();
        }
        assert!(
            along < across,
            "streaks are long in azimuth: {along} vs {across}"
        );
    }

    #[test]
    fn the_mesh_stays_within_its_triangle_cap_and_is_denser_for_the_primary_image() {
        for (inclination, scale) in [
            (80.0, 27.0),
            (80.0, 13.5),
            (40.0, 27.0),
            (0.0, 20.0),
            (86.0, 40.0),
        ] {
            let disk = disk_at(inclination, scale);
            assert!(
                disk.triangles() <= TRIANGLE_CAP,
                "{inclination} deg {scale} px/M: {}",
                disk.triangles()
            );
            let (first, second) = (
                disk.primary.as_ref().unwrap(),
                disk.secondary.as_ref().unwrap(),
            );
            assert!(
                first.facets.len() > second.facets.len() / 2,
                "primary is the finer"
            );
            assert!(first.facets.len() > 1000);
        }
        // A small disk is cut at the full density, a large one coarser to stay within the cap.
        let small = disk_at(80.0, 8.0);
        assert!(small.triangles() < 40_000);
    }

    #[test]
    fn the_mesh_is_watertight_and_covers_the_disk_once() {
        let disk = disk_at(80.0, 20.0);
        let mesh = disk.primary.as_ref().unwrap();
        // Every interior edge of a periodic strip of cells is shared by exactly two triangles,
        // the edges at the inner and outer radius by one.
        let mut edges = std::collections::HashMap::new();
        for facet in &mesh.facets {
            for k in 0..3 {
                let (a, b) = (facet.corners[k], facet.corners[(k + 1) % 3]);
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&count| count == 1 || count == 2));
        let boundary = edges.values().filter(|&&count| count == 1).count();
        // Vertices are rows + 1 rings of `columns`, cells are two triangles each.
        let columns = mesh.offsets.len() - mesh.facets.len() / 2;
        assert_eq!(
            boundary,
            2 * columns,
            "only the inner and the outer rim are open"
        );
    }

    #[test]
    fn the_extent_of_the_disk_grows_as_the_view_goes_face_on() {
        let edge_on = disk_at(86.0, 20.0).extent();
        let tilted = disk_at(60.0, 20.0).extent();
        let face_on = disk_at(0.0, 20.0).extent();
        assert!(edge_on[0] > 29.0 && edge_on[0] < 40.0, "{edge_on:?}");
        assert!(
            edge_on[1] < tilted[1] && tilted[1] < face_on[1],
            "{edge_on:?} {tilted:?} {face_on:?}"
        );
        assert!(
            (face_on[1] - face_on[0]).abs() < 2.0,
            "round face-on: {face_on:?}"
        );
    }

    #[test]
    fn a_drawn_disk_is_triangles_stars_in_front_and_none_pickable() {
        let mut disk = disk_at(80.0, 13.5);
        let mut frame = blank(960, 540);
        let screen = Screen {
            hole: [480.0, 270.0],
            scale: 13.5,
            width: 960.0,
            height: 540.0,
        };
        let light = Light {
            gain: 0.9,
            time: 100.0,
            rotation: 0.0,
            sky: SkyField::new(&frame.sky, 960, 540),
        };
        disk.stars(&mut frame, &screen);
        let stars = frame.items.len();
        assert!(stars > 300, "{stars} items of sky");
        assert!(
            frame
                .items
                .iter()
                .all(|item| matches!(item, Item::Star(..) | Item::Glow { .. }))
        );
        assert!(
            frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Glow { .. }))
                .count()
                <= BRIGHT_STARS
        );
        disk.draw_secondary(&mut frame, &screen, &light);
        disk.draw_primary(&mut frame, &screen, &light);
        draw_shadow(&mut frame, &screen);
        draw_ring(&mut frame, &screen, 80.0_f32.to_radians());
        let triangles = frame.items[stars..]
            .iter()
            .filter(|item| matches!(item, Item::Triangle(_, _, pick) if *pick == NONE))
            .count();
        assert_eq!(triangles, frame.items.len() - stars);
        assert!(triangles > 5000);
    }

    #[test]
    fn the_shadow_covers_what_the_primary_disk_does_not_and_the_ring_sits_outside_it() {
        let mut frame = blank(400, 300);
        let screen = Screen {
            hole: [200.0, 150.0],
            scale: 10.0,
            width: 400.0,
            height: 300.0,
        };
        draw_shadow(&mut frame, &screen);
        draw_ring(&mut frame, &screen, 80.0_f32.to_radians());
        frame.rasterize();
        let at = |x: usize, y: usize| {
            let i = (y * 400 + x) * 3;
            [frame.pixels[i], frame.pixels[i + 1], frame.pixels[i + 2]]
        };
        // Inside the shadow, 5.0 M from the centre: black.
        assert_eq!(at(200 + 49, 150), [0, 0, 0]);
        // On the ring, at 5.27 M, brighter where the disk approaches (left) than where it
        // recedes (right).
        let (approaching, receding) = (at(200 - 53, 150), at(200 + 52, 150));
        assert!(
            approaching.iter().any(|&channel| channel > 200),
            "{approaching:?}"
        );
        assert!(
            receding.iter().all(|&channel| channel < approaching[0]),
            "{receding:?}"
        );
        assert!(receding.iter().any(|&channel| channel > 40), "{receding:?}");
        // Beyond the ring it is the sky again.
        assert_eq!(at(200 + 58, 150), [0, 0, 0]);
    }

    #[test]
    fn dashes_alternate_by_pixel_length_along_the_polyline() {
        let mut frame = blank(200, 50);
        let screen = Screen {
            hole: [0.0, 25.0],
            scale: 1.0,
            width: 200.0,
            height: 50.0,
        };
        let line: Vec<[f32; 2]> = (0..=100).map(|k| [k as f32, 0.0]).collect();
        draw_dashed(&mut frame, &screen, &line, (6.0, 4.0), 1.5, [255, 255, 255]);
        // Each polyline segment draws its own piece; pieces that meet are one dash.
        let mut spans: Vec<(f32, f32)> = Vec::new();
        for pair in frame.items.chunks(2) {
            let Item::Triangle(corners, ..) = pair[0] else {
                unreachable!()
            };
            let from = corners.iter().map(|c| c[0]).fold(f32::MAX, f32::min);
            let to = corners.iter().map(|c| c[0]).fold(f32::MIN, f32::max);
            match spans.last_mut() {
                Some(last) if from <= last.1 + 1e-3 => last.1 = last.1.max(to),
                _ => spans.push((from, to)),
            }
        }
        // Ten dashes of six pixels, ten pixels apart.
        assert_eq!(spans.len(), 10);
        for (k, (from, to)) in spans.iter().enumerate() {
            assert!(
                (from - k as f32 * 10.0).abs() < 1e-3 && (to - from - 6.0).abs() < 1e-3,
                "{k}: {from} {to}"
            );
        }
    }

    #[test]
    fn glare_is_a_few_separate_glows_on_the_bright_side() {
        let observer = Observer::new(80.0_f32.to_radians());
        let screen = Screen {
            hole: [480.0, 270.0],
            scale: 13.5,
            width: 960.0,
            height: 540.0,
        };
        let mut frame = blank(960, 540);
        let light = Light {
            gain: 0.9,
            time: 0.0,
            rotation: 0.0,
            sky: SkyField::new(&frame.sky, 960, 540),
        };
        draw_glare(&mut frame, &screen, LensTable::shared(), observer, &light);
        let glows: Vec<([f32; 2], f32, f32)> = frame
            .items
            .iter()
            .filter_map(|item| match *item {
                Item::Glow {
                    center,
                    radius,
                    strength,
                    ..
                } => Some((center, radius, strength)),
                _ => None,
            })
            .collect();
        assert!(!glows.is_empty() && glows.len() <= GLARES);
        for (k, (a, radius, _)) in glows.iter().enumerate() {
            for (b, ..) in &glows[k + 1..] {
                assert!(
                    (a[0] - b[0]).hypot(a[1] - b[1]) >= 2.0 * radius - 1e-3,
                    "glows touch"
                );
            }
        }
        // Stronger on the approaching, left, side.
        let strength = |left: bool| -> f32 {
            glows
                .iter()
                .filter(|(center, ..)| (center[0] < 480.0) == left)
                .map(|(.., strength)| strength)
                .sum()
        };
        assert!(
            strength(true) > 1.5 * strength(false),
            "{} vs {}",
            strength(true),
            strength(false)
        );
    }
}
