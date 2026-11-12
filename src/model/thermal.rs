//! The CPU's physical layout and thermal state: the topology (packages, dies, cores, SMT
//! siblings, clusters, cache groups), the temperature sensors mapped onto it, the thermal
//! throttle counters and the RAPL package power. The platform reads one source per call
//! (`ThermalReader`); this module decides whether to read at all (`Reads`, so only while a shown
//! view declares one of the four sources in `SOURCES`), keeps the topology between passes and
//! assembles `Snapshot::thermal`. The demo's synthesized block is in `demo`.
//!
//! Nothing here is a measurement a view may draw as zero: a sensor with no reading is
//! `Sensor::celsius` None, a throttle counter that cannot be read has no entry, a package with
//! no power reading yet has `PackagePower::watts` None, and a source the machine cannot read is
//! in `Snapshot::missing` while it is demanded.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the die view (#56) is the first reader of most of this"
    )
)]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use super::{CoreKind, Measured, Reads, timed};
use crate::platform::{Source, Sources};

pub(super) mod demo;

/// The four sources `Snapshot::thermal` is made of. A view that draws the block declares them
/// all on its registry row.
pub const SOURCES: Sources = Sources::of(&[
    Source::Topology,
    Source::Temperature,
    Source::ThrottleCounts,
    Source::PackagePower,
]);

/// The thermal state of the machine, as one background pass read it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Thermal {
    /// The CPUs' physical layout. Read once while demanded and shared, so a snapshot history does
    /// not copy it. Empty (`Topology::cpus`) when it cannot be read (`Source::Topology` is then
    /// missing if it was demanded) and when none of `Source::Topology`, `Temperature` and
    /// `ThrottleCounts` was demanded, which is the only time it is not read.
    pub topology: Arc<Topology>,
    /// The CPU temperature sensors, package first, in the order the platform lists them. Empty
    /// until `Source::Temperature` is read, and when the machine has no CPU sensor.
    pub sensors: Vec<Sensor>,
    /// How often each physical core was throttled for heat since boot, one entry per core whose
    /// counter could be read and whose package and core id are known. Empty when
    /// `Source::ThrottleCounts` is not read or the machine has no counter (Intel only).
    pub core_throttles: Vec<CoreThrottle>,
    /// How often each package was throttled for heat since boot, as `core_throttles`.
    pub package_throttles: Vec<PackageThrottle>,
    /// The power of each package with a readable RAPL domain, over the time since the previous
    /// pass. Empty until `Source::PackagePower` is read and when no domain is readable.
    pub package_power: Vec<PackagePower>,
    /// All packages together: Known when every readable domain had a rate, Pending before the
    /// second pass (and while a domain's rate is unknown), Unreadable where RAPL cannot be read
    /// (root only on most kernels; isotop never escalates privilege), and Pending while
    /// `Source::PackagePower` is not demanded. Never 0 W for a reading that was not taken.
    pub package_watts: Measured<f32>,
    /// The number of the background pass that read this, counting from 1. The foreground samples
    /// between two passes carry the same block, and this is how a view tells a new reading from a
    /// repeat. The demo numbers each sample by its time (`f64::to_bits`), as `kernel_pass`.
    pub pass: u64,
}

/// One temperature sensor.
#[derive(Clone, Debug, PartialEq)]
pub struct Sensor {
    /// The sensor's label as the driver gives it (`Package id 0`, `Core 4`, `Tdie`, `Tccd1`), raw
    /// and untrusted: it reaches the terminal only as `terminal::SafeText`.
    pub label: String,
    /// What the sensor reads.
    pub place: SensorPlace,
    /// Degrees Celsius; None for a sensor that could not be read or reported a value outside
    /// -100 to 200 degrees.
    pub celsius: Option<f32>,
    /// The temperature the driver calls critical (`temp*_crit`), which for `coretemp` is TjMax.
    /// None when it has none.
    pub critical: Option<f32>,
}

/// What a `Sensor` measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorPlace {
    /// One physical core, by its package and the `core_id` of `Topology` (not a CPU number: on
    /// an Intel hybrid part the P-cores are 0, 4, 8, ...).
    Core { package: u32, core_id: u32 },
    /// One AMD core complex die. The number is the sensor's rank among the machine's `Tccd`
    /// sensors from 0, whatever indices the driver uses; `Topology::ccds` says which CPUs it is.
    Ccd(u32),
    /// A whole package (the die, as far as one sensor goes).
    Package(u32),
}

