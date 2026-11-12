//! What the die makes of a sample, pure of drawing: which topology it draws, how each core block's
//! temperature is known (its own sensor, a core complex's, the package's, or not at all), each
//! block's power proxy, the sensors' nudges, the 60 s series the panel plots and the throttle
//! counters it watches.
//!
//! Nothing here invents a temperature. A block no sensor speaks for has `Coverage::Unknown`, and
//! when none does the die is in `Mode::Relative`: its field is the model's, in units of its own,
//! and is never read as degrees. A power proxy needs a load reading (`Cpu::busy`) and is None
//! without one, so a core nobody measured is not drawn as an idle one.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};

use super::floorplan::{Block, Floorplan};
use super::heat::{self, Nudge};
use crate::model::thermal::{CpuPlace, SensorPlace, Thermal, Topology};
use crate::model::{Cpu, Measured, Snapshot};

/// The temperature the colour scale ends at when no sensor reports TjMax, in degrees Celsius.
pub const TJ_MAX_DEFAULT: f32 = 100.0;
/// The coolest end of the colour scale in degrees, and how far above TjMax the hottest is.
pub const SCALE_LOW: f32 = 30.0;
pub const SCALE_HEADROOM: f32 = 5.0;
/// The width of the colour scale in relative mode: the model's own units, as many as the
/// default scale has degrees (30 to 105).
pub const RELATIVE_SPAN: f32 = 75.0;
/// How far below the coolest sensor reading of the last minute the model's ambient is, in kelvin.
pub const AMBIENT_MARGIN: f32 = 5.0;
/// The seconds of history the panel's sparklines and the ambient look back over.
pub const WINDOW: f64 = 60.0;
/// How long one throttle increment flashes a core's outline, in seconds.
pub const FLASH: f32 = 1.4;
/// The seconds over which the RAPL scale of the heat source relaxes to a new reading.
pub const RAPL_TIME: f64 = 2.0;
/// What the model counts a full P-core at proxy 1 as drawing, and an E-core, in watts, as the
/// prototype's blocks do; and what the rest of the package (ring, I/O, leakage) adds.
const P_WATTS: f32 = 10.5;
const E_WATTS: f32 = 3.6;
const REST_WATTS: f32 = 2.2;

/// How the temperature of a core block is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Coverage {
    /// The core's own sensor.
    Own(f32),
    /// The sensor of the core complex (CCD) the core is in: one reading for all of its cores.
    Ccd(u32, f32),
    /// The package's sensor, when no sensor is any finer.
    Package(f32),
    /// No sensor speaks for it: its colour is the model's.
    Unknown,
}

impl Coverage {
    pub fn celsius(self) -> Option<f32> {
        match self {
            Coverage::Own(celsius) | Coverage::Ccd(_, celsius) | Coverage::Package(celsius) => {
                Some(celsius)
            }
            Coverage::Unknown => None,
        }
    }
}

/// Whether the field is in degrees or in the model's own units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// No sensor reads: the colour is the model's relative heat, not a temperature.
    #[default]
    Relative,
    /// At least one sensor reads: the scale is degrees Celsius.
    Celsius,
}

/// The topology to draw: the sample's, or, where it has none (Linux without sysfs topology, a
/// machine that was not asked), one made of the CPU list, each CPU a core of its own.
pub fn topology_of(snapshot: &Snapshot) -> Cow<'_, Topology> {
    match &snapshot.thermal {
        Some(thermal) if !thermal.topology.is_empty() => Cow::Borrowed(&*thermal.topology),
        _ => {
            let mut cpus: Vec<CpuPlace> = snapshot
                .cpus
                .iter()
                .map(|cpu| CpuPlace {
                    cpu: cpu.id,
                    package: None,
                    die: None,
                    core_id: None,
                    cluster: None,
                    siblings: Vec::new(),
                    l2: None,
                    l3: None,
                    max_mhz: None,
                    kind: cpu.kind,
                })
                .collect();
            cpus.sort_by_key(|place| place.cpu);
            Cow::Owned(Topology { cpus })
        }
    }
}

/// What one sample says about each block of the floorplan.
#[derive(Clone, Debug, PartialEq)]
pub struct Readings {
    pub mode: Mode,
    /// How each block's temperature is known, in block order.
    pub coverage: Vec<Coverage>,
    /// Each block's power proxy `busy * (mhz / fmax)^3`, None where no CPU of the block has a
    /// load reading.
    pub proxies: Vec<Option<f32>>,
    pub tj_max: f32,
    pub tj_max_known: bool,
    /// Blocks whose proxy had to go without a clock (`busy` alone).
    pub without_clock: usize,
    /// CCD sensors that read but could not be tied to cores (the cache groups do not say).
    pub unmapped_ccds: usize,
    /// The reading of the drawn package's own sensor, else of the hottest package sensor.
    pub package_celsius: Option<f32>,
    /// The coolest reading of any sensor of the sample.
    pub coolest: Option<f32>,
    pub watts: Measured<f32>,
}

