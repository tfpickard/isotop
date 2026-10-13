//! The ground: one field the whole slab is drawn as, made of static sand, the tint and string
//! outline of each region, and the pheromone's glow added over them each frame. Everything but the
//! glow is decoration: the dunes, ripples, specks and the rim's darkening carry no data.

use super::colony::Pheromone;
use super::slab::{Rect, Region, Slab};
use crate::geometry::{dot, normalize};
use crate::medium::Grid;
use crate::render::{Camera, Color, Field, Frame, LIGHT, Placement, Point, Relief};
use crate::simulation::hash;

/// Seeds the sand, apart from every other field the app draws.
const SAND_SEED: u32 = 0x5a4d_0a17;

/// The field's key: stable, and distinct from any other view's.
const KEY: u64 = 0x0a27_0f1e_1d00_0001;

/// The colour of dry sand before it is lit.
const SAND: [f32; 3] = [100.0, 83.0, 65.0];

/// The colour of the string that outlines a region.
const STRING: [f32; 3] = [196.0, 178.0, 150.0];

/// How strongly the pheromone's glow is added to the sand, from 0 to 1.
const GLOW: f32 = 0.78;

/// How quickly the glow saturates with the amount of pheromone.
const SATURATION: f32 = 2.2;

/// How far a region's string lies inside its edge and how wide it is, in world units.
const STRING_INSET: f32 = 0.45;
const STRING_WIDTH: f32 = 0.3;

/// A uniform number in [0, 1) from three integers.
fn lattice(seed: u32, x: i32, y: i32) -> f32 {
    let mixed =
        hash::murmur_mix(seed ^ hash::murmur_mix(x as u32).wrapping_add(hash::knuth(y as u32)));
    (mixed >> 8) as f32 / 16_777_216.0
}

/// Value noise: the lattice's values, smoothly interpolated.
fn noise(seed: u32, x: f32, y: f32) -> f32 {
    let (cx, cy) = (x.floor(), y.floor());
    let (fx, fy) = (x - cx, y - cy);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (smooth(fx), smooth(fy));
    let at = |dx: i32, dy: i32| lattice(seed, cx as i32 + dx, cy as i32 + dy);
    let top = at(0, 0) + (at(1, 0) - at(0, 0)) * sx;
    let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * sx;
    top + (bottom - top) * sy
}

/// The static sand of a slab on the grid of `nodes` along a side: lit dunes and ripples, a speck
/// of darker and lighter grains, the tint and string of each region, and a rim that darkens.
fn sand(slab: &Slab, nodes: usize) -> Vec<[f32; 3]> {
    let last = (nodes - 1) as f32;
    // The height of the sand, in fractions of the slab, at node coordinates (i, j).
    let height = |i: f32, j: f32| {
        let (u, v) = (i / last, j / last);
        let warp = noise(SAND_SEED, u * 5.0, v * 5.0) + 0.5 * noise(SAND_SEED, u * 11.5, v * 11.5);
        let phase = (u * 0.83 + v * 0.56) * 95.0 + warp * 2.4;
        let ripple = phase.sin() + 0.38 * (2.0 * phase + 0.9).sin();
        let amplitude = (0.55 + 0.5 * noise(SAND_SEED ^ 1, u * 4.0, v * 4.0)).clamp(0.05, 1.0);
        let dunes = noise(SAND_SEED ^ 2, u * 4.0, v * 4.0)
            + 0.45 * noise(SAND_SEED ^ 2, u * 8.4, v * 8.4)
            + 0.2 * noise(SAND_SEED ^ 2, u * 18.8, v * 18.8);
        0.0024 * ripple * amplitude + 0.010 * dunes
    };
    // The heights on the grid and a ring of nodes round it, so that every slope is a central
    // difference.
    let side = nodes + 2;
    let mut heights = Vec::with_capacity(side * side);
    for j in 0..side {
        for i in 0..side {
            heights.push(height(i as f32 - 1.0, j as f32 - 1.0));
        }
    }
    let light = normalize(LIGHT);
    let half = slab.half();
    let mut colors = Vec::with_capacity(nodes * nodes);
    for j in 0..nodes {
        for i in 0..nodes {
            let at_node = |i: usize, j: usize| heights[j * side + i];
            // Slopes in height per slab fraction, by central differences.
            let slope_x = (at_node(i + 2, j + 1) - at_node(i, j + 1)) * last * 0.5;
            let slope_y = (at_node(i + 1, j + 2) - at_node(i + 1, j)) * last * 0.5;
            let normal = normalize([-slope_x, -slope_y, 1.0]);
            let diffuse = dot(normal, light).clamp(0.0, 1.0);
            let (u, v) = (i as f32 / last, j as f32 / last);
            let tone = 1.0 + 0.10 * (noise(SAND_SEED ^ 3, u * 9.0, v * 9.0) - 0.5) * 2.0;
            // A grain: the sum of two uniforms is close enough to a bell.
            let grain = 1.0
                + 0.24
                    * (lattice(SAND_SEED ^ 4, i as i32, j as i32)
                        + lattice(SAND_SEED ^ 5, j as i32, i as i32)
                        - 1.0);
            let speck = if lattice(SAND_SEED ^ 7, i as i32, j as i32) < 0.0035 {
                0.55 + 0.8 * lattice(SAND_SEED ^ 6, i as i32, j as i32)
            } else {
                1.0
            };
            let lit = (0.30 + 0.72 * diffuse) * tone * grain * speck;
            let mut color = SAND.map(|channel| channel * lit);
            let at = [(u - 0.5) * 2.0 * half, (v - 0.5) * 2.0 * half];
            for region in Region::ALL {
                let rect = slab.rect(region);
                if rect.contains(at) {
                    for (channel, tint) in color.iter_mut().zip(region.tint()) {
                        *channel += tint;
                    }
                }
                color = stringed(color, at, &rect);
            }
            color = stringed(color, at, &slab.middle());
            // A rim that darkens, so the slab reads as a lit patch.
            let rim = u.min(1.0 - u).min(v).min(1.0 - v);
            let rim = 0.72 + 0.28 * (rim / 0.12).clamp(0.0, 1.0);
            let centre = 0.88 + 0.12 * (1.0 - (u - 0.5).hypot(v - 0.55) / 0.6).clamp(0.0, 1.0);
            colors.push(color.map(|channel| channel * rim * centre));
        }
    }
    colors
}

