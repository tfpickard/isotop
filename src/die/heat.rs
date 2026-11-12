//! The heat model of the die, pure: a field on the 192 by 96 grid that diffuses, is warmed by the
//! cores' load and cooled towards ambient, and is pulled towards what the sensors read.
//!
//! The equation is `dT/dt = kappa laplacian(T) + gain P - (T - T_amb) / tau` with `kappa` 90
//! cells squared a second and `tau` 2 s, stepped explicitly on the 5-point stencil with zero-flux
//! edges. The step is `STEP = 0.2 / kappa`, so the Courant number `kappa STEP` is 0.2, below the
//! 2D limit of 1/4 (the checkerboard mode, the worst, has amplification `1 - 8 * 0.2 - STEP / tau`
//! of magnitude below 1). A frame never decides the step: `Clock` runs whole steps of `STEP`
//! through `simulation::FixedStep`, and a frame longer than `MOST_FRAME` is cut to it, so any
//! frame rate and any stall give a stable scheme, and 10 and 60 frames a second the same state.
//!
//! The constants are the model's, not silicon's: they make lateral spreading about 90 times
//! slower than real silicon's, so that a core's glow can be seen. What the field shows between
//! sensors is this model; what a sensor reads is a measurement. A `Nudge` pulls what is shown
//! towards a reading: it keeps an offset, added over the region the sensor speaks for, that
//! relaxes (time constant `NUDGE_TIME`) to the reading minus the model's value at the sensor's
//! spot, so the displayed value there converges on the reading, and outside the footprints
//! the field is the model alone. The model itself is never bent to a reading.

use std::sync::Mutex;

use super::floorplan::{COLUMNS, ROWS, Rect};
use crate::simulation::FixedStep;

/// Cells squared per second.
pub const KAPPA: f32 = 90.0;
/// The relaxation towards ambient, in seconds: package-scale thermal inertia.
pub const TAU: f32 = 2.0;
/// The fixed step, in seconds.
pub const STEP: f32 = 0.2 / KAPPA;
/// The largest `KAPPA * STEP` the explicit 5-point scheme tolerates in two dimensions.
pub const COURANT_LIMIT: f32 = 0.25;
/// The time constant of a sensor's pull on the field, in seconds.
pub const NUDGE_TIME: f32 = 0.25;
/// The longest frame, in seconds, that is stepped: a longer gap counts as this much.
pub const MOST_FRAME: f32 = 0.25;
/// The whole steps one frame takes at most.
const MOST_STEPS: usize = 120;
/// The settling a cold start is given, in seconds.
pub const PRE_ROLL: f32 = 3.0 * TAU;
/// Kelvin above ambient that one P-core at full proxy settles at: what `gain` is calibrated to.
pub const CALIBRATED_RISE: f32 = 40.0;
/// How far the pull of a sensor reaches beyond the region it speaks for, in cells, falling off as
/// a Gaussian.
pub const SKIRT: f32 = 4.0;
/// The seconds the calibration runs a reference core for: it is within 1 % of settled by then.
const CALIBRATION_TIME: f32 = 5.0 * TAU;

const CELLS: usize = COLUMNS * ROWS;

/// The grid and the temperatures on it. Cell `(i, j)` is `field[j * COLUMNS + i]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Heat {
    pub field: Vec<f32>,
    /// What heats each cell, in kelvin a second at the current load.
    pub source: Vec<f32>,
    /// The temperature the sink pulls towards.
    pub ambient: f32,
    next: Vec<f32>,
}

/// One step of the scheme: its Courant number `courant`, the time step `step` it takes, the sink's
/// `1 / tau` and the ambient it relaxes to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheme {
    pub courant: f32,
    pub step: f32,
    pub inverse_tau: f32,
    pub ambient: f32,
}

impl Heat {
    pub fn new(ambient: f32) -> Self {
        Self {
            field: vec![ambient; CELLS],
            source: vec![0.0; CELLS],
            ambient,
            next: vec![0.0; CELLS],
        }
    }

    /// The scheme the view runs: Courant number `KAPPA * STEP`, the model's `tau`.
    pub fn scheme(&self) -> Scheme {
        Scheme {
            courant: KAPPA * STEP,
            step: STEP,
            inverse_tau: 1.0 / TAU,
            ambient: self.ambient,
        }
    }