/// A core's thermal throttle counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreThrottle {
    pub package: u32,
    pub core_id: u32,
    /// Throttle events since boot; it never decreases while the machine runs.
    pub count: u64,
}

/// A package's thermal throttle counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageThrottle {
    pub package: u32,
    pub count: u64,
}

/// The power of one package.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackagePower {
    pub package: u32,
    /// Watts over the interval between the last two passes, summed over the package's RAPL
    /// domains; None until there are two readings and whenever the interval is ambiguous (a
    /// counter that did not move or that may have wrapped more than once).
    pub watts: Option<f32>,
}

/// Where each online CPU sits. Every field the kernel may leave out or print as -1 is an Option:
/// unknown is not zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Topology {
    /// One entry per online CPU, sorted by `CpuPlace::cpu`.
    pub cpus: Vec<CpuPlace>,
}

/// One logical CPU's place in the machine.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuPlace {
    pub cpu: u32,
    pub package: Option<u32>,
    pub die: Option<u32>,
    /// The core's number within its package (`core_id`). Not unique across packages, and not
    /// contiguous: an Intel hybrid part numbers its P-cores 0, 4, 8, ...
    pub core_id: Option<u32>,
    pub cluster: Option<u32>,
    /// The online CPUs on the same core, itself included (`core_cpus_list`, else
    /// `thread_siblings_list`); empty when the kernel says neither.
    pub siblings: Vec<u32>,
    /// The online CPUs that share this CPU's L2 and L3 caches; None when the kernel lists no such
    /// cache. On a tiled part the low-power E-cores have an L2 and no L3.
    pub l2: Option<Vec<u32>>,
    pub l3: Option<Vec<u32>>,
    /// The highest clock the CPU may reach (`cpuinfo_max_freq`), in MHz.
    pub max_mhz: Option<f32>,
    pub kind: CoreKind,
}

/// A physical core: the logical CPUs that share it.
#[derive(Clone, Debug, PartialEq)]
pub struct Core {
    /// Sorted, never empty.
    pub cpus: Vec<u32>,
    pub package: Option<u32>,
    pub die: Option<u32>,
    pub core_id: Option<u32>,
    pub kind: CoreKind,
}

/// A group of cores that share an L2 cache (or, where the kernel lists no L2, a cluster id).
#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    /// Sorted, never empty.
    pub cpus: Vec<u32>,
    /// The kind of its CPUs, Unknown when they differ.
    pub kind: CoreKind,
    /// An efficiency cluster with no L3 while another cluster has one: the low-power E-cores of
    /// a tiled part, which sit outside the compute tile's cache.
    pub low_power: bool,
}

/// What identifies the CPUs of one core.
#[derive(Clone, PartialEq, Eq, Hash)]
enum CoreKey {
    Siblings(Vec<u32>),
    Place(u32, Option<u32>, u32),
    Alone(u32),
}

/// What identifies the CPUs of one cluster.
#[derive(Clone, PartialEq, Eq, Hash)]
enum ClusterKey {
    L2(Vec<u32>),
    Id(Option<u32>, Option<u32>, u32),
    Siblings(Vec<u32>),
}

impl Topology {
    pub fn is_empty(&self) -> bool {
        self.cpus.is_empty()
    }

    /// The place of `cpu`, None when it is not an online CPU of this topology.
    pub fn place(&self, cpu: u32) -> Option<&CpuPlace> {
        self.cpus
            .binary_search_by_key(&cpu, |place| place.cpu)
            .ok()
            .map(|index| &self.cpus[index])
    }

