//! The Switchboard view: hardware interrupts as a cord telephone exchange, for Linux, where
//! `/proc/interrupts` is readable without privilege. Every IRQ line is a jack with a lamp, every
//! CPU an operator, and each line's cord runs to the operator whose CPU serviced most of its
//! interrupts in the last interval. No process is drawn.
//!
//! The data is `Snapshot::interrupts` and `softirqs` (B4's per-CPU rates and effective
//! affinities), so a paused or rewound frame replays history. `lines` turns a sample into jacks
//! and cords and decides when a cord swings; `cord` is the catenary and the swing; `lamp` is the
//! brightness and the flicker; `scenery` and `figures` are the room and the people in it.

mod cord;
mod figures;
mod lamp;
mod lines;
mod scenery;

use std::collections::BTreeMap;

use crate::model::{CoreKind, Process, Snapshot};
use crate::palette::{mix, tint};
use crate::platform::Source;
use crate::render::{Card, Color, NONE, Stage, Thing};
use crate::simulation::FixedStep;
use crate::status::{missing_note, sources_note};
use crate::terminal::clean_text;
use crate::views::ViewModule;

use cord::{MOVE, Spot};
use lines::{CAP, Line, Lines, Reading, Table, sum_known};
use scenery::{Pen, Plan};

/// The kind of thing a jack, its lamp or its cord is, for picking and the inspector.
pub const LINE: &str = "interrupt line";

/// Text on the board is cut to this many characters.
const LABEL: usize = 14;
/// A cord is drawn this many times wider than the formula says, which is in the prototype's
/// pixels and reads thin beside the lamps.
const CORD_WEIGHT: f32 = 1.6;
const BRASS: Color = [186, 146, 76];
const LENS_OFF: Color = [92, 58, 30];
const LENS_ON: Color = [255, 214, 140];
const AMBER: Color = [255, 178, 82];
const SOFT_OFF: Color = [90, 56, 30];
const SOFT_UNKNOWN: Color = [44, 28, 18];
/// The softirqs in the kernel's order, for a board whose table is not read.
const SOFTIRQS: [&str; 10] = [
    "HI", "TIMER", "NET_TX", "NET_RX", "BLOCK", "IRQ_POLL", "TASKLET", "SCHED", "HRTIMER", "RCU",
];
/// Interrupts per second at which an inter-office lamp is fully lit.
const CALLS_FULL: f32 = 1000.0;

/// Whether the view runs here: only Linux has the unprivileged per-CPU counters. macOS has no
/// such table, and the view is skipped there rather than shown empty.
pub fn available() -> bool {
    cfg!(target_os = "linux")
}

/// Steps the operators' motion at a fixed rate whatever the frame rate.
struct Clock(FixedStep);

impl Default for Clock {
    fn default() -> Self {
        Clock(FixedStep::new(1.0 / 60.0, 8, 0.5))
    }
}

/// A cord already hung between two points, until either moves.
struct Hung {
    start: Spot,
    end: Spot,
    air: bool,
    points: Vec<Spot>,
}

#[derive(Default)]
pub struct Switchboard {
    lines: Lines,
    clock: Clock,
    /// Each operator's sway and reach phases, which advance with her CPU's busy share.
    phases: Vec<(f32, f32)>,
    hung: BTreeMap<u64, Hung>,
    /// The scene frame number the last draw was, to tell a gap (another view was shown).
    drawn: Option<u64>,
    now: f32,
    fps: u32,
    header: String,
}

/// `value` with thousands separated by commas.
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

fn rate_text(rate: Option<f32>) -> String {
    rate.map_or_else(|| "?".to_owned(), |rate| grouped(rate.round() as u64))
}

fn letter(kind: CoreKind) -> &'static str {
    match kind {
        CoreKind::Performance => " P",
        CoreKind::Efficiency => " E",
        CoreKind::Unknown => "",
    }
}

impl Switchboard {
    fn plan(&self) -> Plan {
        let (columns, rows) = self.lines.grid();
        Plan::new(columns, rows, self.lines.roster.seats.len())
    }

    fn seat_color(&self, seat: usize) -> Color {
        let seats = &self.lines.roster.seats;
        seats
            .get(seat.min(seats.len().saturating_sub(1)))
            .map_or(cord::operator_color(CoreKind::Unknown, 0), |seat| {
                cord::operator_color(seat.kind, seat.ordinal)
            })
    }

    /// Takes in the sample if it is new, and moves the operators.
    fn advance(&mut self, stage: &Stage, snapshot: &Snapshot) {
        let gap = self.drawn.is_none_or(|frame| stage.frames != frame + 1);
        self.drawn = Some(stage.frames);
        if self.lines.last != Some(snapshot.elapsed) {
            self.lines.take_in(snapshot, stage.time, gap);
        }
        self.now = stage.time;
        self.fps = stage.fps;
        let busy: Vec<f32> = self
            .lines
            .roster
            .seats
            .iter()
            .map(|seat| seat.busy.unwrap_or(0.0))
            .collect();
        while self.phases.len() < busy.len() {
            let seat = self.phases.len() as f32;
            self.phases.push((seat, seat * 1.7));
        }
        let phases = &mut self.phases;
        self.clock.0.advance(stage.time, |step| {
            for (phase, busy) in phases.iter_mut().zip(&busy) {
                phase.0 += (1.3 + 3.0 * busy) * step;
                phase.1 += (2.0 + 4.0 * busy) * step;
            }
        });
    }