    /// One explicit step of `scheme` over the whole grid.
    pub fn diffuse(&mut self, scheme: Scheme) {
        diffuse(&self.field, &mut self.next, &self.source, scheme);
        std::mem::swap(&mut self.field, &mut self.next);
    }

    /// The value at `cell`.
    pub fn at(&self, cell: u32) -> f32 {
        self.field[cell as usize]
    }

    /// The highest value anywhere.
    pub fn peak(&self) -> f32 {
        self.field.iter().copied().fold(f32::MIN, f32::max)
    }
}

/// `next` from `field`: the explicit 5-point step of `dT/dt = kappa lap(T) + source - sink`, with
/// each cell's missing neighbours at the grid's edge taken as the cell itself (no flux).
pub fn diffuse(field: &[f32], next: &mut [f32], source: &[f32], scheme: Scheme) {
    let Scheme {
        courant,
        step,
        inverse_tau,
        ambient,
    } = scheme;
    for row in 0..ROWS {
        let base = row * COLUMNS;
        let above = if row > 0 { base - COLUMNS } else { base };
        let below = if row + 1 < ROWS { base + COLUMNS } else { base };
        let line = &field[base..base + COLUMNS];
        let (up, down) = (&field[above..above + COLUMNS], &field[below..below + COLUMNS]);
        let (out, heat) = (
            &mut next[base..base + COLUMNS],
            &source[base..base + COLUMNS],
        );
        for column in 0..COLUMNS {
            let here = line[column];
            let left = line[column.saturating_sub(1)];
            let right = line[(column + 1).min(COLUMNS - 1)];
            let laplacian = left + right + up[column] + down[column] - 4.0 * here;
            out[column] =
                here + courant * laplacian + step * (heat[column] - (here - ambient) * inverse_tau);
        }
    }
}

/// A reading that pulls what is shown: the cells it speaks for (a core's block, a core complex,
/// the whole package), each with a weight 1 inside the region falling off as a Gaussian of
/// `SKIRT / 3` cells over the `SKIRT` cells beyond it, and the cells of the hot spots whose
/// highest value is what the reading is compared with. The offset it keeps is added over the
/// whole region, so the model keeps its own shape inside it.
///
/// A sensor's own Gaussian of radius 4 cells cannot be written into the field itself and hold a
/// reading at `KAPPA` 90: it leaks about 50 a second to diffusion against a pull of 4 a second,
/// so the sensor's point would reach a tenth of the way to the reading. The offset is not
/// diffused: it is a correction of what is shown, whose footprint is the region the sensor speaks
/// for (a core's block, a CCD, the package) with the Gaussian as its skirt.
#[derive(Clone, Debug, PartialEq)]
pub struct Nudge {
    pub footprint: Vec<(u32, f32)>,
    pub probes: Vec<u32>,
    pub target: f32,
}

impl Nudge {
    /// The highest model value at the probes; None for a nudge with none.
    pub fn reference(&self, field: &[f32]) -> Option<f32> {
        self.probes
            .iter()
            .map(|&cell| field[cell as usize])
            .reduce(f32::max)
    }

    /// Relaxes `offset` towards the reading minus the model's value at the probes by `alpha` of
    /// what is left of the error, once: `alpha` is `1 - exp(-STEP / NUDGE_TIME)` for a step.
    pub fn relax(&self, field: &[f32], offset: &mut f32, alpha: f32) {
        if let Some(reference) = self.reference(field) {
            *offset += alpha * (self.target - (reference + *offset));
        }
    }
}

/// The weight of one step's pull: `1 - exp(-STEP / NUDGE_TIME)`, which is 0.35 over a frame of
/// 0.1 s and 0.0088 over a step.
pub fn alpha() -> f32 {
    1.0 - (-STEP / NUDGE_TIME).exp()
}

/// `field` with every nudge's offset added over its footprint: what the die shows. The offsets
/// are in `nudges` order.
pub fn compose(field: &[f32], nudges: &[Nudge], offsets: &[f32], shown: &mut Vec<f32>) {
    shown.clear();
    shown.extend_from_slice(field);
    for (nudge, &offset) in nudges.iter().zip(offsets) {
        for &(cell, weight) in &nudge.footprint {
            shown[cell as usize] += weight * offset;
        }
    }
}

