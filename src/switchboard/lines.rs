//! What the switchboard makes of the interrupt table: the lines (jacks), the operators (CPUs) and
//! the cord between each line and the operator that serviced it, as a state machine fed one
//! sample at a time. Everything here is a pure function of the samples and the times they were
//! taken in, so a frame at a given time is the same every run.
//!
//! Rates are the per-CPU rates of `Snapshot::interrupts`, which the background thread works out
//! from the change of each count between two passes. A rate that is None (a first pass, a CPU
//! that was not there before, a count that went backwards) is unknown: it never wins a line's
//! servicing CPU and is never drawn as a rate of zero.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::kernel::timer_row;
use crate::model::{CoreKind, Cpu, CpuCount, Interrupt, Measured, Snapshot};
use crate::simulation::hash::fnv1a_64;

/// The most jacks the board shows.
pub const CAP: usize = 128;
/// A device with more vectors than this folds into one jack when there are too many to show.
pub const FOLD_ABOVE: usize = 8;
/// The most operators; above it each seat covers several CPUs.
pub const SEATS: usize = 64;
/// Set in the key of a jack that stands for a folded device, so it never equals an IRQ number.
const FOLDED: u64 = 1 << 62;
/// The columns a board has: `ceil(sqrt(3 n))` for the `n` jacks it starts with, clamped.
const COLUMNS: (usize, usize) = (12, 16);
/// The fewest rows a board has.
const MIN_ROWS: usize = 4;
/// Seconds the red lamp stays lit after a non-maskable interrupt.
pub const NMI_LIT: f32 = 2.0;
/// Samples a new servicing CPU must persist before the cord moves to it.
const PERSISTENCE: u8 = 2;

/// The queue suffix of an interrupt's device name taken away, so the vectors of one device group
/// together: `[-:_]?(q|rx-|tx-|TxRx-)?\d+$`. A name without a trailing number keeps its whole
/// self, as does one that is only digits.
pub fn device_key(name: &str) -> String {
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
    if stem.len() == name.len() || stem.is_empty() {
        return name.to_owned();
    }
    let stem = ["TxRx-", "rx-", "tx-", "q"]
        .iter()
        .find_map(|queue| stem.strip_suffix(queue))
        .unwrap_or(stem);
    let stem = stem.strip_suffix(['-', ':', '_']).unwrap_or(stem);
    if stem.is_empty() {
        name.to_owned()
    } else {
        stem.to_owned()
    }
}

/// How an interrupt table was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    /// Not read yet.
    Pending,
    /// Could not be read.
    Unreadable,
    Known,
}

/// A line's rate on each CPU, in the table's column order.
pub type Rates = Vec<(u32, Option<f32>)>;

/// The rate of the CPUs `cpus` together: the sum of those whose rate is known, and None while
/// none is, so that a softirq not measured yet is not drawn as one that did not run.
pub fn seat_rate(rates: &[(u32, Option<f32>)], cpus: &[u32]) -> Option<f32> {
    sum_known(
        rates
            .iter()
            .filter(|(cpu, _)| cpus.contains(cpu))
            .filter_map(|&(_, rate)| rate)
            .map(|rate| rate.max(0.0)),
    )
}

/// The sum of `rates`, None when there are none: a sum of nothing measured is unknown, not zero.
pub fn sum_known(rates: impl Iterator<Item = f32>) -> Option<f32> {
    rates.fold(None, |sum, rate| Some(sum.map_or(rate, |held| held + rate)))
}

/// One jack: an interrupt line, or a device whose many lines were folded into one.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// An IRQ number, or `FOLDED` and a hash of the device. Stable for the line's lifetime.
    pub key: u64,
    /// The row's label in the table (`27`); the first of a folded device's.
    pub id: String,
    pub name: String,
    pub devices: Vec<String>,
    /// What the line's group is called: its device key.
    pub device: String,
    pub chip: String,
    pub rates: Rates,
    /// The CPUs the line is routed to now (`effective_affinity_list`), when it was read.
    pub affinity: Option<Vec<u32>>,
    /// The lines this jack stands for: 1, or a folded device's count.
    pub members: usize,
    /// The lowest IRQ number of the line's device, which orders the groups.
    pub order: u64,
}

impl Line {
    /// `rate` as a lamp or a cord draws it: a line whose rate was not measured is drawn at rest,
    /// as a quiet one is.
    pub fn drawn_rate(&self) -> f32 {
        self.rate().unwrap_or(0.0)
    }

    /// Events per second over the interval, summed over the CPUs whose rate is known; None when
    /// none is.
    pub fn rate(&self) -> Option<f32> {
        sum_known(
            self.rates
                .iter()
                .filter_map(|&(_, rate)| rate)
                .map(|rate| rate.max(0.0)),
        )
    }

    /// The CPU that serviced most of the line's interrupts in the interval: the one with the
    /// largest rate, the lowest CPU on a tie. None when no CPU has a known rate above zero (a
    /// quiet line, or one not measured yet).
    pub fn servicing(&self) -> Option<u32> {
        let mut best: Option<(u32, f32)> = None;
        for &(cpu, rate) in &self.rates {
            let Some(rate) = rate.filter(|rate| *rate > 0.0) else {
                continue;
            };
            if best.is_none_or(|(held, top)| rate > top || (rate == top && cpu < held)) {
                best = Some((cpu, rate));
            }
        }
        best.map(|(cpu, _)| cpu)
    }

