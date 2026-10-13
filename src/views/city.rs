//! The City view: every process is a building on a plot of a district. A district is a 4 by 4
//! block of plots, one cgroup's processes filling theirs in turn, and the districts spiral out in
//! square shells. Height is CPU, the footprint is memory, amber windows are CPU, a beacon marks
//! GPU memory and a cyan pulse running along a wall is I/O.
//!
//! A process keeps its plot for as long as it lives, and a plot is freed only when its process is
//! gone from the sample, so the city does not move when CPU, memory or the population changes.
//! Links rise above the rooftops by the row's `link_lift`, and the camera fit leaves room for
//! the tallest building through `Stage::headroom`.

use std::collections::{HashMap, HashSet};

use crate::model::{Identity, Process, Snapshot, bounded};
use crate::palette::{DISTRICTS, process_color, tint};
use crate::platform;
use crate::render::{Color, NONE, NVIDIA, Point, Stage};
use crate::views::ViewModule;

/// Plots in a district: four by four.
const PLOTS: usize = 16;
/// Plots along one side of a district, and the world units each takes.
const SIDE: usize = 4;
const PLOT_SIZE: f32 = 4.0;

/// Where a process stands: its district and the plot of that district.
#[derive(Clone, Copy)]
struct Plot {
    district: usize,
    slot: usize,
}

/// The city: where each process stands, which only ends when the process does.
#[derive(Default)]
pub struct City {
    plots: HashMap<Identity, Plot>,
    /// The districts each cgroup has been given, in the order it got them.
    groups: HashMap<String, Vec<usize>>,
    /// The process on each plot of each district.
    occupancy: Vec<HashMap<usize, Identity>>,
}

/// The south-west corner of a district. The shells grow square by square without relocating any
/// district allocated before.
fn district_origin(index: usize) -> [f32; 2] {
    let shell = (index as f32).sqrt() as usize;
    let offset = index - shell * shell;
    let (x, y) = if offset <= shell {
        (shell, offset)
    } else {
        (2 * shell - offset, shell)
    };
    [x as f32 * 23.0, y as f32 * 23.0]
}

impl City {
    /// Frees the plots of processes that left the sample, gives the new ones the first vacant
    /// plot of their cgroup's districts (or a new district), and puts every drawn process at the
    /// middle of its plot in `stage.positions`.
    fn place(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) {
        let alive: HashSet<_> = snapshot.processes.iter().map(|p| p.id).collect();
        self.plots.retain(|id, plot| {
            if alive.contains(id) {
                true
            } else {
                self.occupancy[plot.district].remove(&plot.slot);
                false
            }
        });
        for process in processes {
            if !self.plots.contains_key(&process.id) {
                let districts = self.groups.entry(process.group.clone()).or_default();
                let mut vacant = None;
                for &district in districts.iter() {
                    if let Some(slot) =
                        (0..PLOTS).find(|s| !self.occupancy[district].contains_key(s))
                    {
                        vacant = Some(Plot { district, slot });
                        break;
                    }
                }
                let plot = vacant.unwrap_or_else(|| {
                    let district = self.occupancy.len();
                    self.occupancy.push(HashMap::new());
                    districts.push(district);
                    Plot { district, slot: 0 }
                });
                self.occupancy[plot.district].insert(plot.slot, process.id);
                self.plots.insert(process.id, plot);
            }
            let plot = self.plots[&process.id];
            let base = district_origin(plot.district);
            stage.positions.insert(
                process.id,
                [
                    base[0] + (plot.slot % SIDE) as f32 * PLOT_SIZE + 2.0,
                    base[1] + (plot.slot / SIDE) as f32 * PLOT_SIZE + 2.0,
                    0.0,
                ],
            );
        }
    }
}

/// The ground of one district: its slab, the grid of its plots and the border in its colour.
fn draw_district(stage: &mut Stage, district: usize) {
    let camera = stage.camera;
    let [x, y] = district_origin(district);
    stage.frame.quad(
        camera,
        [
            [x - 1.0, y - 1.0, -0.12],
            [x + 17.0, y - 1.0, -0.12],
            [x + 17.0, y + 17.0, -0.12],
            [x - 1.0, y + 17.0, -0.12],
        ],
        [18, 29, 43],
        NONE,
    );
    let color = tint(DISTRICTS[district % 8], 0.35);
    for i in 0..=4 {
        let offset = i as f32 * 4.0;
        stage.frame.line(
            camera,
            [x + offset, y, 0.0],
            [x + offset, y + 16.0, 0.0],
            [29, 43, 57],
        );
        stage.frame.line(
            camera,
            [x, y + offset, 0.0],
            [x + 16.0, y + offset, 0.0],
            [29, 43, 57],
        );
    }
    stage.frame.line(
        camera,
        [x - 0.5, y - 0.5, 0.0],
        [x + 16.5, y - 0.5, 0.0],
        color,
    );
    stage.frame.line(
        camera,
        [x - 0.5, y - 0.5, 0.0],
        [x - 0.5, y + 16.5, 0.0],
        color,
    );
}