/// The index of the cell holding point `(x, y)` of the plan, clamped to the grid.
pub fn cell_at(x: f32, y: f32) -> u32 {
    let column = (x.floor() as i32).clamp(0, COLUMNS as i32 - 1) as usize;
    let row = (y.floor() as i32).clamp(0, ROWS as i32 - 1) as usize;
    (row * COLUMNS + column) as u32
}

/// How far point `(x, y)` is from `rect`, 0 inside it.
fn distance(rect: Rect, x: f32, y: f32) -> f32 {
    let across = (rect[0] - x).max(x - rect[2]).max(0.0);
    let down = (rect[1] - y).max(y - rect[3]).max(0.0);
    across.hypot(down)
}

/// The cells of the union of `rects`, with their weights: 1 inside, a Gaussian skirt beyond.
pub fn footprint(rects: &[Rect]) -> Vec<(u32, f32)> {
    let sigma = SKIRT / 3.0;
    let mut cells = Vec::new();
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let reach = rects
                .iter()
                .map(|&rect| distance(rect, x, y))
                .fold(f32::MAX, f32::min);
            if reach < SKIRT {
                let weight = if reach <= 0.0 {
                    1.0
                } else {
                    (-reach * reach / (2.0 * sigma * sigma)).exp()
                };
                cells.push(((row * COLUMNS + column) as u32, weight));
            }
        }
    }
    cells
}

/// Every cell of the plan with weight 1: what a package sensor speaks for.
pub fn whole_die() -> Vec<(u32, f32)> {
    (0..CELLS as u32).map(|cell| (cell, 1.0)).collect()
}

/// How a core's source is spread over its block: a floor of 0.45 and a Gaussian hot spot of
/// peak 1.6 round `hot` (0.28 of the width and 0.22 of the height wide), the prototype's, scaled
/// so that the weights average 1 over the block. The pairs are (cell, weight).
pub fn shape(rect: Rect, hot: [f32; 2]) -> Vec<(u32, f32)> {
    let (width, height) = (rect[2] - rect[0], rect[3] - rect[1]);
    let (spread_x, spread_y) = (width * 0.28, height * 0.22);
    let mut cells = Vec::new();
    for row in rect[1].floor().max(0.0) as usize..(rect[3].ceil() as usize).min(ROWS) {
        for column in rect[0].floor().max(0.0) as usize..(rect[2].ceil() as usize).min(COLUMNS) {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            if x < rect[0] || x > rect[2] || y < rect[1] || y > rect[3] {
                continue;
            }
            let (dx, dy) = ((x - hot[0]) / spread_x, (y - hot[1]) / spread_y);
            let weight = 0.45 + 1.6 * (-(dx * dx) - dy * dy).exp();
            cells.push(((row * COLUMNS + column) as u32, weight));
        }
    }
    let mean = cells.iter().map(|&(_, weight)| weight).sum::<f32>() / cells.len().max(1) as f32;
    for (_, weight) in &mut cells {
        *weight /= mean;
    }
    cells
}

/// The gain, in kelvin a second per unit of proxy, that makes a core block `rect` (hot spot
/// `hot`) at full proxy settle `CALIBRATED_RISE` above ambient, found by running the scheme on an
/// empty die for `CALIBRATION_TIME` and scaling (the scheme is linear). Calibrations are kept
/// by block size: they cost a few tens of milliseconds.
pub fn gain(rect: Rect, hot: [f32; 2]) -> f32 {
    static KNOWN: Mutex<Vec<([i32; 2], f32)>> = Mutex::new(Vec::new());
    let size = [
        ((rect[2] - rect[0]) * 4.0).round() as i32,
        ((rect[3] - rect[1]) * 4.0).round() as i32,
    ];
    let mut known = KNOWN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&(_, gain)) = known.iter().find(|(found, _)| *found == size) {
        return gain;
    }
    // The block is moved to the middle of the die, so the edges have no say in its calibration.
    let (width, height) = (rect[2] - rect[0], rect[3] - rect[1]);
    let at = [
        ((COLUMNS as f32 - width) / 2.0).floor(),
        ((ROWS as f32 - height) / 2.0).floor(),
    ];
    let moved = [at[0], at[1], at[0] + width, at[1] + height];
    let centre = [
        moved[0] + (hot[0] - rect[0]),
        moved[1] + (hot[1] - rect[1]),
    ];
    let mut heat = Heat::new(0.0);
    for (cell, weight) in shape(moved, centre) {
        heat.source[cell as usize] = weight;
    }
    let scheme = heat.scheme();
    for _ in 0..(CALIBRATION_TIME / STEP) as usize {
        heat.diffuse(scheme);
    }
    let gain = CALIBRATED_RISE / heat.peak().max(f32::EPSILON);
    known.push((size, gain));
    gain
}