    /// The CPUs with the most of the line's events, as shares of its rate, largest first; at most
    /// `count`.
    pub fn shares(&self, count: usize) -> Vec<(u32, f32)> {
        let Some(total) = self.rate() else {
            return Vec::new();
        };
        let mut known: Vec<(u32, f32)> = self
            .rates
            .iter()
            .filter_map(|&(cpu, rate)| rate.filter(|rate| *rate > 0.0).map(|rate| (cpu, rate)))
            .collect();
        known.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        known
            .into_iter()
            .take(count)
            .map(|(cpu, rate)| (cpu, rate / total))
            .collect()
    }
}

/// The named rows each operator shows: per CPU, the count and rate of its rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Named {
    /// The timer's count on each CPU, and the rate of the interval.
    pub timer: BTreeMap<u32, (u64, Option<f32>)>,
    /// Rescheduling interrupts per second, per CPU.
    pub rescheduling: BTreeMap<u32, Option<f32>>,
    /// Function-call and TLB-shootdown interrupts per second, per CPU, summed.
    pub calls: BTreeMap<u32, Option<f32>>,
    /// Non-maskable interrupts per second, per CPU.
    pub non_maskable: BTreeMap<u32, Option<f32>>,
}

/// The rows of `rows` that count `ids` or whose description has one of the `phrases`, which the
/// arm64 kernel puts on `IPIn` rows.
fn named_rows<'a>(
    rows: &'a [Interrupt],
    ids: &'a [&str],
    phrases: &'a [&str],
) -> impl Iterator<Item = &'a Interrupt> + 'a {
    rows.iter().filter(|row| {
        let description = row.description.to_ascii_lowercase();
        !row.is_numbered()
            && (ids.iter().any(|id| row.id.eq_ignore_ascii_case(id))
                || phrases.iter().any(|phrase| description.contains(phrase)))
    })
}

fn add_rates(into: &mut BTreeMap<u32, Option<f32>>, counts: &[CpuCount]) {
    for &(cpu, _, rate) in counts {
        let slot = into.entry(cpu).or_insert(None);
        *slot = match (*slot, rate) {
            (Some(held), Some(rate)) => Some(held + rate),
            (held, rate) => held.or(rate),
        };
    }
}

/// Everything the board reads from one snapshot.
#[derive(Clone, Debug)]
pub struct Reading {
    pub table: Table,
    /// The numbered rows, one line each, in the table's order.
    pub lines: Vec<Line>,
    pub named: Named,
    pub softirq_table: Table,
    /// Each softirq's rate on each CPU, in the kernel's order.
    pub softirqs: Vec<(String, Rates)>,
    /// The CPUs that appear in either table, for a machine whose snapshot lists none.
    pub cpu_ids: BTreeSet<u32>,
}

impl Reading {
    pub fn new(snapshot: &Snapshot) -> Reading {
        let mut reading = Reading {
            table: Table::Pending,
            lines: Vec::new(),
            named: Named::default(),
            softirq_table: Table::Pending,
            softirqs: Vec::new(),
            cpu_ids: BTreeSet::new(),
        };
        match &snapshot.interrupts {
            Measured::Pending => {}
            Measured::Unreadable => reading.table = Table::Unreadable,
            Measured::Known(rows) => {
                reading.table = Table::Known;
                reading.read_interrupts(rows);
            }
        }
        match &snapshot.softirqs {
            Measured::Pending => {}
            Measured::Unreadable => reading.softirq_table = Table::Unreadable,
            Measured::Known(rows) => {
                reading.softirq_table = Table::Known;
                for (name, counts) in rows {
                    reading.cpu_ids.extend(counts.iter().map(|count| count.0));
                    let rates = counts.iter().map(|&(cpu, _, rate)| (cpu, rate)).collect();
                    reading.softirqs.push((name.clone(), rates));
                }
            }
        }
        reading
    }

    fn read_interrupts(&mut self, rows: &[Interrupt]) {
        let mut order: BTreeMap<String, u64> = BTreeMap::new();
        let numbered: Vec<(&Interrupt, u64, String)> = rows
            .iter()
            .filter(|row| row.is_numbered())
            .map(|row| {
                let number = row.id.parse().unwrap_or(u64::MAX);
                let name = row.devices.first().cloned().unwrap_or_default();
                let device = if name.is_empty() {
                    row.id.clone()
                } else {
                    device_key(&name)
                };
                let lowest = order.entry(device.clone()).or_insert(number);
                *lowest = (*lowest).min(number);
                (row, number, device)
            })
            .collect();
        for (row, number, device) in numbered {
            let name = row
                .devices
                .first()
                .cloned()
                .unwrap_or_else(|| row.id.clone());
            self.cpu_ids.extend(row.per_cpu.iter().map(|count| count.0));
            self.lines.push(Line {
                key: number,
                id: row.id.clone(),
                name,
                devices: row.devices.clone(),
                chip: row.chip.clone(),
                rates: row
                    .per_cpu
                    .iter()
                    .map(|&(cpu, _, rate)| (cpu, rate))
                    .collect(),
                affinity: row.affinity.clone(),
                members: 1,
                order: order[&device],
                device,
            });
        }
        if let Some(row) = timer_row(rows) {
            for &(cpu, count, rate) in &row.per_cpu {
                self.named.timer.insert(cpu, (count, rate));
            }
        }
        for row in named_rows(rows, &["RES"], &["rescheduling"]) {
            add_rates(&mut self.named.rescheduling, &row.per_cpu);
        }
        for row in named_rows(rows, &["CAL", "TLB"], &["function call", "tlb shootdown"]) {
            add_rates(&mut self.named.calls, &row.per_cpu);
        }
        for row in named_rows(rows, &["NMI"], &["non-maskable"]) {
            add_rates(&mut self.named.non_maskable, &row.per_cpu);
        }
    }