    /// The packages the CPUs name, sorted.
    pub fn packages(&self) -> Vec<u32> {
        self.cpus
            .iter()
            .filter_map(|place| place.package)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// The physical cores, ordered by their lowest CPU. CPUs are grouped by the sibling list the
    /// kernel gives; where it gives none, by package, die and core id; where those are unknown
    /// too, each CPU is a core of its own.
    pub fn cores(&self) -> Vec<Core> {
        let mut index: HashMap<CoreKey, usize> = HashMap::new();
        let mut cores: Vec<Core> = Vec::new();
        for place in &self.cpus {
            let key = if !place.siblings.is_empty() {
                CoreKey::Siblings(place.siblings.clone())
            } else if let (Some(package), Some(core_id)) = (place.package, place.core_id) {
                CoreKey::Place(package, place.die, core_id)
            } else {
                CoreKey::Alone(place.cpu)
            };
            match index.get(&key) {
                Some(&at) => cores[at].cpus.push(place.cpu),
                None => {
                    index.insert(key, cores.len());
                    cores.push(Core {
                        cpus: vec![place.cpu],
                        package: place.package,
                        die: place.die,
                        core_id: place.core_id,
                        kind: place.kind,
                    });
                }
            }
        }
        cores
    }

    /// The clusters, ordered by their lowest CPU: CPUs that share an L2 cache; where the kernel
    /// lists no L2, CPUs with the same cluster id in the same package and die; where there is
    /// none either, the core.
    pub fn clusters(&self) -> Vec<Cluster> {
        let mut index: HashMap<ClusterKey, usize> = HashMap::new();
        let mut groups: Vec<Vec<&CpuPlace>> = Vec::new();
        for place in &self.cpus {
            let key = if let Some(l2) = &place.l2 {
                ClusterKey::L2(l2.clone())
            } else if let Some(cluster) = place.cluster {
                ClusterKey::Id(place.package, place.die, cluster)
            } else if place.siblings.is_empty() {
                ClusterKey::Siblings(vec![place.cpu])
            } else {
                ClusterKey::Siblings(place.siblings.clone())
            };
            match index.get(&key) {
                Some(&at) => groups[at].push(place),
                None => {
                    index.insert(key, groups.len());
                    groups.push(vec![place]);
                }
            }
        }
        let any_l3 = self.cpus.iter().any(|place| place.l3.is_some());
        groups
            .into_iter()
            .map(|group| {
                let kind = group[0].kind;
                let kind = if group.iter().all(|place| place.kind == kind) {
                    kind
                } else {
                    CoreKind::Unknown
                };
                Cluster {
                    cpus: group.iter().map(|place| place.cpu).collect(),
                    kind,
                    low_power: any_l3
                        && kind == CoreKind::Efficiency
                        && group.iter().all(|place| place.l3.is_none()),
                }
            })
            .collect()
    }

    /// The groups of CPUs that share an L3 cache, each sorted, ordered by their lowest CPU.
    pub fn l3_groups(&self) -> Vec<Vec<u32>> {
        let mut groups: Vec<Vec<u32>> = Vec::new();
        for place in &self.cpus {
            if let Some(group) = &place.l3
                && !groups.contains(group)
            {
                groups.push(group.clone());
            }
        }
        groups
    }

    /// The CPUs of each of `count` AMD core complex dies, ordered by their lowest CPU, so that
    /// the n-th `Tccd` sensor (`SensorPlace::Ccd(n)`) reads the n-th entry. This is a heuristic,
    /// since the kernel names no CCD: a core complex (CCX) shares an L3, a Zen 3 and later CCD
    /// has one CCX and a Zen 2 CCD has two. As many L3 groups as sensors makes each group a CCD,
    /// twice as many makes each consecutive pair one, and any other count says nothing (None).
    pub fn ccds(&self, count: usize) -> Option<Vec<Vec<u32>>> {
        let groups = self.l3_groups();
        let per = if count == 0 || groups.is_empty() {
            return None;
        } else if groups.len() == count {
            1
        } else if groups.len() == 2 * count {
            2
        } else {
            return None;
        };
        Some(
            groups
                .chunks(per)
                .map(|chunk| {
                    let mut cpus: Vec<u32> = chunk.concat();
                    cpus.sort_unstable();
                    cpus.dedup();
                    cpus
                })
                .collect(),
        )
    }
}

impl Thermal {
    /// The reading of package `package`'s own sensor.
    pub fn package_celsius(&self, package: u32) -> Option<f32> {
        self.sensors.iter().find_map(|sensor| match sensor.place {
            SensorPlace::Package(found) if found == package => sensor.celsius,
            _ => None,
        })
    }

    /// The reading of `core`'s sensor, matched by package and `core_id`, never by CPU number.
    /// None for a core without one (AMD has none), with an unknown identity or no reading.
    pub fn core_celsius(&self, core: &Core) -> Option<f32> {
        let (package, core_id) = (core.package?, core.core_id?);
        self.sensors.iter().find_map(|sensor| match sensor.place {
            SensorPlace::Core {
                package: found,
                core_id: id,
            } if found == package && id == core_id => sensor.celsius,
            _ => None,
        })
    }

    /// `core`'s thermal throttle count; None when it has no counter.
    pub fn core_throttle(&self, core: &Core) -> Option<u64> {
        let (package, core_id) = (core.package?, core.core_id?);
        self.core_throttles
            .iter()
            .find(|throttle| throttle.package == package && throttle.core_id == core_id)
            .map(|throttle| throttle.count)
    }