/// The block of `plan` that a core sensor (`package`, `core_id`) reads.
fn block_of_core(plan: &Floorplan, package: u32, core_id: u32) -> Option<usize> {
    plan.blocks.iter().position(|block| {
        block.package.is_none_or(|found| found == package) && block.core_id == Some(core_id)
    })
}

impl Readings {
    /// Reads `snapshot` for `plan`. `running_max` keeps, per block, the highest clock seen, which
    /// stands in for `cpuinfo_max_freq` where the topology has none.
    pub fn of(plan: &Floorplan, snapshot: &Snapshot, running_max: &mut [f32]) -> Readings {
        let thermal = snapshot.thermal.as_ref();
        let mut coverage = vec![Coverage::Unknown; plan.blocks.len()];
        let mut unmapped_ccds = 0;
        if let Some(thermal) = thermal {
            for sensor in &thermal.sensors {
                let Some(celsius) = sensor.celsius else {
                    continue;
                };
                match sensor.place {
                    SensorPlace::Core { package, core_id } => {
                        if let Some(block) = block_of_core(plan, package, core_id) {
                            coverage[block] = Coverage::Own(celsius);
                        }
                    }
                    SensorPlace::Ccd(number) => match thermal.ccd_cpus(number) {
                        Some(cpus) => {
                            for (index, block) in plan.blocks.iter().enumerate() {
                                let inside = block.cpus.iter().all(|cpu| cpus.contains(cpu));
                                if inside && coverage[index] == Coverage::Unknown {
                                    coverage[index] = Coverage::Ccd(number, celsius);
                                }
                            }
                        }
                        None => unmapped_ccds += 1,
                    },
                    SensorPlace::Package(_) => {}
                }
            }
        }
        let package_celsius = thermal.and_then(|thermal| {
            plan.package
                .and_then(|package| thermal.package_celsius(package))
                .or_else(|| {
                    thermal
                        .sensors
                        .iter()
                        .filter(|sensor| matches!(sensor.place, SensorPlace::Package(_)))
                        .filter_map(|sensor| sensor.celsius)
                        .reduce(f32::max)
                })
        });
        // A package sensor speaks for the whole die only when nothing is any finer: beside
        // per-core sensors it is just the hottest of them.
        let finer = coverage.iter().any(|found| *found != Coverage::Unknown);
        if !finer && let Some(celsius) = package_celsius {
            coverage.fill(Coverage::Package(celsius));
        }
        let mode = if coverage.iter().any(|found| *found != Coverage::Unknown) {
            Mode::Celsius
        } else {
            Mode::Relative
        };
        let cpus: HashMap<u32, &Cpu> = snapshot.cpus.iter().map(|cpu| (cpu.id, cpu)).collect();
        let mut without_clock = 0;
        let proxies = plan
            .blocks
            .iter()
            .zip(running_max.iter_mut())
            .map(|(block, seen)| {
                let (proxy, clock) = proxy(block, &cpus, seen);
                without_clock += usize::from(proxy.is_some() && !clock);
                proxy
            })
            .collect();
        let tj_max = thermal.and_then(Thermal::tj_max);
        Readings {
            mode,
            coverage,
            proxies,
            tj_max: tj_max.unwrap_or(TJ_MAX_DEFAULT),
            tj_max_known: tj_max.is_some(),
            without_clock,
            unmapped_ccds,
            package_celsius,
            coolest: thermal.and_then(|thermal| {
                thermal
                    .sensors
                    .iter()
                    .filter_map(|sensor| sensor.celsius)
                    .reduce(f32::min)
            }),
            watts: thermal.map_or(Measured::Pending, |thermal| thermal.package_watts.clone()),
        }
    }

    /// The colour scale: its coolest and hottest values, in degrees or in model units.
    pub fn scale(&self) -> (f32, f32) {
        match self.mode {
            Mode::Celsius => (SCALE_LOW, self.tj_max + SCALE_HEADROOM),
            Mode::Relative => (0.0, RELATIVE_SPAN),
        }
    }

    /// Where `value` sits on the colour scale, 0 to 1 (unclamped).
    pub fn level(&self, value: f32) -> f32 {
        let (low, high) = self.scale();
        (value - low) / (high - low)
    }
}