/// Whole steps of `STEP` from the animation clock, through `FixedStep`.
#[derive(Debug, Clone)]
pub struct Clock(FixedStep);

impl Default for Clock {
    fn default() -> Self {
        Clock(FixedStep::new(STEP, MOST_STEPS, MOST_FRAME))
    }
}

impl Clock {
    /// Advances to animation time `time`, calling `tick` once per whole step taken. The first
    /// call, an unchanged time and a time that went back take none.
    pub fn advance(&mut self, time: f32, mut tick: impl FnMut()) {
        self.0.advance(time, |_| tick());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::Rng;

    /// The sum of the field in f64, for the conservation checks.
    fn total(field: &[f32]) -> f64 {
        field.iter().map(|&value| f64::from(value)).sum()
    }

    fn checkerboard(amplitude: f32) -> Vec<f32> {
        (0..CELLS)
            .map(|cell| {
                let (column, row) = (cell % COLUMNS, cell / COLUMNS);
                if (column + row) % 2 == 0 { amplitude } else { -amplitude }
            })
            .collect()
    }

    #[test]
    fn heat_steps_stay_stable_at_the_cfl_limit_for_any_frame_dt() {
        // The scheme's own number: below the 2D limit, with the margin the issue names.
        let courant = KAPPA * STEP;
        assert!(courant <= COURANT_LIMIT, "{courant} against {COURANT_LIMIT}");
        assert!((courant - 0.2).abs() < 1e-6);
        assert!(
            (MOST_FRAME / STEP).ceil() as usize <= MOST_STEPS,
            "a frame of the longest length is stepped whole"
        );
        // The worst mode, a checkerboard, through the clock at every frame length, with nothing
        // to drain it but the sink: it only shrinks, however long the frames are.
        for frame in [0.001_f32, 1.0 / 60.0, 0.1, 0.25, 0.5, 5.0, 100.0] {
            let mut heat = Heat::new(0.0);
            heat.field = checkerboard(50.0);
            let (scheme, mut clock) = (heat.scheme(), Clock::default());
            let mut time = 0.0;
            let mut worst = 0.0_f32;
            for _ in 0..((3.0 / frame.min(1.0)) as usize).min(3000) {
                clock.advance(time, || heat.diffuse(scheme));
                time += frame;
                let largest = heat.field.iter().fold(0.0_f32, |a, &v| a.max(v.abs()));
                assert!(largest.is_finite(), "frame {frame}: the field stayed finite");
                worst = worst.max(largest);
            }
            assert!(worst <= 50.0 + 1e-3, "frame {frame}: peak grew to {worst}");
        }
        // The bound is the scheme's and not luck: above it the same mode grows without limit.
        let mut heat = Heat::new(0.0);
        heat.field = checkerboard(1.0);
        let unstable = Scheme {
            courant: 0.3,
            ..heat.scheme()
        };
        for _ in 0..200 {
            heat.diffuse(unstable);
        }
        assert!(heat.peak() > 1000.0, "a Courant number of 0.3 blows up");
    }

    #[test]
    fn heat_is_conserved_without_sinks_or_sources() {
        let mut rng = Rng::new(7);
        let mut heat = Heat::new(0.0);
        heat.field = (0..CELLS).map(|_| rng.unit() * 100.0).collect();
        let before = total(&heat.field);
        let isolated = Scheme {
            inverse_tau: 0.0,
            ..heat.scheme()
        };
        for _ in 0..2000 {
            heat.diffuse(isolated);
        }
        let after = total(&heat.field);
        assert!(
            ((after - before) / before).abs() < 1e-5,
            "the sum went from {before} to {after}"
        );
        // And it spread: the field is smoother than it was.
        let spread = heat.peak() - heat.field.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread < 5.0, "uniform to within {spread}");
        // With the sink it relaxes to ambient instead.
        let mut sunk = Heat::new(20.0);
        sunk.field = vec![60.0; CELLS];
        let scheme = sunk.scheme();
        for _ in 0..(5.0 * TAU / STEP) as usize {
            sunk.diffuse(scheme);
        }
        assert!((sunk.peak() - 20.0).abs() < 0.5, "settles at {}", sunk.peak());
    }