/// The lit and dark windows of one wall, `side` of the building whose footing is `corners`, up to
/// `height`. Drawn only when zoomed in.
fn draw_windows(
    stage: &mut Stage,
    process: &Process,
    index: usize,
    side: usize,
    corners: &[Point; 4],
    height: f32,
) {
    let next = (side + 1) % 4;
    let levels = (height / 0.85) as usize;
    for level in 0..levels {
        for column in 0..3 {
            let a = 0.15 + column as f32 * 0.26;
            let b = a + 0.12;
            let z = 0.35 + level as f32 * 0.85;
            if z + 0.25 >= height {
                continue;
            }
            let point = |t: f32, z: f32| {
                [
                    corners[side][0] + (corners[next][0] - corners[side][0]) * t,
                    corners[side][1] + (corners[next][1] - corners[side][1]) * t,
                    z,
                ]
            };
            let lit = !(process.id.pid as usize + level * 7 + column * 11).is_multiple_of(5);
            let window = if lit {
                tint([241, 206, 136], 0.55 + 0.35 * bounded(process.cpu, 50.0))
            } else {
                [28, 40, 53]
            };
            let mut points = [
                point(a, z),
                point(b, z),
                point(b, z + 0.25),
                point(a, z + 0.25),
            ];
            // Offset windows outward so the depth buffer doesn't z-fight with the wall.
            let normal = [[0.0, -0.015], [0.015, 0.0], [0.0, 0.015], [-0.015, 0.0]][side];
            for p in &mut points {
                p[0] += normal[0];
                p[1] += normal[1];
            }
            stage.frame.quad(stage.camera, points, window, index as u32);
        }
    }
}

/// One building at `[x, y]` on plot `plot`: its shadow, walls, windows, roof, rooftop lamp, GPU
/// beacon, selection outline and I/O pulse. Returns its height.
fn draw_building(
    stage: &mut Stage,
    process: &Process,
    index: usize,
    plot: Plot,
    [x, y, _]: Point,
) -> f32 {
    let camera = stage.camera;
    let time = stage.time;
    let grow = stage.growth(process.id);
    let half = (0.42 + 1.05 * bounded(process.memory as f32 / 1048576.0, 220.0).sqrt())
        * (0.35 + 0.65 * grow);
    // Idle processes stay low blocks; one busy core is a ~23-unit tower and multi-core
    // work climbs towards 45, against 4-unit plots.
    let height = (1.0 + 44.0 * bounded(process.cpu, 100.0)) * grow.max(0.05);
    let shadow = height.min(14.0);
    let color: Color = process_color(process, plot.district);
    stage.frame.quad(
        camera,
        [
            [x - half, y - half, 0.01],
            [x + half + shadow * 0.5, y - half + shadow * 0.3, 0.01],
            [x + half + shadow * 0.5, y + half + shadow * 0.3, 0.01],
            [x - half, y + half, 0.01],
        ],
        [10, 18, 29],
        NONE,
    );
    let corners = [
        [x - half, y - half, 0.05],
        [x + half, y - half, 0.05],
        [x + half, y + half, 0.05],
        [x - half, y + half, 0.05],
    ];
    let top = corners.map(|mut p| {
        p[2] = height;
        p
    });
    for side in 0..4 {
        let next = (side + 1) % 4;
        stage.frame.quad(
            camera,
            [corners[side], corners[next], top[next], top[side]],
            tint(color, [0.40, 0.66, 0.55, 0.30][side]),
            index as u32,
        );
        if camera.zoom > 3.0 {
            draw_windows(stage, process, index, side, &corners, height);
        }
    }
    stage
        .frame
        .quad(camera, top, tint(color, 0.95), index as u32);
    if process.cpu > 1.0 {
        let brightness = 0.65 + 0.25 * (time * 2.0 + process.id.pid as f32).sin();
        stage.frame.sphere(
            camera,
            [x, y, height + 0.22],
            0.12,
            tint([255, 196, 104], brightness),
            index as u32,
            false,
        );
    }
    if process.gpu_memory.is_some() {
        let beacon = [x - half * 0.5, y - half * 0.5, height + 0.3];
        stage
            .frame
            .glow(camera, beacon, (1.4 * camera.zoom).max(8.0), NVIDIA, 0.7);
        stage
            .frame
            .sphere(camera, beacon, 0.16, NVIDIA, index as u32, false);
    }
    if stage.selected == Some(process.id) {
        for side in 0..4 {
            let next = (side + 1) % 4;
            stage
                .frame
                .line(camera, top[side], top[next], [255, 235, 171]);
            stage
                .frame
                .line(camera, corners[side], top[side], [255, 235, 171]);
        }
    }
    if process.io_rate.is_some_and(|v| v > 1024.0) {
        let t = (time * 0.7 + process.id.pid as f32 * 0.13).fract();
        stage.frame.sphere(
            camera,
            [x + half + 0.2, y - half + t * half * 2.0, 0.15],
            0.12,
            [105, 228, 216],
            index as u32,
            false,
        );
    }
    height
}

