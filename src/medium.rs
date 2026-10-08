//! Simulated media behind the ripple and flow views: a damped 2D wave equation and a particle
//! flow over a cached velocity field. Rendering lives in render.rs; this module is pure state.

use crate::render::{Color, Point};

/// Wave speed in grid cells per second.
const WAVE_SPEED: f32 = 14.0;
/// Background damping per second, and the extra damping of the absorbing border.
const DAMPING: f32 = 0.35;
const SPONGE: f32 = 6.0;
const SPONGE_CELLS: f32 = 10.0;
/// Pull back to the rest level per second squared. A uniform offset has no curvature, so without
/// it the impulses of births and exits would leave the whole pond permanently raised or lowered.
const RESTORE: f32 = 0.5;
/// Trail points kept per particle.
pub const TRAIL: usize = 20;

/// A regular grid laid over a world-space rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub origin: [f32; 2],
    pub cell: f32,
    pub columns: usize,
    pub rows: usize,
}

impl Grid {
    /// Covers [min, max] with about `cells` cells along the longer side.
    pub fn covering(min: [f32; 2], max: [f32; 2], cells: usize) -> Self {
        let span = [(max[0] - min[0]).max(1.0), (max[1] - min[1]).max(1.0)];
        let cell = span[0].max(span[1]) / cells as f32;
        Self {
            origin: min,
            cell,
            columns: (span[0] / cell).ceil() as usize + 1,
            rows: (span[1] / cell).ceil() as usize + 1,
        }
    }

    pub fn len(&self) -> usize {
        self.columns * self.rows
    }

    pub fn world(&self, x: f32, y: f32) -> [f32; 2] {
        [
            self.origin[0] + x * self.cell,
            self.origin[1] + y * self.cell,
        ]
    }

    /// Fractional cell coordinates of a world point.
    pub fn locate(&self, p: [f32; 2]) -> [f32; 2] {
        [
            (p[0] - self.origin[0]) / self.cell,
            (p[1] - self.origin[1]) / self.cell,
        ]
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        let [x, y] = self.locate(p);
        x >= 0.0 && y >= 0.0 && x <= (self.columns - 1) as f32 && y <= (self.rows - 1) as f32
    }

    /// Bilinear interpolation of per-node values at a world point, clamped to the grid.
    pub fn sample<T>(&self, values: &[T], p: [f32; 2], mix: impl Fn(T, T, f32) -> T) -> T
    where
        T: Copy,
    {
        let [x, y] = self.locate(p);
        let x = x.clamp(0.0, (self.columns - 1) as f32 - 0.001);
        let y = y.clamp(0.0, (self.rows - 1) as f32 - 0.001);
        let (i, j) = (x as usize, y as usize);
        let (fx, fy) = (x - i as f32, y - j as f32);
        let at = |i: usize, j: usize| values[j * self.columns + i];
        let top = mix(at(i, j), at(i + 1, j), fx);
        let bottom = mix(at(i, j + 1), at(i + 1, j + 1), fx);
        mix(top, bottom, fy)
    }
}

pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn mix2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [mix(a[0], b[0], t), mix(a[1], b[1], t)]
}

/// Damped wave equation h_tt = c^2 laplacian(h) - damping * h_t - restore * h + forcing, integrated with
/// semi-implicit Euler in substeps that respect the CFL limit. The border absorbs outgoing waves.
pub struct Wave {
    pub grid: Grid,
    pub height: Vec<f32>,
    pub velocity: Vec<f32>,
    damping: Vec<f32>,
    pub time: f32,
}

impl Wave {
    pub fn new(grid: Grid, time: f32) -> Self {
        let mut damping = vec![DAMPING; grid.len()];
        for y in 0..grid.rows {
            for x in 0..grid.columns {
                let edge = x.min(y).min(grid.columns - 1 - x).min(grid.rows - 1 - y) as f32;
                let depth = ((SPONGE_CELLS - edge) / SPONGE_CELLS).max(0.0);
                damping[y * grid.columns + x] += SPONGE * depth * depth;
            }
        }
        Self {
            grid,
            height: vec![0.0; grid.len()],
            velocity: vec![0.0; grid.len()],
            damping,
            time,
        }
    }

