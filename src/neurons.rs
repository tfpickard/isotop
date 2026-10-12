//! The simulation behind the Cortex view: Izhikevich (2003) neurons that fire and send spikes down
//! delayed synapses, and the synchrony they show. Pure state with no drawing; the view feeds it
//! currents and reads spikes, membrane potentials and the event queue back.
//!
//! The equations are `v' = 0.04 v^2 + 5 v + 140 - u + I` and `u' = a (b v - u)`, in millivolts and
//! milliseconds, integrated by Euler steps of 0.5 ms as in the paper. A neuron that reaches 30 mV
//! fires: `v` is set to `c` and `d` is added to `u`. Everything counts in whole steps (ticks), so
//! two runs that take the same steps agree exactly, whatever the frame rate that asked for them.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::OnceLock;

use crate::model::Kind;
use crate::simulation::Rng;

/// The integration step in seconds: 0.5 ms, as in the paper.
pub const STEP_SECONDS: f32 = 0.0005;
/// The step in milliseconds, which the equations use.
const STEP_MS: f32 = 0.5;
/// Ticks in a second.
pub const TICKS_PER_SECOND: u64 = 2000;
/// A neuron fires when its membrane potential reaches this, in millivolts.
const PEAK: f32 = 30.0;
/// The resting potential a neuron starts near, in millivolts.
const REST: f32 = -65.0;
/// Synaptic current decays with this time constant, in seconds.
const SYNAPSE_TAU: f32 = 0.005;
/// The standard deviation of the current noise that drives an active neuron, per step.
pub const NOISE: f32 = 1.2;
/// The time constant of the averages synchrony is computed from, in seconds.
const VARIANCE_TAU: f32 = 2.0;
/// The membrane potentials kept for the watched neuron: 320 ms.
pub const TRACE_TICKS: usize = 640;
/// The most spikes in flight at once; a spike past it is lost, as a saturated machine would
/// lose one, rather than letting the queue grow without bound.
const MAX_EVENTS: usize = 262_144;
/// The number of neuron groups synchrony is measured over.
pub const GROUPS: usize = 2;

/// The four firing patterns of Izhikevich 2003 that the process kinds map to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Firing {
    /// Kernel threads: fast spiking.
    FastSpiking,
    /// System services: regular spiking.
    RegularSpiking,
    /// Session apps: chattering.
    Chattering,
    /// Containers: intrinsically bursting.
    IntrinsicBursting,
}

/// The four parameters of a neuron's equations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
}

impl Firing {
    pub const ALL: [Firing; 4] = [
        Firing::FastSpiking,
        Firing::RegularSpiking,
        Firing::Chattering,
        Firing::IntrinsicBursting,
    ];

    pub fn of(kind: Kind) -> Firing {
        match kind {
            Kind::Kernel => Firing::FastSpiking,
            Kind::System => Firing::RegularSpiking,
            Kind::Session => Firing::Chattering,
            Kind::Container => Firing::IntrinsicBursting,
        }
    }

    pub fn params(self) -> Params {
        let (a, b, c, d) = match self {
            Firing::FastSpiking => (0.1, 0.2, -65.0, 2.0),
            Firing::RegularSpiking => (0.02, 0.2, -65.0, 8.0),
            Firing::Chattering => (0.02, 0.2, -50.0, 2.0),
            Firing::IntrinsicBursting => (0.02, 0.2, -55.0, 4.0),
        };
        Params { a, b, c, d }
    }

    /// The two-letter code the legend and the inspector use.
    pub fn code(self) -> &'static str {
        match self {
            Firing::FastSpiking => "FS",
            Firing::RegularSpiking => "RS",
            Firing::Chattering => "CH",
            Firing::IntrinsicBursting => "IB",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Firing::FastSpiking => "fast spiking",
            Firing::RegularSpiking => "regular spiking",
            Firing::Chattering => "chattering",
            Firing::IntrinsicBursting => "intrinsically bursting",
        }
    }

    fn index(self) -> usize {
        match self {
            Firing::FastSpiking => 0,
            Firing::RegularSpiking => 1,
            Firing::Chattering => 2,
            Firing::IntrinsicBursting => 3,
        }
    }
}

/// One Euler step of the equations for a neuron receiving `input`. A neuron that reached the
/// peak in the step before fires now: it resets, and the call returns true.
fn advance(v: &mut f32, u: &mut f32, params: Params, input: f32) -> bool {
    let fired = *v >= PEAK;
    if fired {
        *v = params.c;
        *u += params.d;
    }
    *v += STEP_MS * (0.04 * *v * *v + 5.0 * *v + 140.0 - *u + input);
    *u += STEP_MS * params.a * (params.b * *v - *u);
    *v = v.min(PEAK);
    fired
}

