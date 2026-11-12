//! The CPU topology, temperature sensors, thermal throttle counters and RAPL package power, from
//! sysfs, read on the background thread while their sources are demanded (`ThermalScan`, the
//! `ThermalReader` of the Linux platform; `model::thermal::ThermalPass` decides when each is
//! read). Every reader takes the sysfs root (`/sys` unless a test points it at a tree), reads
//! only the files below, and never escalates privilege.
//!
//! - Topology: `devices/system/cpu/online`, and for each online CPU `topology/*`,
//!   `cache/index*/{level,shared_cpu_list}` and `cpufreq/cpuinfo_max_freq`.
//! - Temperature: the hwmon chips named `coretemp` and `k10temp` (`class/hwmon/hwmon*/name`, then
//!   `temp*_label`, `temp*_input`, `temp*_crit`), and only where neither reads, the chips
//!   `cpu_thermal` and `soc_thermal` and the thermal zones of type `x86_pkg_temp`. Every other
//!   chip (`nvme`, `drivetemp`, `amdgpu`, `acpitz`, ...) has only its `name` read: reading its
//!   sensors can wake the device.
//! - Throttling: `cpuN/thermal_throttle/{core,package}_throttle_count`, once per physical core
//!   and once per package. Intel only.
//! - Power: the RAPL domains `class/powercap/intel-rapl:N` named `package-P` or
//!   `package-P-die-D`: `energy_uj` and `max_energy_range_uj`, whose change between two
//!   passes is the energy used. Since 2020 most kernels let only root read `energy_uj`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(test)]
use std::cell::RefCell;

use super::core_kinds_in;
use super::counters::parse_cpu_list;
use crate::model::CoreKind;
use crate::model::thermal::{
    CoreThrottle, CpuPlace, PackagePower, PackageThrottle, Sensor, SensorPlace, ThermalReader,
    Topology,
};

/// The most characters of a sensor label that are kept: the kernel's are a few words, and the
/// text is untrusted.
const LABEL_LIMIT: usize = 64;

/// The temperatures a sensor may report, in degrees Celsius, as the driver does not: a value
/// outside it is no reading.
const COLDEST: f32 = -100.0;
const HOTTEST: f32 = 200.0;

/// The power no package draws, in watts. A rate above it is a counter that went wrong, and the
/// longest a counter may go unread is the time it needs to wrap at this power.
const CEILING_WATTS: f64 = 2000.0;

/// A sysfs tree.
struct Sysfs {
    root: PathBuf,
    /// Every file `text` tried to read, in order, for the tests that say which files were opened.
    #[cfg(test)]
    reads: RefCell<Vec<PathBuf>>,
}

impl Sysfs {
    fn new(root: &Path) -> Self {
        Sysfs {
            root: root.to_path_buf(),
            #[cfg(test)]
            reads: RefCell::new(Vec::new()),
        }
    }

    /// A file's text with invalid UTF-8 replaced; None when it cannot be read.
    fn text(&self, relative: impl AsRef<Path>) -> Option<String> {
        let path = self.root.join(relative);
        #[cfg(test)]
        self.reads.borrow_mut().push(path.clone());
        fs::read(path)
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The names in a directory, sorted; empty when it cannot be listed.
    fn entries(&self, relative: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.root.join(relative))
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .collect();
        names.sort_unstable();
        names
    }

    /// The directories of `prefix` and a number in `relative`, by number: `hwmon2` before
    /// `hwmon10`, which a sort of the names would reverse.
    fn numbered(&self, relative: &str, prefix: &str) -> Vec<(u32, String)> {
        let mut found: Vec<(u32, String)> = self
            .entries(relative)
            .into_iter()
            .filter_map(|name| Some((name.strip_prefix(prefix)?.parse().ok()?, name)))
            .collect();
        found.sort_unstable();
        found
    }
}

/// An id the kernel prints, None for the -1 it prints for one it does not know.
fn parse_id(text: &str) -> Option<u32> {
    text.trim().parse().ok()
}