    /// The hung cord for jack `key`, from `start` to `end`, long enough to sag as much as its
    /// slack allows without hanging below the end.
    fn hang(&mut self, key: u64, start: Spot, end: Spot, air: bool) -> Vec<Spot> {
        if let Some(hung) = self.hung.get(&key)
            && hung.start == start
            && hung.end == end
            && hung.air == air
        {
            return hung.points.clone();
        }
        let chord = cord::polyline_length(&[start, end]);
        let mut slack = cord::slack(key) - 1.0 + if air { 0.04 } else { 0.0 };
        let floor = start[2].min(end[2]) - 3.0;
        let mut points = cord::catenary(start, end, chord * (1.0 + slack), 24);
        for _ in 0..12 {
            let lowest = points.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
            if lowest >= floor || slack < 0.0015 {
                break;
            }
            slack *= 0.6;
            points = cord::catenary(start, end, chord * (1.0 + slack), 24);
        }
        self.hung.insert(
            key,
            Hung {
                start,
                end,
                air,
                points: points.clone(),
            },
        );
        points
    }

    fn glow_radius(pen: &Pen, design: f32) -> f32 {
        pen.glow_radius(design)
    }

    /// The jacks with their lamps, sockets and group dividers, and the cords that run from them.
    fn draw_lines(&mut self, pen: &mut Pen, plan: &Plan, chosen: Option<u64>, dark: bool) {
        let time = f64::from(self.now);
        let fps = self.fps;
        let capacity = plan.columns * plan.rows;
        let jacks = self.lines.jacks.clone();
        let step = plan.column_step();
        for place in 0..capacity {
            let (x, y) = plan.jack(place);
            let key = self.lines.pegs.holder(place).filter(|_| !dark);
            let jack = key.and_then(|key| jacks.iter().find(|jack| jack.key == key));
            let pick = jack.map_or(NONE, |jack| {
                pen.frame.thing_pick(Thing {
                    kind: LINE,
                    key: jack.key,
                })
            });
            let z = scenery::TOP_Z - y;
            pen.disc([x, y, 0.6], 7.2, 8, [58, 34, 18], pick);
            if chosen.is_some() && chosen == key {
                pen.disc([x, y, 0.7], 9.4, 10, [250, 214, 150], pick);
                pen.disc([x, y, 0.8], 7.6, 10, [58, 34, 18], pick);
            }
            let brightness = jack.map_or(0.0, |jack| {
                let rate = jack.rate().unwrap_or(0.0);
                let moving = self
                    .lines
                    .cords
                    .get(&jack.key)
                    .and_then(|cord| cord.swing)
                    .is_some_and(|swing| (0.0..MOVE).contains(&(self.now - swing.started)));
                if moving {
                    lamp::fluttering(rate, time)
                } else {
                    lamp::jack(rate, jack.key, fps, time)
                }
            });
            let lens = mix(LENS_OFF, LENS_ON, brightness);
            pen.ball([x, 1.6, z], 5.4, lens, pick);
            if brightness > 0.02 {
                pen.glow(
                    [x, 3.0, z],
                    Self::glow_radius(pen, 11.0),
                    AMBER,
                    0.75 * brightness.powf(1.3),
                );
            }
            pen.disc([x, y + 24.0, 0.6], 6.6, 8, [176, 136, 70], pick);
            pen.disc([x, y + 24.0, 0.8], 3.8, 6, [6, 4, 3], pick);
            // A divider between this jack and the next when they are of different devices.
            let column = place % plan.columns;
            if column + 1 < plan.columns
                && let (Some(a), Some(b)) = (
                    self.lines.pegs.holder(place),
                    self.lines.pegs.holder(place + 1),
                )
                && !dark
                && device_of(&jacks, a) != device_of(&jacks, b)
            {
                let at = x + step / 2.0;
                pen.face(
                    (at - 0.8, at + 0.8),
                    (y - 36.0, y + 36.0),
                    0.7,
                    [150, 112, 60],
                );
            }
        }
        let outline = jacks.len() <= 64;
        for jack in &jacks {
            if !dark {
                self.draw_cord(pen, plan, jack, chosen == Some(jack.key), outline);
            }
        }
    }