/// `color` drawn toward the string's where `at` lies on the outline of `rect`.
fn stringed(color: [f32; 3], at: [f32; 2], rect: &Rect) -> [f32; 3] {
    let inside = [
        at[0] - rect.min[0] - STRING_INSET,
        rect.max[0] - STRING_INSET - at[0],
        at[1] - rect.min[1] - STRING_INSET,
        rect.max[1] - STRING_INSET - at[1],
    ];
    if inside.iter().any(|&distance| distance < -STRING_WIDTH) {
        return color;
    }
    let near = inside.iter().copied().fold(f32::INFINITY, f32::min).abs();
    if near > STRING_WIDTH {
        return color;
    }
    let weight = 0.55 * (1.0 - near / STRING_WIDTH);
    std::array::from_fn(|k| color[k] * (1.0 - weight) + STRING[k] * weight)
}

/// The ground of one slab.
pub struct Terrain {
    slab: Slab,
    sand: Vec<[f32; 3]>,
    field: Field,
    grid: Grid,
}

impl Terrain {
    /// The ground of `slab` on the grid `pheromone` is laid on.
    pub fn new(slab: &Slab, pheromone: &Pheromone) -> Terrain {
        let nodes = pheromone.nodes;
        Terrain {
            slab: *slab,
            sand: sand(slab, nodes),
            field: Field::new(KEY, nodes, nodes),
            grid: Grid {
                origin: pheromone.origin,
                cell: pheromone.cell,
                columns: nodes,
                rows: nodes,
            },
        }
    }

    /// The slab it was built for.
    pub fn slab(&self) -> &Slab {
        &self.slab
    }

    /// Takes the sand and adds the pheromone's glow over it. The glow is the colour of the nests
    /// that laid it, brightest where there is most.
    pub fn paint(&mut self, pheromone: &Pheromone) {
        for (index, (color, sand)) in self.field.colors.iter_mut().zip(&self.sand).enumerate() {
            let laid = pheromone.at(index);
            let density = laid[0] + laid[1] + laid[2];
            let mut glow = [0.0; 3];
            if density > 1e-3 {
                let strongest = laid[0].max(laid[1]).max(laid[2]);
                let bright = 1.0 - (-SATURATION * density).exp();
                glow = laid.map(|channel| channel / strongest * bright * 255.0 * GLOW);
            }
            *color = std::array::from_fn(|k| (sand[k] + glow[k]).clamp(0.0, 255.0) as u8);
        }
        self.field.revision += 1;
    }

    /// Records the ground and the slab's cut sides.
    pub fn draw(&self, frame: &mut Frame, camera: &Camera) {
        frame.field(
            camera,
            Placement::Grid {
                grid: self.grid,
                z: 0.0,
            },
            &self.field,
            Relief::Flat,
        );
        self.sides(frame, camera);
    }