/// The power proxy of `block`: the cores' load, `busy`, times the cube of the clock over the
/// highest clock. The busy shares of SMT siblings add up to at most 1 (a core is busy when either
/// thread is). The highest clock is the topology's `cpuinfo_max_freq`, else the highest the block
/// has run at (`seen`, which this updates). Without a clock reading the proxy is `busy` alone
/// (the second value is false then). None where no CPU of the block has a load reading.
fn proxy(block: &Block, cpus: &HashMap<u32, &Cpu>, seen: &mut f32) -> (Option<f32>, bool) {
    let mut busy: Option<f32> = None;
    let mut clock: Option<f32> = None;
    for cpu in block.cpus.iter().filter_map(|cpu| cpus.get(cpu)) {
        if let Some(share) = cpu.busy {
            busy = Some(busy.unwrap_or(0.0) + share);
        }
        if let Some(mhz) = cpu.mhz {
            clock = Some(clock.map_or(mhz, |other: f32| other.max(mhz)));
        }
    }
    let Some(busy) = busy.map(|busy| busy.clamp(0.0, 1.0)) else {
        return (None, clock.is_some());
    };
    if let Some(mhz) = clock {
        *seen = seen.max(mhz);
    }
    let top = block.max_mhz.unwrap_or(*seen);
    match clock {
        Some(mhz) if top > 0.0 => (Some(busy * (mhz / top).clamp(0.0, 1.2).powi(3)), true),
        _ => (Some(busy), false),
    }
}

/// How hard a block of this role draws at proxy 1, in watts, as the model counts: the prototype's
/// 10.5 W for a P-core and 3.6 W for an E-core.
pub fn nominal_watts(block: &Block) -> f32 {
    match block.role {
        super::floorplan::Role::Efficiency | super::floorplan::Role::LowPower => E_WATTS,
        _ => P_WATTS,
    }
}

/// How the heat of a block scales with its role against the calibrated P-core.
pub fn weight(block: &Block) -> f32 {
    nominal_watts(block) / P_WATTS
}

/// The model's total at these proxies, in watts, the cores' share only.
pub fn model_watts(plan: &Floorplan, proxies: &[Option<f32>]) -> f32 {
    plan.blocks
        .iter()
        .zip(proxies)
        .filter_map(|(block, proxy)| proxy.map(|proxy| proxy * nominal_watts(block)))
        .sum()
}

/// The factor that scales the model's source to the package's measured power: the measured watts
/// less the rest of the package over the model's core watts, relaxed with `RAPL_TIME`. 1 until a
/// reading comes, and held where the model's watts are too few to divide by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RaplScale {
    value: f32,
    at: Option<f64>,
}

impl Default for RaplScale {
    fn default() -> Self {
        Self {
            value: 1.0,
            at: None,
        }
    }
}

impl RaplScale {
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Takes in a new measured reading at `elapsed`.
    pub fn take(&mut self, measured: f32, model: f32, elapsed: f64) {
        // Below a watt of modelled load the ratio is noise: hold the last scale.
        if model < 1.0 {
            self.at = Some(elapsed);
            return;
        }
        let target = ((measured - REST_WATTS).max(0.0) / model).clamp(0.2, 5.0);
        let weight = match self.at {
            Some(before) if elapsed > before => {
                1.0 - (-(elapsed - before) / RAPL_TIME).exp() as f32
            }
            Some(_) => 0.0,
            None => 1.0,
        };
        self.value += weight * (target - self.value);
        self.at = Some(elapsed);
    }
}

/// One sample's point of the panel's series.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub elapsed: f64,
    pub celsius: Option<f32>,
    pub watts: Option<f32>,
    pub coolest: Option<f32>,
}

/// The last `WINDOW` seconds of package temperature and power, one point per sample.
#[derive(Clone, Debug, Default)]
pub struct Series {
    points: VecDeque<Point>,
}

impl Series {
    /// Adds `point` if its sample is newer than the last; false when it is not (a repeat, or a
    /// rewind), which is how a sample is taken in once.
    pub fn push(&mut self, point: Point) -> bool {
        if self
            .points
            .back()
            .is_some_and(|last| point.elapsed <= last.elapsed)
        {
            return false;
        }
        self.points.push_back(point);
        while self
            .points
            .front()
            .is_some_and(|first| first.elapsed < point.elapsed - WINDOW)
        {
            self.points.pop_front();
        }
        true
    }

    pub fn points(&self) -> impl Iterator<Item = &Point> {
        self.points.iter()
    }

    pub fn last(&self) -> Option<&Point> {
        self.points.back()
    }

    /// The model's ambient, in degrees: `AMBIENT_MARGIN` under the coolest sensor reading of the
    /// window. None while no sensor has read.
    pub fn ambient(&self) -> Option<f32> {
        self.points
            .iter()
            .filter_map(|point| point.coolest)
            .reduce(f32::min)
            .map(|coolest| coolest - AMBIENT_MARGIN)
    }
}