    /// The highest critical temperature any sensor reports: TjMax on Intel. None when no sensor
    /// reports one; a view takes 100 degrees then.
    pub fn tj_max(&self) -> Option<f32> {
        self.sensors
            .iter()
            .filter_map(|sensor| sensor.critical)
            .reduce(f32::max)
    }

    /// The CPUs that the `Ccd(ccd)` sensor reads, from `Topology::ccds` over the machine's CCD
    /// sensors. None when there is no such sensor or the cache groups do not say.
    pub fn ccd_cpus(&self, ccd: u32) -> Option<Vec<u32>> {
        let count = self
            .sensors
            .iter()
            .filter(|sensor| matches!(sensor.place, SensorPlace::Ccd(_)))
            .count();
        self.topology
            .ccds(count)?
            .into_iter()
            .nth(usize::try_from(ccd).ok()?)
    }
}

/// What a platform reads for the thermal block: one call per source, each None when this machine
/// cannot read that source (the pass then reports it missing). The pass, which decides what is
/// read and when, is `ThermalPass`.
pub trait ThermalReader {
    /// The online CPUs and where they sit. None when the platform cannot say.
    fn topology(&mut self) -> Option<Topology>;

    /// The CPU sensors, given the packages the topology names (empty when it was not read).
    /// None when no sensor has a reading.
    fn temperatures(&mut self, packages: &[u32]) -> Option<Vec<Sensor>>;

    /// The throttle counters of the cores and packages of `topology`. None when there is none.
    fn throttle_counts(
        &mut self,
        topology: &Topology,
    ) -> Option<(Vec<CoreThrottle>, Vec<PackageThrottle>)>;

    /// The power of each package since the previous call (None watts on the first). None when no
    /// energy counter can be read.
    fn package_power(&mut self) -> Option<Vec<PackagePower>>;