/// The currents that mark a neuron type: the smallest that makes it fire at least once a second,
/// and the one that makes it fire forty times a second, both measured without noise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    pub rheobase: f32,
    pub forty_hertz: f32,
}

/// Spikes per second of a neuron held at `current` without noise: it settles for one second, then
/// the spikes of the next two are counted.
pub fn steady_rate(params: Params, current: f32) -> f32 {
    let (mut v, mut u) = (REST, params.b * REST);
    let mut spikes = 0;
    for step in 0..3 * TICKS_PER_SECOND {
        if advance(&mut v, &mut u, params, current) && step >= TICKS_PER_SECOND {
            spikes += 1;
        }
    }
    spikes as f32 / 2.0
}

/// The smallest current, found to 0.001, at which `params` fires at `target` spikes a second:
/// a scan in steps of 0.5, then bisection inside the step that crosses.
fn crossing(params: Params, target: f32) -> f32 {
    let mut high = 0.0;
    while steady_rate(params, high) < target && high < 400.0 {
        high += 0.5;
    }
    let mut low = (high - 0.5).max(0.0);
    for _ in 0..9 {
        let middle = 0.5 * (low + high);
        if steady_rate(params, middle) >= target {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

/// The calibration of `firing`, measured once.
pub fn calibration(firing: Firing) -> Calibration {
    static CALIBRATIONS: OnceLock<[Calibration; 4]> = OnceLock::new();
    CALIBRATIONS.get_or_init(|| {
        Firing::ALL.map(|firing| {
            let params = firing.params();
            Calibration {
                rheobase: crossing(params, 1.0),
                forty_hertz: crossing(params, 40.0),
            }
        })
    })[firing.index()]
}

/// The current a neuron of type `firing` gets from `cpu` percent of a core, which rises with it:
/// idle is zero, a half percent is well below the rheobase, five percent just reaches it, one
/// core gives forty hertz, and three cores or more twice as far again. Straight lines between.
pub fn current(firing: Firing, cpu: f32) -> f32 {
    let Calibration {
        rheobase,
        forty_hertz,
    } = calibration(firing);
    let share = cpu / 100.0;
    if share.is_nan() || share <= 0.0 {
        return 0.0;
    }
    let shares = [0.0, 0.005, 0.05, 1.0, 3.0];
    let currents = [
        0.0,
        0.62 * rheobase,
        0.98 * rheobase,
        forty_hertz,
        forty_hertz + 2.0 * (forty_hertz - 0.98 * rheobase),
    ];
    let Some(at) = shares.iter().position(|&limit| share < limit) else {
        return currents[4];
    };
    let t = (share - shares[at - 1]) / (shares[at] - shares[at - 1]);
    currents[at - 1] + t * (currents[at] - currents[at - 1])
}

/// A spike's path from one neuron to another: it leaves `from` when that neuron fires, and `to`
/// takes `weight` of synaptic current `delay` ticks later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Synapse {
    pub from: usize,
    pub to: usize,
    pub weight: f32,
    /// Conduction time in ticks, at least one.
    pub delay: u32,
    /// The axon this synapse runs along, for the view that draws it; the network does not read it.
    pub axon: usize,
    /// Whether it runs from the axon's end to its start, for the view.
    pub backward: bool,
}

/// A spike in flight: when it leaves, when it arrives and along which synapse.
type Event = (u64, u32, u64);

/// A set of neurons whose synchrony is measured, with the average of their mean potential.
#[derive(Default)]
struct Group {
    count: usize,
    mean: f32,
    mean_square: f32,
}

/// The neurons, their synapses and the spikes in flight. A neuron lives in a slot that a later
/// neuron may take once it is removed.
pub struct Network {
    tick: u64,
    decay: f32,
    average: f32,
    params: Vec<Params>,
    v: Vec<f32>,
    u: Vec<f32>,
    drive: Vec<f32>,
    synaptic: Vec<f32>,
    incoming: Vec<f32>,
    noise: Vec<Rng>,
    live: Vec<bool>,
    spikes: Vec<Vec<u64>>,
    /// Exponential averages of each neuron's potential and its square, which give its variance.
    average_v: Vec<f32>,
    average_square: Vec<f32>,
    /// Bit `g` is set where the neuron is in group `g`.
    member: Vec<u8>,
    groups: [Group; GROUPS],
    free: Vec<usize>,
    synapses: Vec<Option<Synapse>>,
    keys: HashMap<(usize, usize), usize>,
    free_synapses: Vec<usize>,
    outgoing: Vec<Vec<usize>>,
    events: BinaryHeap<Reverse<Event>>,
    watched: Option<usize>,
    trace: VecDeque<f32>,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            tick: 0,
            decay: (-STEP_SECONDS / SYNAPSE_TAU).exp(),
            average: STEP_SECONDS / VARIANCE_TAU,
            params: Vec::new(),
            v: Vec::new(),
            u: Vec::new(),
            drive: Vec::new(),
            synaptic: Vec::new(),
            incoming: Vec::new(),
            noise: Vec::new(),
            live: Vec::new(),
            spikes: Vec::new(),
            average_v: Vec::new(),
            average_square: Vec::new(),
            member: Vec::new(),
            groups: Default::default(),
            free: Vec::new(),
            synapses: Vec::new(),
            keys: HashMap::new(),
            free_synapses: Vec::new(),
            outgoing: Vec::new(),
            events: BinaryHeap::new(),
            watched: None,
            trace: VecDeque::new(),
        }
    }
}

