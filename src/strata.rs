//! Strata: a minute of CPU history as a ridgeline landscape. Each busy process is a ridge whose
//! profile is its CPU use over time, newest at the front edge. Ridges are seated from kernel
//! threads at the back to containers at the front and keep their row while they stay busy.

use std::collections::HashMap;

use crate::history::Sample;
use crate::model::{Identity, Kind, Process, Snapshot, bounded};
use crate::palette::{kind_color, tint};
use crate::render::{Point, Stage, label_name};
use crate::views::ViewModule;

const WINDOW: f64 = 60.0;
const ROWS: usize = 40;
/// Ground units per second of history, between rows, and of ridge height at saturating CPU.
const SPAN: f32 = 1.0;
const SPACING: f32 = 2.4;
const PEAK: f32 = 16.0;
/// A process must reach this much CPU in the window to earn a ridge.
const BUSY: f32 = 0.5;

#[derive(Default)]
pub struct Strata {
    /// Each ridge's row, 0 at the back; a ridge keeps its row for as long as it has one.
    rows: HashMap<Identity, usize>,
}

impl Strata {
    #[cfg(test)]
    fn rows(&self) -> Vec<Identity> {
        let mut rows: Vec<(usize, Identity)> = self.rows.iter().map(|(&id, &r)| (r, id)).collect();
        rows.sort();
        rows.into_iter().map(|(_, id)| id).collect()
    }
}