/// A throttle count's growth seen while watching.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Throttled {
    /// The animation time of the latest increment seen.
    pub since: f32,
    /// Increments seen since watching began (the count itself is since boot).
    pub seen: u64,
}

/// The series and the throttle counters, fed once per sample.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    pub series: Series,
    last_pass: Option<u64>,
    counts: HashMap<(u32, u32), u64>,
    package_counts: HashMap<u32, u64>,
    /// By (package, core id).
    pub cores: HashMap<(u32, u32), Throttled>,
    pub packages: HashMap<u32, Throttled>,
    /// The RAPL scale of the heat source.
    pub scale: RaplScale,
}

impl Watch {
    /// Takes in `snapshot` (shown at animation time `time`) once: a sample that is not newer
    /// than the last, or that has no thermal block, changes nothing. A reading counts as new when
    /// its pass number is, so the samples between two background passes, which carry the same
    /// block, add points to the series and no throttle increments. A throttle count that rises
    /// between passes marks its core (or package); one that falls, or has no earlier reading, is
    /// only taken as the new baseline.
    pub fn ingest(&mut self, snapshot: &Snapshot, time: f32, plan: &Floorplan) -> bool {
        let Some(thermal) = &snapshot.thermal else {
            return false;
        };
        let package = plan.package;
        let point = Point {
            elapsed: snapshot.elapsed,
            celsius: package
                .and_then(|package| thermal.package_celsius(package))
                .or_else(|| {
                    thermal
                        .sensors
                        .iter()
                        .filter(|sensor| matches!(sensor.place, SensorPlace::Package(_)))
                        .filter_map(|sensor| sensor.celsius)
                        .reduce(f32::max)
                }),
            watts: thermal.package_watts.known().copied(),
            coolest: thermal
                .sensors
                .iter()
                .filter_map(|sensor| sensor.celsius)
                .reduce(f32::min),
        };
        if !self.series.push(point) {
            return false;
        }
        if self.last_pass == Some(thermal.pass) {
            return true;
        }
        self.last_pass = Some(thermal.pass);
        for throttle in &thermal.core_throttles {
            let key = (throttle.package, throttle.core_id);
            if let Some(&before) = self.counts.get(&key)
                && throttle.count > before
            {
                let marked = self.cores.entry(key).or_insert(Throttled {
                    since: time,
                    seen: 0,
                });
                marked.since = time;
                marked.seen += throttle.count - before;
            }
            self.counts.insert(key, throttle.count);
        }
        for throttle in &thermal.package_throttles {
            if let Some(&before) = self.package_counts.get(&throttle.package)
                && throttle.count > before
            {
                let marked = self.packages.entry(throttle.package).or_insert(Throttled {
                    since: time,
                    seen: 0,
                });
                marked.since = time;
                marked.seen += throttle.count - before;
            }
            self.package_counts.insert(throttle.package, throttle.count);
        }
        true
    }
}

/// What a nudge speaks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Origin {
    Core(usize),
    Ccd(u32, Vec<usize>),
    Package(Vec<usize>),
}

/// The sensors' nudges for one floorplan. Their footprints are built when the set of sensors that
/// read changes, and only their targets move with the readings.
#[derive(Clone, Debug, Default)]
pub struct Regions {
    signature: Vec<SensorPlace>,
    built: bool,
    pub origins: Vec<Origin>,
    pub nudges: Vec<Nudge>,
    pub offsets: Vec<f32>,
}

impl Regions {
    /// Brings the nudges up to date with `readings`: rebuilds them if the sensors that read are
    /// not the ones they were made for, then sets each target to its sensor's value. Returns true
    /// if they were rebuilt (their offsets start from 0).
    pub fn update(&mut self, plan: &Floorplan, snapshot: &Snapshot, readings: &Readings) -> bool {
        let signature: Vec<SensorPlace> = snapshot
            .thermal
            .iter()
            .flat_map(|thermal| &thermal.sensors)
            .filter(|sensor| sensor.celsius.is_some())
            .map(|sensor| sensor.place)
            .collect();
        let rebuilt = !self.built || signature != self.signature;
        if rebuilt {
            self.build(plan, readings);
            self.signature = signature;
            self.built = true;
        }
        for (origin, nudge) in self.origins.iter().zip(&mut self.nudges) {
            let block = match origin {
                Origin::Core(block) => *block,
                Origin::Ccd(_, blocks) | Origin::Package(blocks) => blocks[0],
            };
            if let Some(celsius) = readings.coverage[block].celsius() {
                nudge.target = celsius;
            }
        }
        rebuilt
    }