impl Network {
    /// Steps taken so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn is_live(&self, slot: usize) -> bool {
        self.live.get(slot).copied().unwrap_or(false)
    }

    /// Adds a neuron of type `firing` near rest, its noise and first potential from `rng`, and
    /// returns its slot.
    pub fn add(&mut self, firing: Firing, mut rng: Rng) -> usize {
        let params = firing.params();
        let v = REST + 10.0 * (rng.unit() - 0.5);
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                self.params.push(params);
                self.v.push(v);
                self.u.push(0.0);
                self.drive.push(0.0);
                self.synaptic.push(0.0);
                self.incoming.push(0.0);
                self.noise.push(Rng::default());
                self.live.push(false);
                self.spikes.push(Vec::new());
                self.average_v.push(v);
                self.average_square.push(v * v);
                self.member.push(0);
                self.outgoing.push(Vec::new());
                self.v.len() - 1
            }
        };
        self.params[slot] = params;
        self.v[slot] = v;
        self.u[slot] = params.b * v;
        self.drive[slot] = 0.0;
        self.synaptic[slot] = 0.0;
        self.incoming[slot] = 0.0;
        self.noise[slot] = rng;
        self.live[slot] = true;
        self.spikes[slot].clear();
        self.average_v[slot] = v;
        self.average_square[slot] = v * v;
        self.member[slot] = 0;
        slot
    }

    /// Removes the neuron in `slot` with the synapses that touch it and the spikes on their way.
    pub fn remove(&mut self, slot: usize) {
        if !self.is_live(slot) {
            return;
        }
        for group in 0..GROUPS {
            if self.member[slot] >> group & 1 == 1 {
                self.groups[group].count -= 1;
            }
        }
        self.member[slot] = 0;
        self.live[slot] = false;
        self.spikes[slot].clear();
        if self.watched == Some(slot) {
            self.watched = None;
            self.trace.clear();
        }
        let touching: Vec<usize> = self
            .keys
            .iter()
            .filter(|((from, to), _)| *from == slot || *to == slot)
            .map(|(_, &index)| index)
            .collect();
        self.drop_synapses(&touching);
        self.rebuild_outgoing();
        self.free.push(slot);
    }

    pub fn set_drive(&mut self, slot: usize, current: f32) {
        self.drive[slot] = current;
    }

    pub fn drive(&self, slot: usize) -> f32 {
        self.drive[slot]
    }

    pub fn firing_params(&self, slot: usize) -> Params {
        self.params[slot]
    }

    /// Makes the synapses exactly `wanted`: one per (from, to), the old ones kept with their spikes
    /// in flight where they still exist, the others removed with theirs.
    pub fn set_synapses(&mut self, wanted: &[Synapse]) {
        let keep: HashMap<(usize, usize), &Synapse> = wanted
            .iter()
            .map(|synapse| ((synapse.from, synapse.to), synapse))
            .collect();
        let stale: Vec<usize> = self
            .keys
            .iter()
            .filter(|(key, _)| !keep.contains_key(*key))
            .map(|(_, &index)| index)
            .collect();
        self.drop_synapses(&stale);
        for synapse in wanted {
            let key = (synapse.from, synapse.to);
            match self.keys.get(&key) {
                Some(&index) => self.synapses[index] = Some(*synapse),
                None => {
                    let index = match self.free_synapses.pop() {
                        Some(index) => index,
                        None => {
                            self.synapses.push(None);
                            self.synapses.len() - 1
                        }
                    };
                    self.synapses[index] = Some(*synapse);
                    self.keys.insert(key, index);
                }
            }
        }
        self.rebuild_outgoing();
    }

    fn drop_synapses(&mut self, indices: &[usize]) {
        if indices.is_empty() {
            return;
        }
        for &index in indices {
            if let Some(synapse) = self.synapses[index].take() {
                self.keys.remove(&(synapse.from, synapse.to));
                self.free_synapses.push(index);
            }
        }
        let synapses = &self.synapses;
        self.events
            .retain(|Reverse((_, synapse, _))| synapses[*synapse as usize].is_some());
    }

    /// Lists each neuron's synapses in index order, so a spike leaves along them the same way
    /// every run.
    fn rebuild_outgoing(&mut self) {
        for list in &mut self.outgoing {
            list.clear();
        }
        for (index, synapse) in self.synapses.iter().enumerate() {
            if let Some(synapse) = synapse {
                self.outgoing[synapse.from].push(index);
            }
        }
    }

    /// Measures the synchrony of group `group` over the neurons in `members`, which replace its
    /// members. A group that had none starts its average at the members' present potential.
    pub fn set_group(&mut self, group: usize, members: &[usize]) {
        for bits in &mut self.member {
            *bits &= !(1 << group);
        }
        for &slot in members {
            self.member[slot] |= 1 << group;
        }
        let state = &mut self.groups[group];
        let started = state.count == 0 && !members.is_empty();
        state.count = members.len();
        if started {
            let mean = members.iter().map(|&slot| self.v[slot]).sum::<f32>() / members.len() as f32;
            state.mean = mean;
            state.mean_square = mean * mean;
        }
    }

    /// Records the membrane potential of `slot`, 640 steps of it, from now on; None stops.
    pub fn watch(&mut self, slot: Option<usize>) {
        if self.watched != slot {
            self.watched = slot;
            self.trace.clear();
        }
    }

    /// The watched neuron's potentials, oldest first, at most `TRACE_TICKS` of them.
    pub fn trace(&self) -> &VecDeque<f32> {
        &self.trace
    }

    /// Takes one step: spikes that arrive add their weight, every neuron integrates, those that
    /// fire send their spikes down their synapses.
    pub fn step(&mut self) {
        self.tick += 1;
        let tick = self.tick;
        while let Some(&Reverse((arrive, synapse, _))) = self.events.peek() {
            if arrive > tick {
                break;
            }
            self.events.pop();
            if let Some(synapse) = &self.synapses[synapse as usize] {
                self.incoming[synapse.to] += synapse.weight;
            }
        }
        let mut sums = [0.0_f32; GROUPS];
        for slot in 0..self.v.len() {
            if !self.live[slot] {
                continue;
            }
            self.synaptic[slot] = self.synaptic[slot] * self.decay + self.incoming[slot];
            self.incoming[slot] = 0.0;
            let drive = self.drive[slot];
            let noise = if drive > 0.0 {
                NOISE * self.noise[slot].normal()
            } else {
                0.0
            };
            let fired = advance(
                &mut self.v[slot],
                &mut self.u[slot],
                self.params[slot],
                drive + self.synaptic[slot] + noise,
            );
            if fired {
                self.spikes[slot].push(tick);
                for &index in &self.outgoing[slot] {
                    if self.events.len() >= MAX_EVENTS {
                        break;
                    }
                    if let Some(synapse) = &self.synapses[index] {
                        let arrive = tick + u64::from(synapse.delay);
                        self.events.push(Reverse((arrive, index as u32, tick)));
                    }
                }
            }
            let shown = if fired { PEAK } else { self.v[slot] };
            self.average_v[slot] += self.average * (shown - self.average_v[slot]);
            self.average_square[slot] += self.average * (shown * shown - self.average_square[slot]);
            let bits = self.member[slot];
            if bits != 0 {
                for (group, sum) in sums.iter_mut().enumerate() {
                    if bits >> group & 1 == 1 {
                        *sum += shown;
                    }
                }
            }
            if self.watched == Some(slot) {
                if self.trace.len() == TRACE_TICKS {
                    self.trace.pop_front();
                }
                self.trace.push_back(shown);
            }
        }
        for (group, sum) in self.groups.iter_mut().zip(sums) {
            if group.count > 0 {
                let mean = sum / group.count as f32;
                group.mean += self.average * (mean - group.mean);
                group.mean_square += self.average * (mean * mean - group.mean_square);
            }
        }
    }

    /// Golomb's synchrony measure of group `group`: the square root of the variance over time of
    /// the group's mean potential over the mean over its neurons of their variance over time, all
    /// from averages over about two seconds. 0 for neurons that never agree, 1 for neurons that
    /// always do; None for fewer than two neurons or for neurons that hardly vary.
    pub fn synchrony(&self, group: usize) -> Option<f32> {
        let state = &self.groups[group];
        if state.count < 2 {
            return None;
        }
        let mut sum = 0.0;
        for slot in 0..self.v.len() {
            if self.live[slot] && self.member[slot] >> group & 1 == 1 {
                sum += self.average_square[slot] - self.average_v[slot] * self.average_v[slot];
            }
        }
        let own = sum / state.count as f32;
        if own < 1.0 {
            return None;
        }
        let shared = (state.mean_square - state.mean * state.mean).max(0.0);
        Some((shared / own).sqrt().min(1.0))
    }

    pub fn group_size(&self, group: usize) -> usize {
        self.groups[group].count
    }

    /// The ticks at which the neuron in `slot` fired, oldest first, since the last `trim`.
    pub fn spikes(&self, slot: usize) -> &[u64] {
        &self.spikes[slot]
    }

    /// Spikes per second of `slot` over the last `seconds`.
    pub fn rate(&self, slot: usize, seconds: f32) -> f32 {
        let from = self
            .tick
            .saturating_sub((seconds * TICKS_PER_SECOND as f32) as u64);
        let spikes = &self.spikes[slot];
        let recent = spikes.len() - spikes.partition_point(|&tick| tick <= from);
        recent as f32 / seconds
    }

    /// Forgets the spikes older than `ticks` ago.
    pub fn trim(&mut self, ticks: u64) {
        let from = self.tick.saturating_sub(ticks);
        for spikes in &mut self.spikes {
            let old = spikes.partition_point(|&tick| tick < from);
            spikes.drain(..old);
        }
    }

    /// The synaptic current `slot` carries now.
    #[cfg(test)]
    pub fn synaptic(&self, slot: usize) -> f32 {
        self.synaptic[slot]
    }

    /// The synapses that end in `slot`.
    pub fn synapses_into(&self, slot: usize) -> impl Iterator<Item = &Synapse> {
        self.synapses
            .iter()
            .flatten()
            .filter(move |synapse| synapse.to == slot)
    }

    /// The spikes in flight: where each left, when it arrives, and along which synapse. In the
    /// queue's own order, which is the same every run.
    pub fn in_flight(&self) -> impl Iterator<Item = (u64, u64, &Synapse)> {
        self.events
            .iter()
            .filter_map(|Reverse((arrive, index, emit))| {
                self.synapses[*index as usize]
                    .as_ref()
                    .map(|synapse| (*emit, *arrive, synapse))
            })
    }

    pub fn flights(&self) -> usize {
        self.events.len()
    }
}