    #[test]
    fn heat_follows_the_same_path_at_ten_and_sixty_frames_per_second() {
        // A warm block under constant drive, run to 2.95 s: a time that is half a step past a
        // whole number of them, so rounding of the frame times cannot tip a step over.
        let rect = [40.0, 20.0, 60.0, 50.0];
        let run = |frame: f32| {
            let mut heat = Heat::new(30.0);
            for (cell, weight) in shape(rect, [50.0, 32.0]) {
                heat.source[cell as usize] = 12.0 * weight;
            }
            let (scheme, mut clock) = (heat.scheme(), Clock::default());
            let mut time = 0.0_f32;
            let mut steps = 0;
            while time + frame < 2.95 {
                clock.advance(time, || {
                    heat.diffuse(scheme);
                    steps += 1;
                });
                time += frame;
            }
            clock.advance(2.95, || {
                heat.diffuse(scheme);
                steps += 1;
            });
            (heat, steps)
        };
        let ((ten, ten_steps), (sixty, sixty_steps)) = (run(0.1), run(1.0 / 60.0));
        assert_eq!(ten_steps, sixty_steps, "the same whole steps");
        assert_eq!(ten_steps, (2.95 / STEP) as usize);
        assert_eq!(ten.field, sixty.field, "the same state, bit for bit");
        assert!(ten.peak() > 31.0, "and something happened");
    }

    #[test]
    fn the_scheme_is_deterministic() {
        let rect = [10.0, 10.0, 30.0, 40.0];
        let run = || {
            let mut heat = Heat::new(25.0);
            for (cell, weight) in shape(rect, [20.0, 24.0]) {
                heat.source[cell as usize] = 9.0 * weight;
            }
            let scheme = heat.scheme();
            for _ in 0..500 {
                heat.diffuse(scheme);
            }
            heat
        };
        assert_eq!(run(), run());
    }


    /// A warm block under constant drive with a sensor that reads `target`, run for `seconds`
    /// with the offset relaxed every step; returns the model, the nudge, its offset and the
    /// composed field.
    fn nudged(target: f32, seconds: f32) -> (Heat, Nudge, f32, Vec<f32>) {
        let block = [60.0, 20.0, 84.0, 50.0];
        let hot = [70.0, 32.0];
        let mut heat = Heat::new(35.0);
        for (cell, weight) in shape(block, hot) {
            heat.source[cell as usize] = 10.0 * weight;
        }
        let nudge = Nudge {
            footprint: footprint(&[block]),
            probes: vec![cell_at(hot[0], hot[1])],
            target,
        };
        let (scheme, alpha) = (heat.scheme(), alpha());
        let mut offset = 0.0;
        for _ in 0..(seconds / STEP) as usize {
            heat.diffuse(scheme);
            nudge.relax(&heat.field, &mut offset, alpha);
        }
        let mut shown = Vec::new();
        compose(&heat.field, std::slice::from_ref(&nudge), &[offset], &mut shown);
        (heat, nudge, offset, shown)
    }

    #[test]
    fn nudging_pulls_sensor_points_to_measured_values() {
        // A model about 20 K cooler than the sensor reads where the sensor sits: the point shows
        // the reading within a tenth of a degree once the model has settled.
        let (heat, nudge, offset, shown) = nudged(92.0, 6.0);
        let probe = nudge.probes[0];
        let model = heat.at(probe);
        assert!(model < 80.0, "the model alone reads {model}");
        assert!(
            (shown[probe as usize] - 92.0).abs() < 0.1,
            "the sensor's point shows {}",
            shown[probe as usize]
        );
        assert!((offset - (92.0 - model)).abs() < 0.1);
        // A reading below the model pulls it down the same way.
        let (_, cool, _, shown) = nudged(50.0, 6.0);
        assert!((shown[cool.probes[0] as usize] - 50.0).abs() < 0.1);
        // The pull has the time constant 0.25 s: a quarter second in it has closed 1 - 1/e.
        let (heat, nudge, offset, _) = nudged(92.0, NUDGE_TIME);
        let closed = offset / (92.0 - heat.at(nudge.probes[0]));
        assert!((closed - 0.632).abs() < 0.05, "closed {closed} of the gap");
    }

