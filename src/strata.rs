//! Strata: a minute of CPU history as a ridgeline landscape. Each busy process is a ridge whose
//! profile is its CPU use over time, newest at the front edge. Ridges are seated from kernel
//! threads at the back to containers at the front and keep their row while they stay busy.

use std::collections::{HashMap, VecDeque};

use crate::model::{Identity, Kind, Process, Snapshot, bounded};
use crate::render::{Point, Stage, kind_color, tint};

const WINDOW: f64 = 60.0;
const SAMPLES: usize = 240;
const ROWS: usize = 40;
/// Ground units per second of history, between rows, and of ridge height at saturating CPU.
const SPAN: f32 = 1.0;
const SPACING: f32 = 2.4;
const PEAK: f32 = 16.0;
/// A process must reach this much CPU in the window to earn a ridge.
const BUSY: f32 = 0.5;

#[derive(Default)]
pub struct Strata {
    history: VecDeque<(f64, HashMap<Identity, f32>)>,
    /// Scene time at which the newest sample arrived, so the landscape scrolls smoothly in between.
    arrived: f32,
    /// Each ridge's row, 0 at the back; a ridge keeps its row for as long as it has one.
    rows: HashMap<Identity, usize>,
}

impl Strata {
    #[cfg(test)]
    pub fn rows(&self) -> Vec<Identity> {
        let mut rows: Vec<(usize, Identity)> = self.rows.iter().map(|(&id, &r)| (r, id)).collect();
        rows.sort();
        rows.into_iter().map(|(_, id)| id).collect()
    }

    /// Keeps a sample in the history. The scene records every sample, whichever view is shown, so
    /// the landscape is already a minute deep when the view opens.
    pub fn record(&mut self, snapshot: &Snapshot, time: f32) {
        if self
            .history
            .back()
            .is_none_or(|&(elapsed, _)| snapshot.elapsed > elapsed)
        {
            let sample = snapshot.processes.iter().map(|p| (p.id, p.cpu)).collect();
            self.history.push_back((snapshot.elapsed, sample));
            self.arrived = time;
            while self.history.len() > SAMPLES {
                self.history.pop_front();
            }
        }
    }

    pub fn draw(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        snapshot: &Snapshot,
    ) -> usize {
        self.record(snapshot, stage.time);
        let now = snapshot.elapsed;
        let window: Vec<&(f64, HashMap<Identity, f32>)> = self
            .history
            .iter()
            .filter(|(elapsed, _)| (now - WINDOW..=now).contains(elapsed))
            .collect();
        let interval = match window.as_slice() {
            [.., a, b] => (b.0 - a.0) as f32,
            _ => 1.0,
        };
        let live = self.history.back().is_some_and(|&(e, _)| e == now);
        let scroll = if live {
            (stage.time - self.arrived).clamp(0.0, interval)
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
                .filter_map(|(_, sample)| sample.get(id).copied())
                .fold(0.0_f32, f32::max)
        };
        self.rows
            .retain(|id, _| index.contains_key(id) && peak(id) >= BUSY);
        let mut candidates: Vec<(f32, Identity)> = processes
            .iter()
            .filter(|p| !self.rows.contains_key(&p.id))
            .map(|p| (peak(&p.id), p.id))
            .filter(|&(value, _)| value >= BUSY)
            .collect();
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, id) in candidates {
            if self.rows.len() >= ROWS {
                break;
            }
            // Each kind owns a quarter of the floor, back to front; a newcomer takes the free row
            // nearest the middle of its kind's quarter, spilling into neighbours when it is full.
            let rank = match processes[index[&id]].kind {
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
        for (id, &row) in &self.rows {
            let process = processes[index[id]];
            let pick = index[id] as u32;
            let y = row as f32 * SPACING;
            let mut profile: Vec<[f32; 2]> = window
                .iter()
                .map(|(elapsed, sample)| {
                    let x = ((elapsed - now) as f32 - scroll) * SPAN;
                    [x, height(sample.get(id).copied().unwrap_or(0.0))]
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
                format!("{} {:.0}%", process.name, process.cpu),
            ));
            stage.notes.insert(
                *id,
                vec![format!(
                    "peak {:.0}% CPU over the last {:.0} s",
                    peak(id),
                    window.last().map_or(0.0, |l| l.0 - window[0].0)
                )],
            );
        }
        if self.rows.is_empty() {
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
        self.rows.len()
    }
}