    /// Adds `amount` to the surface velocity around a world point with a soft 3x3 footprint.
    pub fn drive(&mut self, p: [f32; 2], amount: f32) {
        let [x, y] = self.grid.locate(p);
        let (cx, cy) = (x.round() as isize, y.round() as isize);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (x, y) = (cx + dx, cy + dy);
                if x < 1
                    || y < 1
                    || x >= self.grid.columns as isize - 1
                    || y >= self.grid.rows as isize - 1
                {
                    continue;
                }
                let weight = [1.0, 0.5, 0.25][(dx.abs() + dy.abs()) as usize];
                self.velocity[y as usize * self.grid.columns + x as usize] += amount * weight;
            }
        }
    }

    pub fn step(&mut self, dt: f32) {
        let (w, h) = (self.grid.columns, self.grid.rows);
        let c2 = WAVE_SPEED * WAVE_SPEED;
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = y * w + x;
                let h = &self.height;
                // Nine-point stencil: its error is nearly isotropic, so rings stay round instead
                // of squaring off along the grid axes.
                let laplacian = (4.0 * (h[i - 1] + h[i + 1] + h[i - w] + h[i + w])
                    + h[i - w - 1]
                    + h[i - w + 1]
                    + h[i + w - 1]
                    + h[i + w + 1]
                    - 20.0 * h[i])
                    / 6.0;
                self.velocity[i] +=
                    (c2 * laplacian - self.damping[i] * self.velocity[i] - RESTORE * h[i]) * dt;
            }
        }
        for (height, velocity) in self.height.iter_mut().zip(&self.velocity) {
            *height += velocity * dt;
        }
    }

    /// Advances to `time` in substeps no longer than half a cell's travel time.
    pub fn advance(&mut self, time: f32, mut force: impl FnMut(&mut Self, f32, f32)) {
        let elapsed = (time - self.time).clamp(0.0, 0.25);
        let steps = (elapsed * WAVE_SPEED / 0.5).ceil() as usize;
        for step in 0..steps {
            let dt = elapsed / steps as f32;
            let now = self.time + dt * (step + 1) as f32;
            force(self, now, dt);
            self.step(dt);
        }
        self.time = time;
    }

    #[cfg(test)]
    pub fn energy(&self) -> f32 {
        self.velocity.iter().map(|v| v * v).sum::<f32>()
            + self.height.iter().map(|h| h * h).sum::<f32>()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    /// Newest position first; the rest form the fading trail.
    pub trail: [Point; TRAIL],
    pub age: f32,
    pub life: f32,
    pub color: Color,
    /// Bright particles come from busy processes; dim ones only reveal the field.
    pub bright: bool,
    /// Rising particles leave the sheet: connections that leave the machine.
    pub rising: bool,
    pub kick: [f32; 2],
}

/// Particles advected through a velocity field cached on a grid, over a sheet of depths.
pub struct Flow {
    pub grid: Grid,
    pub velocity: Vec<[f32; 2]>,
    pub depth: Vec<f32>,
    pub particles: Vec<Particle>,
    pub field_time: f32,
    pub time: f32,
    seed: u32,
}

impl Flow {
    pub fn new(grid: Grid, time: f32) -> Self {
        Self {
            grid,
            velocity: vec![[0.0; 2]; grid.len()],
            depth: vec![0.0; grid.len()],
            particles: Vec::new(),
            field_time: f32::NEG_INFINITY,
            time,
            seed: 0x9e37_79b9,
        }
    }