impl ViewModule for Strata {
    #[expect(clippy::too_many_lines, reason = "split in #71 (A12)")]
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let now = snapshot.elapsed;
        let window: Vec<&Sample> = stage.history.window(now, WINDOW).collect();
        let interval = match window.as_slice() {
            [.., a, b] => (b.elapsed - a.elapsed) as f32,
            _ => 1.0,
        };
        // The landscape scrolls smoothly between samples while the newest one is the shown one.
        let live = stage.history.newest() == Some(now);
        let scroll = if live {
            (stage.time - stage.history.arrived()).clamp(0.0, interval)
        } else {
            0.0
        };
        let index: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, i))
            .collect();
        let peak = |id: &Identity| {
            window
                .iter()
                .filter_map(|sample| sample.cpu(*id))
                .fold(0.0_f32, f32::max)
        };
        // Rows are given to every sampled process, drawn or not, so focusing a subtree, or the
        // process limit (and a selection swapped across it) hiding some, frees none of their
        // rows: clearing it brings every ridge back to the row it had. Drawn processes come
        // first, though: they are offered rows before hidden ones, and when every row is taken
        // a drawn one takes the row of the least busy hidden holder. Only drawn ridges are
        // drawn below.
        let mut kinds: HashMap<Identity, Kind> =
            snapshot.processes.iter().map(|p| (p.id, p.kind)).collect();
        kinds.extend(processes.iter().map(|p| (p.id, p.kind)));
        self.rows
            .retain(|id, _| kinds.contains_key(id) && peak(id) >= BUSY);
        let mut candidates: Vec<(bool, f32, Identity)> = kinds
            .keys()
            .filter(|id| !self.rows.contains_key(id))
            .map(|&id| (!index.contains_key(&id), peak(&id), id))
            .filter(|&(_, value, _)| value >= BUSY)
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)).then(a.2.cmp(&b.2)));
        for (hidden, _, id) in candidates {
            if self.rows.len() >= ROWS {
                if hidden {
                    break;
                }
                let Some(holder) = self
                    .rows
                    .keys()
                    .filter(|held| !index.contains_key(held))
                    .map(|&held| (peak(&held), held))
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                    .map(|(_, held)| held)
                else {
                    break;
                };
                self.rows.remove(&holder);
            }
            // Each kind owns a quarter of the floor, back to front; a newcomer takes the free row
            // nearest the middle of its kind's quarter, spilling into neighbours when it is full.
            let rank = match kinds[&id] {
                Kind::Kernel => 0,
                Kind::System => 1,
                Kind::Session => 2,
                Kind::Container => 3,
            };
            let home = rank * ROWS / 4 + ROWS / 8;
            let taken: Vec<usize> = self.rows.values().copied().collect();
            let row = (0..ROWS)
                .filter(|row| !taken.contains(row))
                .min_by_key(|&row| (row.abs_diff(home), row))
                .expect("fewer than ROWS rows are taken");
            self.rows.insert(id, row);
        }
        let width = WINDOW as f32 * SPAN;
        let depth = ROWS as f32 * SPACING;
        let height = |cpu: f32| 0.15 + PEAK * bounded(cpu, 60.0);
        let frame = &mut *stage.frame;
        let camera = stage.camera;
        frame.quad(
            camera,
            [
                [-width - 2.0, -SPACING, -0.05],
                [3.0, -SPACING, -0.05],
                [3.0, depth + 1.0, -0.05],
                [-width - 2.0, depth + 1.0, -0.05],
            ],
            [14, 22, 32],
            crate::render::NONE,
        );
        for tick in 0..=(WINDOW as usize / 15) {
            let x = -(tick as f32 * 15.0) * SPAN;
            frame.line(
                camera,
                [x, -SPACING, 0.0],
                [x, depth + 1.0, 0.0],
                [30, 44, 58],
            );
            let text = if tick == 0 {
                "now".into()
            } else {
                format!("-{} s", tick * 15)
            };
            stage.places.push(([x, depth + 2.5, 0.0], text));
        }
        let mut shown = 0;
        for (id, &row) in &self.rows {
            let Some(&pick) = index.get(id) else {
                continue;
            };
            shown += 1;
            let process = processes[pick];
            let pick = pick as u32;
            let y = row as f32 * SPACING;
            let mut profile: Vec<[f32; 2]> = window
                .iter()
                .map(|sample| {
                    let x = ((sample.elapsed - now) as f32 - scroll) * SPAN;
                    [x, height(sample.cpu(*id).unwrap_or(0.0))]
                })
                .filter(|&[x, _]| x >= -width)
                .collect();
            if let Some(&[_, z]) = profile.last() {
                profile.push([0.0, z]);
            }
            if let [[x, z], _] = profile[..] {
                profile.insert(0, [x - interval * SPAN, z]);
            }
            let base = kind_color(process);
            let selected = stage.selected == Some(*id);
            for pair in profile.windows(2) {
                let ([x0, z0], [x1, z1]) = (pair[0], pair[1]);
                let fraction = (z0 + z1) / (2.0 * PEAK);
                let (m0, m1) = (z0 * 0.45, z1 * 0.45);
                frame.quad(
                    camera,
                    [[x0, y, 0.0], [x1, y, 0.0], [x1, y, m1], [x0, y, m0]],
                    tint(base, 0.10 + 0.12 * fraction),
                    pick,
                );
                frame.quad(
                    camera,
                    [[x0, y, m0], [x1, y, m1], [x1, y, z1], [x0, y, z0]],
                    tint(base, 0.20 + 0.45 * fraction),
                    pick,
                );
                let ridge = if selected {
                    [255, 235, 171]
                } else {
                    tint(base, 0.75 + 0.6 * fraction)
                };
                frame.line(camera, [x0, y, z0], [x1, y, z1], ridge);
            }
            let top = profile.last().map_or(0.0, |&[_, z]| z);
            let front: Point = [0.0, y, top];
            stage.positions.insert(*id, front);
            stage.places.push((
                [1.5, y, 0.0],
                format!("{} {:.0}%", label_name(&process.name), process.cpu),
            ));
            stage.notes.insert(
                *id,
                vec![format!(
                    "peak {:.0}% CPU over the last {:.0} s",
                    peak(id),
                    window.last().map_or(0.0, |l| l.elapsed - window[0].elapsed)
                )],
            );
        }
        if shown == 0 {
            stage.places.push((
                [-width * 0.5, depth * 0.5, 0.0],
                "no process has used CPU yet".into(),
            ));
        }
        stage.bounds.extend(
            [-width - 2.0, 3.0]
                .into_iter()
                .flat_map(|x| [-SPACING, depth + 2.5].map(|y| [x, y]))
                .flat_map(|[x, y]| [[x, y, 0.0], [x, y, PEAK * 0.7]]),
        );
        shown
    }

    /// The status legend.
    fn legend(&self, _snapshot: &Snapshot) -> String {
        " Ridge = process, height = CPU over the last minute, newest at the front | rows: kernel, system, session, containers".into()
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Strata;
    use crate::model::{Identity, Kind, Snapshot, demo};
    use crate::render::{Camera, RenderInput, Scene, hiding};
    use crate::views::{View, view};

    fn render(scene: &mut Scene, snapshot: &Snapshot, view: View, time: f32) {
        let mut frame = scene.render(&RenderInput::small(
            snapshot,
            view,
            &Camera::default(),
            time,
        ));
        frame.rasterize();
    }

    /// The ridges' identities from the back row to the front.
    fn ridges(scene: &Scene) -> Vec<Identity> {
        scene
            .module::<Strata>(view("strata"))
            .expect("the strata view exists")
            .rows()
    }

    /// Few enough processes that every busy one gets one of the forty rows. With more, a drawn
    /// process takes a hidden one's row, and the hidden one finds none free when it is back.
    fn sample() -> Snapshot {
        let sample = demo(30.0, 60);
        let busy = sample.processes.iter().filter(|p| p.cpu >= super::BUSY);
        assert!(busy.count() <= super::ROWS);
        sample
    }

    #[test]
    fn focusing_and_clearing_focus_keeps_every_ridge_in_its_row() {
        hiding::focus_and_clear(view("strata"), &sample());
    }

    #[test]
    fn processes_beyond_the_limit_keep_their_strata_row() {
        hiding::limit_and_lift(view("strata"), &sample());
    }

    /// With more busy processes than rows, the ones still drawn under a focus or a limit get
    /// rows ahead of the hidden ones, so as many ridges show as on a fresh landscape.
    #[test]
    fn hidden_processes_do_not_keep_drawn_busy_ones_from_a_row() {
        let sample = demo(30.0, 160);
        let busy = sample
            .processes
            .iter()
            .filter(|p| p.cpu >= super::BUSY)
            .count();
        assert!(busy > super::ROWS);
        let (mut scene, time, _) = hiding::settled(view("strata"), &sample);
        let drawn = |scene: &mut Scene, limit: usize, focus: Option<Identity>| {
            hiding::place(scene, &sample, view("strata"), time, limit, None, focus).len()
        };
        let hides = sample
            .processes
            .iter()
            .filter(|p| p.id.pid >= 1016 && p.id.pid % 16 == 0)
            .map(|p| (4096, Some(p.id)))
            .chain([(80, None)]);
        let mut shown = 0;
        for (limit, focus) in hides {
            let fresh = drawn(&mut Scene::new(), limit, focus);
            shown += fresh;
            assert_eq!(drawn(&mut scene, limit, focus), fresh, "{limit} {focus:?}");
        }
        assert!(shown > super::ROWS);
    }

    #[test]
    fn strata_ridges_follow_busy_processes_in_kind_order() {
        let mut scene = Scene::new();
        let mut snapshot = demo(0.0, 64);
        for step in 0..30 {
            snapshot.elapsed = step as f64;
            for (i, p) in snapshot.processes.iter_mut().enumerate() {
                p.cpu = if i % 4 == 0 { 30.0 + step as f32 } else { 0.0 };
            }
            // History builds up while another view is shown.
            scene.record(&snapshot, step as f32);
            render(&mut scene, &snapshot, view("city"), step as f32);
        }
        render(&mut scene, &snapshot, view("strata"), 29.0);
        assert_eq!(scene.visible, 16);
        let first = snapshot.processes[0].id;
        assert!(
            scene.notes[&first][0].ends_with("over the last 29 s"),
            "{:?}",
            scene.notes[&first]
        );
        let rows: HashMap<Identity, f32> =
            scene.positions.iter().map(|(id, p)| (*id, p[1])).collect();
        snapshot.elapsed = 30.0;
        snapshot.processes[33].cpu = 50.0;
        render(&mut scene, &snapshot, view("strata"), 30.0);
        assert_eq!(scene.visible, 17, "a newly busy process earns a ridge");
        for (id, y) in &rows {
            assert_eq!(scene.positions[id][1], *y, "an existing ridge moved row");
        }
        let order: Vec<u8> = ridges(&scene)
            .iter()
            .map(|id| {
                let kind = snapshot
                    .processes
                    .iter()
                    .find(|p| p.id == *id)
                    .unwrap()
                    .kind;
                [Kind::Kernel, Kind::System, Kind::Session, Kind::Container]
                    .iter()
                    .position(|&k| k == kind)
                    .unwrap() as u8
            })
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] <= pair[1]), "{order:?}");
    }

    #[test]
    fn strata_reads_the_history_recorded_while_the_city_is_shown() {
        let mut scene = Scene::new();
        for step in 0..=10 {
            let snapshot = demo(step as f64, 64);
            scene.record(&snapshot, step as f32);
            render(&mut scene, &snapshot, view("city"), step as f32);
        }
        render(&mut scene, &demo(10.0, 64), view("strata"), 10.0);
        assert!(
            scene
                .notes
                .values()
                .any(|notes| notes[0].ends_with("over the last 10 s")),
            "{:?}",
            scene.notes.values().next()
        );
    }
}