    fn draw_cord(&mut self, pen: &mut Pen, plan: &Plan, jack: &Line, chosen: bool, outline: bool) {
        let Some(place) = self.lines.pegs.of(jack.key) else {
            return;
        };
        let Some(cord) = self.lines.cords.get(&jack.key) else {
            return;
        };
        let Some((seat, slot)) = cord.plugged else {
            return;
        };
        let seats = plan.seats.saturating_sub(1);
        let target = plan.hole(seat.min(seats), slot);
        let swing = cord
            .swing
            .filter(|swing| (0.0..MOVE).contains(&(self.now - swing.started)));
        let (end, lifted, mut color) = match swing {
            Some(swing) => {
                let from = plan.hole(swing.from.0.min(seats), swing.from.1);
                let plug = cord::plug_at(from, target, self.now - swing.started);
                let color = mix(
                    self.seat_color(swing.from.0),
                    self.seat_color(seat),
                    plug.blend,
                );
                (plug.at, plug.lifted, color)
            }
            None => (target, 0.0, self.seat_color(seat)),
        };
        if chosen {
            color = mix(color, [255, 255, 255], 0.35);
        }
        let (x, y) = plan.jack(place);
        let start: Spot = [x, 4.0, scenery::TOP_Z - (y + 24.0)];
        let points = self.hang(jack.key, start, end, lifted > 0.0);
        let pick = pen.frame.thing_pick(Thing {
            kind: LINE,
            key: jack.key,
        });
        let width = cord::width(jack.rate().unwrap_or(0.0)) * CORD_WEIGHT;
        let behind: Vec<Spot> = points.iter().map(|p| [p[0], p[1] - 0.5, p[2]]).collect();
        // The dark edge that sets a cord off from those it crosses costs a second ribbon, so only
        // a board of at most 64 cords has it: all cords together stay within 128 x 24 x 2
        // triangles.
        if outline {
            pen.stroke(&behind, width + 1.4, tint(color, 0.35), pick);
        }
        pen.stroke(&points, width, color, pick);
        pen.ball([x, 4.0, start[2]], 4.2, BRASS, pick);
        if lifted > 0.0 {
            let tip: Spot = [end[0], end[1], end[2]];
            pen.stroke(
                &[tip, [tip[0], tip[1], tip[2] - lifted.min(9.0)]],
                3.0,
                BRASS,
                NONE,
            );
            pen.ball(tip, 3.6, [230, 196, 120], pick);
            pen.glow(tip, Self::glow_radius(pen, 10.0), [255, 220, 160], 0.6);
        } else {
            pen.flat(
                [end[0], end[1], scenery::TOP_Z - end[2] - 0.15],
                (3.6, 2.2),
                8,
                BRASS,
            );
        }
    }

    /// The operators, their telephones, clocks and lamps.
    fn draw_operators(&mut self, pen: &mut Pen, plan: &Plan, reading: Option<&Reading>) {
        let time = self.now;
        for (index, seat) in self.lines.roster.seats.iter().enumerate() {
            let busy = seat.busy.unwrap_or(0.0);
            let (sway, reach) = self.phases.get(index).copied().unwrap_or((0.0, 0.0));
            figures::operator(pen, plan, index, &figures::Motion { busy, sway, reach });
            let sum = |rates: &BTreeMap<u32, Option<f32>>| {
                sum_known(
                    seat.cpus
                        .iter()
                        .filter_map(|cpu| rates.get(cpu).copied().flatten()),
                )
            };
            let rattles = reading
                .and_then(|reading| sum(&reading.named.rescheduling))
                .is_some_and(|rate| rate >= 1.0);
            let plugs = self
                .lines
                .cords
                .values()
                .filter(|cord| cord.plugged.is_some_and(|(held, _)| held == index))
                .count();
            figures::holes(pen, plan, index, plugs.min(32));
            figures::telephone(pen, plan, index, rattles.then_some(time));
            let revolutions = reading.and_then(|reading| {
                let counts: Vec<&(u64, Option<f32>)> = seat
                    .cpus
                    .iter()
                    .filter_map(|cpu| reading.named.timer.get(cpu))
                    .collect();
                if counts.is_empty() {
                    return None;
                }
                let count: u64 = counts.iter().map(|c| c.0).sum();
                let rate: f32 = counts.iter().filter_map(|c| c.1).sum();
                let since = f64::from((time - self.lines.taken).clamp(0.0, 3.0));
                Some(((count % 6000) as f64 + f64::from(rate) * since) / 6000.0)
            });
            let calls = reading
                .and_then(|reading| sum(&reading.named.calls))
                .map_or(0.0, |rate| lamp::brightness(rate, CALLS_FULL));
            let nmi = self
                .lines
                .nmi
                .get(&index)
                .is_some_and(|&at| (0.0..lines::NMI_LIT).contains(&(time - at)));
            figures::rail(
                pen,
                plan,
                index,
                &figures::Rail {
                    revolutions,
                    calls,
                    nmi,
                },
            );
        }
    }