    /// Whether the table has a timer row, which the clocks turn with.
    pub fn has_timer(&self) -> bool {
        !self.named.timer.is_empty()
    }

    /// Interrupts per second across every line, None while no line has a known rate.
    pub fn total_rate(&self) -> Option<f32> {
        sum_known(self.lines.iter().filter_map(Line::rate))
    }

    /// Rescheduling interrupts per second across the machine, None while unknown.
    pub fn rescheduling_rate(&self) -> Option<f32> {
        sum_known(self.named.rescheduling.values().filter_map(|rate| *rate))
    }

    /// The jacks to show, in the order they are first seated: groups by their lowest IRQ number,
    /// lines by number. Above `CAP` the lines whose count has never changed (when `moved` says
    /// which have; None before a second sample) go first, then every device with more than
    /// `FOLD_ABOVE` lines folds into one jack, then the rest are cut.
    pub fn jacks(&self, moved: Option<&BTreeSet<u64>>) -> Vec<Line> {
        let mut lines: Vec<Line> = self.lines.clone();
        let canonical = |a: &Line, b: &Line| (a.order, a.key).cmp(&(b.order, b.key));
        lines.sort_by(canonical);
        if lines.len() > CAP
            && let Some(moved) = moved
        {
            lines.retain(|line| moved.contains(&line.key));
        }
        if lines.len() > CAP {
            lines = fold(lines);
            lines.sort_by(canonical);
        }
        lines.truncate(CAP);
        lines
    }
}

/// `lines` with every device of more than `FOLD_ABOVE` lines as one jack that sums its rates.
fn fold(lines: Vec<Line>) -> Vec<Line> {
    let mut by_device: BTreeMap<String, Vec<Line>> = BTreeMap::new();
    for line in lines {
        by_device.entry(line.device.clone()).or_default().push(line);
    }
    let mut folded = Vec::new();
    for (device, group) in by_device {
        if group.len() <= FOLD_ABOVE {
            folded.extend(group);
            continue;
        }
        let names: Vec<&str> = group.iter().map(|line| line.name.as_str()).collect();
        let prefix = common_prefix(&names);
        let label = if prefix.is_empty() { &device } else { &prefix };
        let mut rates: BTreeMap<u32, Option<f32>> = BTreeMap::new();
        for line in &group {
            for &(cpu, rate) in &line.rates {
                let slot = rates.entry(cpu).or_insert(None);
                *slot = match (*slot, rate) {
                    (Some(held), Some(rate)) => Some(held + rate),
                    (held, rate) => held.or(rate),
                };
            }
        }
        folded.push(Line {
            key: FOLDED | (fnv1a_64(&device) & (FOLDED - 1)),
            id: group[0].id.clone(),
            name: format!("{label}* ({})", group.len()),
            devices: group[0].devices.clone(),
            chip: group[0].chip.clone(),
            rates: rates.into_iter().collect(),
            affinity: None,
            members: group.len(),
            order: group[0].order,
            device,
        });
    }
    folded
}

fn common_prefix(names: &[&str]) -> String {
    let Some((first, rest)) = names.split_first() else {
        return String::new();
    };
    let mut length = first.len();
    for name in rest {
        length = first
            .bytes()
            .zip(name.bytes())
            .take(length)
            .take_while(|(a, b)| a == b)
            .count();
    }
    String::from_utf8_lossy(&first.as_bytes()[..length]).into_owned()
}

/// Numbered places that keep what sits in them: a key keeps its place for as long as it is held,
/// and a freed place goes to the next newcomer, lowest first.
#[derive(Clone, Debug, Default)]
pub struct Pegs {
    held: Vec<Option<u64>>,
}

impl Pegs {
    /// The place of `key`, taking the lowest free one if it has none.
    pub fn seat(&mut self, key: u64) -> usize {
        if let Some(place) = self.of(key) {
            return place;
        }
        let place = self
            .held
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.held.len());
        if place == self.held.len() {
            self.held.push(None);
        }
        self.held[place] = Some(key);
        place
    }

    pub fn of(&self, key: u64) -> Option<usize> {
        self.held.iter().position(|held| *held == Some(key))
    }

    pub fn release(&mut self, key: u64) {
        if let Some(place) = self.of(key) {
            self.held[place] = None;
        }
    }

    /// Places up to and including the highest held one.
    pub fn span(&self) -> usize {
        self.held
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |last| last + 1)
    }

    pub fn holder(&self, place: usize) -> Option<u64> {
        self.held.get(place).copied().flatten()
    }
}

/// One operator's chair: a CPU, or with more than `SEATS` CPUs a run of them.
#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    pub cpus: Vec<u32>,
    pub kind: CoreKind,
    /// Which seat this is among those of its kind, from 0, for its colour.
    pub ordinal: usize,
    /// The busy share of the seat's CPUs, None while none is known.
    pub busy: Option<f32>,
}

impl Seat {
    /// `busy` as an operator moves to it: a seat whose CPUs are not measured sits still, as an
    /// idle one does.
    pub fn drawn_busy(&self) -> f32 {
        self.busy.unwrap_or(0.0)
    }
}

/// The operators: performance cores first, then efficiency cores, then cores of unknown kind,
/// each group by CPU number.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Roster {
    pub seats: Vec<Seat>,
    /// How many CPUs each seat covers.
    pub per_seat: usize,
}