    /// Uniform random number in [0, 1) from a xorshift generator.
    pub fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1 << 24) as f32
    }

    pub fn depth_at(&self, p: [f32; 2]) -> f32 {
        self.grid.sample(&self.depth, p, mix)
    }

    pub fn velocity_at(&self, p: [f32; 2]) -> [f32; 2] {
        self.grid.sample(&self.velocity, p, mix2)
    }

    pub fn spawn(
        &mut self,
        p: [f32; 2],
        color: Color,
        life: f32,
        bright: bool,
        rising: bool,
        kick: [f32; 2],
    ) {
        let point = [p[0], p[1], self.depth_at(p) + 0.15];
        self.particles.push(Particle {
            trail: [point; TRAIL],
            age: 0.0,
            life,
            color,
            bright,
            rising,
            kick,
        });
    }

    /// Moves every particle by `dt` with a midpoint (RK2) step; `stir` adds turbulence.
    pub fn advance(&mut self, dt: f32, stir: impl Fn([f32; 2]) -> [f32; 2]) {
        let mut particles = std::mem::take(&mut self.particles);
        for particle in &mut particles {
            particle.age += dt;
            let head = particle.trail[0];
            let p = [head[0], head[1]];
            let flow = |q: [f32; 2]| {
                let field = self.velocity_at(q);
                let swirl = stir(q);
                [
                    field[0] + swirl[0] + particle.kick[0],
                    field[1] + swirl[1] + particle.kick[1],
                ]
            };
            let v1 = flow(p);
            let mid = [p[0] + v1[0] * dt * 0.5, p[1] + v1[1] * dt * 0.5];
            let v2 = flow(mid);
            let next = [p[0] + v2[0] * dt, p[1] + v2[1] * dt];
            let decay = (-2.5 * dt).exp();
            particle.kick = [particle.kick[0] * decay, particle.kick[1] * decay];
            let z = if particle.rising {
                head[2] + 2.5 * dt
            } else {
                self.depth_at(next) + 0.15
            };
            particle.trail.rotate_right(1);
            particle.trail[0] = [next[0], next[1], z];
        }
        particles.retain(|p| p.age < p.life && self.grid.contains([p.trail[0][0], p.trail[0][1]]));
        self.particles = particles;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waves_spread_at_their_speed_and_die_out_at_the_border() {
        let grid = Grid::covering([0.0, 0.0], [100.0, 100.0], 100);
        let mut wave = Wave::new(grid, 0.0);
        wave.drive([50.0, 50.0], 40.0);
        for step in 1..=20 {
            wave.advance(step as f32 * 0.05, |_, _, _| {});
        }
        let ring = |wave: &Wave, r: f32| {
            (0..64)
                .map(|k| {
                    let a = k as f32 / 64.0 * std::f32::consts::TAU;
                    wave.grid
                        .sample(&wave.height, [50.0 + r * a.cos(), 50.0 + r * a.sin()], mix)
                        .abs()
                })
                .sum::<f32>()
        };
        assert!(
            ring(&wave, WAVE_SPEED) > ring(&wave, 2.0 * WAVE_SPEED) * 4.0,
            "front near c*t"
        );
        let early = wave.energy();
        for step in 21..=600 {
            wave.advance(step as f32 * 0.05, |_, _, _| {});
        }
        assert!(wave.energy().is_finite());
        assert!(
            wave.energy() < early * 0.01,
            "{} -> {}",
            early,
            wave.energy()
        );
    }

    #[test]
    fn particles_circle_a_vortex_and_ride_a_jet() {
        let grid = Grid::covering([-20.0, -20.0], [20.0, 20.0], 80);
        let mut flow = Flow::new(grid, 0.0);
        for j in 0..grid.rows {
            for i in 0..grid.columns {
                let [x, y] = grid.world(i as f32, j as f32);
                flow.velocity[j * grid.columns + i] = [-y, x];
            }
        }
        flow.spawn([10.0, 0.0], [255; 3], 100.0, true, false, [0.0; 2]);
        for _ in 0..157 {
            flow.advance(0.01, |_| [0.0; 2]);
        }
        let head = flow.particles[0].trail[0];
        assert!(
            (head[0].hypot(head[1]) - 10.0).abs() < 0.2,
            "radius kept: {head:?}"
        );
        assert!(head[1] > 9.5, "a quarter turn counterclockwise: {head:?}");
        flow.velocity.fill([3.0, 0.0]);
        flow.particles.clear();
        flow.spawn([-10.0, 0.0], [255; 3], 100.0, true, false, [0.0; 2]);
        for _ in 0..100 {
            flow.advance(0.05, |_| [0.0; 2]);
        }
        assert!((flow.particles[0].trail[0][0] - 5.0).abs() < 0.1);
    }
}