    /// The four cut sides of the slab: two shades of earth, with strata lines.
    fn sides(&self, frame: &mut Frame, camera: &Camera) {
        let h = self.slab.half();
        let thickness = 0.034 * self.slab.size();
        // Each side: its two top corners, and its colour.
        let walls: [([f32; 2], [f32; 2], Color); 4] = [
            ([-h, h], [h, h], [58, 44, 34]),
            ([h, h], [h, -h], [36, 28, 24]),
            ([-h, -h], [h, -h], [44, 34, 27]),
            ([-h, h], [-h, -h], [50, 38, 30]),
        ];
        // Bands of the cut face, from the top down: fraction of the thickness and a shade.
        let bands = [(0.0, 1.0), (0.33, 1.28), (0.62, 0.8), (0.82, 1.1)];
        for (from, to, color) in walls {
            for (index, &(top, shade)) in bands.iter().enumerate() {
                let bottom = bands.get(index + 1).map_or(1.0, |next| next.0);
                let corner =
                    |at: [f32; 2], depth: f32| -> Point { [at[0], at[1], -thickness * depth] };
                let tinted = color.map(|channel| (f32::from(channel) * shade).min(255.0) as u8);
                frame.quad(
                    camera,
                    [
                        corner(from, top),
                        corner(to, top),
                        corner(to, bottom),
                        corner(from, bottom),
                    ],
                    tinted,
                    crate::render::NONE,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> (Pheromone, Terrain) {
        let pheromone = Pheromone::new(Slab::default().size());
        let terrain = Terrain::new(&Slab::default(), &pheromone);
        (pheromone, terrain)
    }

    #[test]
    fn the_sand_is_the_same_every_time_and_covers_every_node() {
        let (_, one) = built();
        let (_, two) = built();
        assert_eq!(one.sand, two.sand);
        assert_eq!(one.sand.len(), 257 * 257);
        assert!(
            one.sand
                .iter()
                .flatten()
                .all(|&channel| (0.0..=255.0).contains(&channel))
        );
    }

    #[test]
    fn a_region_is_tinted_and_outlined_in_string() {
        let (_, terrain) = built();
        let nodes = 257;
        let slab = Slab::default();
        let node = |at: [f32; 2]| {
            let column = ((at[0] + slab.half()) / 0.25).round() as usize;
            let row = ((at[1] + slab.half()) / 0.25).round() as usize;
            terrain.sand[row * nodes + column]
        };
        // On the string of the clearing, which runs 0.45 inside its edge, against plain sand.
        let on = node([-slab.clearing() + STRING_INSET, 0.0]);
        let off = node([-slab.clearing() + 3.0, 0.0]);
        let sum = |color: [f32; 3]| color.iter().sum::<f32>();
        assert!(sum(on) > sum(off), "{on:?} against {off:?}");
    }

    #[test]
    fn pheromone_glows_in_the_colour_of_the_nest_that_laid_it_and_nowhere_else() {
        let (mut pheromone, mut terrain) = built();
        terrain.paint(&pheromone);
        let bare: Vec<[u8; 3]> = terrain.field.colors.clone();
        let revision = terrain.field.revision;
        let middle = pheromone.nodes * 128 + 128;
        let at = [0.0, 0.0];
        pheromone.lay(at, [0.1, 0.9, 1.0].map(|c| c * 3.0));
        terrain.paint(&pheromone);
        assert_eq!(
            terrain.field.revision,
            revision + 1,
            "a changed field is shaded again"
        );
        let changed: Vec<usize> = (0..bare.len())
            .filter(|&index| bare[index] != terrain.field.colors[index])
            .collect();
        assert_eq!(changed, [middle]);
        let [r, g, b] = terrain.field.colors[middle];
        let [r0, g0, b0] = bare[middle];
        // Teal: green and blue gain most.
        assert!(i32::from(b) - i32::from(b0) > i32::from(r) - i32::from(r0));
        assert!(i32::from(g) - i32::from(g0) > i32::from(r) - i32::from(r0));
    }

    #[test]
    fn the_ground_is_one_field_of_two_triangles_and_the_slab_has_sixteen_cut_faces() {
        let (_, terrain) = built();
        let mut frame = crate::render::Frame::new(
            320,
            180,
            crate::render::Sky {
                backdrop: crate::render::Backdrop::Dusk,
                haze: [0.0; 3],
                time: 0.0,
            },
            crate::render::Buffers::default(),
        );
        terrain.draw(&mut frame, &Camera::default());
        let fields = frame
            .items
            .iter()
            .filter(|item| matches!(item, crate::render::Item::Field { .. }))
            .count();
        let triangles = frame
            .items
            .iter()
            .filter(|item| matches!(item, crate::render::Item::Triangle(..)))
            .count();
        assert_eq!(fields, 2, "one field, never one pair of triangles a cell");
        assert_eq!(
            triangles, 32,
            "four sides of four bands, two triangles each"
        );
        assert_eq!(frame.fields.len(), 1);
    }
}