impl Roster {
    pub fn new(cpus: &[Cpu], fallback: &BTreeSet<u32>) -> Roster {
        let mut sorted: Vec<(u8, u32, CoreKind, Option<f32>)> = cpus
            .iter()
            .map(|cpu| {
                let rank = match cpu.kind {
                    CoreKind::Performance => 0,
                    CoreKind::Efficiency => 1,
                    CoreKind::Unknown => 2,
                };
                (rank, cpu.id, cpu.kind, cpu.busy)
            })
            .collect();
        if sorted.is_empty() {
            sorted = fallback
                .iter()
                .map(|&id| (2, id, CoreKind::Unknown, None))
                .collect();
        }
        sorted.sort_by_key(|&(rank, id, ..)| (rank, id));
        let per_seat = sorted.len().div_ceil(SEATS).max(1);
        let mut ordinals = [0_usize; 3];
        let seats = sorted
            .chunks(per_seat)
            .map(|chunk| {
                let (rank, _, kind, _) = chunk[0];
                let ordinal = ordinals[usize::from(rank)];
                ordinals[usize::from(rank)] += 1;
                let known: Vec<f32> = chunk.iter().filter_map(|c| c.3).collect();
                Seat {
                    cpus: chunk.iter().map(|c| c.1).collect(),
                    kind,
                    ordinal,
                    busy: (!known.is_empty())
                        .then(|| known.iter().sum::<f32>() / known.len() as f32),
                }
            })
            .collect();
        Roster { seats, per_seat }
    }

    /// The seat that holds `cpu`.
    pub fn seat_of(&self, cpu: u32) -> Option<usize> {
        self.seats.iter().position(|seat| seat.cpus.contains(&cpu))
    }

    /// How many seats are of `kind`.
    pub fn count(&self, kind: CoreKind) -> usize {
        self.seats.iter().filter(|seat| seat.kind == kind).count()
    }
}

/// A move of a plug from one operator to another that is under way or just done.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swing {
    /// The seat and slot the plug was pulled from.
    pub from: (usize, usize),
    /// When it was pulled, in scene time.
    pub started: f32,
}

/// The last time a cord changed operator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moved {
    pub from: usize,
    pub to: usize,
    pub at: f32,
}

/// One line's cord.
#[derive(Clone, Debug, Default)]
pub struct Cord {
    /// The operator seat the cord is plugged into (the one it swings to, while it swings) and the
    /// slot on that operator's shelf. None until the line has been seen to go anywhere.
    pub plugged: Option<(usize, usize)>,
    pub swing: Option<Swing>,
    pub moved: Option<Moved>,
    /// A servicing seat that differs from the plugged one, and for how many samples in a row.
    candidate: Option<(usize, u8)>,
    affinity: Option<Vec<u32>>,
}

/// The board's memory between samples.
#[derive(Clone, Debug, Default)]
pub struct Lines {
    /// The jacks of the last sample, in seat order of their first seating.
    pub jacks: Vec<Line>,
    pub roster: Roster,
    pub cords: BTreeMap<u64, Cord>,
    pub reading: Option<Reading>,
    /// The jack seats and the slots on each operator's shelf.
    pub pegs: Pegs,
    shelf: BTreeMap<usize, Pegs>,
    /// The IRQ numbers whose count has changed since the board began watching.
    moved: BTreeSet<u64>,
    totals: BTreeMap<u64, u64>,
    /// When the red lamp of each operator was last lit by a non-maskable interrupt.
    pub nmi: BTreeMap<usize, f32>,
    /// The elapsed time of the sample taken in last.
    pub last: Option<f64>,
    /// When the sample was taken in, in scene time.
    pub taken: f32,
    pub columns: usize,
    pub rows: usize,
}

impl Lines {
    /// The jack seats' grid: columns chosen when the board was first populated, rows grown to fit.
    pub fn grid(&self) -> (usize, usize) {
        (self.columns.max(COLUMNS.0), self.rows.max(MIN_ROWS))
    }

    /// Takes in the sample `snapshot` that was drawn at scene time `time`. `reseed` means the
    /// board was not watching the sample before (another view was shown, or time went back): the
    /// cords are plugged where the sample says and nothing swings.
    pub fn take_in(&mut self, snapshot: &Snapshot, time: f32, reseed: bool) {
        let reading = Reading::new(snapshot);
        self.roster = Roster::new(&snapshot.cpus, &reading.cpu_ids);
        let reseed = reseed || self.last.is_none_or(|last| snapshot.elapsed < last);
        if reading.table == Table::Known {
            if reseed {
                self.totals.clear();
            }
            let watching = !self.totals.is_empty();
            let mut present = BTreeSet::new();
            for row in snapshot.interrupts.known().into_iter().flatten() {
                let Ok(number) = row.id.parse::<u64>() else {
                    continue;
                };
                let total: u64 = row.per_cpu.iter().map(|count| count.1).sum();
                if watching
                    && self
                        .totals
                        .get(&number)
                        .is_some_and(|&before| before != total)
                {
                    self.moved.insert(number);
                }
                self.totals.insert(number, total);
                present.insert(number);
            }
            // A line that left the table (a device removed) takes its history with it, so neither
            // map grows with every IRQ number the machine ever had.
            self.totals.retain(|number, _| present.contains(number));
            self.moved.retain(|number| present.contains(number));
            self.seat_jacks(&reading, watching);
            self.plug(time, reseed);
            self.track_nmi(&reading, time);
        }
        self.last = Some(snapshot.elapsed);
        self.taken = time;
        self.reading = Some(reading);
    }