    /// The softirq board: a lamp per softirq per operator.
    fn draw_softirqs(&self, pen: &mut Pen, plan: &Plan, reading: Option<&Reading>) {
        let (left, _) = scenery::SOFTIRQ_BOARD;
        let time = f64::from(self.now);
        let rows = reading.map_or(SOFTIRQS.len(), |reading| {
            if reading.softirq_table == Table::Known {
                reading.softirqs.len()
            } else {
                SOFTIRQS.len()
            }
        });
        let half = (plan.lamp_pitch * 0.36).clamp(1.0, 3.7);
        for row in 0..rows {
            let y = 134.0 + row as f32 * 40.5;
            for (index, seat) in self.lines.roster.seats.iter().enumerate() {
                let x = left + 84.0 + index as f32 * plan.lamp_pitch;
                let rate = reading
                    .filter(|reading| reading.softirq_table == Table::Known)
                    .and_then(|reading| reading.softirqs.get(row))
                    .and_then(|(_, rates)| lines::seat_rate(rates, &seat.cpus));
                let seed = (row * 101 + index) as u64;
                let lit = rate.map_or(0.0, |rate| lamp::softirq(rate, seed, self.fps, time));
                // A lamp whose rate is not known is darker than one that is known to be quiet.
                let color = rate.map_or(SOFT_UNKNOWN, |_| mix(SOFT_OFF, LENS_ON, lit));
                // A square bezel and a hexagonal lens: 160 lamps stay within 1,300 triangles.
                pen.face(
                    (x - half - 0.9, x + half + 0.9),
                    (y - half - 0.9, y + half + 0.9),
                    0.6,
                    [58, 34, 18],
                );
                if half >= 2.5 {
                    pen.disc([x, y, 0.8], half, 6, color, NONE);
                } else {
                    pen.face((x - half, x + half), (y - half, y + half), 0.8, color);
                }
                if lit > 0.02 {
                    pen.glow(
                        [x, 3.0, scenery::TOP_Z - y],
                        Self::glow_radius(pen, 6.0),
                        AMBER,
                        0.5 * lit.powf(1.3),
                    );
                }
            }
        }
    }

    /// The labels of the board: titles, one per device group, the softirq names and the
    /// operators' plates. All device text is cut and cleaned of anything a terminal would obey.
    fn label(&self, stage: &mut Stage, plan: &Plan, reading: Option<&Reading>) {
        let mut put = |x: f32, d: f32, z: f32, text: &str| {
            stage
                .places
                .push((scenery::point(x, d, z), clean_text(text, LABEL + 12)));
        };
        put(
            (scenery::BOARD.0 + scenery::BOARD.1) / 2.0,
            1.0,
            scenery::TOP_Z - (scenery::CORNICE + 8.0),
            "IRQ LINES  /proc/interrupts",
        );
        put(
            (scenery::SOFTIRQ_BOARD.0 + scenery::SOFTIRQ_BOARD.1) / 2.0,
            1.0,
            scenery::TOP_Z - 90.0,
            "SOFTIRQ  /proc/softirqs",
        );
        for (x, y, text) in self.group_labels(plan) {
            put(x, 1.0, scenery::TOP_Z - (y - 27.0), &text);
        }
        let names: Vec<&str> = match reading.filter(|r| r.softirq_table == Table::Known) {
            Some(reading) => reading
                .softirqs
                .iter()
                .map(|(name, _)| name.as_str())
                .collect(),
            None => SOFTIRQS.to_vec(),
        };
        for (row, name) in names.iter().enumerate() {
            put(
                scenery::SOFTIRQ_BOARD.0 + 42.0,
                1.0,
                scenery::TOP_Z - (134.0 + row as f32 * 40.5),
                name,
            );
        }
        let every = if plan.seats > 16 {
            plan.seats.div_ceil(16)
        } else {
            1
        };
        for (index, seat) in self.lines.roster.seats.iter().enumerate() {
            let text = match (seat.cpus.first(), seat.cpus.last()) {
                (Some(first), Some(last)) if first != last => {
                    format!("cpu{first}-{last}{}", letter(seat.kind))
                }
                (Some(first), _) => format!("cpu{first}{}", letter(seat.kind)),
                _ => continue,
            };
            let at = figures::plate(plan, index);
            if index % every == 0 {
                put(at[0], at[1], at[2], &text);
            }
        }
    }

    /// One label for each run of neighbouring jacks of a device: its x, the y of the row's lamps
    /// and its text.
    fn group_labels(&self, plan: &Plan) -> Vec<(f32, f32, String)> {
        let mut labels = Vec::new();
        for row in 0..plan.rows {
            let mut run: Vec<(usize, &Line)> = Vec::new();
            for column in 0..=plan.columns {
                let place = row * plan.columns + column;
                let jack = (column < plan.columns)
                    .then(|| self.lines.pegs.holder(place))
                    .flatten()
                    .and_then(|key| self.lines.jacks.iter().find(|jack| jack.key == key));
                let continues = jack
                    .zip(run.first())
                    .is_some_and(|(jack, (_, first))| jack.device == first.device);
                if !continues && let Some(&(first, head)) = run.first() {
                    let (last, _) = run[run.len() - 1];
                    let (x0, y) = plan.jack(first);
                    let (x1, _) = plan.jack(last);
                    let text = if run.len() == 1 {
                        &head.name
                    } else {
                        &head.device
                    };
                    labels.push(((x0 + x1) / 2.0, y, text.clone()));
                    run.clear();
                }
                if let Some(jack) = jack {
                    run.push((place, jack));
                }
            }
        }
        labels
    }