    /// Drops what `package_power` keeps between calls, so a later demand starts with a first
    /// reading instead of a rate over the gap.
    fn forget_power(&mut self);
}

/// What the background thread keeps of the thermal block between passes.
#[derive(Default)]
pub struct ThermalPass {
    /// The topology, kept while any of the four sources is demanded: it is read once, not every
    /// pass. Dropped when none is, so a later demand finds hotplugged CPUs.
    topology: Option<Arc<Topology>>,
    passes: u64,
}

impl ThermalPass {
    /// One pass. Nothing is read, and None returned, unless one of `SOURCES` is demanded. Each
    /// demanded source is read through `reads`, so it lands in the sample's `sources` or
    /// `missing`; an undemanded one is neither and its files are not opened.
    ///
    /// The temperature and throttle readers need the topology (the packages, the cores), so it
    /// is read when either is demanded even when `Source::Topology` is not, and is then reported
    /// only if it was.
    pub(super) fn pass(
        &mut self,
        reader: &mut impl ThermalReader,
        reads: &mut Reads,
        latency: &mut Vec<(Source, Duration)>,
    ) -> Option<Thermal> {
        let demanded = |source| reads.demand.contains(source);
        if !SOURCES.iter().any(demanded) {
            self.topology = None;
            reader.forget_power();
            return None;
        }
        let (wants_topology, wants_power) =
            (demanded(Source::Topology), demanded(Source::PackagePower));
        if self.topology.is_none()
            && (wants_topology || demanded(Source::Temperature) || demanded(Source::ThrottleCounts))
        {
            self.topology = timed(latency, Source::Topology, || reader.topology()).map(Arc::new);
        }
        if wants_topology {
            if self.topology.is_some() {
                reads.active.insert(Source::Topology);
            } else {
                reads.missing.insert(Source::Topology);
            }
        }
        let topology = self.topology.clone().unwrap_or_default();
        let packages = topology.packages();
        let sensors = reads
            .read(Source::Temperature, || {
                timed(latency, Source::Temperature, || {
                    reader.temperatures(&packages)
                })
            })
            .unwrap_or_default();
        let (core_throttles, package_throttles) = reads
            .read(Source::ThrottleCounts, || {
                timed(latency, Source::ThrottleCounts, || {
                    reader.throttle_counts(&topology)
                })
            })
            .unwrap_or_default();
        let package_power = if wants_power {
            reads.read(Source::PackagePower, || {
                timed(latency, Source::PackagePower, || reader.package_power())
            })
        } else {
            reader.forget_power();
            None
        };
        let package_watts = match &package_power {
            _ if !wants_power => Measured::Pending,
            None => Measured::Unreadable,
            Some(power) => total_watts(power),
        };
        self.passes += 1;
        Some(Thermal {
            topology,
            sensors,
            core_throttles,
            package_throttles,
            package_power: package_power.unwrap_or_default(),
            package_watts,
            pass: self.passes,
        })
    }
}

/// The watts of all packages together: Known only when every package has a rate, and Pending
/// when there is none to add or any rate is not known yet.
fn total_watts(power: &[PackagePower]) -> Measured<f32> {
    let watts: Option<Vec<f32>> = power.iter().map(|package| package.watts).collect();
    match watts {
        Some(watts) if !watts.is_empty() => Measured::Known(watts.iter().sum()),
        _ => Measured::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Demand;

    /// A topology of `cpus` CPUs on one package, each its own core, for tests that need no more.
    fn flat(cpus: u32) -> Topology {
        Topology {
            cpus: (0..cpus)
                .map(|cpu| CpuPlace {
                    cpu,
                    package: Some(0),
                    die: Some(0),
                    core_id: Some(cpu),
                    cluster: None,
                    siblings: vec![cpu],
                    l2: None,
                    l3: None,
                    max_mhz: None,
                    kind: CoreKind::Unknown,
                })
                .collect(),
        }
    }

    fn place(cpu: u32) -> CpuPlace {
        flat(cpu + 1).cpus.pop().expect("one CPU")
    }

    fn sensor(place: SensorPlace, celsius: Option<f32>, critical: Option<f32>) -> Sensor {
        Sensor {
            label: String::from("label"),
            place,
            celsius,
            critical,
        }
    }

    /// A reader that counts its calls and returns what it was given.
    #[derive(Default)]
    struct Fake {
        topology: Option<Topology>,
        sensors: Option<Vec<Sensor>>,
        throttles: Option<(Vec<CoreThrottle>, Vec<PackageThrottle>)>,
        power: Option<Vec<PackagePower>>,
        calls: Vec<&'static str>,
        packages_seen: Vec<Vec<u32>>,
        /// How many CPUs the topology handed to `throttle_counts` held.
        topologies_seen: Vec<usize>,
    }

    impl ThermalReader for Fake {
        fn topology(&mut self) -> Option<Topology> {
            self.calls.push("topology");
            self.topology.clone()
        }

        fn temperatures(&mut self, packages: &[u32]) -> Option<Vec<Sensor>> {
            self.calls.push("temperatures");
            self.packages_seen.push(packages.to_vec());
            self.sensors.clone()
        }

        fn throttle_counts(
            &mut self,
            topology: &Topology,
        ) -> Option<(Vec<CoreThrottle>, Vec<PackageThrottle>)> {
            self.calls.push("throttle_counts");
            self.topologies_seen.push(topology.cpus.len());
            self.throttles.clone()
        }

        fn package_power(&mut self) -> Option<Vec<PackagePower>> {
            self.calls.push("package_power");
            self.power.clone()
        }

        fn forget_power(&mut self) {
            self.calls.push("forget_power");
        }
    }

    fn working() -> Fake {
        Fake {
            topology: Some(flat(4)),
            sensors: Some(vec![sensor(
                SensorPlace::Package(0),
                Some(51.0),
                Some(100.0),
            )]),
            throttles: Some((
                vec![CoreThrottle {
                    package: 0,
                    core_id: 0,
                    count: 3,
                }],
                vec![PackageThrottle {
                    package: 0,
                    count: 1,
                }],
            )),
            power: Some(vec![PackagePower {
                package: 0,
                watts: Some(12.5),
            }]),
            ..Fake::default()
        }
    }

    fn pass(
        pass: &mut ThermalPass,
        reader: &mut Fake,
        demand: Sources,
    ) -> (Option<Thermal>, Reads) {
        let mut reads = Reads::new(Demand::from(demand));
        let thermal = pass.pass(reader, &mut reads, &mut Vec::new());
        (thermal, reads)
    }

    #[test]
    fn thermal_is_sampled_only_while_demanded() {
        let mut state = ThermalPass::default();
        let mut reader = working();
        let (thermal, reads) = pass(&mut state, &mut reader, Sources::default());
        assert!(thermal.is_none(), "nothing demanded, no block");
        assert!(reads.active.is_empty() && reads.missing.is_empty());
        assert!(
            reader.calls.iter().all(|&call| call == "forget_power"),
            "no file is read for an undemanded source: {:?}",
            reader.calls
        );
        // A source of another view does not wake it.
        let (thermal, _) = pass(&mut state, &mut reader, Source::Journal.into());
        assert!(thermal.is_none());

        reader.calls.clear();
        let (thermal, reads) = pass(&mut state, &mut reader, SOURCES);
        let thermal = thermal.expect("demanded, so read");
        assert_eq!(reads.active, SOURCES);
        assert!(reads.missing.is_empty());
        assert_eq!(
            reader.calls,
            [
                "topology",
                "temperatures",
                "throttle_counts",
                "package_power"
            ]
        );
        assert_eq!(thermal.topology.cpus.len(), 4);
        assert_eq!(reader.topologies_seen, [4], "the reader got the topology");
        assert_eq!(thermal.sensors.len(), 1);
        assert_eq!(thermal.core_throttles[0].count, 3);
        assert_eq!(thermal.package_watts, Measured::Known(12.5));
        assert_eq!(thermal.pass, 1);

        // The demand ends: no block, and what the reader kept for the next demand is dropped.
        reader.calls.clear();
        let (thermal, _) = pass(&mut state, &mut reader, Sources::default());
        assert!(thermal.is_none());
        assert_eq!(reader.calls, ["forget_power"]);
    }

    #[test]
    fn the_topology_is_read_once_while_demanded_and_again_after_a_gap() {
        let mut state = ThermalPass::default();
        let mut reader = working();
        for _ in 0..3 {
            pass(&mut state, &mut reader, SOURCES);
        }
        let reads_of = |reader: &Fake| reader.calls.iter().filter(|&&c| c == "topology").count();
        assert_eq!(reads_of(&reader), 1, "once, however many passes");
        let (thermal, _) = pass(&mut state, &mut reader, SOURCES);
        assert_eq!(thermal.expect("block").pass, 4, "passes keep counting");
        pass(&mut state, &mut reader, Sources::default());
        pass(&mut state, &mut reader, SOURCES);
        assert_eq!(reads_of(&reader), 2, "a new demand reads it again");
    }

    #[test]
    fn missing_names_appear_only_while_thermal_is_demanded() {
        // A machine with a topology and nothing else, as a virtual machine is.
        let mut state = ThermalPass::default();
        let mut reader = Fake {
            topology: Some(flat(4)),
            ..Fake::default()
        };
        let (thermal, reads) = pass(&mut state, &mut reader, Sources::default());
        assert!(thermal.is_none());
        assert!(reads.missing.is_empty(), "nobody asked, nothing is missing");

        let (thermal, reads) = pass(&mut state, &mut reader, SOURCES);
        let thermal = thermal.expect("the topology is a block");
        assert_eq!(reads.active, Sources::from(Source::Topology));
        assert_eq!(
            reads.missing,
            Sources::from([
                Source::Temperature,
                Source::ThrottleCounts,
                Source::PackagePower
            ])
        );
        assert!(thermal.sensors.is_empty() && thermal.core_throttles.is_empty());
        assert_eq!(
            thermal.package_watts,
            Measured::Unreadable,
            "a gap, not zero watts"
        );

        // Asking for one source reports only that one.
        let (_, reads) = pass(&mut state, &mut reader, Source::Temperature.into());
        assert_eq!(reads.missing, Sources::from(Source::Temperature));
        assert!(reads.active.is_empty());
    }

    #[test]
    fn a_topology_that_cannot_be_read_is_missing_and_empty() {
        let mut state = ThermalPass::default();
        let mut reader = Fake {
            sensors: working().sensors,
            ..Fake::default()
        };
        let (thermal, reads) = pass(&mut state, &mut reader, SOURCES);
        let thermal = thermal.expect("the other sources still make a block");
        assert!(reads.missing.contains(Source::Topology));
        assert!(thermal.topology.is_empty());
        assert_eq!(thermal.sensors.len(), 1);
        assert_eq!(reader.calls.iter().filter(|&&c| c == "topology").count(), 1);
        pass(&mut state, &mut reader, SOURCES);
        assert_eq!(
            reader.calls.iter().filter(|&&c| c == "topology").count(),
            2,
            "a failed read is tried again, not cached"
        );
    }

    #[test]
    fn temperature_alone_still_gets_the_packages_without_reporting_the_topology() {
        let mut state = ThermalPass::default();
        let mut reader = working();
        let (_, reads) = pass(&mut state, &mut reader, Source::Temperature.into());
        assert_eq!(reader.packages_seen, [vec![0]]);
        assert_eq!(reads.active, Sources::from(Source::Temperature));
        assert!(!reads.active.contains(Source::Topology));
    }

    #[test]
    fn package_watts_is_known_only_when_every_package_has_a_rate() {
        let power = |watts: &[Option<f32>]| {
            watts
                .iter()
                .enumerate()
                .map(|(package, &watts)| PackagePower {
                    package: package as u32,
                    watts,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            total_watts(&power(&[Some(10.0), Some(5.5)])),
            Measured::Known(15.5)
        );
        assert_eq!(total_watts(&power(&[Some(10.0), None])), Measured::Pending);
        assert_eq!(total_watts(&power(&[None])), Measured::Pending);
        assert_eq!(total_watts(&[]), Measured::Pending);

        // The first pass has no rate yet: Pending, though the source is readable and active.
        let mut state = ThermalPass::default();
        let mut reader = Fake {
            power: Some(power(&[None])),
            ..working()
        };
        let (thermal, reads) = pass(&mut state, &mut reader, SOURCES);
        assert_eq!(thermal.expect("block").package_watts, Measured::Pending);
        assert!(reads.active.contains(Source::PackagePower));
        assert!(!reads.missing.contains(Source::PackagePower));
    }

    #[test]
    fn cores_group_by_siblings_then_by_place_then_alone() {
        let mut cpus: Vec<CpuPlace> = (0..6).map(place).collect();
        // CPUs 0 and 1 name each other; 2 and 3 name no siblings but share a core id; 4 and 5
        // know nothing.
        cpus[0].siblings = vec![0, 1];
        cpus[1].siblings = vec![0, 1];
        for (cpu, place) in cpus.iter_mut().enumerate().skip(2) {
            place.siblings = Vec::new();
            place.core_id = Some(if cpu < 4 { 7 } else { 8 + cpu as u32 });
        }
        for place in &mut cpus[4..] {
            place.package = None;
            place.core_id = None;
        }
        let cores = Topology { cpus }.cores();
        assert_eq!(
            cores
                .iter()
                .map(|core| core.cpus.clone())
                .collect::<Vec<_>>(),
            [vec![0, 1], vec![2, 3], vec![4], vec![5]]
        );
        assert_eq!(cores[1].core_id, Some(7));
        assert_eq!(cores[2].core_id, None);
    }

    #[test]
    fn clusters_fall_back_to_cluster_ids_and_flag_only_low_power_efficiency_cores() {
        // Two P cores with their own L2 and the L3, two E cores sharing an L2 and the L3, two
        // low-power E cores sharing an L2 and no L3.
        let mut cpus: Vec<CpuPlace> = (0..6).map(place).collect();
        let l3: Vec<u32> = vec![0, 1, 2, 3];
        for (cpu, place) in cpus.iter_mut().enumerate() {
            place.kind = if cpu < 2 {
                CoreKind::Performance
            } else {
                CoreKind::Efficiency
            };
            place.l2 = Some(match cpu {
                0 => vec![0],
                1 => vec![1],
                2 | 3 => vec![2, 3],
                _ => vec![4, 5],
            });
            place.l3 = (cpu < 4).then(|| l3.clone());
        }
        let clusters = Topology { cpus: cpus.clone() }.clusters();
        assert_eq!(
            clusters
                .iter()
                .map(|cluster| (cluster.cpus.clone(), cluster.low_power))
                .collect::<Vec<_>>(),
            [
                (vec![0], false),
                (vec![1], false),
                (vec![2, 3], false),
                (vec![4, 5], true)
            ]
        );
        // Without any L3 no cluster is low power: nothing to compare with.
        for place in &mut cpus {
            place.l3 = None;
        }
        assert!(
            Topology { cpus: cpus.clone() }
                .clusters()
                .iter()
                .all(|cluster| !cluster.low_power)
        );
        // Without an L2 the cluster id groups, and without that the CPU is alone.
        for (cpu, place) in cpus.iter_mut().enumerate() {
            place.l2 = None;
            place.cluster = (cpu < 4).then_some(cpu as u32 / 2);
        }
        let clusters = Topology { cpus }.clusters();
        assert_eq!(
            clusters.iter().map(|c| c.cpus.clone()).collect::<Vec<_>>(),
            [vec![0, 1], vec![2, 3], vec![4], vec![5]]
        );
        assert_eq!(clusters[0].kind, CoreKind::Performance);
    }

    #[test]
    fn ccds_follow_the_l3_groups_in_the_order_of_their_lowest_cpu() {
        let with_l3 = |groups: &[&[u32]]| {
            let mut cpus: Vec<CpuPlace> = (0..8).map(place).collect();
            for group in groups {
                for &cpu in *group {
                    cpus[cpu as usize].l3 = Some(group.to_vec());
                }
            }
            Topology { cpus }
        };
        let two = with_l3(&[&[0, 1, 2, 3], &[4, 5, 6, 7]]);
        assert_eq!(two.ccds(2), Some(vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]));
        // Zen 2: two core complexes to a CCD.
        let four = with_l3(&[&[0, 1], &[2, 3], &[4, 5], &[6, 7]]);
        assert_eq!(four.ccds(2), Some(vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]));
        assert_eq!(
            four.ccds(4).map(|ccds| ccds.len()),
            Some(4),
            "as many sensors as groups"
        );
        assert_eq!(four.ccds(3), None, "no CCD count the groups explain");
        assert_eq!(four.ccds(0), None);
        assert_eq!(flat(4).ccds(1), None, "no L3 information");
    }

    #[test]
    fn the_helpers_read_sensors_by_package_and_core_id_and_never_by_cpu_number() {
        let mut topology = flat(4);
        // An Intel hybrid-style numbering: cores 0, 4, 8 and 12.
        for (index, place) in topology.cpus.iter_mut().enumerate() {
            place.core_id = Some(4 * index as u32);
        }
        let cores = topology.cores();
        let thermal = Thermal {
            topology: Arc::new(topology),
            sensors: vec![
                sensor(SensorPlace::Package(0), Some(60.0), Some(100.0)),
                sensor(
                    SensorPlace::Core {
                        package: 0,
                        core_id: 4,
                    },
                    Some(55.0),
                    Some(100.0),
                ),
                sensor(
                    SensorPlace::Core {
                        package: 0,
                        core_id: 8,
                    },
                    None,
                    Some(105.0),
                ),
                sensor(
                    SensorPlace::Core {
                        package: 1,
                        core_id: 0,
                    },
                    Some(40.0),
                    None,
                ),
            ],
            core_throttles: vec![CoreThrottle {
                package: 0,
                core_id: 8,
                count: 5,
            }],
            ..Thermal::default()
        };
        assert_eq!(thermal.package_celsius(0), Some(60.0));
        assert_eq!(thermal.package_celsius(1), None);
        assert_eq!(thermal.core_celsius(&cores[0]), None, "package 1's core 0");
        assert_eq!(thermal.core_celsius(&cores[1]), Some(55.0));
        assert_eq!(thermal.core_celsius(&cores[2]), None, "no reading");
        assert_eq!(thermal.core_celsius(&cores[3]), None, "no sensor");
        assert_eq!(thermal.tj_max(), Some(105.0), "the highest critical");
        assert_eq!(thermal.core_throttle(&cores[2]), Some(5));
        assert_eq!(thermal.core_throttle(&cores[1]), None);
        assert_eq!(Thermal::default().tj_max(), None);
        let mut unknown = cores[1].clone();
        unknown.core_id = None;
        assert_eq!(thermal.core_celsius(&unknown), None);
    }

    #[test]
    fn ccd_sensors_name_the_cpus_of_their_core_complex_dies() {
        let mut topology = flat(8);
        for place in &mut topology.cpus {
            let group: Vec<u32> = if place.cpu < 4 {
                (0..4).collect()
            } else {
                (4..8).collect()
            };
            place.l3 = Some(group);
        }
        let ccd = |n| sensor(SensorPlace::Ccd(n), Some(50.0), None);
        let thermal = Thermal {
            topology: Arc::new(topology),
            sensors: vec![ccd(0), ccd(1)],
            ..Thermal::default()
        };
        assert_eq!(thermal.ccd_cpus(0), Some(vec![0, 1, 2, 3]));
        assert_eq!(thermal.ccd_cpus(1), Some(vec![4, 5, 6, 7]));
        assert_eq!(thermal.ccd_cpus(2), None);
        assert_eq!(Thermal::default().ccd_cpus(0), None);
    }

    #[test]
    fn places_and_packages_are_looked_up_by_cpu_number() {
        let mut topology = flat(3);
        topology.cpus[2].package = Some(1);
        topology.cpus[1].package = None;
        assert_eq!(topology.place(2).map(|place| place.package), Some(Some(1)));
        assert!(topology.place(3).is_none());
        assert_eq!(
            topology.packages(),
            [0, 1],
            "unknown packages are not listed"
        );
        assert!(Topology::default().is_empty());
    }
}