    #[test]
    fn a_nudge_moves_its_region_and_its_skirt_and_nothing_beyond() {
        let block = [60.0, 20.0, 84.0, 50.0];
        let (heat, nudge, _, shown) = nudged(92.0, 6.0);
        let at = |field: &[f32], x: f32, y: f32| field[cell_at(x, y) as usize];
        // Beyond the skirt the shown field is the model's own, bit for bit.
        for (x, y) in [(block[2] + SKIRT + 1.0, 30.0), (10.0, 10.0), (150.0, 80.0)] {
            assert_eq!(at(&shown, x, y), at(&heat.field, x, y), "({x}, {y})");
        }
        // Inside it the model's shape is kept: its hot spot is still the hottest place, and the
        // difference between two cells of the block is the model's difference.
        let corner = (block[0] + 1.0, block[3] - 1.0);
        let hot = (70.0, 32.0);
        assert!(at(&shown, hot.0, hot.1) > at(&shown, corner.0, corner.1) + 1.0);
        let difference = |field: &[f32]| at(field, hot.0, hot.1) - at(field, corner.0, corner.1);
        assert!((difference(&shown) - difference(&heat.field)).abs() < 1e-3);
        // Weights: 1 inside, falling beyond, nothing past the skirt.
        let weight = |x: f32, y: f32| {
            let cell = cell_at(x, y);
            nudge.footprint.iter().find(|&&(found, _)| found == cell).map(|&(_, w)| w)
        };
        assert_eq!(weight(70.0, 30.0), Some(1.0));
        let near = weight(block[2] + 1.5, 30.0).expect("inside the skirt");
        assert!(near < 1.0 && near > 0.1);
        assert_eq!(weight(block[2] + SKIRT + 2.0, 30.0), None);
        // A footprint of two blocks is their union, and the package's is every cell.
        let two = footprint(&[block, [100.0, 20.0, 120.0, 50.0]]);
        assert_eq!(weight(110.0, 30.0), None);
        assert!(two.iter().any(|&(cell, w)| cell == cell_at(110.0, 30.0) && w == 1.0));
        assert_eq!(whole_die().len(), CELLS);
        // Without probes (a sensor whose region holds no core) the nudge does nothing.
        let idle = Nudge {
            footprint: footprint(&[block]),
            probes: Vec::new(),
            target: 200.0,
        };
        let mut offset = 0.0;
        idle.relax(&heat.field, &mut offset, 0.5);
        assert_eq!((idle.reference(&heat.field), offset), (None, 0.0));
    }

    #[test]
    fn a_core_at_full_proxy_settles_forty_kelvin_above_ambient() {
        // A block in the middle of the die, where the calibration runs, so the edges have no say.
        let rect = [87.0, 31.0, 104.0, 65.0];
        let hot = [94.1, 44.6];
        let gain = gain(rect, hot);
        assert!(gain > 0.0 && gain.is_finite());
        // The same block run for real at that gain peaks at the calibrated rise.
        let mut heat = Heat::new(30.0);
        for (cell, weight) in shape(rect, hot) {
            heat.source[cell as usize] = gain * weight;
        }
        let scheme = heat.scheme();
        for _ in 0..(CALIBRATION_TIME / STEP) as usize {
            heat.diffuse(scheme);
        }
        let rise = heat.peak() - 30.0;
        assert!((rise - CALIBRATED_RISE).abs() < 0.5, "rose {rise}");
        // The calibration is kept: asking again costs nothing and agrees.
        assert_eq!(gain, super::gain(rect, hot));
    }

    #[test]
    fn a_shape_spreads_a_unit_of_weight_over_its_block_with_a_hot_spot() {
        let rect = [10.0, 10.0, 26.0, 40.0];
        let cells = shape(rect, [17.0, 22.0]);
        let mean = cells.iter().map(|&(_, w)| w).sum::<f32>() / cells.len() as f32;
        assert!((mean - 1.0).abs() < 1e-4);
        assert_eq!(cells.len(), 16 * 30);
        let at = |x: f32, y: f32| {
            let cell = cell_at(x, y);
            cells.iter().find(|&&(c, _)| c == cell).expect("in the block").1
        };
        assert!(at(17.0, 22.0) > 2.0 * at(11.0, 38.0));
        assert!(cells.iter().all(|&(cell, _)| (cell as usize) < CELLS));
    }
}