impl ViewModule for City {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        self.place(stage, processes, snapshot);
        let districts: HashSet<_> = processes
            .iter()
            .map(|p| self.plots[&p.id].district)
            .collect();
        for district in districts {
            draw_district(stage, district);
        }
        let mut tallest = 0.0_f32;
        for (index, process) in processes.iter().enumerate() {
            let position = stage.positions[&process.id];
            let plot = self.plots[&process.id];
            tallest = tallest.max(draw_building(stage, process, index, plot, position));
        }
        *stage.headroom = tallest;
        processes.len()
    }

    fn legend(&self, _snapshot: &Snapshot) -> String {
        format!(
            " Height = CPU | footprint = {} | district = cgroup | amber lights = CPU | cyan pulses = IO",
            platform::MEMORY_LABEL
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;
    use crate::render::{Camera, Frame, RenderInput, Scene};
    use crate::views::{View, view};

    fn render(scene: &mut Scene, snapshot: &Snapshot, view: View, time: f32) -> Frame {
        let mut frame = scene.render(&RenderInput::small(
            snapshot,
            view,
            &Camera::default(),
            time,
        ));
        frame.rasterize();
        frame
    }

    #[test]
    fn city_does_not_relocate_when_resources_or_population_change() {
        let mut scene = Scene::new();
        let mut snapshot = demo(0.0, 32);
        render(&mut scene, &snapshot, view("city"), 0.0);
        let id = snapshot.processes[10].id;
        let position = scene.positions[&id];
        snapshot.processes.remove(5);
        snapshot.processes[9].cpu = 1600.0;
        snapshot.processes[9].memory *= 100;
        render(&mut scene, &snapshot, view("city"), 0.0);
        assert_eq!(scene.positions[&id], position);
    }

    #[test]
    fn a_district_holds_sixteen_plots_and_a_cgroup_fills_its_districts_in_turn() {
        let mut scene = Scene::new();
        let mut snapshot = demo(0.0, 40);
        for process in &mut snapshot.processes {
            process.group = "one cgroup".into();
        }
        render(&mut scene, &snapshot, view("city"), 0.0);
        let mut districts: HashMap<[i32; 2], usize> = HashMap::new();
        for position in scene.positions.values() {
            let origin = [(position[0] / 23.0) as i32, (position[1] / 23.0) as i32];
            *districts.entry(origin).or_default() += 1;
        }
        let mut sizes: Vec<usize> = districts.into_values().collect();
        sizes.sort_unstable();
        assert_eq!(sizes, [8, 16, 16]);
    }

    #[test]
    fn the_plot_of_a_process_that_left_is_taken_by_the_next_arrival_of_its_cgroup() {
        let mut scene = Scene::new();
        let mut snapshot = demo(0.0, 8);
        for process in &mut snapshot.processes {
            process.group = "one cgroup".into();
        }
        render(&mut scene, &snapshot, view("city"), 0.0);
        let gone = snapshot.processes.remove(3);
        let freed = scene.positions[&gone.id];
        render(&mut scene, &snapshot, view("city"), 0.1);
        let mut arrival = snapshot.processes[0].clone();
        arrival.id.pid += 10_000;
        snapshot.processes.push(arrival.clone());
        render(&mut scene, &snapshot, view("city"), 0.2);
        assert_eq!(scene.positions[&arrival.id], freed);
    }

    #[test]
    fn the_camera_fit_leaves_room_for_the_tallest_building() {
        let fitted = |cpu: f32| {
            let mut scene = Scene::new();
            let mut snapshot = demo(0.0, 6);
            for process in &mut snapshot.processes {
                process.cpu = cpu;
            }
            render(&mut scene, &snapshot, view("city"), 0.0);
            let mut camera = Camera::default();
            scene.fit(&mut camera, 320, 180);
            camera.zoom
        };
        assert!(fitted(100.0) < fitted(0.0));
    }

    #[test]
    fn the_legend_names_the_memory_measure() {
        let scene = Scene::new();
        let legend = scene.legend(view("city"), &Snapshot::default());
        assert!(legend.starts_with(" Height = CPU | footprint = "));
        assert!(legend.contains(platform::MEMORY_LABEL));
    }
}