    fn seat_jacks(&mut self, reading: &Reading, watching: bool) {
        let jacks = reading.jacks(watching.then_some(&self.moved));
        let keys: BTreeSet<u64> = jacks.iter().map(|jack| jack.key).collect();
        for gone in self
            .cords
            .keys()
            .copied()
            .filter(|key| !keys.contains(key))
            .collect::<Vec<_>>()
        {
            if let Some((seat, _)) = self.cords.remove(&gone).and_then(|cord| cord.plugged) {
                self.shelf.entry(seat).or_default().release(gone);
            }
        }
        for key in self
            .pegs_keys()
            .into_iter()
            .filter(|key| !keys.contains(key))
        {
            self.pegs.release(key);
        }
        for jack in &jacks {
            self.pegs.seat(jack.key);
        }
        if self.columns == 0 && !jacks.is_empty() {
            let wanted = (3.0 * jacks.len() as f32).sqrt().ceil() as usize;
            self.columns = wanted.clamp(COLUMNS.0, COLUMNS.1);
        }
        let columns = self.grid().0;
        self.rows = self.rows.max(self.pegs.span().div_ceil(columns));
        self.jacks = jacks;
    }

    fn pegs_keys(&self) -> Vec<u64> {
        (0..self.pegs.span())
            .filter_map(|place| self.pegs.holder(place))
            .collect()
    }

    /// The seat whose CPUs serviced most of `line`'s interrupts, preferring the plugged one on a
    /// tie so that two equal CPUs never take turns.
    fn servicing_seat(&self, line: &Line, plugged: Option<usize>) -> Option<usize> {
        let mut per_seat: BTreeMap<usize, f32> = BTreeMap::new();
        for &(cpu, rate) in &line.rates {
            if let (Some(rate), Some(seat)) =
                (rate.filter(|rate| *rate > 0.0), self.roster.seat_of(cpu))
            {
                *per_seat.entry(seat).or_insert(0.0) += rate;
            }
        }
        let top = per_seat.values().copied().fold(0.0_f32, f32::max);
        if top <= 0.0 {
            return None;
        }
        if let Some(seat) = plugged
            && per_seat.get(&seat) == Some(&top)
        {
            return Some(seat);
        }
        per_seat
            .into_iter()
            .find(|&(_, rate)| rate == top)
            .map(|(seat, _)| seat)
    }

    /// Where a line routed to `affinity` is serviced: the seat it is already seen at, if that
    /// is one of them, else the first.
    fn affinity_seat(&self, affinity: &[u32], serviced: Option<usize>) -> Option<usize> {
        let seats: Vec<usize> = affinity
            .iter()
            .filter_map(|&cpu| self.roster.seat_of(cpu))
            .collect();
        serviced
            .filter(|seat| seats.contains(seat))
            .or_else(|| seats.first().copied())
    }

    fn plug(&mut self, time: f32, reseed: bool) {
        let jacks = self.jacks.clone();
        for jack in &jacks {
            let (held, mut cord) = {
                let cord = self.cords.remove(&jack.key).unwrap_or_default();
                (cord.plugged.map(|(seat, _)| seat), cord)
            };
            let serviced = self.servicing_seat(jack, held);
            let changed = !reseed
                && matches!((&cord.affinity, &jack.affinity), (Some(before), Some(now)) if before != now);
            if jack.affinity.is_some() {
                cord.affinity.clone_from(&jack.affinity);
            }
            let single = jack
                .affinity
                .as_deref()
                .filter(|affinity| affinity.len() == 1)
                .and_then(|affinity| self.affinity_seat(affinity, None));
            let target = match held {
                None => serviced.or(single),
                Some(_) if reseed => serviced.or(held),
                Some(_) if changed => jack
                    .affinity
                    .as_deref()
                    .and_then(|affinity| self.affinity_seat(affinity, serviced))
                    .or(serviced),
                Some(held) => match serviced {
                    Some(seat) if seat != held => {
                        let count = match cord.candidate {
                            Some((candidate, count)) if candidate == seat => count + 1,
                            _ => 1,
                        };
                        cord.candidate = Some((seat, count));
                        (count >= PERSISTENCE).then_some(seat)
                    }
                    _ => {
                        cord.candidate = None;
                        None
                    }
                },
            };
            if let Some(target) = target
                && Some(target) != held
            {
                cord.candidate = None;
                self.move_cord(
                    jack.key,
                    &mut cord,
                    held,
                    target,
                    time,
                    reseed || held.is_none(),
                );
            } else if reseed {
                cord.swing = None;
                cord.candidate = None;
            }
            self.cords.insert(jack.key, cord);
        }
    }

    fn move_cord(
        &mut self,
        key: u64,
        cord: &mut Cord,
        from: Option<usize>,
        to: usize,
        time: f32,
        quiet: bool,
    ) {
        let old = cord.plugged;
        if let Some((seat, _)) = old {
            self.shelf.entry(seat).or_default().release(key);
        }
        let slot = self.shelf.entry(to).or_default().seat(key);
        cord.plugged = Some((to, slot));
        if quiet {
            cord.swing = None;
            return;
        }
        if let (Some(from), Some(old)) = (from, old) {
            cord.swing = Some(Swing {
                from: old,
                started: time,
            });
            cord.moved = Some(Moved { from, to, at: time });
        }
    }