fn parse_count(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

/// Degrees Celsius from the millidegrees of a hwmon or thermal zone file; None outside
/// `COLDEST` to `HOTTEST`.
fn parse_celsius(text: &str) -> Option<f32> {
    let millidegrees: i64 = text.trim().parse().ok()?;
    let celsius = millidegrees as f32 / 1000.0;
    (COLDEST..=HOTTEST).contains(&celsius).then_some(celsius)
}

/// MHz from `cpuinfo_max_freq`, which is in kHz; None for zero.
fn parse_max_mhz(text: &str) -> Option<f32> {
    let kilohertz: u64 = text.trim().parse().ok()?;
    (kilohertz > 0).then(|| kilohertz as f32 / 1000.0)
}

/// The online CPUs a CPU list names, None when it names none.
fn online_list(text: &str, online: &[u32]) -> Option<Vec<u32>> {
    let mut cpus = parse_cpu_list(text)?;
    cpus.retain(|cpu| online.contains(cpu));
    (!cpus.is_empty()).then_some(cpus)
}

/// A sensor label as kept: trimmed and bounded.
fn clean_label(text: &str) -> String {
    text.trim().chars().take(LABEL_LIMIT).collect()
}

/// The package a RAPL domain called `package-P` or `package-P-die-D` is of.
fn package_of_domain(name: &str) -> Option<u32> {
    let rest = name.trim().strip_prefix("package-")?;
    match rest.split_once("-die-") {
        Some((package, die)) => {
            die.parse::<u32>().ok()?;
            package.parse().ok()
        }
        None => rest.parse().ok(),
    }
}

/// One hwmon chip.
struct Chip {
    /// Its directory below the root.
    dir: String,
    /// What its `name` file says.
    name: String,
}

/// One temperature input of a chip with its label, when it has one.
struct Input {
    number: u32,
    label: Option<String>,
}

/// The energy counter of one RAPL domain at one reading.
#[derive(Clone, Copy)]
struct Energy {
    microjoules: u64,
    /// `max_energy_range_uj`: the value the counter wraps at.
    range: Option<u64>,
    at: Instant,
}

/// The watts between two readings of one domain, None where they do not say: the counter did not
/// move, it went backwards by more than one wrap could explain, the interval is long enough for
/// the counter to have wrapped at `CEILING_WATTS` (so a count may hide a whole turn), or the
/// power is above that ceiling.
///
/// A counter that went backwards counts as one wrap only when its range is known and both
/// readings lie within it, and then the energy is the new reading plus the range less the old.
fn watts_between(before: Energy, after: Energy) -> Option<f32> {
    let seconds = after.at.checked_duration_since(before.at)?.as_secs_f64();
    if seconds <= 0.0 {
        return None;
    }
    let microjoules = match (after.microjoules.cmp(&before.microjoules), after.range) {
        (std::cmp::Ordering::Equal, _) => return None,
        (std::cmp::Ordering::Greater, range) => {
            // Forward: unless the interval could hide a whole turn of the counter.
            if range.is_some_and(|range| seconds >= wrap_seconds(range)) {
                return None;
            }
            after.microjoules - before.microjoules
        }
        (std::cmp::Ordering::Less, Some(range))
            if before.microjoules <= range
                && after.microjoules <= range
                && seconds < wrap_seconds(range) =>
        {
            after.microjoules + range - before.microjoules
        }
        (std::cmp::Ordering::Less, _) => return None,
    };
    let watts = microjoules as f64 / 1e6 / seconds;
    (watts <= CEILING_WATTS).then_some(watts as f32)
}

/// How long a counter with this range takes to wrap at `CEILING_WATTS`: an interval shorter than
/// this holds at most one wrap.
fn wrap_seconds(range: u64) -> f64 {
    range as f64 / 1e6 / CEILING_WATTS
}

/// The Linux thermal reader.
pub struct ThermalScan {
    sysfs: Sysfs,
    /// The energy counters of the previous `package_power`, by domain directory.
    energy: HashMap<String, Energy>,
}

impl Default for ThermalScan {
    fn default() -> Self {
        Self::under(Path::new("/sys"))
    }
}

impl ThermalScan {
    pub fn new() -> Self {
        Self::default()
    }

    /// A scan of the sysfs tree at `root`.
    fn under(root: &Path) -> Self {
        ThermalScan {
            sysfs: Sysfs::new(root),
            energy: HashMap::new(),
        }
    }

    fn read_topology(&self) -> Option<Topology> {
        let online = parse_cpu_list(&self.sysfs.text("devices/system/cpu/online")?)?;
        let kinds = core_kinds_in(&self.sysfs.root);
        let cpus = online
            .iter()
            .map(|&cpu| self.place(cpu, &online, &kinds))
            .collect();
        Some(Topology { cpus })
    }

    /// One online CPU's place. A file the kernel does not have, or that holds -1, is None.
    fn place(&self, cpu: u32, online: &[u32], kinds: &HashMap<u32, CoreKind>) -> CpuPlace {
        let base = format!("devices/system/cpu/cpu{cpu}");
        let topology = |name: &str| self.sysfs.text(format!("{base}/topology/{name}"));
        let id = |name: &str| topology(name).and_then(|text| parse_id(&text));
        let list = |name: &str| topology(name).and_then(|text| online_list(&text, online));
        let (l2, l3) = self.caches(&base, online);
        CpuPlace {
            cpu,
            package: id("physical_package_id"),
            die: id("die_id"),
            core_id: id("core_id"),
            cluster: id("cluster_id"),
            siblings: list("core_cpus_list")
                .or_else(|| list("thread_siblings_list"))
                .unwrap_or_default(),
            l2,
            l3,
            max_mhz: self
                .sysfs
                .text(format!("{base}/cpufreq/cpuinfo_max_freq"))
                .and_then(|text| parse_max_mhz(&text)),
            kind: kinds.get(&cpu).copied().unwrap_or(CoreKind::Unknown),
        }
    }

    /// The online CPUs that share a CPU's L2 and L3 caches, found by each cache's `level` file:
    /// the number of the `index` directory says nothing about which cache it is.
    fn caches(&self, base: &str, online: &[u32]) -> (Option<Vec<u32>>, Option<Vec<u32>>) {
        let (mut l2, mut l3) = (None, None);
        for (index, _) in self.sysfs.numbered(&format!("{base}/cache"), "index") {
            let dir = format!("{base}/cache/index{index}");
            let level = self
                .sysfs
                .text(format!("{dir}/level"))
                .and_then(|text| parse_id(&text));
            let slot = match level {
                Some(2) => &mut l2,
                Some(3) => &mut l3,
                _ => continue,
            };
            if slot.is_none() {
                *slot = self
                    .sysfs
                    .text(format!("{dir}/shared_cpu_list"))
                    .and_then(|text| online_list(&text, online));
            }
        }
        (l2, l3)
    }

    /// Every hwmon chip with its name, in numeric order. Only the `name` files are read.
    fn chips(&self) -> Vec<Chip> {
        self.sysfs
            .numbered("class/hwmon", "hwmon")
            .into_iter()
            .filter_map(|(_, dir)| {
                let name = self.sysfs.text(format!("class/hwmon/{dir}/name"))?;
                Some(Chip {
                    dir: format!("class/hwmon/{dir}"),
                    name: name.trim().to_owned(),
                })
            })
            .collect()
    }

    /// The temperature inputs of `chip` in numeric order, each with its label if it has one.
    fn inputs(&self, chip: &Chip) -> Vec<Input> {
        let mut numbers: Vec<u32> = self
            .sysfs
            .entries(&chip.dir)
            .iter()
            .filter_map(|name| {
                name.strip_prefix("temp")?
                    .strip_suffix("_input")?
                    .parse()
                    .ok()
            })
            .collect();
        numbers.sort_unstable();
        numbers
            .into_iter()
            .map(|number| Input {
                number,
                label: self
                    .sysfs
                    .text(format!("{}/temp{number}_label", chip.dir))
                    .map(|text| clean_label(&text)),
            })
            .collect()
    }

    /// The sensor of one input: its reading and its critical temperature.
    fn sensor(&self, chip: &Chip, input: &Input, label: String, place: SensorPlace) -> Sensor {
        let number = input.number;
        Sensor {
            label,
            place,
            celsius: self
                .sysfs
                .text(format!("{}/temp{number}_input", chip.dir))
                .and_then(|text| parse_celsius(&text)),
            critical: self
                .sysfs
                .text(format!("{}/temp{number}_crit", chip.dir))
                .and_then(|text| parse_celsius(&text)),
        }
    }

    /// The sensors of a `coretemp` chip: `Package id P` is the package's, and `Core N` is the core
    /// whose `core_id` is N (the input's file number and the CPU numbers are neither: an Intel
    /// hybrid part numbers its P-cores 0, 4, 8, and `temp2_input` is `Core 0`). A core belongs to
    /// the package its chip's `Package id` label names; a chip with none is the only package's
    /// when the machine has one, and places no core sensor otherwise.
    fn coretemp(&self, chip: &Chip, packages: &[u32]) -> Vec<Sensor> {
        let inputs = self.inputs(chip);
        let labelled = |input: &Input| input.label.as_deref().map(str::to_owned);
        let package_label =
            |label: &str| -> Option<u32> { label.strip_prefix("Package id ")?.trim().parse().ok() };
        let own = inputs
            .iter()
            .filter_map(labelled)
            .find_map(|label| package_label(&label));
        let package = own.or(match packages {
            [only] => Some(*only),
            _ => None,
        });
        inputs
            .iter()
            .filter_map(|input| {
                let label = labelled(input)?;
                let place = if let Some(package) = package_label(&label) {
                    SensorPlace::Package(package)
                } else {
                    let core_id = label.strip_prefix("Core ")?.trim().parse().ok()?;
                    SensorPlace::Core {
                        package: package?,
                        core_id,
                    }
                };
                Some(self.sensor(chip, input, label, place))
            })
            .collect()
    }

    /// The sensors of the `index`-th `k10temp` chip: one package sensor, `Tdie` where the chip has
    /// it and `Tctl` (which carries an offset on some parts) where it does not, and one `Ccd` per
    /// `TccdK` input, numbered from `first_ccd` by rank of K, whatever indices K has. A chip of an
    /// older family has one unlabelled input, which is the package's. No per-core value is made
    /// up: the chip has none.
    fn k10temp(&self, chip: &Chip, index: usize, first_ccd: u32, packages: &[u32]) -> Vec<Sensor> {
        let inputs = self.inputs(chip);
        let package = packages
            .get(index)
            .copied()
            .unwrap_or_else(|| u32::try_from(index).unwrap_or(u32::MAX));
        let labelled = |wanted: &str| {
            inputs
                .iter()
                .find(|input| input.label.as_deref() == Some(wanted))
        };
        let mut sensors = Vec::new();
        let chosen = labelled("Tdie")
            .or_else(|| labelled("Tctl"))
            .map(|input| (input, input.label.clone().unwrap_or_default()))
            .or_else(|| {
                inputs
                    .iter()
                    .find(|input| input.label.is_none())
                    .map(|input| (input, clean_label(&chip.name)))
            });
        if let Some((input, label)) = chosen {
            sensors.push(self.sensor(chip, input, label, SensorPlace::Package(package)));
        }
        let mut ccds: Vec<(u32, &Input, &str)> = inputs
            .iter()
            .filter_map(|input| {
                let label = input.label.as_deref()?;
                Some((label.strip_prefix("Tccd")?.parse().ok()?, input, label))
            })
            .collect();
        ccds.sort_unstable_by_key(|&(k, _, _)| k);
        for (rank, (_, input, label)) in (first_ccd..).zip(ccds) {
            sensors.push(self.sensor(chip, input, label.to_owned(), SensorPlace::Ccd(rank)));
        }
        sensors
    }

    /// Package-only sensors for machines with no CPU chip that reads: the hwmon chips
    /// `cpu_thermal` and `soc_thermal` (ARM), and the thermal zones of type `x86_pkg_temp`. The
    /// n-th is the n-th package's.
    fn fallbacks(&self, chips: &[Chip], packages: &[u32]) -> Vec<Sensor> {
        let package = |index: usize| {
            packages
                .get(index)
                .copied()
                .unwrap_or_else(|| u32::try_from(index).unwrap_or(u32::MAX))
        };
        let mut sensors: Vec<Sensor> = Vec::new();
        for chip in chips
            .iter()
            .filter(|chip| chip.name == "cpu_thermal" || chip.name == "soc_thermal")
        {
            if let Some(input) = self.inputs(chip).first() {
                let label = input
                    .label
                    .clone()
                    .unwrap_or_else(|| clean_label(&chip.name));
                let place = SensorPlace::Package(package(sensors.len()));
                sensors.push(self.sensor(chip, input, label, place));
            }
        }
        for (_, zone) in self.sysfs.numbered("class/thermal", "thermal_zone") {
            let dir = format!("class/thermal/{zone}");
            let kind = self.sysfs.text(format!("{dir}/type"));
            if kind.as_deref().map(str::trim) == Some("x86_pkg_temp") {
                sensors.push(Sensor {
                    label: String::from("x86_pkg_temp"),
                    place: SensorPlace::Package(package(sensors.len())),
                    celsius: self
                        .sysfs
                        .text(format!("{dir}/temp"))
                        .and_then(|text| parse_celsius(&text)),
                    critical: None,
                });
            }
        }
        sensors
    }

    fn read_temperatures(&self, packages: &[u32]) -> Option<Vec<Sensor>> {
        let chips = self.chips();
        let mut sensors = Vec::new();
        let (mut k10temps, mut ccds) = (0, 0);
        for chip in &chips {
            match chip.name.as_str() {
                "coretemp" => sensors.extend(self.coretemp(chip, packages)),
                "k10temp" => {
                    let found = self.k10temp(chip, k10temps, ccds, packages);
                    k10temps += 1;
                    ccds += found
                        .iter()
                        .filter(|sensor| matches!(sensor.place, SensorPlace::Ccd(_)))
                        .count() as u32;
                    sensors.extend(found);
                }
                _ => {}
            }
        }
        let reads = |sensors: &[Sensor]| sensors.iter().any(|sensor| sensor.celsius.is_some());
        if !reads(&sensors) {
            sensors = self.fallbacks(&chips, packages);
        }
        reads(&sensors).then_some(sensors)
    }

    /// A throttle counter of `cpu`.
    fn throttle_count(&self, cpu: u32, name: &str) -> Option<u64> {
        self.sysfs
            .text(format!(
                "devices/system/cpu/cpu{cpu}/thermal_throttle/{name}"
            ))
            .and_then(|text| parse_count(&text))
    }

    /// The core counters, one read per physical core (from the lowest of its CPUs that has the
    /// file; SMT siblings show the same count) and the package counters, one read per package.
    fn read_throttles(
        &self,
        topology: &Topology,
    ) -> Option<(Vec<CoreThrottle>, Vec<PackageThrottle>)> {
        let cores: Vec<CoreThrottle> = topology
            .cores()
            .iter()
            .filter_map(|core| {
                Some(CoreThrottle {
                    package: core.package?,
                    core_id: core.core_id?,
                    count: core
                        .cpus
                        .iter()
                        .find_map(|&cpu| self.throttle_count(cpu, "core_throttle_count"))?,
                })
            })
            .collect();
        let packages: Vec<PackageThrottle> = topology
            .packages()
            .into_iter()
            .filter_map(|package| {
                let count = topology
                    .cpus
                    .iter()
                    .filter(|place| place.package == Some(package))
                    .find_map(|place| self.throttle_count(place.cpu, "package_throttle_count"))?;
                Some(PackageThrottle { package, count })
            })
            .collect();
        (!cores.is_empty() || !packages.is_empty()).then_some((cores, packages))
    }

    /// The RAPL package domains in numeric order: the directory and the package it is of.
    /// Subdomains (`intel-rapl:0:0`: core, uncore, dram), `intel-rapl-mmio` and the platform
    /// domain (`psys`) are not read.
    fn rapl_domains(&self) -> Vec<(String, u32)> {
        let mut domains: Vec<(u32, String, u32)> = self
            .sysfs
            .entries("class/powercap")
            .into_iter()
            .filter_map(|dir| {
                let number = dir.strip_prefix("intel-rapl:")?.parse().ok()?;
                let name = self.sysfs.text(format!("class/powercap/{dir}/name"))?;
                Some((number, dir, package_of_domain(&name)?))
            })
            .collect();
        domains.sort_unstable();
        domains
            .into_iter()
            .map(|(_, dir, package)| (dir, package))
            .collect()
    }

    /// One pass of the package power at `now`: the energy of every readable package domain, and
    /// the watts of each package that has a rate. A domain that cannot be read is left out, and
    /// its earlier reading forgotten. None when no domain can be read.
    fn power_at(&mut self, now: Instant) -> Option<Vec<PackagePower>> {
        let mut next: HashMap<String, Energy> = HashMap::new();
        let mut packages: Vec<(u32, Option<f64>)> = Vec::new();
        for (dir, package) in self.rapl_domains() {
            let base = format!("class/powercap/{dir}");
            let Some(microjoules) = self
                .sysfs
                .text(format!("{base}/energy_uj"))
                .and_then(|text| parse_count(&text))
            else {
                continue;
            };
            let energy = Energy {
                microjoules,
                range: self
                    .sysfs
                    .text(format!("{base}/max_energy_range_uj"))
                    .and_then(|text| parse_count(&text)),
                at: now,
            };
            let watts = self
                .energy
                .get(&dir)
                .and_then(|&before| watts_between(before, energy))
                .map(f64::from);
            match packages.iter_mut().find(|(found, _)| *found == package) {
                // A package is as known as its least known domain.
                Some((_, total)) => *total = total.zip(watts).map(|(a, b)| a + b),
                None => packages.push((package, watts)),
            }
            next.insert(dir, energy);
        }
        self.energy = next;
        (!packages.is_empty()).then(|| {
            packages
                .into_iter()
                .map(|(package, watts)| PackagePower {
                    package,
                    watts: watts.map(|watts| watts as f32),
                })
                .collect()
        })
    }
}

impl ThermalReader for ThermalScan {
    fn topology(&mut self) -> Option<Topology> {
        self.read_topology()
    }

    fn temperatures(&mut self, packages: &[u32]) -> Option<Vec<Sensor>> {
        self.read_temperatures(packages)
    }

    fn throttle_counts(
        &mut self,
        topology: &Topology,
    ) -> Option<(Vec<CoreThrottle>, Vec<PackageThrottle>)> {
        self.read_throttles(topology)
    }

    fn package_power(&mut self) -> Option<Vec<PackagePower>> {
        self.power_at(Instant::now())
    }

    fn forget_power(&mut self) {
        self.energy.clear();
    }
}

/// Test only: the topologies the reader finds in the sysfs trees its own tests use, by name: a
/// capture of a real virtual machine (`vm`), and the synthetic Intel hybrid part with SMT
/// (`hybrid`), AMD part with two CCDs (`amd`) and tiled part with low-power E-cores (`tiled`).
/// A view that draws a topology (the die's floorplan) tests against what the parse-from-root
/// path gives, not against a hand-built value.
#[cfg(test)]
#[expect(dead_code, reason = "the die's floorplan test is the first reader")]
pub fn fixture_topologies() -> Vec<(&'static str, Topology)> {
    tests::fixture_topologies()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    /// A sysfs tree in a directory of its own below the temporary directory, removed when it goes.
    struct Tree {
        root: PathBuf,
    }

    impl Tree {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "isotop-thermal-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).expect("a directory to build the tree in");
            Tree { root }
        }

        /// A tree from a capture: `== path` headers, each followed by that file's text, with
        /// `#` comment lines before the first.
        fn from_capture(capture: &str) -> Self {
            let tree = Tree::new();
            let mut files: Vec<(&str, Vec<&str>)> = Vec::new();
            for line in capture.lines() {
                if let Some(path) = line.strip_prefix("== ") {
                    files.push((path, Vec::new()));
                } else if let Some((_, text)) = files.last_mut() {
                    text.push(line);
                } else {
                    assert!(
                        line.starts_with('#'),
                        "text before the first header: {line}"
                    );
                }
            }
            for (path, text) in files {
                tree.file(path, &format!("{}\n", text.join("\n")));
            }
            tree
        }

        fn file(&self, path: &str, text: &str) -> &Self {
            let path = self.root.join(path);
            fs::create_dir_all(path.parent().expect("a parent")).expect("directories");
            fs::write(path, text).expect("a file");
            self
        }

        /// A directory where a file would be, which cannot be read as one.
        fn unreadable(&self, path: &str) -> &Self {
            fs::create_dir_all(self.root.join(path)).expect("a directory");
            self
        }

        fn scan(&self) -> ThermalScan {
            ThermalScan::under(&self.root)
        }

        /// One CPU's topology and cache files. `-1` is what the kernel prints for an id it does
        /// not know; `None` leaves out a file or a cache.
        fn cpu(&self, spec: &Cpu) -> &Self {
            let base = format!("devices/system/cpu/cpu{}", spec.cpu);
            for (name, value) in [
                ("physical_package_id", spec.package),
                ("die_id", spec.die),
                ("core_id", spec.core),
                ("cluster_id", spec.cluster),
            ] {
                self.file(&format!("{base}/topology/{name}"), &format!("{value}\n"));
            }
            self.file(
                &format!("{base}/topology/core_cpus_list"),
                &format!("{}\n", spec.siblings),
            );
            self.file(&format!("{base}/cache/index0/level"), "1\n");
            self.file(&format!("{base}/cache/index1/level"), "1\n");
            if let Some(l2) = &spec.l2 {
                self.file(&format!("{base}/cache/index2/level"), "2\n");
                self.file(
                    &format!("{base}/cache/index2/shared_cpu_list"),
                    &format!("{l2}\n"),
                );
            }
            if let Some(l3) = &spec.l3 {
                self.file(&format!("{base}/cache/index3/level"), "3\n");
                self.file(
                    &format!("{base}/cache/index3/shared_cpu_list"),
                    &format!("{l3}\n"),
                );
            }
            if let Some(khz) = spec.max_khz {
                self.file(
                    &format!("{base}/cpufreq/cpuinfo_max_freq"),
                    &format!("{khz}\n"),
                );
            }
            self
        }

        fn hwmon(&self, number: u32, name: &str) -> &Self {
            self.file(
                &format!("class/hwmon/hwmon{number}/name"),
                &format!("{name}\n"),
            )
        }

        /// A temperature input of a chip: its label, reading in millidegrees and critical value.
        fn temp(
            &self,
            chip: u32,
            input: u32,
            label: Option<&str>,
            milli: &str,
            crit: Option<&str>,
        ) -> &Self {
            let base = format!("class/hwmon/hwmon{chip}/temp{input}");
            if let Some(label) = label {
                self.file(&format!("{base}_label"), &format!("{label}\n"));
            }
            self.file(&format!("{base}_input"), &format!("{milli}\n"));
            if let Some(crit) = crit {
                self.file(&format!("{base}_crit"), &format!("{crit}\n"));
            }
            self
        }

        fn online(&self, list: &str) -> &Self {
            self.file("devices/system/cpu/online", &format!("{list}\n"))
        }

        fn rapl(&self, dir: &str, name: &str, energy: &str, range: Option<&str>) -> &Self {
            let base = format!("class/powercap/{dir}");
            self.file(&format!("{base}/name"), &format!("{name}\n"));
            self.file(&format!("{base}/energy_uj"), &format!("{energy}\n"));
            if let Some(range) = range {
                self.file(
                    &format!("{base}/max_energy_range_uj"),
                    &format!("{range}\n"),
                );
            }
            self
        }

        fn energy(&self, dir: &str, value: &str) -> &Self {
            self.file(
                &format!("class/powercap/{dir}/energy_uj"),
                &format!("{value}\n"),
            )
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).ok();
        }
    }

    /// The files of one CPU: a string is the text of a file or a cache, None leaves it out, and
    /// an id of -1 is what the kernel prints for one it does not know.
    struct Cpu {
        cpu: u32,
        package: i32,
        die: i32,
        core: i32,
        cluster: i32,
        siblings: String,
        l2: Option<String>,
        l3: Option<String>,
        max_khz: Option<u32>,
    }

    /// A CPU on package 0 and die 0 with no cluster, caches or clock; the tests add what they need.
    fn cpu(cpu: u32, core: i32, siblings: &str) -> Cpu {
        Cpu {
            cpu,
            package: 0,
            die: 0,
            core,
            cluster: -1,
            siblings: siblings.to_owned(),
            l2: None,
            l3: None,
            max_khz: None,
        }
    }

    fn text(list: &str) -> Option<String> {
        Some(list.to_owned())
    }

    /// Synthetic, not a capture: an Intel hybrid part of two P-cores with SMT (CPUs 0 to 3, core
    /// ids 0 and 4, as such parts number them) and four E-cores sharing one L2 (CPUs 4 to 7, core
    /// ids 8 to 11), one L3 over all of them.
    fn synthetic_hybrid() -> Tree {
        let tree = Tree::new();
        tree.online("0-7");
        tree.file("devices/cpu_core/cpus", "0-3\n");
        tree.file("devices/cpu_atom/cpus", "4-7\n");
        for number in 0..4 {
            let pair = if number < 2 { "0-1" } else { "2-3" };
            tree.cpu(&Cpu {
                l2: text(pair),
                l3: text("0-7"),
                max_khz: Some(5_000_000),
                ..cpu(number, (number / 2 * 4) as i32, pair)
            });
        }
        for number in 4..8 {
            tree.cpu(&Cpu {
                cluster: 2,
                l2: text("4-7"),
                l3: text("0-7"),
                max_khz: Some(3_800_000),
                ..cpu(number, (number + 4) as i32, &number.to_string())
            });
        }
        tree
    }

    /// Synthetic, not a capture: an AMD part with two CCDs of two cores, each core with SMT whose
    /// sibling is 4 CPUs on (the numbering Linux gives Ryzen), sparse core ids 0, 1, 8 and 9, and
    /// one L3 per CCD.
    fn synthetic_amd() -> Tree {
        let tree = Tree::new();
        tree.online("0-7");
        for number in 0..8_u32 {
            let core = number % 4;
            let siblings = format!("{core},{}", core + 4);
            tree.cpu(&Cpu {
                l2: Some(siblings.clone()),
                l3: text(if core < 2 { "0-1,4-5" } else { "2-3,6-7" }),
                max_khz: Some(4_700_000),
                ..cpu(number, [0, 1, 8, 9][core as usize], &siblings)
            });
        }
        tree
    }

    /// Synthetic, not a capture: a tiled part with two P-cores with SMT (CPUs 0 to 3), two
    /// E-cores sharing an L2 on the compute tile (CPUs 4 and 5) and two low-power E-cores that
    /// share an L2 and have no L3 (CPUs 6 and 7).
    fn synthetic_tiled() -> Tree {
        let tree = Tree::new();
        tree.online("0-7");
        tree.file("devices/cpu_core/cpus", "0-3\n");
        tree.file("devices/cpu_atom/cpus", "4-7\n");
        // (siblings, L2, L3, core id) of CPUs 0 to 7.
        let table = [
            ("0-1", "0-1", Some("0-5"), 0),
            ("0-1", "0-1", Some("0-5"), 0),
            ("2-3", "2-3", Some("0-5"), 4),
            ("2-3", "2-3", Some("0-5"), 4),
            ("4", "4-5", Some("0-5"), 8),
            ("5", "4-5", Some("0-5"), 9),
            ("6", "6-7", None, 16),
            ("7", "6-7", None, 17),
        ];
        for (number, (siblings, l2, l3, core)) in (0..).zip(table) {
            tree.cpu(&Cpu {
                l2: text(l2),
                l3: l3.and_then(text),
                ..cpu(number, core, siblings)
            });
        }
        tree
    }

    fn read(tree: &Tree) -> Topology {
        tree.scan().topology().expect("a topology")
    }

    /// See `super::fixture_topologies`.
    #[expect(dead_code, reason = "the die's floorplan test is the first reader")]
    pub(super) fn fixture_topologies() -> Vec<(&'static str, Topology)> {
        vec![
            (
                "vm",
                read(&Tree::from_capture(include_str!(
                    "fixtures/x86_cpu_topology_vm_4cpu.txt"
                ))),
            ),
            ("hybrid", read(&synthetic_hybrid())),
            ("amd", read(&synthetic_amd())),
            ("tiled", read(&synthetic_tiled())),
        ]
    }

    fn cores_of(topology: &Topology) -> Vec<Vec<u32>> {
        topology.cores().into_iter().map(|core| core.cpus).collect()
    }

    #[test]
    fn this_virtual_machines_real_topology_parses_from_its_capture() {
        let tree = Tree::from_capture(include_str!("fixtures/x86_cpu_topology_vm_4cpu.txt"));
        let topology = read(&tree);
        assert_eq!(topology.cpus.len(), 4);
        assert_eq!(topology.packages(), [0]);
        assert_eq!(
            cores_of(&topology),
            [[0], [1], [2], [3]],
            "one thread to a core"
        );
        let first = topology.place(0).expect("cpu0");
        assert_eq!(
            (first.package, first.die, first.core_id, first.cluster),
            (Some(0), Some(0), Some(0), Some(0))
        );
        assert_eq!(first.siblings, [0]);
        assert_eq!(first.l2, Some(vec![0]), "each vCPU has an L2 of its own");
        assert_eq!(first.l3, Some(vec![0, 1, 2, 3]), "and one L3 spans them");
        assert_eq!(first.max_mhz, None, "the VM has no cpufreq");
        assert_eq!(first.kind, CoreKind::Unknown, "no hybrid sysfs either");
        assert_eq!(topology.l3_groups(), [[0, 1, 2, 3]]);
        assert_eq!(topology.clusters().len(), 4);
        // Nothing else was captured: the machine has no sensors, counters or power.
        let mut scan = tree.scan();
        assert!(scan.temperatures(&[0]).is_none());
        assert!(scan.throttle_counts(&topology).is_none());
        assert!(scan.package_power().is_none());
    }

    #[test]
    fn topology_maps_an_intel_hybrid_part_with_smt() {
        let tree = synthetic_hybrid();
        let topology = read(&tree);
        assert_eq!(
            cores_of(&topology),
            [vec![0, 1], vec![2, 3], vec![4], vec![5], vec![6], vec![7]],
            "SMT siblings share a core, E-cores have none"
        );
        let cores = topology.cores();
        assert_eq!(
            cores
                .iter()
                .filter_map(|core| core.core_id)
                .collect::<Vec<_>>(),
            [0, 4, 8, 9, 10, 11],
            "core ids are not CPU numbers"
        );
        assert_eq!(cores[0].kind, CoreKind::Performance);
        assert_eq!(cores[2].kind, CoreKind::Efficiency);
        let clusters = topology.clusters();
        assert_eq!(
            clusters.iter().map(|c| c.cpus.clone()).collect::<Vec<_>>(),
            [vec![0, 1], vec![2, 3], vec![4, 5, 6, 7]],
            "each P-core has an L2 of its own, the E-cores share one"
        );
        assert!(clusters.iter().all(|cluster| !cluster.low_power));
        assert_eq!(topology.place(5).and_then(|p| p.max_mhz), Some(3800.0));
        assert_eq!(topology.place(1).and_then(|p| p.max_mhz), Some(5000.0));
        assert_eq!(topology.place(6).and_then(|p| p.cluster), Some(2));
        assert_eq!(
            topology.place(0).and_then(|p| p.cluster),
            None,
            "printed -1"
        );
    }

    #[test]
    fn topology_maps_an_amd_part_with_two_ccds() {
        let tree = synthetic_amd();
        let topology = read(&tree);
        assert_eq!(
            cores_of(&topology),
            [vec![0, 4], vec![1, 5], vec![2, 6], vec![3, 7]],
            "siblings four CPUs apart, as Ryzen numbers them"
        );
        let ids: Vec<u32> = topology
            .cores()
            .iter()
            .filter_map(|core| core.core_id)
            .collect();
        assert_eq!(ids, [0, 1, 8, 9]);
        assert_eq!(topology.l3_groups(), [vec![0, 1, 4, 5], vec![2, 3, 6, 7]]);
        assert_eq!(
            topology.ccds(2),
            Some(vec![vec![0, 1, 4, 5], vec![2, 3, 6, 7]]),
            "two L3 groups for two CCD sensors"
        );
        assert_eq!(
            topology.ccds(1),
            Some(vec![vec![0, 1, 2, 3, 4, 5, 6, 7]]),
            "a Zen 2 CCD holds two core complexes"
        );
        assert_eq!(topology.ccds(3), None);
        assert!(
            topology
                .cpus
                .iter()
                .all(|place| place.kind == CoreKind::Unknown)
        );
    }

    #[test]
    fn topology_finds_a_tiled_part_lp_e_core_cluster() {
        let topology = read(&synthetic_tiled());
        let clusters = topology.clusters();
        assert_eq!(
            clusters
                .iter()
                .map(|cluster| (cluster.cpus.clone(), cluster.kind, cluster.low_power))
                .collect::<Vec<_>>(),
            [
                (vec![0, 1], CoreKind::Performance, false),
                (vec![2, 3], CoreKind::Performance, false),
                (vec![4, 5], CoreKind::Efficiency, false),
                (vec![6, 7], CoreKind::Efficiency, true),
            ],
            "the E-cores with an L2 and no L3 are their own, low-power cluster"
        );
        assert_eq!(topology.place(6).map(|place| place.l3.clone()), Some(None));
        assert_eq!(topology.l3_groups(), [[0, 1, 2, 3, 4, 5]]);
    }

    #[test]
    fn offline_cpus_are_left_out_of_the_topology_and_the_sibling_lists() {
        let tree = synthetic_hybrid();
        // CPUs 1 and 6 go offline: they are not read, and the others no longer list them.
        tree.online("0,2-5,7");
        let topology = read(&tree);
        assert_eq!(
            topology
                .cpus
                .iter()
                .map(|place| place.cpu)
                .collect::<Vec<_>>(),
            [0, 2, 3, 4, 5, 7]
        );
        assert_eq!(topology.place(0).map(|p| p.siblings.clone()), Some(vec![0]));
        assert_eq!(
            topology.place(4).and_then(|p| p.l2.clone()),
            Some(vec![4, 5, 7])
        );
        assert_eq!(
            topology.place(2).and_then(|p| p.l3.clone()),
            Some(vec![0, 2, 3, 4, 5, 7])
        );
        assert_eq!(
            cores_of(&topology),
            [vec![0], vec![2, 3], vec![4], vec![5], vec![7]]
        );
        let scan = tree.scan();
        scan.read_topology();
        assert!(
            scan.sysfs
                .reads
                .borrow()
                .iter()
                .all(|path| !path.to_string_lossy().contains("/cpu1/")),
            "an offline CPU's files are not opened"
        );
    }

    #[test]
    fn missing_topology_files_and_minus_one_are_none_not_zero() {
        let tree = Tree::new();
        tree.online("0-1");
        // CPU 0 has no topology files at all; CPU 1 prints -1 for what it does not know and has
        // no sibling list.
        tree.file(
            "devices/system/cpu/cpu1/topology/physical_package_id",
            "-1\n",
        );
        tree.file("devices/system/cpu/cpu1/topology/core_id", "-1\n");
        tree.file("devices/system/cpu/cpu1/topology/die_id", "-1\n");
        tree.file("devices/system/cpu/cpu1/cpufreq/cpuinfo_max_freq", "0\n");
        let topology = read(&tree);
        for place in &topology.cpus {
            assert_eq!(
                (place.package, place.die, place.core_id, place.cluster),
                (None, None, None, None)
            );
            assert!(place.siblings.is_empty() && place.l2.is_none() && place.l3.is_none());
            assert_eq!(place.max_mhz, None);
        }
        assert_eq!(
            cores_of(&topology),
            [[0], [1]],
            "each CPU is a core of its own"
        );
        assert!(topology.packages().is_empty());
        // The sibling list falls back to the older name.
        tree.file(
            "devices/system/cpu/cpu0/topology/thread_siblings_list",
            "0-1\n",
        );
        tree.file(
            "devices/system/cpu/cpu1/topology/thread_siblings_list",
            "0-1\n",
        );
        assert_eq!(cores_of(&read(&tree)), [[0, 1]]);
    }

    #[test]
    fn caches_are_found_by_their_level_file_not_their_index_number() {
        let tree = Tree::new();
        tree.online("0-1");
        let base = "devices/system/cpu/cpu0/cache";
        // L3 first, then an L1, then the L2: and one directory with no level at all.
        tree.file(&format!("{base}/index0/level"), "3\n");
        tree.file(&format!("{base}/index0/shared_cpu_list"), "0-1\n");
        tree.file(&format!("{base}/index1/level"), "1\n");
        tree.file(&format!("{base}/index1/shared_cpu_list"), "0\n");
        tree.file(&format!("{base}/index2/level"), "2\n");
        tree.file(&format!("{base}/index2/shared_cpu_list"), "0\n");
        tree.file(&format!("{base}/index3/shared_cpu_list"), "0-1\n");
        let place = read(&tree).place(0).cloned().expect("cpu0");
        assert_eq!(place.l3, Some(vec![0, 1]));
        assert_eq!(place.l2, Some(vec![0]));
    }

    #[test]
    fn the_topology_is_missing_when_the_online_list_cannot_be_read() {
        let tree = Tree::new();
        assert!(tree.scan().topology().is_none(), "no online file");
        tree.online("garbage");
        assert!(tree.scan().topology().is_none(), "no CPU list");
        tree.online("");
        assert!(tree.scan().topology().is_none(), "an empty list is no CPU");
    }

    #[test]
    fn readings_parse_millidegrees_ids_counts_and_clocks_strictly() {
        assert_eq!(parse_celsius("45000\n"), Some(45.0));
        assert_eq!(parse_celsius("-5500"), Some(-5.5));
        assert_eq!(parse_celsius("200000"), Some(200.0));
        assert_eq!(parse_celsius("200001"), None, "above 200 degrees");
        assert_eq!(parse_celsius("-100001"), None, "below -100 degrees");
        assert_eq!(parse_celsius("4294967295"), None, "an all-ones register");
        assert_eq!(parse_celsius("hot"), None);
        assert_eq!(parse_celsius(""), None);
        assert_eq!(parse_id("3\n"), Some(3));
        assert_eq!(parse_id("-1\n"), None);
        assert_eq!(parse_count("17"), Some(17));
        assert_eq!(parse_count("-1"), None);
        assert_eq!(parse_max_mhz("4800000\n"), Some(4800.0));
        assert_eq!(parse_max_mhz("0"), None);
        assert_eq!(package_of_domain("package-0\n"), Some(0));
        assert_eq!(package_of_domain("package-1-die-3"), Some(1));
        for name in [
            "core",
            "uncore",
            "dram",
            "psys",
            "package-",
            "package-x",
            "package-0-die-",
        ] {
            assert_eq!(package_of_domain(name), None, "{name}");
        }
        assert_eq!(
            clean_label(&format!(" {}\n", "x".repeat(200))).len(),
            LABEL_LIMIT
        );
    }

    /// A coretemp chip with a package sensor and cores, as the driver names them.
    fn coretemp(tree: &Tree, chip: u32, package: u32, cores: &[(u32, &str)]) {
        tree.hwmon(chip, "coretemp");
        tree.temp(
            chip,
            1,
            Some(&format!("Package id {package}")),
            "61000",
            Some("100000"),
        );
        for (input, &(core_id, milli)) in (2..).zip(cores) {
            tree.temp(
                chip,
                input,
                Some(&format!("Core {core_id}")),
                milli,
                Some("100000"),
            );
        }
    }

    fn core(package: u32, core_id: u32) -> SensorPlace {
        SensorPlace::Core { package, core_id }
    }

    fn readings(sensors: &[Sensor]) -> Vec<(SensorPlace, Option<f32>)> {
        sensors.iter().map(|s| (s.place, s.celsius)).collect()
    }

    #[test]
    fn coretemp_core_labels_map_to_core_ids_per_package() {
        let tree = Tree::new();
        // Package 1's chip has the lower number and package 0's the higher, and `hwmon10`
        // sorts before `hwmon9` as text: chips are read in numeric order.
        coretemp(&tree, 9, 0, &[(0, "40000"), (1, "41000")]);
        coretemp(&tree, 10, 1, &[(0, "50000"), (1, "51000")]);
        let sensors = tree.scan().temperatures(&[0, 1]).expect("sensors");
        assert_eq!(
            readings(&sensors),
            [
                (SensorPlace::Package(0), Some(61.0)),
                (core(0, 0), Some(40.0)),
                (core(0, 1), Some(41.0)),
                (SensorPlace::Package(1), Some(61.0)),
                (core(1, 0), Some(50.0)),
                (core(1, 1), Some(51.0)),
            ]
        );
        assert_eq!(sensors[1].label, "Core 0");
        assert!(sensors.iter().all(|sensor| sensor.critical == Some(100.0)));
    }

    #[test]
    fn hybrid_coretemp_inputs_are_matched_by_label_not_by_file_number_or_cpu_number() {
        let tree = Tree::new();
        // An Intel hybrid part: the cores are 0, 4 and 8, so `temp3_input` is `Core 4` and there
        // is no CPU 4 sensor.
        coretemp(&tree, 3, 0, &[(0, "42000"), (4, "44000"), (8, "48000")]);
        let sensors = tree.scan().temperatures(&[0]).expect("sensors");
        assert_eq!(
            readings(&sensors),
            [
                (SensorPlace::Package(0), Some(61.0)),
                (core(0, 0), Some(42.0)),
                (core(0, 4), Some(44.0)),
                (core(0, 8), Some(48.0)),
            ]
        );
        // And the topology's core for those ids finds the same sensors.
        let hybrid = synthetic_hybrid();
        let topology = read(&hybrid);
        let thermal = crate::model::Thermal {
            sensors,
            topology: topology.clone().into(),
            ..crate::model::Thermal::default()
        };
        let cores = topology.cores();
        assert_eq!(thermal.core_celsius(&cores[0]), Some(42.0));
        assert_eq!(thermal.core_celsius(&cores[1]), Some(44.0));
        assert_eq!(
            thermal.core_celsius(&cores[2]),
            Some(48.0),
            "core id 8 is CPU 4"
        );
    }

    #[test]
    fn a_coretemp_chip_without_a_package_label_takes_the_only_package_or_places_no_core() {
        let tree = Tree::new();
        tree.hwmon(0, "coretemp");
        tree.temp(0, 1, Some("Core 0"), "40000", None);
        tree.temp(0, 2, Some("Core 4"), "41000", None);
        tree.temp(0, 3, Some("Unexpected label"), "42000", None);
        let sensors = tree.scan().temperatures(&[3]).expect("one package");
        assert_eq!(
            readings(&sensors),
            [(core(3, 0), Some(40.0)), (core(3, 4), Some(41.0))]
        );
        assert!(
            tree.scan().temperatures(&[0, 1]).is_none(),
            "two packages and no way to say which: no core sensor, so no reading"
        );
        assert!(
            tree.scan().temperatures(&[]).is_none(),
            "an unknown topology too"
        );
    }

    #[test]
    fn k10temp_prefers_tdie_and_maps_sparse_tccd_indices() {
        let tree = Tree::new();
        tree.hwmon(1, "k10temp");
        tree.temp(1, 1, Some("Tctl"), "62500", Some("95000"));
        tree.temp(1, 2, Some("Tdie"), "52500", Some("95000"));
        // A part with CCDs 1 and 3 enabled and 2 and 4 absent.
        tree.temp(1, 3, Some("Tccd3"), "55000", None);
        tree.temp(1, 4, Some("Tccd1"), "51000", None);
        let sensors = tree.scan().temperatures(&[0]).expect("sensors");
        assert_eq!(
            readings(&sensors),
            [
                (SensorPlace::Package(0), Some(52.5)),
                (SensorPlace::Ccd(0), Some(51.0)),
                (SensorPlace::Ccd(1), Some(55.0)),
            ],
            "Tdie not Tctl, and the CCDs ranked by their index"
        );
        assert_eq!(sensors[0].label, "Tdie");
        assert_eq!(sensors[0].critical, Some(95.0));
        assert_eq!(sensors[1].label, "Tccd1");
        assert!(
            sensors
                .iter()
                .all(|s| !matches!(s.place, SensorPlace::Core { .. })),
            "no per-core value is made up"
        );
    }

    #[test]
    fn k10temp_without_tdie_uses_tctl_and_two_chips_continue_the_ccd_ranks() {
        let tree = Tree::new();
        tree.hwmon(1, "k10temp");
        tree.temp(1, 1, Some("Tctl"), "60000", None);
        tree.temp(1, 2, Some("Tccd1"), "50000", None);
        tree.temp(1, 3, Some("Tccd2"), "51000", None);
        tree.hwmon(2, "k10temp");
        tree.temp(2, 1, Some("Tctl"), "61000", None);
        tree.temp(2, 2, Some("Tccd1"), "52000", None);
        let sensors = tree.scan().temperatures(&[0, 1]).expect("sensors");
        assert_eq!(
            readings(&sensors),
            [
                (SensorPlace::Package(0), Some(60.0)),
                (SensorPlace::Ccd(0), Some(50.0)),
                (SensorPlace::Ccd(1), Some(51.0)),
                (SensorPlace::Package(1), Some(61.0)),
                (SensorPlace::Ccd(2), Some(52.0)),
            ]
        );
        assert_eq!(sensors[0].label, "Tctl");
    }

    #[test]
    fn a_legacy_k10temp_with_one_unlabelled_input_gives_a_package_sensor() {
        let tree = Tree::new();
        tree.hwmon(0, "k10temp");
        tree.temp(0, 1, None, "47000", Some("100000"));
        let sensors = tree.scan().temperatures(&[0]).expect("sensors");
        assert_eq!(readings(&sensors), [(SensorPlace::Package(0), Some(47.0))]);
        assert_eq!(sensors[0].label, "k10temp");
    }

    #[test]
    fn only_cpu_sensor_chips_are_read() {
        let tree = Tree::new();
        for (number, name) in [(0, "nvme"), (1, "drivetemp"), (2, "amdgpu"), (3, "acpitz")] {
            tree.hwmon(number, name);
            tree.temp(number, 1, Some("Composite"), "35000", Some("85000"));
        }
        coretemp(&tree, 4, 0, &[(0, "40000")]);
        tree.file("class/thermal/thermal_zone0/type", "acpitz\n");
        tree.file("class/thermal/thermal_zone0/temp", "30000\n");
        let mut scan = tree.scan();
        let sensors = scan.temperatures(&[0]).expect("the CPU chip reads");
        assert_eq!(sensors.len(), 2);
        let reads: Vec<String> = scan
            .sysfs
            .reads
            .borrow()
            .iter()
            .map(|path| {
                path.strip_prefix(&tree.root)
                    .expect("inside the tree")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        for chip in 0..4 {
            let prefix = format!("class/hwmon/hwmon{chip}/");
            let opened: Vec<&String> = reads.iter().filter(|r| r.starts_with(&prefix)).collect();
            assert_eq!(
                opened,
                [&format!("{prefix}name")],
                "only the name of hwmon{chip} is read"
            );
        }
        assert!(
            reads.iter().any(|r| r == "class/hwmon/hwmon4/temp2_input"),
            "{reads:?}"
        );
        assert!(
            reads.iter().all(|r| !r.starts_with("class/thermal")),
            "no zone is opened while a CPU chip reads: {reads:?}"
        );
    }

    #[test]
    fn fallbacks_are_read_only_where_no_cpu_chip_gives_a_reading() {
        let tree = Tree::new();
        tree.hwmon(0, "nvme");
        tree.temp(0, 1, Some("Composite"), "35000", None);
        tree.file("class/thermal/thermal_zone0/type", "acpitz\n");
        tree.file("class/thermal/thermal_zone0/temp", "30000\n");
        tree.file("class/thermal/thermal_zone2/type", "x86_pkg_temp\n");
        tree.file("class/thermal/thermal_zone2/temp", "58000\n");
        let mut scan = tree.scan();
        let sensors = scan.temperatures(&[0]).expect("the package zone");
        assert_eq!(readings(&sensors), [(SensorPlace::Package(0), Some(58.0))]);
        assert_eq!(sensors[0].label, "x86_pkg_temp");
        let reads = scan.sysfs.reads.borrow();
        assert!(
            reads
                .iter()
                .any(|path| path.ends_with("thermal_zone2/temp"))
        );
        assert!(
            reads
                .iter()
                .all(|path| !path.ends_with("thermal_zone0/temp")),
            "the acpitz zone's temperature is not read"
        );
        drop(reads);

        // An ARM board: the chips `cpu_thermal` and `soc_thermal`, one unlabelled input each.
        let board = Tree::new();
        board.hwmon(1, "cpu_thermal");
        board.temp(1, 1, None, "41000", None);
        board.hwmon(2, "soc_thermal");
        board.temp(2, 1, Some("soc"), "43000", None);
        let sensors = board.scan().temperatures(&[]).expect("the board's chips");
        assert_eq!(
            readings(&sensors),
            [
                (SensorPlace::Package(0), Some(41.0)),
                (SensorPlace::Package(1), Some(43.0))
            ]
        );
        assert_eq!(sensors[0].label, "cpu_thermal");
        assert_eq!(sensors[1].label, "soc");

        // A CPU chip that reads wins, and a CPU chip that does not lets the fallback in.
        tree.hwmon(3, "coretemp");
        tree.temp(3, 1, Some("Package id 0"), "-5000000", None);
        assert_eq!(
            readings(&tree.scan().temperatures(&[0]).expect("fallback")),
            [(SensorPlace::Package(0), Some(58.0))],
            "a coretemp chip with no plausible reading is no reading"
        );
    }

    #[test]
    fn a_sensor_that_cannot_be_read_is_none_and_a_machine_with_none_has_no_temperature() {
        let tree = Tree::new();
        tree.hwmon(0, "coretemp");
        tree.temp(0, 1, Some("Package id 0"), "55000", None);
        tree.file("class/hwmon/hwmon0/temp2_label", "Core 0\n");
        tree.unreadable("class/hwmon/hwmon0/temp2_input");
        tree.temp(0, 3, Some("Core 1"), "999999999", None);
        let sensors = tree.scan().temperatures(&[0]).expect("one reads");
        assert_eq!(
            readings(&sensors),
            [
                (SensorPlace::Package(0), Some(55.0)),
                (core(0, 0), None),
                (core(0, 1), None),
            ],
            "unreadable and implausible are None, never 0 degrees"
        );
        let empty = Tree::new();
        empty.hwmon(0, "coretemp");
        empty.file("class/hwmon/hwmon0/temp1_label", "Package id 0\n");
        empty.unreadable("class/hwmon/hwmon0/temp1_input");
        assert!(empty.scan().temperatures(&[0]).is_none());
        assert!(
            Tree::new().scan().temperatures(&[0]).is_none(),
            "no hwmon at all"
        );
    }

    /// Four online CPUs, SMT siblings (0, 1) and (2, 3), and the throttle files of all four.
    fn throttling() -> (Tree, Topology) {
        let tree = Tree::new();
        tree.online("0-3");
        for cpu_number in 0..4 {
            tree.cpu(&cpu(
                cpu_number,
                (cpu_number / 2 * 4) as i32,
                if cpu_number < 2 { "0-1" } else { "2-3" },
            ));
            let base = format!("devices/system/cpu/cpu{cpu_number}/thermal_throttle");
            tree.file(
                &format!("{base}/core_throttle_count"),
                &format!("{}\n", 5 + cpu_number / 2 * 10),
            );
            tree.file(&format!("{base}/package_throttle_count"), "7\n");
        }
        let topology = read(&tree);
        (tree, topology)
    }

    #[test]
    fn throttle_counts_are_read_once_per_core() {
        let (tree, topology) = throttling();
        let mut scan = tree.scan();
        let (cores, packages) = scan.throttle_counts(&topology).expect("counters");
        assert_eq!(
            cores,
            [
                CoreThrottle {
                    package: 0,
                    core_id: 0,
                    count: 5
                },
                CoreThrottle {
                    package: 0,
                    core_id: 4,
                    count: 15
                },
            ]
        );
        assert_eq!(
            packages,
            [PackageThrottle {
                package: 0,
                count: 7
            }]
        );
        let opened = |name: &str| {
            scan.sysfs
                .reads
                .borrow()
                .iter()
                .filter(|path| path.ends_with(name))
                .cloned()
                .collect::<Vec<_>>()
        };
        let cores_read = opened("core_throttle_count");
        assert_eq!(cores_read.len(), 2, "{cores_read:?}");
        assert!(cores_read[0].to_string_lossy().contains("/cpu0/"));
        assert!(cores_read[1].to_string_lossy().contains("/cpu2/"));
        assert_eq!(
            opened("package_throttle_count").len(),
            1,
            "once per package"
        );
    }

    #[test]
    fn throttle_counts_are_absent_where_the_kernel_has_none_or_the_core_is_unnamed() {
        let tree = Tree::new();
        tree.online("0-1");
        for cpu_number in 0..2 {
            tree.cpu(&cpu(cpu_number, cpu_number as i32, &cpu_number.to_string()));
        }
        let topology = read(&tree);
        assert!(
            tree.scan().throttle_counts(&topology).is_none(),
            "no counter files: AMD, a VM"
        );
        // CPU 1's counter is unreadable and CPU 0's is there: only core 0 has an entry.
        tree.file(
            "devices/system/cpu/cpu0/thermal_throttle/core_throttle_count",
            "3\n",
        );
        tree.unreadable("devices/system/cpu/cpu1/thermal_throttle/core_throttle_count");
        let (cores, packages) = tree.scan().throttle_counts(&topology).expect("one counter");
        assert_eq!(
            cores,
            [CoreThrottle {
                package: 0,
                core_id: 0,
                count: 3
            }]
        );
        assert!(packages.is_empty());
        // A core with no package id cannot be keyed, whatever its file says.
        let mut unnamed = topology.clone();
        unnamed.cpus[0].package = None;
        assert!(tree.scan().throttle_counts(&unnamed).is_none());
        assert!(tree.scan().throttle_counts(&Topology::default()).is_none());
    }

    #[test]
    fn rapl_domains_are_the_package_domains_only() {
        let tree = Tree::new();
        tree.rapl("intel-rapl:0", "package-0", "1000", Some("262143328850"));
        tree.rapl("intel-rapl:0:0", "core", "5", None);
        tree.rapl("intel-rapl:0:1", "uncore", "6", None);
        tree.rapl("intel-rapl:0:2", "dram", "7", None);
        tree.rapl("intel-rapl:1", "psys", "8", None);
        tree.rapl("intel-rapl:2", "package-1-die-0", "9", None);
        tree.rapl("intel-rapl-mmio:0", "package-0", "10", None);
        let mut scan = tree.scan();
        let power = scan.power_at(Instant::now()).expect("readable");
        assert_eq!(
            power.iter().map(|p| p.package).collect::<Vec<_>>(),
            [0, 1],
            "package-0 and package-1-die-0 only"
        );
        let reads = scan.sysfs.reads.borrow();
        let touched = |part: &str| {
            reads
                .iter()
                .any(|path| path.to_string_lossy().contains(part))
        };
        assert!(!touched("intel-rapl:0:") && !touched("mmio"), "{reads:?}");
        assert!(
            !reads.iter().any(|p| p.ends_with("intel-rapl:1/energy_uj")),
            "the platform domain's energy is not read"
        );
    }

    #[test]
    fn the_first_rapl_reading_has_no_watts_and_the_second_has() {
        let tree = Tree::new();
        tree.rapl("intel-rapl:0", "package-0", "1000000", Some("262143328850"));
        let mut scan = tree.scan();
        let start = Instant::now();
        let first = scan.power_at(start).expect("readable");
        assert_eq!(
            first,
            [PackagePower {
                package: 0,
                watts: None
            }],
            "pending, not 0 W"
        );
        // 30 J in 2 s is 15 W.
        tree.energy("intel-rapl:0", "31000000");
        let second = scan
            .power_at(start + Duration::from_secs(2))
            .expect("readable");
        assert_eq!(
            second,
            [PackagePower {
                package: 0,
                watts: Some(15.0)
            }]
        );
        // Forgetting starts over: the next reading has no earlier one to difference with.
        scan.forget_power();
        tree.energy("intel-rapl:0", "61000000");
        let third = scan
            .power_at(start + Duration::from_secs(4))
            .expect("readable");
        assert_eq!(third[0].watts, None);
    }

    #[test]
    fn rapl_watts_survive_counter_wrap() {
        let range = 262_143_328_850_u64;
        let tree = Tree::new();
        tree.rapl(
            "intel-rapl:0",
            "package-0",
            &(range - 1_000_000).to_string(),
            Some(&range.to_string()),
        );
        let mut scan = tree.scan();
        let start = Instant::now();
        scan.power_at(start);
        // The counter passes its range and starts again: 1 J before the wrap and 3 J after is
        // 4 J in 2 s.
        tree.energy("intel-rapl:0", "3000000");
        let power = scan
            .power_at(start + Duration::from_secs(2))
            .expect("readable");
        let watts = power[0].watts.expect("a rate over one wrap");
        assert!((watts - 2.0).abs() < 1e-4, "{watts}");
    }

    #[test]
    fn rapl_ambiguity_gives_no_watts_instead_of_a_guess() {
        let range = 262_143_328_850_u64;
        let before = Energy {
            microjoules: 1_000_000,
            range: Some(range),
            at: Instant::now(),
        };
        let after = |microjoules: u64, range: Option<u64>, seconds: u64| Energy {
            microjoules,
            range,
            at: before.at + Duration::from_secs(seconds),
        };
        // A backwards counter without a range is not a wrap, and nor is one outside its range.
        assert_eq!(watts_between(before, after(500, None, 2)), None);
        assert_eq!(
            watts_between(before, after(500, Some(400), 2)),
            None,
            "reading above the range"
        );
        let high = Energy {
            microjoules: range + 5,
            ..before
        };
        assert_eq!(
            watts_between(high, after(500, Some(range), 2)),
            None,
            "old reading above it"
        );
        // An interval as long as the counter takes to wrap at 2 kW (131 s) may hide a whole turn,
        // whichever way the counter looks to have moved.
        assert_eq!(
            watts_between(before, after(500, Some(range), 140)),
            None,
            "backwards"
        );
        assert_eq!(
            watts_between(before, after(2_000_000, Some(range), 140)),
            None,
            "forwards"
        );
        let near_the_top = Energy {
            microjoules: range - 1000,
            ..before
        };
        assert!(
            watts_between(near_the_top, after(500, Some(range), 100)).is_some(),
            "a shorter interval holds one wrap, and that wrap is 1,500 uJ"
        );
        // A forward interval with an unknown range is taken as it reads.
        assert_eq!(watts_between(before, after(21_000_000, None, 4)), Some(5.0));
        // Time that did not pass, or ran backwards, has no rate.
        assert_eq!(watts_between(before, after(2_000_000, None, 0)), None);
        assert_eq!(watts_between(after(2_000_000, None, 4), before), None);
    }

    #[test]
    fn a_rapl_counter_that_did_not_move_has_no_watts_and_nor_has_an_impossible_rate() {
        let tree = Tree::new();
        tree.rapl("intel-rapl:0", "package-0", "5000000", Some("262143328850"));
        let mut scan = tree.scan();
        let start = Instant::now();
        scan.power_at(start);
        let still = scan
            .power_at(start + Duration::from_secs(2))
            .expect("readable");
        assert_eq!(
            still[0].watts, None,
            "a counter that did not move is not 0 W"
        );
        // 5,000 J in 2 s is 2.5 kW.
        tree.energy("intel-rapl:0", "5005000000");
        let absurd = scan
            .power_at(start + Duration::from_secs(4))
            .expect("readable");
        assert_eq!(absurd[0].watts, None, "above the 2 kW ceiling");
        // 3,995 J in 2 s is 1,997.5 W, which is not.
        tree.energy("intel-rapl:0", "9000000000");
        let heavy = scan
            .power_at(start + Duration::from_secs(6))
            .expect("readable");
        assert_eq!(heavy[0].watts, Some(1997.5));
    }

    #[test]
    fn unreadable_rapl_is_unreadable_not_zero() {
        // Since 2020 most kernels let only root read `energy_uj`: here a directory stands in for
        // a file whose open is refused (the tests may run as root, which no mode bit stops).
        let tree = Tree::new();
        tree.rapl("intel-rapl:0", "package-0", "0", Some("262143328850"));
        fs::remove_file(tree.root.join("class/powercap/intel-rapl:0/energy_uj")).expect("remove");
        tree.unreadable("class/powercap/intel-rapl:0/energy_uj");
        let mut scan = tree.scan();
        assert!(
            scan.package_power().is_none(),
            "no readable domain: the source is missing"
        );
        assert!(scan.package_power().is_none());
        // With one package unreadable and one readable, the readable one is reported.
        tree.rapl("intel-rapl:1", "package-1", "100", None);
        let power = scan.power_at(Instant::now()).expect("one reads");
        assert_eq!(
            power,
            [PackagePower {
                package: 1,
                watts: None
            }]
        );
        // And a machine with no powercap tree has nothing to read.
        assert!(Tree::new().scan().package_power().is_none());
    }

    #[test]
    fn the_domains_of_one_package_add_up_and_need_every_rate() {
        let tree = Tree::new();
        tree.rapl("intel-rapl:0", "package-0-die-0", "1000000", None);
        tree.rapl("intel-rapl:1", "package-0-die-1", "1000000", None);
        let mut scan = tree.scan();
        let start = Instant::now();
        scan.power_at(start);
        tree.energy("intel-rapl:0", "11000000");
        tree.energy("intel-rapl:1", "21000000");
        let both = scan
            .power_at(start + Duration::from_secs(2))
            .expect("readable");
        assert_eq!(
            both,
            [PackagePower {
                package: 0,
                watts: Some(15.0)
            }],
            "5 W and 10 W"
        );
        // One die's counter does not move: the package's power is not known, not half of it.
        tree.energy("intel-rapl:1", "21000000");
        tree.energy("intel-rapl:0", "31000000");
        let half = scan
            .power_at(start + Duration::from_secs(4))
            .expect("readable");
        assert_eq!(
            half,
            [PackagePower {
                package: 0,
                watts: None
            }]
        );
    }

    #[test]
    fn a_whole_tree_is_read_through_the_thermal_reader() {
        let tree = synthetic_hybrid();
        coretemp(
            &tree,
            2,
            0,
            &[
                (0, "40000"),
                (4, "41000"),
                (8, "42000"),
                (9, "43000"),
                (10, "44000"),
                (11, "45000"),
            ],
        );
        tree.hwmon(0, "nvme");
        tree.rapl("intel-rapl:0", "package-0", "100", Some("262143328850"));
        for cpu_number in 0..8 {
            let base = format!("devices/system/cpu/cpu{cpu_number}/thermal_throttle");
            tree.file(&format!("{base}/core_throttle_count"), "2\n");
            tree.file(&format!("{base}/package_throttle_count"), "4\n");
        }
        let mut scan = tree.scan();
        let topology = scan.topology().expect("topology");
        let packages = topology.packages();
        assert_eq!(packages, [0]);
        assert_eq!(
            scan.package_power().expect("readable"),
            [PackagePower {
                package: 0,
                watts: None
            }],
            "the first reading has no rate"
        );
        let sensors = scan.temperatures(&packages).expect("sensors");
        assert_eq!(sensors.len(), 7);
        let (cores, package_counts) = scan.throttle_counts(&topology).expect("counters");
        assert_eq!((cores.len(), package_counts.len()), (6, 1));
        let thermal = crate::model::Thermal {
            topology: topology.into(),
            sensors,
            core_throttles: cores,
            package_throttles: package_counts,
            ..crate::model::Thermal::default()
        };
        let hot = thermal
            .topology
            .cores()
            .into_iter()
            .filter_map(|core| Some((core.cpus.clone(), thermal.core_celsius(&core)?)))
            .collect::<Vec<_>>();
        assert_eq!(
            hot,
            [
                (vec![0, 1], 40.0),
                (vec![2, 3], 41.0),
                (vec![4], 42.0),
                (vec![5], 43.0),
                (vec![6], 44.0),
                (vec![7], 45.0)
            ]
        );
        assert_eq!(thermal.tj_max(), Some(100.0));
        assert_eq!(thermal.package_celsius(0), Some(61.0));
    }

    #[test]
    fn two_packages_keep_their_core_ids_sensors_and_throttle_counts_apart() {
        // Synthetic, not a capture: two packages of two cores, each package with core ids 0 and
        // 1, one thread to a core.
        let tree = Tree::new();
        tree.online("0-3");
        for number in 0..4_u32 {
            tree.cpu(&Cpu {
                package: (number / 2) as i32,
                ..cpu(number, (number % 2) as i32, &number.to_string())
            });
            let base = format!("devices/system/cpu/cpu{number}/thermal_throttle");
            tree.file(
                &format!("{base}/core_throttle_count"),
                &format!("{}\n", 10 + number),
            );
            tree.file(
                &format!("{base}/package_throttle_count"),
                &format!("{}\n", number / 2 + 100),
            );
        }
        coretemp(&tree, 1, 0, &[(0, "40000"), (1, "41000")]);
        coretemp(&tree, 2, 1, &[(0, "50000"), (1, "51000")]);
        let mut scan = tree.scan();
        let topology = scan.topology().expect("topology");
        assert_eq!(topology.packages(), [0, 1]);
        let cores = topology.cores();
        assert_eq!(cores.len(), 4, "core id 0 of each package is its own core");
        let sensors = scan.temperatures(&topology.packages()).expect("sensors");
        let (core_throttles, package_throttles) =
            scan.throttle_counts(&topology).expect("counters");
        let thermal = crate::model::Thermal {
            topology: topology.into(),
            sensors,
            core_throttles,
            package_throttles,
            ..crate::model::Thermal::default()
        };
        let celsius: Vec<Option<f32>> = cores
            .iter()
            .map(|core| thermal.core_celsius(core))
            .collect();
        assert_eq!(celsius, [Some(40.0), Some(41.0), Some(50.0), Some(51.0)]);
        let throttles: Vec<Option<u64>> = cores
            .iter()
            .map(|core| thermal.core_throttle(core))
            .collect();
        assert_eq!(throttles, [Some(10), Some(11), Some(12), Some(13)]);
        assert_eq!(
            thermal.package_throttles,
            [
                PackageThrottle {
                    package: 0,
                    count: 100
                },
                PackageThrottle {
                    package: 1,
                    count: 101
                }
            ]
        );
    }

    #[test]
    fn this_machines_sysfs_reads_without_failing_whatever_it_has() {
        // Sandboxes hide or lack some of these, so only what is shown is checked.
        let mut scan = ThermalScan::new();
        if Path::new("/sys/devices/system/cpu/online").exists() {
            let topology = scan.topology().expect("the online list is there");
            assert!(!topology.is_empty());
            assert!(
                topology
                    .cpus
                    .windows(2)
                    .all(|pair| pair[0].cpu < pair[1].cpu)
            );
            scan.throttle_counts(&topology);
            scan.temperatures(&topology.packages());
        }
        scan.package_power();
        scan.package_power();
    }
}