/// A membrane trace as braille: `rows` lines of `cells` cells, each two dot columns wide and four
/// dot rows tall, the newest potential at the right end. Each dot column covers an equal share
/// of the 640 steps and draws the span from its lowest to its highest potential, on a scale
/// from -80 to +30 mV. Cells with no dot are spaces, and a trace shorter than the window starts
/// part-way across.
pub fn sparkline(trace: &VecDeque<f32>, cells: usize, rows: usize) -> Vec<String> {
    let columns = cells * 2;
    let levels = rows * 4;
    let per_column = (TRACE_TICKS / columns).max(1);
    let missing = TRACE_TICKS.saturating_sub(trace.len());
    let mut dots = vec![0_u8; cells * rows];
    let level = |v: f32| {
        let t = ((v + 80.0) / 110.0).clamp(0.0, 1.0);
        (t * (levels - 1) as f32).round() as usize
    };
    for column in 0..columns {
        let from = (column * per_column).saturating_sub(missing);
        let until = ((column + 1) * per_column).saturating_sub(missing);
        if until <= from || from >= trace.len() {
            continue;
        }
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for &v in trace.range(from..until.min(trace.len())) {
            low = low.min(v);
            high = high.max(v);
        }
        for step in level(low)..=level(high) {
            // Row 0 is the top dot row.
            let row = levels - 1 - step;
            let (line, sub) = (row / 4, row % 4);
            let bit = match (column % 2, sub) {
                (0, 0) => 0,
                (0, 1) => 1,
                (0, 2) => 2,
                (0, _) => 6,
                (_, 0) => 3,
                (_, 1) => 4,
                (_, 2) => 5,
                (_, _) => 7,
            };
            dots[line * cells + column / 2] |= 1 << bit;
        }
    }
    (0..rows)
        .map(|line| {
            dots[line * cells..(line + 1) * cells]
                .iter()
                .map(|&mask| {
                    if mask == 0 {
                        ' '
                    } else {
                        char::from_u32(0x2800 + u32::from(mask)).unwrap_or(' ')
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A network of one neuron of type `firing` held at `current`, seeded by `seed`.
    fn lone(firing: Firing, current: f32, seed: u64) -> Network {
        let mut network = Network::default();
        let slot = network.add(firing, Rng::new(seed));
        network.set_drive(slot, current);
        network
    }

    fn run(network: &mut Network, seconds: f32) {
        for _ in 0..(seconds * TICKS_PER_SECOND as f32) as u64 {
            network.step();
        }
    }

    /// The intervals between a neuron's spikes, in milliseconds, after its first second.
    fn intervals(network: &Network, slot: usize) -> Vec<f32> {
        let spikes: Vec<u64> = network
            .spikes(slot)
            .iter()
            .copied()
            .filter(|&tick| tick >= TICKS_PER_SECOND)
            .collect();
        spikes
            .windows(2)
            .map(|pair| (pair[1] - pair[0]) as f32 * 0.5)
            .collect()
    }

    #[test]
    fn the_f_i_curve_is_monotonic_for_every_neuron_type() {
        for firing in Firing::ALL {
            // Over the currents CPU can ask for, up to three cores, rates only rise, give or take
            // what counting the spikes of a burster over a two-second window costs: a hertz and
            // a twentieth.
            let params = firing.params();
            let most = current(firing, 300.0);
            let mut last = 0.0;
            for step in 0..=(most * 4.0) as u32 {
                let at = step as f32 * 0.25;
                let rate = steady_rate(params, at);
                assert!(
                    rate + 1.0 >= 0.95 * last,
                    "{firing:?} fell from {last} to {rate} Hz at I = {at}"
                );
                last = last.max(rate);
            }
            assert!(last > 40.0, "{firing:?} reaches {last} Hz");
            let mut last = 0.0;
            for cpu in 0..=400 {
                let drive = current(firing, cpu as f32);
                assert!(drive >= last, "{firing:?} lost current at {cpu}% CPU");
                last = drive;
            }
        }
    }

    #[test]
    fn an_idle_neuron_stays_silent() {
        for firing in Firing::ALL {
            let mut network = lone(firing, current(firing, 0.0), 7);
            run(&mut network, 20.0);
            assert!(network.spikes(0).is_empty(), "{firing:?} fired idle");
        }
    }

    #[test]
    fn five_percent_cpu_fires_occasionally_and_one_core_fires_near_forty_hertz() {
        let rate_at = |firing: Firing, cpu: f32| {
            let mut network = lone(firing, current(firing, cpu), 11);
            run(&mut network, 20.0);
            network.rate(0, 18.0)
        };
        for firing in Firing::ALL {
            let (faint, light, core) = (
                rate_at(firing, 0.5),
                rate_at(firing, 5.0),
                rate_at(firing, 100.0),
            );
            assert!(faint < 1.0, "{firing:?} at half a percent fires {faint} Hz");
            assert!(
                0.0 < light && light < 20.0 && light < 0.5 * core,
                "{firing:?} at 5% CPU fires {light} Hz, at one core {core}"
            );
            assert!(
                (32.0..52.0).contains(&core),
                "{firing:?} at one core fires {core} Hz"
            );
        }
    }

    #[test]
    fn a_spike_arrives_after_the_axon_conduction_delay() {
        let mut network = Network::default();
        let sender = network.add(Firing::RegularSpiking, Rng::new(1));
        let receiver = network.add(Firing::RegularSpiking, Rng::new(2));
        network.set_synapses(&[Synapse {
            from: sender,
            to: receiver,
            weight: 1.5,
            delay: 240,
            axon: 0,
            backward: false,
        }]);
        network.v[sender] = PEAK;
        network.step();
        let fired = network.tick();
        assert_eq!(network.spikes(sender), [fired]);
        assert_eq!(network.flights(), 1);
        let (emit, arrive, _) = network.in_flight().next().expect("one spike in flight");
        assert_eq!((emit, arrive), (fired, fired + 240));
        for _ in 1..240 {
            network.step();
            assert_eq!(network.synaptic(receiver), 0.0, "tick {}", network.tick());
        }
        network.step();
        assert_eq!(
            network.synaptic(receiver),
            1.5,
            "the spike lands on tick {arrive}"
        );
        assert_eq!(network.flights(), 0);
        network.step();
        assert!(network.synaptic(receiver) < 1.5, "and then decays");
        // A neuron removed with a spike on its way takes the spike with it.
        network.v[sender] = PEAK;
        network.step();
        assert_eq!(network.flights(), 1);
        network.remove(receiver);
        assert_eq!(network.flights(), 0);
    }

    #[test]
    fn regular_spiking_adapts_and_chattering_bursts_by_isi_statistics() {
        let regular = Firing::RegularSpiking;
        // Without the settling second the first intervals are the ones to compare.
        let params = regular.params();
        let (mut v, mut u) = (REST, params.b * REST);
        let mut ticks = Vec::new();
        for tick in 0..2 * TICKS_PER_SECOND {
            if advance(&mut v, &mut u, params, 14.0) {
                ticks.push(tick);
            }
        }
        let gaps: Vec<u64> = ticks.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(gaps.len() >= 8, "{} gaps", gaps.len());
        assert!(
            *gaps.last().expect("gaps") as f32 > 1.5 * gaps[0] as f32,
            "regular spiking slows down: {gaps:?}"
        );

        // Chattering fires bursts: short intervals inside a burst, long ones between bursts.
        let chatter = Firing::Chattering;
        let mut network = lone(chatter, current(chatter, 100.0), 5);
        run(&mut network, 6.0);
        let gaps = intervals(&network, 0);
        let short = gaps.iter().filter(|&&gap| gap < 12.0).count();
        let long = gaps.iter().filter(|&&gap| gap > 30.0).count();
        assert!(
            short >= 8 && long >= 4,
            "{short} short and {long} long in {gaps:?}"
        );
    }

    #[test]
    fn fast_spiking_and_intrinsic_bursting_match_their_isi_statistics() {
        // Fast spiking is tonic with no adaptation: the intervals hardly vary.
        let fast = Firing::FastSpiking;
        let mut network = lone(fast, current(fast, 100.0), 5);
        run(&mut network, 6.0);
        let gaps = intervals(&network, 0);
        let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
        let spread =
            (gaps.iter().map(|g| (g - mean).powi(2)).sum::<f32>() / gaps.len() as f32).sqrt();
        assert!(
            gaps.len() > 100 && spread / mean < 0.45,
            "mean {mean} ms, spread {spread}"
        );

        // Intrinsically bursting opens with a burst, then settles to a slower tonic rhythm.
        let params = Firing::IntrinsicBursting.params();
        let (mut v, mut u) = (REST, params.b * REST);
        let mut ticks = Vec::new();
        for tick in 0..2 * TICKS_PER_SECOND {
            if advance(&mut v, &mut u, params, 16.0) {
                ticks.push(tick);
            }
        }
        let gaps: Vec<f32> = ticks
            .windows(2)
            .map(|pair| (pair[1] - pair[0]) as f32 * 0.5)
            .collect();
        let (early, late) = (
            gaps[..3].iter().sum::<f32>() / 3.0,
            gaps[gaps.len() - 3..].iter().sum::<f32>() / 3.0,
        );
        assert!(
            early * 1.5 < late,
            "burst {early} ms then rhythm {late} ms: {gaps:?}"
        );
    }

    /// `count` regular-spiking neurons at one core of CPU, wired all to all with `weight` and a
    /// delay of 20 ms unless `weight` is 0; the synchrony of all of them after `seconds`.
    fn chi_of(count: usize, weight: f32, seconds: f32) -> f32 {
        let mut network = Network::default();
        let firing = Firing::RegularSpiking;
        let slots: Vec<usize> = (0..count)
            .map(|index| {
                let slot = network.add(firing, Rng::new(100 + index as u64));
                network.set_drive(slot, current(firing, 100.0));
                slot
            })
            .collect();
        let mut synapses = Vec::new();
        if weight > 0.0 {
            for &from in &slots {
                for &to in &slots {
                    if from != to {
                        synapses.push(Synapse {
                            from,
                            to,
                            weight,
                            delay: 40,
                            axon: 0,
                            backward: false,
                        });
                    }
                }
            }
        }
        network.set_synapses(&synapses);
        network.set_group(0, &slots);
        run(&mut network, seconds);
        network
            .synchrony(0)
            .expect("neurons that vary have a measure")
    }

    #[test]
    fn talking_busy_neurons_synchronize_more_than_unlinked_ones() {
        let alone = chi_of(8, 0.0, 12.0);
        let talking = chi_of(8, 2.0, 12.0);
        // Eight independent neurons agree by chance as 1 over the root of eight, a third.
        assert!((0.2..0.5).contains(&alone), "unlinked chi {alone}");
        assert!(talking > 0.8, "linked chi {talking}");
    }

    #[test]
    fn the_same_steps_give_the_same_spikes_whoever_asked_for_them() {
        let spikes = |chunks: u64| {
            let mut network = lone(Firing::Chattering, current(Firing::Chattering, 40.0), 9);
            for _ in 0..chunks {
                for _ in 0..20_000 / chunks {
                    network.step();
                }
            }
            network.spikes(0).to_vec()
        };
        assert_eq!(spikes(1), spikes(10));
        assert!(!spikes(1).is_empty());
    }

    #[test]
    fn a_synapse_set_keeps_the_spikes_of_the_synapses_it_keeps() {
        let mut network = Network::default();
        let slots: Vec<usize> = (0..3)
            .map(|index| network.add(Firing::FastSpiking, Rng::new(index)))
            .collect();
        let synapse = |from: usize, to: usize, delay: u32| Synapse {
            from,
            to,
            weight: 1.0,
            delay,
            axon: 0,
            backward: false,
        };
        network.set_synapses(&[
            synapse(slots[0], slots[1], 100),
            synapse(slots[0], slots[2], 100),
        ]);
        network.v[slots[0]] = PEAK;
        network.step();
        assert_eq!(network.flights(), 2);
        network.set_synapses(&[synapse(slots[0], slots[1], 300)]);
        assert_eq!(
            network.flights(),
            1,
            "the dropped synapse took its spike with it"
        );
        let (_, arrive, kept) = network.in_flight().next().expect("one left");
        assert_eq!(
            (arrive, kept.delay),
            (101, 300),
            "the kept spike keeps its own arrival"
        );
    }

    #[test]
    fn calibration_reaches_one_hertz_and_forty() {
        for firing in Firing::ALL {
            let Calibration {
                rheobase,
                forty_hertz,
            } = calibration(firing);
            assert!(0.0 < rheobase && rheobase < forty_hertz, "{firing:?}");
            assert!(steady_rate(firing.params(), rheobase) >= 1.0);
            assert!(steady_rate(firing.params(), rheobase - 0.05) < 1.0);
            assert!(steady_rate(firing.params(), forty_hertz) >= 40.0);
        }
    }

    #[test]
    fn a_sparkline_draws_the_span_of_each_column_and_blanks_what_is_missing() {
        let mut trace: VecDeque<f32> = VecDeque::new();
        // A flat trace at rest, then a spike at the newest end.
        for _ in 0..TRACE_TICKS - 8 {
            trace.push_back(-80.0);
        }
        for _ in 0..8 {
            trace.push_back(30.0);
        }
        let lines = sparkline(&trace, 40, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.chars().count() == 40));
        // The last column spans the full range, so both lines have dots there: the bottom line at
        // the bottom dots for the rest and the spike's span passes through the top line.
        assert_ne!(lines[0].chars().last(), Some(' '));
        assert_ne!(lines[1].chars().last(), Some(' '));
        assert_eq!(
            lines[0].chars().next(),
            Some(' '),
            "nothing above rest before the spike"
        );
        assert_ne!(
            lines[1].chars().next(),
            Some(' '),
            "rest sits at the bottom"
        );
        // A short trace starts part-way across.
        let short: VecDeque<f32> = std::iter::repeat_n(-65.0, 64).collect();
        let lines = sparkline(&short, 40, 2);
        assert!(lines[1].starts_with(' ') && !lines[1].ends_with(' '));
        assert!(
            sparkline(&VecDeque::new(), 40, 2)
                .iter()
                .all(|line| line.trim().is_empty())
        );
        for line in lines {
            assert!(
                line.chars()
                    .all(|c| c == ' ' || ('\u{2800}'..='\u{28ff}').contains(&c))
            );
        }
    }

    #[test]
    fn kinds_map_to_the_four_firing_patterns() {
        use crate::model::Kind::*;
        assert_eq!(Firing::of(Kernel), Firing::FastSpiking);
        assert_eq!(Firing::of(System), Firing::RegularSpiking);
        assert_eq!(Firing::of(Session), Firing::Chattering);
        assert_eq!(Firing::of(Container), Firing::IntrinsicBursting);
    }
}