    fn track_nmi(&mut self, reading: &Reading, time: f32) {
        for (&cpu, rate) in &reading.named.non_maskable {
            if rate.is_some_and(|rate| rate > 0.0)
                && let Some(seat) = self.roster.seat_of(cpu)
            {
                self.nmi.insert(seat, time);
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn cpus(count: u32) -> Vec<Cpu> {
        (0..count)
            .map(|id| Cpu {
                id,
                kind: if id < count / 2 {
                    CoreKind::Performance
                } else {
                    CoreKind::Efficiency
                },
                busy: Some(0.5),
                mhz: None,
                wait: None,
            })
            .collect()
    }

    /// A numbered row served by `cpu` at `rate`, with the effective affinity `affinity`.
    pub fn row(irq: u32, name: &str, cpu: u32, rate: f32, affinity: Option<u32>) -> Interrupt {
        Interrupt {
            id: irq.to_string(),
            chip: "IR-PCI-MSIX".into(),
            devices: vec![name.into()],
            per_cpu: (0..4)
                .map(|id| (id, 1000, Some(if id == cpu { rate } else { 0.0 })))
                .collect(),
            affinity: affinity.map(|cpu| vec![cpu]),
            ..Interrupt::default()
        }
    }

    pub fn sample(rows: Vec<Interrupt>, elapsed: f64) -> Snapshot {
        Snapshot {
            interrupts: Measured::Known(rows),
            cpus: cpus(4),
            elapsed,
            ..Snapshot::default()
        }
    }

    #[test]
    fn device_names_group_by_dropping_the_queue_suffix() {
        for (name, key) in [
            ("nvme0q0", "nvme0"),
            ("nvme0q32", "nvme0"),
            ("eth0-rx-2", "eth0"),
            ("eth0-tx-0", "eth0"),
            ("iwlwifi:q7", "iwlwifi"),
            ("iwlwifi", "iwlwifi"),
            ("mlx5_comp12", "mlx5_comp"),
            ("ens1f0-TxRx-3", "ens1f0"),
            ("idma64.1", "idma64."),
            ("xhci_hcd", "xhci_hcd"),
            ("8250", "8250"),
        ] {
            assert_eq!(device_key(name), key, "{name}");
        }
    }

    #[test]
    fn the_servicing_cpu_is_the_one_with_the_most_interrupts_in_the_interval() {
        let mut rows = vec![row(30, "eth0-rx-0", 0, 0.0, None)];
        // CPU 1 has the most events in the interval although CPU 0 has the larger count.
        rows[0].per_cpu = vec![
            (0, 9_000_000, Some(120.0)),
            (1, 40, Some(900.0)),
            (2, 40, Some(900.0)),
            (3, 40, None),
        ];
        let reading = Reading::new(&sample(rows, 1.0));
        let line = &reading.lines[0];
        assert_eq!(line.servicing(), Some(1), "the lowest CPU on a tie");
        assert_eq!(line.rate(), Some(1920.0));
        let shares = line.shares(3);
        assert_eq!(shares.len(), 3);
        assert_eq!(shares[0].0, 1);
        assert!((shares.iter().map(|s| s.1).sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_softirq_rate_that_was_not_measured_stays_unknown_and_never_reads_zero() {
        let rates = [(0, Some(5.0)), (1, None), (2, None), (3, Some(2.5))];
        assert_eq!(
            seat_rate(&rates, &[0, 1]),
            Some(5.0),
            "the known part of a seat counts"
        );
        assert_eq!(
            seat_rate(&rates, &[1, 2]),
            None,
            "nothing known is not nothing counted"
        );
        assert_eq!(seat_rate(&rates, &[0, 1, 2, 3]), Some(7.5));
        assert_eq!(
            seat_rate(&[(0, Some(0.0))], &[0]),
            Some(0.0),
            "a measured zero is a zero"
        );
        let mut snapshot = sample(Vec::new(), 1.0);
        snapshot.softirqs = Measured::Known(vec![
            ("NET_RX".into(), vec![(0, 10, None), (1, 20, None)]),
            ("TIMER".into(), vec![(0, 10, Some(0.0)), (1, 20, Some(4.0))]),
        ]);
        let reading = Reading::new(&snapshot);
        assert_eq!(reading.softirq_table, Table::Known);
        assert_eq!(seat_rate(&reading.softirqs[0].1, &[0, 1]), None);
        assert_eq!(seat_rate(&reading.softirqs[1].1, &[0, 1]), Some(4.0));
    }

    #[test]
    fn an_unknown_rate_is_neither_zero_nor_a_winner() {
        let mut first = row(31, "ahci", 0, 0.0, None);
        first.per_cpu = (0..4).map(|id| (id, 7, None)).collect();
        let line = Reading::new(&sample(vec![first], 1.0)).lines.remove(0);
        assert_eq!(line.rate(), None);
        assert_eq!(line.servicing(), None);
        assert!(line.shares(3).is_empty());
    }

    fn board(rows: Vec<Interrupt>) -> Lines {
        let mut lines = Lines::default();
        lines.take_in(&sample(rows, 1.0), 1.0, false);
        lines
    }

    fn plugged(lines: &Lines, irq: u64) -> Option<usize> {
        lines
            .cords
            .get(&irq)
            .and_then(|cord| cord.plugged)
            .map(|p| p.0)
    }

    #[test]
    fn a_quiet_line_keeps_its_cord_where_it_was() {
        let mut lines = board(vec![row(40, "hda_intel", 2, 50.0, None)]);
        assert_eq!(plugged(&lines, 40), Some(2));
        for (step, elapsed) in [2.0, 3.0, 4.0].into_iter().enumerate() {
            lines.take_in(
                &sample(vec![row(40, "hda_intel", 0, 0.0, None)], elapsed),
                elapsed as f32,
                false,
            );
            assert_eq!(plugged(&lines, 40), Some(2), "after quiet sample {step}");
            assert!(lines.cords[&40].swing.is_none());
        }
    }

    #[test]
    fn an_effective_affinity_change_swings_the_cord_over() {
        let mut lines = board(vec![row(41, "eth0-rx-2", 3, 900.0, Some(3))]);
        assert!(
            lines.cords[&41].swing.is_none(),
            "the first plug is not a swing"
        );
        // The new affinity is read first; the counts have not moved yet.
        lines.take_in(
            &sample(vec![row(41, "eth0-rx-2", 3, 900.0, Some(1))], 2.0),
            2.0,
            false,
        );
        let cord = &lines.cords[&41];
        assert_eq!(cord.plugged.map(|p| p.0), Some(1));
        let swing = cord.swing.expect("it swings at once");
        assert_eq!((swing.from.0, swing.started), (3, 2.0));
        assert_eq!(cord.moved.map(|m| (m.from, m.to)), Some((3, 1)));
    }

    #[test]
    fn a_servicing_cpu_change_swings_only_after_two_samples() {
        let mut lines = board(vec![row(42, "eth0-tx-1", 0, 400.0, None)]);
        lines.take_in(
            &sample(vec![row(42, "eth0-tx-1", 2, 400.0, None)], 2.0),
            2.0,
            false,
        );
        assert_eq!(plugged(&lines, 42), Some(0), "one sample is not enough");
        assert!(lines.cords[&42].swing.is_none());
        lines.take_in(
            &sample(vec![row(42, "eth0-tx-1", 2, 400.0, None)], 3.0),
            3.0,
            false,
        );
        assert_eq!(plugged(&lines, 42), Some(2));
        assert_eq!(lines.cords[&42].swing.map(|s| s.started), Some(3.0));
    }

    #[test]
    fn a_blip_on_another_cpu_does_not_move_the_cord() {
        let mut lines = board(vec![row(43, "xhci_hcd", 0, 400.0, None)]);
        let steps = [(1, 2.0), (0, 3.0), (1, 4.0)];
        for (cpu, elapsed) in steps {
            lines.take_in(
                &sample(vec![row(43, "xhci_hcd", cpu, 400.0, None)], elapsed),
                elapsed as f32,
                false,
            );
        }
        assert_eq!(
            plugged(&lines, 43),
            Some(0),
            "a changed mind in between starts again"
        );
        // A quiet sample in between breaks the run too.
        let mut lines = board(vec![row(43, "xhci_hcd", 0, 400.0, None)]);
        lines.take_in(
            &sample(vec![row(43, "xhci_hcd", 1, 400.0, None)], 2.0),
            2.0,
            false,
        );
        lines.take_in(
            &sample(vec![row(43, "xhci_hcd", 1, 0.0, None)], 3.0),
            3.0,
            false,
        );
        lines.take_in(
            &sample(vec![row(43, "xhci_hcd", 1, 400.0, None)], 4.0),
            4.0,
            false,
        );
        assert_eq!(plugged(&lines, 43), Some(0));
    }

    #[test]
    fn a_reseeded_board_plugs_where_the_sample_says_without_a_swing() {
        let mut lines = board(vec![row(44, "i915", 0, 100.0, None)]);
        lines.take_in(
            &sample(vec![row(44, "i915", 3, 100.0, Some(3))], 9.0),
            9.0,
            true,
        );
        assert_eq!(plugged(&lines, 44), Some(3));
        assert!(lines.cords[&44].swing.is_none());
        lines.take_in(
            &sample(vec![row(44, "i915", 1, 100.0, Some(1))], 2.0),
            10.0,
            false,
        );
        assert_eq!(
            plugged(&lines, 44),
            Some(1),
            "a sample from the past reseeds"
        );
        assert!(lines.cords[&44].swing.is_none());
    }

    #[test]
    fn a_line_with_no_measured_rate_plugs_into_its_single_effective_cpu() {
        let mut first = row(45, "mei_me", 0, 0.0, Some(2));
        first.per_cpu = (0..4).map(|id| (id, 5, None)).collect();
        let lines = board(vec![first]);
        assert_eq!(plugged(&lines, 45), Some(2));
        let mut unknown = row(46, "mei_me", 0, 0.0, None);
        unknown.per_cpu = (0..4).map(|id| (id, 5, None)).collect();
        assert_eq!(
            plugged(&board(vec![unknown]), 46),
            None,
            "nothing to go on, no cord"
        );
    }

    #[test]
    fn named_rows_never_become_jacks_or_cords() {
        let mut rows = vec![row(47, "ahci", 1, 10.0, None)];
        for (id, description) in [
            ("RES", "Rescheduling interrupts"),
            ("CAL", "Function call interrupts"),
            ("TLB", "TLB shootdowns"),
            ("NMI", "Non-maskable interrupts"),
            ("IPI0", "Rescheduling interrupts"),
            ("LOC", "Local timer interrupts"),
        ] {
            rows.push(Interrupt {
                id: id.into(),
                description: description.into(),
                per_cpu: (0..4).map(|cpu| (cpu, 100, Some(3.0))).collect(),
                ..Interrupt::default()
            });
        }
        let lines = board(rows);
        assert_eq!(lines.jacks.len(), 1);
        assert_eq!(
            lines.cords.len(),
            1,
            "no cord runs from one operator to another"
        );
        let named = &lines.reading.as_ref().expect("read").named;
        assert_eq!(
            named.rescheduling[&0],
            Some(6.0),
            "RES and the arm64 IPI row add up"
        );
        assert_eq!(named.calls[&1], Some(6.0));
        assert_eq!(named.timer.len(), 4);
        assert_eq!(named.non_maskable[&2], Some(3.0));
    }

    #[test]
    fn ipis_never_connect_two_operators() {
        let rows = vec![
            Interrupt {
                id: "RES".into(),
                description: "Rescheduling interrupts".into(),
                per_cpu: (0..4).map(|cpu| (cpu, 900, Some(40.0))).collect(),
                ..Interrupt::default()
            },
            row(48, "nvme0q1", 0, 80.0, None),
        ];
        let lines = board(rows);
        assert!(
            lines
                .cords
                .keys()
                .all(|&key| lines.jacks.iter().any(|jack| jack.key == key)),
            "every cord starts at a jack"
        );
        assert_eq!(lines.cords.len(), lines.jacks.len());
    }

    #[test]
    fn jacks_keep_their_places_when_rates_or_lines_change() {
        let names = [
            "nvme0q0",
            "nvme0q1",
            "eth0-rx-0",
            "eth0-rx-1",
            "ahci",
            "i915",
        ];
        let rows = |skip: Option<usize>, rate: f32| -> Vec<Interrupt> {
            names
                .iter()
                .enumerate()
                .filter(|(index, _)| Some(*index) != skip)
                .map(|(index, name)| row(30 + index as u32, name, (index % 4) as u32, rate, None))
                .collect()
        };
        let mut lines = board(rows(None, 100.0));
        let seats: Vec<Option<usize>> = (30..36).map(|irq| lines.pegs.of(irq)).collect();
        assert_eq!(
            seats,
            (0..6).map(Some).collect::<Vec<_>>(),
            "grouped, in IRQ order"
        );
        lines.take_in(&sample(rows(None, 7000.0), 2.0), 2.0, false);
        assert_eq!(
            (30..36).map(|irq| lines.pegs.of(irq)).collect::<Vec<_>>(),
            seats,
            "rates change, places do not"
        );
        lines.take_in(&sample(rows(Some(2), 50.0), 3.0), 3.0, false);
        assert_eq!(lines.pegs.of(32), None);
        for irq in [30, 31, 33, 34, 35] {
            assert_eq!(
                lines.pegs.of(irq),
                seats[(irq - 30) as usize],
                "{irq} stays"
            );
        }
        let mut more = rows(Some(2), 50.0);
        more.push(row(99, "ucsi_acpi", 0, 1.0, None));
        more.push(row(5, "timer", 0, 1.0, None));
        lines.take_in(&sample(more, 4.0), 4.0, false);
        assert_eq!(
            lines.pegs.of(5),
            Some(2),
            "a newcomer takes the lowest free place"
        );
        assert_eq!(lines.pegs.of(99), Some(6));
        assert_eq!(lines.pegs.of(35), seats[5]);
    }

    #[test]
    fn the_board_has_twelve_columns_for_48_lines_and_keeps_them_as_lines_arrive() {
        let rows = |count: u32| -> Vec<Interrupt> {
            (0..count)
                .map(|n| row(100 + n, &format!("card{n}x"), 0, 5.0, None))
                .collect()
        };
        let mut lines = board(rows(48));
        assert_eq!(lines.grid(), (12, 4));
        lines.take_in(&sample(rows(60), 2.0), 2.0, false);
        assert_eq!(lines.grid(), (12, 5), "rows grow, columns stay");
        assert_eq!(board(rows(130)).columns, 16);
    }

    #[test]
    fn above_the_cap_quiet_lines_are_hidden_first_and_big_devices_fold() {
        let many = |changing: bool| -> Snapshot {
            let rows = (0..150)
                .map(|n| {
                    let name = if n < 40 {
                        format!("nvme0q{n}")
                    } else {
                        format!("dev{n}x")
                    };
                    let mut line = row(100 + n, &name, 0, 5.0, None);
                    line.per_cpu[0].1 = if changing { 2000 + u64::from(n) } else { 1000 };
                    line
                })
                .collect();
            sample(rows, if changing { 2.0 } else { 1.0 })
        };
        let first = Reading::new(&many(false));
        assert_eq!(
            first.jacks(None).len(),
            111,
            "nothing is known to be quiet yet: only the nvme folds"
        );
        let mut lines = Lines::default();
        lines.take_in(&many(false), 1.0, false);
        // Only the 40 nvme lines change; the 110 others never do.
        let mut next = many(false);
        next.elapsed = 2.0;
        if let Measured::Known(rows) = &mut next.interrupts {
            for row in rows.iter_mut().take(40) {
                row.per_cpu[0].1 += 77;
            }
        }
        lines.take_in(&next, 2.0, false);
        assert_eq!(lines.jacks.len(), 40, "only lines that changed remain");
        let reading = Reading::new(&many(false));
        let all: BTreeSet<u64> = reading.lines.iter().map(|line| line.key).collect();
        let folded = reading.jacks(Some(&all));
        assert!(folded.len() <= CAP);
        let nvme = folded.iter().find(|line| line.members > 1).expect("folded");
        assert_eq!(nvme.name, "nvme0q* (40)");
        assert!(nvme.key & FOLDED != 0);
        assert_eq!(nvme.rate(), Some(200.0));
    }

    #[test]
    fn a_line_that_leaves_the_table_takes_its_history_with_it() {
        let mut lines = Lines::default();
        let rows = |numbers: &[u32], total: u64| -> Vec<Interrupt> {
            numbers
                .iter()
                .map(|&number| {
                    let mut line = row(number, &format!("dev{number}"), 0, 5.0, None);
                    line.per_cpu[0].1 = total;
                    line
                })
                .collect()
        };
        lines.take_in(&sample(rows(&[1, 2, 3], 10), 1.0), 1.0, false);
        lines.take_in(&sample(rows(&[1, 2, 3], 20), 2.0), 2.0, false);
        assert_eq!(lines.totals.len(), 3);
        assert_eq!(lines.moved.len(), 3);
        lines.take_in(&sample(rows(&[1], 30), 3.0), 3.0, false);
        assert_eq!(lines.totals.keys().copied().collect::<Vec<_>>(), [1]);
        assert_eq!(lines.moved.iter().copied().collect::<Vec<_>>(), [1]);
    }
}