    fn describe(&self, reading: &Reading) -> String {
        match reading.table {
            Table::Pending => " reading the interrupt table".to_owned(),
            Table::Unreadable => {
                " the interrupt table cannot be read: the board stays dark".to_owned()
            }
            Table::Known => {
                let lines = reading.lines.len();
                let jacks = self.lines.jacks.len();
                let shown = if jacks == lines {
                    format!("{lines} lines")
                } else {
                    format!("{lines} lines on {jacks} jacks")
                };
                let reschedules = reading.rescheduling_rate().map_or(String::new(), |rate| {
                    format!(" | RES {}/s", rate_text(Some(rate)))
                });
                let seats = &self.lines.roster;
                let kinds: Vec<String> = [
                    (CoreKind::Performance, "P"),
                    (CoreKind::Efficiency, "E"),
                    (CoreKind::Unknown, "unknown"),
                ]
                .iter()
                .filter_map(|&(kind, name)| {
                    let count = seats.count(kind);
                    (count > 0).then(|| format!("{count} {name}"))
                })
                .collect();
                let operators = seats.seats.len();
                format!(
                    " {shown} | {} irq/s{reschedules} | {operators} operators: {}",
                    rate_text(reading.total_rate()),
                    kinds.join(", ")
                )
            }
        }
    }

    fn move_text(&self, key: u64) -> String {
        match self.lines.cords.get(&key).and_then(|cord| cord.moved) {
            Some(moved) => {
                let cpu = |seat: usize| {
                    self.lines
                        .roster
                        .seats
                        .get(seat)
                        .and_then(|seat| seat.cpus.first())
                        .map_or_else(|| "?".to_owned(), ToString::to_string)
                };
                format!(
                    "last move cpu{} -> cpu{}, {:.1} s ago",
                    cpu(moved.from),
                    cpu(moved.to),
                    (self.now - moved.at).max(0.0)
                )
            }
            None => "no move seen".to_owned(),
        }
    }
}

fn device_of(jacks: &[Line], key: u64) -> Option<&str> {
    jacks
        .iter()
        .find(|jack| jack.key == key)
        .map(|jack| jack.device.as_str())
}

impl ViewModule for Switchboard {
    fn draw(&mut self, stage: &mut Stage, _processes: &[&Process], snapshot: &Snapshot) -> usize {
        self.advance(stage, snapshot);
        let plan = self.plan();
        let reading = self.lines.reading.clone();
        let dark = reading
            .as_ref()
            .is_none_or(|reading| reading.table != Table::Known);
        let chosen = stage
            .thing
            .filter(|thing| thing.kind == LINE)
            .map(|thing| thing.key);
        stage.bounds.extend(plan.bounds());
        {
            let mut pen = Pen {
                frame: &mut *stage.frame,
                camera: stage.camera,
            };
            scenery::furniture(&mut pen, &plan);
            self.draw_lines(&mut pen, &plan, chosen, dark);
            self.draw_operators(&mut pen, &plan, reading.as_ref());
            self.draw_softirqs(&mut pen, &plan, reading.as_ref());
        }
        self.label(stage, &plan, reading.as_ref());
        self.header = reading.as_ref().map_or_else(
            || " reading the interrupt table".to_owned(),
            |r| self.describe(r),
        );
        0
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let reading = self.lines.reading.as_ref();
        let folded = reading.map_or(0, |reading| {
            reading.lines.len().saturating_sub(self.lines.jacks.len())
        });
        let mut notes = String::new();
        for note in [
            missing_note(snapshot, Source::Interrupts, "the board is dark"),
            missing_note(snapshot, Source::Softirqs, "the softirq board is dark"),
            missing_note(
                snapshot,
                Source::IrqAffinity,
                "only a change of servicing CPU swings a cord",
            ),
        ]
        .into_iter()
        .flatten()
        {
            notes.push_str(&note);
        }
        if reading.is_some_and(|reading| reading.table == Table::Known && !reading.has_timer()) {
            notes.push_str(" | no timer row: the clocks stop");
        }
        if self.lines.roster.per_seat > 1 {
            notes.push_str(&format!(
                " | each operator covers {} CPUs",
                self.lines.roster.per_seat
            ));
        }
        if folded > 0 {
            notes.push_str(&format!(
                " | {folded} lines folded into their devices' jacks (cap {CAP})"
            ));
        }
        if let Some(sources) = sources_note(snapshot) {
            notes.push_str(&sources);
        }
        format!(
            " jack = IRQ line | lamp = rate (log), flicker capped at {} Hz | cord = CPU that serviced most of the line this interval, colour by CPU, thickness by rate | operator = CPU, motion by busy | phone rings = RES | clock = LOC | green lamp = CAL/TLB | red lamp = NMI | right: softirqs per CPU | IPI senders unknown, no operator links{notes}",
            (0.9 * self.fps as f32).floor().max(1.0)
        )
    }