    fn build(&mut self, plan: &Floorplan, readings: &Readings) {
        self.origins.clear();
        self.nudges.clear();
        let hot = |blocks: &[usize]| -> Vec<u32> {
            blocks
                .iter()
                .map(|&block| heat::cell_at(plan.blocks[block].hot[0], plan.blocks[block].hot[1]))
                .collect()
        };
        let mut complexes: Vec<(u32, Vec<usize>)> = Vec::new();
        let mut package: Vec<usize> = Vec::new();
        for (index, found) in readings.coverage.iter().enumerate() {
            match *found {
                Coverage::Own(_) => {
                    self.origins.push(Origin::Core(index));
                    self.nudges.push(Nudge {
                        footprint: heat::footprint(&[plan.blocks[index].rect]),
                        probes: hot(&[index]),
                        target: 0.0,
                    });
                }
                Coverage::Ccd(number, _) => {
                    match complexes.iter_mut().find(|(other, _)| *other == number) {
                        Some((_, blocks)) => blocks.push(index),
                        None => complexes.push((number, vec![index])),
                    }
                }
                Coverage::Package(_) => package.push(index),
                Coverage::Unknown => {}
            }
        }
        for (number, blocks) in complexes {
            // A complex speaks for its whole box when its cores are all of one group.
            let group = plan.blocks[blocks[0]].group;
            let one_box = group.is_some()
                && blocks
                    .iter()
                    .all(|&block| plan.blocks[block].group == group)
                && group.is_some_and(|group| plan.groups[group].blocks.len() == blocks.len());
            let rects: Vec<_> = match group {
                Some(group) if one_box => vec![plan.groups[group].rect],
                _ => blocks.iter().map(|&block| plan.blocks[block].rect).collect(),
            };
            self.nudges.push(Nudge {
                footprint: heat::footprint(&rects),
                probes: hot(&blocks),
                target: 0.0,
            });
            self.origins.push(Origin::Ccd(number, blocks));
        }
        if !package.is_empty() {
            self.nudges.push(Nudge {
                footprint: heat::whole_die(),
                probes: hot(&package),
                target: 0.0,
            });
            self.origins.push(Origin::Package(package));
        }
        self.offsets = vec![0.0; self.nudges.len()];
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::floorplan::tests::{amd, hybrid};
    use super::*;
    use crate::model::thermal::{CoreThrottle, PackageThrottle, Sensor};
    use crate::model::{CoreKind, DemoOptions, demo_with};

    pub(crate) fn cpu(id: u32, kind: CoreKind, busy: Option<f32>, mhz: Option<f32>) -> Cpu {
        Cpu {
            id,
            kind,
            busy,
            mhz,
            wait: None,
        }
    }

    pub(crate) fn sensor(label: &str, place: SensorPlace, celsius: Option<f32>) -> Sensor {
        Sensor {
            label: label.to_owned(),
            place,
            celsius,
            critical: None,
        }
    }

    /// A sample over `topology` with these sensors and CPUs.
    pub(crate) fn sample(
        topology: Topology,
        sensors: Vec<Sensor>,
        cpus: Vec<Cpu>,
        elapsed: f64,
        pass: u64,
    ) -> Snapshot {
        Snapshot {
            cpus,
            elapsed,
            thermal: Some(Thermal {
                topology: std::sync::Arc::new(topology),
                sensors,
                pass,
                ..Thermal::default()
            }),
            ..Snapshot::default()
        }
    }

    fn demo(time: f64) -> Snapshot {
        demo_with(
            time,
            16,
            DemoOptions {
                thermal: true,
                ..DemoOptions::NONE
            },
        )
    }

    #[test]
    fn coverage_follows_the_finest_sensor_that_speaks_for_a_core() {
        let topology = hybrid(2, 1, true);
        let plan = Floorplan::of(&topology);
        let cpus = |busy: f32| -> Vec<Cpu> {
            (0..8)
                .map(|id| cpu(id, CoreKind::Performance, Some(busy), Some(4000.0)))
                .collect()
        };
        // Per-core sensors for the two P-cores, a package sensor beside them.
        let sensors = vec![
            sensor("Package id 0", SensorPlace::Package(0), Some(71.0)),
            sensor("Core 0", SensorPlace::Core { package: 0, core_id: 0 }, Some(66.0)),
            sensor("Core 4", SensorPlace::Core { package: 0, core_id: 4 }, Some(70.0)),
        ];
        let snapshot = sample(topology.clone(), sensors, cpus(0.5), 1.0, 1);
        let readings = Readings::of(&plan, &snapshot, &mut vec![0.0; plan.blocks.len()]);
        assert_eq!(readings.mode, Mode::Celsius);
        assert_eq!(readings.coverage[0], Coverage::Own(66.0));
        assert_eq!(readings.coverage[1], Coverage::Own(70.0));
        assert_eq!(
            readings.coverage[2],
            Coverage::Unknown,
            "an E-core with no sensor is not covered by the package's, which is only the hottest"
        );
        assert_eq!(readings.package_celsius, Some(71.0));
        assert_eq!(readings.coolest, Some(66.0));
        // Package only: the package's reading speaks for every core.
        let sensors = vec![sensor("x86_pkg_temp", SensorPlace::Package(0), Some(55.0))];
        let snapshot = sample(topology.clone(), sensors, cpus(0.5), 1.0, 1);
        let readings = Readings::of(&plan, &snapshot, &mut vec![0.0; plan.blocks.len()]);
        assert!(readings
            .coverage
            .iter()
            .all(|found| *found == Coverage::Package(55.0)));
        // No sensor: relative, and nothing is covered; a sensor with no reading is no sensor.
        let sensors = vec![sensor("Core 0", SensorPlace::Core { package: 0, core_id: 0 }, None)];
        let snapshot = sample(topology, sensors, cpus(0.5), 1.0, 1);
        let readings = Readings::of(&plan, &snapshot, &mut vec![0.0; plan.blocks.len()]);
        assert_eq!(readings.mode, Mode::Relative);
        assert!(readings.coverage.iter().all(|found| *found == Coverage::Unknown));
        assert_eq!(readings.scale(), (0.0, RELATIVE_SPAN));
        assert!(!readings.tj_max_known && readings.tj_max == TJ_MAX_DEFAULT);
    }

    #[test]
    fn a_ccd_sensor_speaks_for_the_cores_of_its_complex_and_no_others() {
        let topology = amd(2);
        let plan = Floorplan::of(&topology);
        let cpus: Vec<Cpu> = (0..16)
            .map(|id| cpu(id, CoreKind::Unknown, Some(0.2), Some(3000.0)))
            .collect();
        let sensors = vec![
            sensor("Tctl", SensorPlace::Package(0), Some(58.0)),
            sensor("Tccd1", SensorPlace::Ccd(0), Some(61.0)),
            sensor("Tccd3", SensorPlace::Ccd(1), Some(49.0)),
        ];
        let snapshot = sample(topology, sensors, cpus, 1.0, 1);
        let readings = Readings::of(&plan, &snapshot, &mut vec![0.0; plan.blocks.len()]);
        for (index, block) in plan.blocks.iter().enumerate() {
            let expected = if block.cpus[0] < 8 {
                Coverage::Ccd(0, 61.0)
            } else {
                Coverage::Ccd(1, 49.0)
            };
            assert_eq!(readings.coverage[index], expected, "{}", block.name);
        }
        // The nudges: one per complex, each speaking for the whole box, and none for the package.
        let mut regions = Regions::default();
        assert!(regions.update(&plan, &snapshot, &readings));
        assert_eq!(regions.nudges.len(), 2);
        assert_eq!(regions.nudges[0].target, 61.0);
        assert_eq!(regions.nudges[1].target, 49.0);
        let first = &regions.nudges[0].footprint;
        let box_cells = {
            let rect = plan.groups[0].rect;
            ((rect[2] - rect[0]) * (rect[3] - rect[1])) as usize
        };
        assert!(first.len() > box_cells, "the whole box and its skirt, not the cores alone");
        assert_eq!(regions.nudges[0].probes.len(), 8);
        // The same sensors again rebuild nothing; a sensor that stops reading does.
        assert!(!regions.update(&plan, &snapshot, &readings));
        let mut silent = snapshot.clone();
        silent.thermal.as_mut().expect("thermal").sensors[2].celsius = None;
        let readings = Readings::of(&plan, &silent, &mut vec![0.0; plan.blocks.len()]);
        assert!(regions.update(&plan, &silent, &readings));
        assert_eq!(regions.nudges.len(), 1);
        // A sensor the cache groups cannot place is counted, and covers nothing.
        let mut stray = snapshot;
        stray.thermal.as_mut().expect("thermal").sensors.push(sensor(
            "Tccd5",
            SensorPlace::Ccd(2),
            Some(70.0),
        ));
        let readings = Readings::of(&plan, &stray, &mut vec![0.0; plan.blocks.len()]);
        assert_eq!(readings.unmapped_ccds, 3, "three CCD sensors against two L3 groups: none map");
        assert!(
            readings.coverage.iter().all(|found| *found == Coverage::Package(58.0)),
            "with no CCD placed the package sensor speaks for the die"
        );
    }

    #[test]
    fn the_power_proxy_is_busy_times_the_cube_of_the_clock_over_its_highest() {
        let topology = hybrid(1, 0, true);
        let plan = Floorplan::of(&topology);
        let make = |busy: Option<f32>, mhz: Option<f32>| -> Snapshot {
            sample(
                topology.clone(),
                Vec::new(),
                vec![
                    cpu(0, CoreKind::Performance, busy, mhz),
                    cpu(1, CoreKind::Performance, busy, mhz),
                ],
                1.0,
                1,
            )
        };
        let proxy = |snapshot: &Snapshot, seen: &mut Vec<f32>| {
            Readings::of(&plan, snapshot, seen).proxies[0]
        };
        // The topology says 4800 MHz: half busy per thread, two threads, at half the clock.
        let mut seen = vec![0.0];
        let half = proxy(&make(Some(0.5), Some(2400.0)), &mut seen);
        assert!((half.expect("measured") - 1.0 * 0.125).abs() < 1e-5, "{half:?}");
        // Threads' shares add to at most 1.
        let both = proxy(&make(Some(0.9), Some(4800.0)), &mut seen);
        assert_eq!(both, Some(1.0));
        // No load reading is no proxy, not 0.
        assert_eq!(proxy(&make(None, Some(4800.0)), &mut seen), None);
        // No clock reading: busy alone, and the reading says so.
        let readings = Readings::of(&plan, &make(Some(0.3), None), &mut vec![0.0]);
        assert_eq!(readings.proxies[0], Some(0.6));
        assert_eq!(readings.without_clock, 1);
        // With no maximum in the topology the block's own running maximum stands in.
        let mut bare = topology.clone();
        for place in &mut bare.cpus {
            place.max_mhz = None;
        }
        let plan = Floorplan::of(&bare);
        let bare_sample = |mhz| {
            let mut snapshot = make(Some(1.0), Some(mhz));
            snapshot.thermal = Some(Thermal {
                topology: std::sync::Arc::new(bare.clone()),
                ..Thermal::default()
            });
            snapshot
        };
        let mut seen = vec![0.0];
        let first = Readings::of(&plan, &bare_sample(2000.0), &mut seen).proxies[0];
        assert_eq!(first, Some(1.0), "the first clock seen is the highest so far");
        let slower = Readings::of(&plan, &bare_sample(1000.0), &mut seen).proxies[0];
        assert!((slower.expect("measured") - 0.125).abs() < 1e-5);
        assert_eq!(seen[0], 2000.0);
    }

    #[test]
    fn throttle_marks_come_from_rising_counts_between_passes_only() {
        let topology = hybrid(1, 0, true);
        let plan = Floorplan::of(&topology);
        let counted = |core: u64, package: u64, pass: u64, elapsed: f64| -> Snapshot {
            let mut snapshot = sample(topology.clone(), Vec::new(), Vec::new(), elapsed, pass);
            let thermal = snapshot.thermal.as_mut().expect("thermal");
            thermal.core_throttles = vec![CoreThrottle {
                package: 0,
                core_id: 0,
                count: core,
            }];
            thermal.package_throttles = vec![PackageThrottle { package: 0, count: package }];
            snapshot
        };
        let mut watch = Watch::default();
        // The first reading is a baseline, whatever the count: 90 throttles before we looked.
        assert!(watch.ingest(&counted(90, 7, 1, 1.0), 1.0, &plan));
        assert!(watch.cores.is_empty() && watch.packages.is_empty());
        // The same pass again (a sample between passes) and an older sample take nothing in.
        assert!(!watch.ingest(&counted(95, 7, 1, 1.0), 1.1, &plan));
        assert!(watch.ingest(&counted(90, 7, 1, 2.0), 2.0, &plan), "a new sample, the same pass");
        assert!(watch.cores.is_empty());
        // A rise between passes marks the core once, at the time it was seen.
        assert!(watch.ingest(&counted(91, 7, 2, 3.0), 3.0, &plan));
        assert_eq!(
            watch.cores.get(&(0, 0)),
            Some(&Throttled { since: 3.0, seen: 1 })
        );
        assert!(watch.packages.is_empty(), "the package's count did not move");
        // A sample of the same pass does not mark it twice.
        watch.ingest(&counted(91, 7, 2, 4.0), 4.0, &plan);
        assert_eq!(watch.cores.get(&(0, 0)).map(|marked| marked.seen), Some(1));
        // A count that falls is a new baseline (a reboot, a hot-plugged core), never a flash.
        watch.ingest(&counted(3, 7, 3, 5.0), 5.0, &plan);
        assert_eq!(watch.cores.get(&(0, 0)).map(|marked| marked.since), Some(3.0));
        // Rising from the new baseline marks it again, by the difference, and the package too.
        watch.ingest(&counted(5, 8, 4, 6.0), 6.0, &plan);
        assert_eq!(watch.cores.get(&(0, 0)), Some(&Throttled { since: 6.0, seen: 3 }));
        assert_eq!(watch.packages.get(&0), Some(&Throttled { since: 6.0, seen: 1 }));
        // No counter at all (None is no entry) marks nothing, and no thermal block is no sample.
        let mut bare = sample(topology.clone(), Vec::new(), Vec::new(), 7.0, 5);
        bare.thermal.as_mut().expect("thermal").core_throttles.clear();
        watch.ingest(&bare, 7.0, &plan);
        assert_eq!(watch.cores.get(&(0, 0)).map(|marked| marked.seen), Some(3));
        assert!(!watch.ingest(&Snapshot::default(), 8.0, &plan));
    }

    #[test]
    fn the_series_keeps_a_minute_and_the_ambient_is_five_under_its_coolest() {
        let mut series = Series::default();
        let point = |elapsed: f64, coolest: Option<f32>| Point {
            elapsed,
            celsius: Some(60.0),
            watts: None,
            coolest,
        };
        assert_eq!(series.ambient(), None, "no sensor has read");
        assert!(series.push(point(1.0, Some(40.0))));
        assert!(!series.push(point(1.0, Some(10.0))), "not newer");
        assert!(!series.push(point(0.5, Some(10.0))), "a rewind");
        assert!(series.push(point(30.0, Some(45.0))));
        assert_eq!(series.ambient(), Some(35.0));
        // Past a minute the old coolest drops out, and so does the old point.
        assert!(series.push(point(62.0, Some(50.0))));
        assert_eq!(series.points().count(), 2);
        assert_eq!(series.ambient(), Some(40.0));
        assert_eq!(series.last().map(|last| last.elapsed), Some(62.0));
    }

    #[test]
    fn the_rapl_scale_follows_measured_power_and_holds_when_the_model_has_nothing_to_divide() {
        let mut scale = RaplScale::default();
        assert_eq!(scale.value(), 1.0);
        // The first reading is taken whole: 30 W less 2.2 W of the rest over 20 W modelled.
        scale.take(30.0, 20.0, 1.0);
        assert!((scale.value() - 1.39).abs() < 1e-5, "{}", scale.value());
        // A second reading two seconds on closes 1 - 1/e of the gap to its target.
        scale.take(12.2, 20.0, 3.0);
        let expected = 1.39 + (0.5 - 1.39) * (1.0 - (-1.0_f32).exp());
        assert!((scale.value() - expected).abs() < 1e-4, "{}", scale.value());
        // Almost nothing modelled: held, however much power is read.
        let held = scale.value();
        scale.take(80.0, 0.2, 5.0);
        assert_eq!(scale.value(), held);
        // A reading from the past changes nothing.
        scale.take(5.0, 20.0, 4.0);
        assert_eq!(scale.value(), held);
    }

    #[test]
    fn the_demo_reads_as_degrees_with_every_core_covered() {
        let snapshot = demo(12.0);
        let topology = topology_of(&snapshot);
        let plan = Floorplan::of(&topology);
        let readings = Readings::of(&plan, &snapshot, &mut vec![0.0; plan.blocks.len()]);
        assert_eq!(readings.mode, Mode::Celsius);
        assert!(readings.tj_max_known && readings.tj_max == 100.0);
        assert!(readings.coverage.iter().all(|found| matches!(found, Coverage::Own(_))));
        assert!(readings.proxies.iter().all(Option::is_some));
        assert!(matches!(readings.watts, Measured::Known(_)));
        let mut regions = Regions::default();
        regions.update(&plan, &snapshot, &readings);
        assert_eq!(regions.nudges.len(), plan.blocks.len(), "a nudge per core, none for the package");
        assert!(regions.origins.iter().all(|origin| matches!(origin, Origin::Core(_))));
    }

    #[test]
    fn a_machine_without_topology_draws_the_cpu_list_as_cores_and_without_sensors_is_relative() {
        let snapshot = Snapshot {
            cpus: (0..4).map(|id| cpu(id, CoreKind::Unknown, Some(0.3), None)).collect(),
            ..Snapshot::default()
        };
        let topology = topology_of(&snapshot);
        assert_eq!(topology.cpus.len(), 4);
        let plan = Floorplan::of(&topology);
        assert_eq!(plan.blocks.len(), 4, "each CPU is a core of its own");
        let readings = Readings::of(&plan, &snapshot, &mut vec![0.0; 4]);
        assert_eq!(readings.mode, Mode::Relative);
        assert!(readings.proxies.iter().all(|proxy| *proxy == Some(0.3)), "busy alone");
        assert_eq!(readings.without_clock, 4);
        assert_eq!(readings.watts, Measured::Pending);
        // Nothing at all: no blocks, no panic.
        let empty = Snapshot::default();
        let plan = Floorplan::of(&topology_of(&empty));
        let readings = Readings::of(&plan, &empty, &mut []);
        assert!(readings.coverage.is_empty() && readings.proxies.is_empty());
    }
}