    fn inspector_line(&self) -> Option<String> {
        Some(self.header.clone())
    }

    fn card(&self, thing: Thing, _snapshot: &Snapshot) -> Option<Card> {
        if thing.kind != LINE {
            return None;
        }
        let jack = self.lines.jacks.iter().find(|jack| jack.key == thing.key)?;
        let plan = self.plan();
        let place = self.lines.pegs.of(jack.key)?;
        let (x, y) = plan.jack(place);
        let seats = &self.lines.roster;
        let name = |cpu: u32| {
            let kind = seats
                .seat_of(cpu)
                .and_then(|seat| seats.seats.get(seat))
                .map_or(CoreKind::Unknown, |seat| seat.kind);
            format!("cpu{cpu}{}", letter(kind))
        };
        let mut lines = Vec::new();
        if jack.members > 1 {
            lines.push(format!(
                "{} lines of {}",
                jack.members,
                clean_text(&jack.device, 40)
            ));
        } else {
            lines.push(format!(
                "IRQ {}  {}",
                clean_text(&jack.id, 12),
                clean_text(&jack.chip, 40)
            ));
        }
        let rate = jack.rate();
        lines.push(match rate {
            Some(rate) => format!(
                "{} irq/s  lamp flicker capped at {} Hz",
                rate_text(Some(rate)),
                lamp::flicker_hz(rate, self.fps.max(1)).floor()
            ),
            None => "rate not measured yet".to_owned(),
        });
        lines.push(match jack.servicing() {
            Some(cpu) => format!("serviced by {}", name(cpu)),
            None => "not serviced in the last interval".to_owned(),
        });
        lines.push(match &jack.affinity {
            Some(cpus) if !cpus.is_empty() => format!(
                "effective affinity {}",
                cpus.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            _ => "effective affinity not read".to_owned(),
        });
        lines.push(self.move_text(jack.key));
        let shares: Vec<String> = jack
            .shares(3)
            .into_iter()
            .map(|(cpu, share)| format!("{} {:.0}%", name(cpu), share * 100.0))
            .collect();
        if !shares.is_empty() {
            lines.push(format!("top CPUs {}", shares.join("  ")));
        }
        Some(Card {
            title: clean_text(&jack.name, 40),
            lines,
            at: scenery::point(x, 2.0, scenery::TOP_Z - y),
        })
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::lines::tests::{cpus, row};
    use super::*;
    use crate::model::{DemoOptions, Interrupt, Measured, demo_with};
    use crate::platform::Sources;
    use crate::render::{Camera, Frame, RenderInput, Scene, Target};
    use crate::views::view;

    fn exchange(time: f64) -> Snapshot {
        demo_with(
            time,
            16,
            DemoOptions {
                exchange: true,
                ..DemoOptions::NONE
            },
        )
    }

    fn draw(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> Frame {
        let mut camera = Camera::default();
        let mut input = RenderInput::small(snapshot, view("switchboard"), &camera, time);
        (input.width, input.height) = (640, 360);
        scene.render(&input);
        scene.fit(&mut camera, 640, 360);
        let mut input = RenderInput::small(snapshot, view("switchboard"), &camera, time);
        (input.width, input.height) = (640, 360);
        let mut frame = scene.render(&input);
        frame.rasterize();
        frame
    }

    /// A scene that has watched the demo from 9 s to 12 s, a sample a second.
    fn watched() -> (Scene, Snapshot, Frame) {
        let mut scene = Scene::new();
        let mut last = None;
        for second in 9..=12 {
            let snapshot = exchange(f64::from(second));
            let frame = draw(&mut scene, &snapshot, second as f32);
            last = Some((snapshot, frame));
        }
        let (snapshot, frame) = last.expect("drew four samples");
        (scene, snapshot, frame)
    }

    fn board(scene: &Scene) -> &Switchboard {
        scene
            .module::<Switchboard>(view("switchboard"))
            .expect("the switchboard has drawn")
    }

    #[test]
    fn switchboard_renders_the_same_frame_twice_from_the_same_inputs() {
        let draws = || {
            let mut scene = Scene::new();
            let mut frame = None;
            for second in 9..=12 {
                frame = Some(draw(
                    &mut scene,
                    &exchange(f64::from(second)),
                    second as f32 + 0.3,
                ));
            }
            frame.expect("a frame")
        };
        let (first, second) = (draws(), draws());
        assert_eq!(first.pixels, second.pixels);
        assert_eq!(first.picks, second.picks);
        assert!(
            first
                .pixels
                .windows(3)
                .any(|pixel| *pixel != first.pixels[..3]),
            "the board is not a flat frame"
        );
    }

    #[test]
    fn the_demo_shows_a_cord_in_the_air_a_second_after_it_was_moved() {
        let (scene, _, _) = watched();
        let lines = &board(&scene).lines;
        let cord = &lines.cords[&38];
        let swing = cord.swing.expect("eth0-rx-2 is mid-swing at 12 s");
        assert_eq!(swing.started, 11.0, "the sample at 11 s saw the move");
        let moved = cord.moved.expect("a move was seen");
        assert_eq!((moved.from, moved.to), (3, 9));
        assert!((0.0..cord::MOVE).contains(&(12.0 - swing.started)));
        let others = lines
            .cords
            .iter()
            .filter(|(key, cord)| **key != 38 && cord.swing.is_some());
        assert_eq!(others.count(), 0, "no other line moved");
        assert!(
            lines
                .nmi
                .get(&0)
                .is_some_and(|&at| 12.0 - at < lines::NMI_LIT),
            "the red lamp is lit"
        );
    }

    #[test]
    fn the_header_counts_lines_rates_and_operators_as_the_issue_words_it() {
        let (scene, _, _) = watched();
        let header = board(&scene)
            .inspector_line()
            .expect("the view has its own header");
        assert!(header.starts_with(" 48 lines | "), "{header}");
        assert!(header.contains(" irq/s | RES "), "{header}");
        assert!(header.ends_with(" | 16 operators: 8 P, 8 E"), "{header}");
        let total: u64 = header
            .split(" | ")
            .nth(1)
            .and_then(|part| part.strip_suffix(" irq/s"))
            .and_then(|text| text.replace(',', "").parse().ok())
            .expect("a total");
        assert!((30_000..55_000).contains(&total), "{total} irq/s");
    }

    #[test]
    fn the_legend_names_every_mapping_and_where_the_data_is_missing() {
        let (scene, snapshot, _) = watched();
        let legend = scene.legend(view("switchboard"), &snapshot);
        for words in [
            "jack = IRQ line",
            "lamp = rate (log)",
            "cord = CPU that serviced most of the line this interval",
            "operator = CPU",
            "phone rings = RES",
            "clock = LOC",
            "green lamp = CAL/TLB",
            "red lamp = NMI",
            "softirqs per CPU",
            "IPI senders unknown",
        ] {
            assert!(legend.contains(words), "{words}: {legend}");
        }
        assert!(legend.starts_with(' ') && !legend.contains(" | no "));
        let mut missing = snapshot.clone();
        missing.missing = Sources::of(&[Source::IrqAffinity, Source::Softirqs]);
        let legend = scene.legend(view("switchboard"), &missing);
        assert!(
            legend.contains(" | no irq affinity: only a change of servicing CPU swings a cord"),
            "{legend}"
        );
        assert!(legend.contains("the softirq board is dark"), "{legend}");
    }

    #[test]
    fn jacks_lamps_and_cords_pick_the_line_and_the_card_tells_its_story() {
        let (scene, snapshot, frame) = watched();
        let keys: std::collections::BTreeSet<u64> = frame
            .picks
            .iter()
            .filter_map(|&pick| match frame.target(pick) {
                Some(Target::Thing(thing)) if thing.kind == LINE => Some(thing.key),
                _ => None,
            })
            .collect();
        assert!(
            keys.len() >= 40,
            "{} of 48 lines can be clicked",
            keys.len()
        );
        assert!(keys.contains(&38));
        let card = scene
            .card(
                view("switchboard"),
                Thing {
                    kind: LINE,
                    key: 38,
                },
                &snapshot,
            )
            .expect("a card for eth0-rx-2");
        assert_eq!(card.title, "eth0-rx-2");
        let text = card.lines.join("\n");
        for part in [
            "IRQ 38  IR-PCI-MSIX-0000:03:00.0",
            " irq/s  lamp flicker capped at 9 Hz",
            "serviced by cpu9 E",
            "effective affinity 9",
            "last move cpu3 -> cpu9, 1.0 s ago",
            "top CPUs cpu9 E 100%",
        ] {
            assert!(text.contains(part), "{part} in\n{text}");
        }
        assert!(
            scene
                .card(
                    view("switchboard"),
                    Thing {
                        kind: "grave",
                        key: 38
                    },
                    &snapshot
                )
                .is_none()
                && scene
                    .card(
                        view("switchboard"),
                        Thing {
                            kind: LINE,
                            key: 9999
                        },
                        &snapshot
                    )
                    .is_none()
        );
    }

    #[test]
    fn the_board_stays_within_its_triangle_budget_and_its_glows_within_twelve_pixels() {
        let (_, _, frame) = watched();
        let triangles = frame
            .items
            .iter()
            .filter(|item| matches!(item, crate::render::Item::Triangle(..)))
            .count();
        // The room and its people about 5,300, the jacks 1,000 and 48 cords with their edges 4,600.
        assert!(triangles < 11_500, "{triangles} triangles");
        for item in &frame.items {
            if let crate::render::Item::Puff {
                radius, strength, ..
            } = item
            {
                assert!(
                    *radius <= 12.0 * 360.0 / 1080.0 + 1e-3 || *radius <= 2.5 + 1e-3,
                    "{radius}"
                );
                assert!((0.0..=1.0).contains(strength));
            }
        }
        assert!(
            frame.items.iter().all(|item| !matches!(
                item,
                crate::render::Item::Line(..)
                    | crate::render::Item::Beam(..)
                    | crate::render::Item::Glow { .. }
            )),
            "only primitives that rasterize alike on the CPU and the GPU"
        );
    }

    #[test]
    fn an_unreadable_interrupt_table_leaves_the_board_dark_and_says_so() {
        let snapshot = Snapshot {
            interrupts: Measured::Unreadable,
            softirqs: Measured::Unreadable,
            cpus: cpus(4),
            missing: Sources::of(&[Source::Interrupts, Source::Softirqs]),
            ..Snapshot::default()
        };
        let mut scene = Scene::new();
        let frame = draw(&mut scene, &snapshot, 5.0);
        assert!(
            frame.things.is_empty(),
            "no jack can be picked on a dark board"
        );
        let board = board(&scene);
        assert!(board.lines.cords.is_empty() && board.lines.jacks.is_empty());
        let header = board.inspector_line().expect("a header");
        assert!(
            header.contains("cannot be read") && header.contains("dark"),
            "{header}"
        );
        let legend = scene.legend(view("switchboard"), &snapshot);
        assert!(
            legend.contains(" | no interrupts: the board is dark"),
            "{legend}"
        );
        assert!(
            legend.contains(" | no softirqs: the softirq board is dark"),
            "{legend}"
        );
        let lit = frame
            .items
            .iter()
            .filter(|item| matches!(item, crate::render::Item::Puff { .. }))
            .count();
        assert_eq!(lit, 0, "nothing glows on a dark board");
    }

    #[test]
    fn interrupt_device_names_cannot_inject_terminal_controls() {
        let hostile = "eth0\x1b[2J\x07\u{202e}rx-1\n";
        let mut rows = vec![
            row(30, hostile, 1, 400.0, Some(1)),
            row(31, "plain", 2, 90.0, None),
        ];
        rows[0].chip = "chip\x1b]0;owned\x07".to_owned();
        rows[0].id = "30".to_owned();
        let snapshot = Snapshot {
            interrupts: Measured::Known(rows),
            cpus: cpus(4),
            elapsed: 1.0,
            ..Snapshot::default()
        };
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 1.0);
        assert!(!scene.places.is_empty());
        for (_, text) in &scene.places {
            assert!(
                text.chars().all(|c| c.is_ascii_graphic() || c == ' '),
                "{text:?}"
            );
        }
        let card = scene
            .card(
                view("switchboard"),
                Thing {
                    kind: LINE,
                    key: 30,
                },
                &snapshot,
            )
            .expect("a card");
        for text in std::iter::once(&card.title).chain(&card.lines) {
            assert!(
                text.chars().all(|c| c.is_ascii_graphic() || c == ' '),
                "{text:?}"
            );
        }
        assert!(scene.legend(view("switchboard"), &snapshot).is_ascii());
    }

    #[test]
    fn the_view_is_available_where_the_interrupt_table_is_and_only_there() {
        assert_eq!(available(), cfg!(target_os = "linux"));
        let row = &crate::views::REGISTRY[view("switchboard").index()];
        assert_eq!((row.available)(), available());
        assert!(!row.tour, "there are no processes to visit");
        for source in [Source::Interrupts, Source::Softirqs, Source::IrqAffinity] {
            assert!(row.sources.contains(source), "{}", source.name());
        }
    }

    #[test]
    fn a_rewound_sample_is_drawn_as_it_was_without_a_swing() {
        let (mut scene, _, _) = watched();
        let before = exchange(10.0);
        draw(&mut scene, &before, 12.5);
        let lines = &board(&scene).lines;
        assert!(lines.cords.values().all(|cord| cord.swing.is_none()));
        assert_eq!(
            lines.cords[&38].plugged.map(|p| p.0),
            Some(3),
            "at 10 s the line was on cpu3"
        );
    }

    #[test]
    fn a_table_without_a_timer_row_stops_the_clocks_and_says_so() {
        let snapshot = Snapshot {
            interrupts: Measured::Known(vec![row(30, "ahci", 1, 5.0, None)]),
            cpus: cpus(4),
            ..Snapshot::default()
        };
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 1.0);
        let legend = scene.legend(view("switchboard"), &snapshot);
        assert!(
            legend.contains(" | no timer row: the clocks stop"),
            "{legend}"
        );
        let _unused: Option<Interrupt> = None;
    }
}
