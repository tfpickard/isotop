//! The demo's thermal block (`DemoOptions::thermal`): the demo's 16 CPUs presented as an Intel
//! hybrid part with one package and one L3, four P-cores with SMT on CPUs 0 to 7 (core ids 0, 4,
//! 8 and 12, as such a part numbers them) and two clusters of four E-cores on CPUs 8 to 11 and
//! 12 to 15 (core ids 16 to 23), each cluster sharing an L2.
//!
//! Every value is a pure function of `time`, with no state, so a frame at any time, in any order
//! and at negative times, is the same:
//!
//! - A core's temperature is a first-order lag (time constant `LAG`) of the power proxy
//!   `busy * (mhz / max_mhz)^3` of its CPUs, which the demo's CPUs follow through `demo_cpu`. The
//!   proxy is a trigonometric polynomial in the CPU's load, which is a sine of time, so the lag
//!   has a closed form: each harmonic of the proxy is scaled and delayed by `1 / (1 + i k w
//!   LAG)`. The response is the steady state, the one a machine that has run since long before
//!   time 0 has.
//! - The package sensor reads the hottest core, and every sensor has TjMax 100 degrees.
//! - P-core 8 (CPUs 4 and 5) carries an extra heat pulse every `CYCLE` seconds that takes it to
//!   TjMax, where its throttle count rises once. Counts never decrease.
//! - The package's watts follow the sum of the CPUs' proxy, weighted by what a P or E CPU may
//!   draw.

use std::f64::consts::PI;
use std::sync::Arc;

use super::{
    CoreThrottle, CpuPlace, PackagePower, PackageThrottle, Sensor, SensorPlace, Thermal, Topology,
};
use crate::model::{CoreKind, Cpu, Measured, demo_cpu, demo_cpu_with};

/// The core id of the P-core that reaches TjMax (CPUs 4 and 5).
const HOT_CORE: u32 = 8;

/// Seconds between the hot core's visits to TjMax.
const CYCLE: f64 = 40.0;

/// Seconds into each cycle at which the hot core reaches TjMax and its throttle count rises.
const THROTTLE_AT: f64 = 24.0;

/// TjMax, the critical temperature every sensor reports, in degrees Celsius.
const TJ_MAX: f32 = 100.0;

/// The temperature of a core that draws nothing, in degrees Celsius.
const AMBIENT: f64 = 36.0;

/// Degrees Celsius per unit of the lagged power proxy.
const SPAN: f64 = 85.0;

/// The time constant of the lag, in seconds: package-scale thermal inertia.
const LAG: f64 = 3.0;

/// `demo_load`'s angular rate in radians per second, and the phase each CPU number adds.
const LOAD_RATE: f64 = 0.4;
const LOAD_SKEW: f64 = 0.9;

/// Harmonics of the proxy that are kept, and the samples over one period they are found from. The
/// proxy is a polynomial of degree 4 in a sine, so four harmonics are all there are, and sixteen
/// samples find them exactly.
const HARMONICS: usize = 4;
const SAMPLES: usize = 16;

/// Throttle counts already on the clock at time 0, so that they are positive at negative times.
const THROTTLES_BEFORE: i64 = 90;

/// The package is throttled once every this many seconds, at `THROTTLE_AT` as well.
const PACKAGE_CYCLE: f64 = 120.0;
const PACKAGE_THROTTLES_BEFORE: i64 = 7;

/// Watts of the package with every CPU idle, and of one P or E CPU at proxy 1.
const IDLE_WATTS: f64 = 4.0;
const P_WATTS: f64 = 8.0;
const E_WATTS: f64 = 3.0;

/// The demo's CPUs: 0 to 7 are P-cores' threads, 8 to 15 E-cores.
const CPUS: u32 = 16;

fn is_performance(cpu: u32) -> bool {
    cpu < 8
}

/// The highest clock of demo CPU `cpu`, which `demo_cpu` reaches at full load.
fn max_mhz(cpu: u32) -> f32 {
    if is_performance(cpu) { 4800.0 } else { 3200.0 }
}

/// The demo's topology: see the module comment.
pub fn topology() -> Topology {
    let l3: Vec<u32> = (0..CPUS).collect();
    let cpus = (0..CPUS)
        .map(|cpu| {
            let (core_id, cluster, siblings, l2) = if is_performance(cpu) {
                let first = cpu / 2 * 2;
                let pair = vec![first, first + 1];
                (cpu / 2 * 4, cpu / 2, pair.clone(), pair)
            } else {
                let first = 8 + (cpu - 8) / 4 * 4;
                (
                    16 + (cpu - 8),
                    4 + (cpu - 8) / 4,
                    vec![cpu],
                    (first..first + 4).collect(),
                )
            };
            CpuPlace {
                cpu,
                package: Some(0),
                die: Some(0),
                core_id: Some(core_id),
                cluster: Some(cluster),
                siblings,
                l2: Some(l2),
                l3: Some(l3.clone()),
                max_mhz: Some(max_mhz(cpu)),
                kind: if is_performance(cpu) {
                    CoreKind::Performance
                } else {
                    CoreKind::Efficiency
                },
            }
        })
        .collect();
    Topology { cpus }
}

/// The power proxy of `cpu`: `busy * (mhz / max_mhz)^3`.
fn proxy(cpu: &Cpu, max_mhz: f32) -> f64 {
    cpu.busy.zip(cpu.mhz).map_or(0.0, |(busy, mhz)| {
        f64::from(busy) * (f64::from(mhz) / f64::from(max_mhz)).powi(3)
    })
}

/// The proxy of demo CPU `cpu` when its load is `load`, 0 to 1.
fn proxy_at(cpu: u32, load: f64) -> f64 {
    proxy(&demo_cpu_with(cpu, load as f32), max_mhz(cpu))
}

/// The steady response of a first-order lag of time constant `lag` seconds to the proxy of CPU
/// `cpu` at `time`. The proxy is `p(theta)` with `theta = LOAD_RATE * time + LOAD_SKEW * cpu`;
/// its harmonics `a_k cos(k theta) + b_k sin(k theta)` come through the lag as
/// `(a_k - i b_k) / (1 + i q)` with `q = k * LOAD_RATE * lag`.
fn lagged(cpu: u32, time: f64, lag: f64) -> f64 {
    let samples: [f64; SAMPLES] = std::array::from_fn(|index| {
        let theta = 2.0 * PI * index as f64 / SAMPLES as f64;
        proxy_at(cpu, 0.5 + 0.5 * theta.sin())
    });
    let theta = LOAD_RATE * time + LOAD_SKEW * f64::from(cpu);
    let mut response = samples.iter().sum::<f64>() / SAMPLES as f64;
    for harmonic in 1..=HARMONICS {
        let (mut cosine, mut sine) = (0.0, 0.0);
        for (index, sample) in samples.iter().enumerate() {
            let angle = 2.0 * PI * (harmonic * index) as f64 / SAMPLES as f64;
            cosine += sample * angle.cos();
            sine += sample * angle.sin();
        }
        let (a, b) = (2.0 * cosine / SAMPLES as f64, 2.0 * sine / SAMPLES as f64);
        let q = harmonic as f64 * LOAD_RATE * lag;
        let (re, im) = ((a - b * q) / (1.0 + q * q), (b + a * q) / (1.0 + q * q));
        let angle = harmonic as f64 * theta;
        response += re * angle.cos() + im * angle.sin();
    }
    response
}

/// How far the hot core has been taken from its own temperature to TjMax, 0 to 1: it rises over
/// the ten seconds before `THROTTLE_AT`, holds three seconds and falls back over ten.
fn pulse(time: f64) -> f64 {
    let phase = time.rem_euclid(CYCLE);
    let ramp = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    ramp((phase - (THROTTLE_AT - 10.0)) / 10.0) * (1.0 - ramp((phase - (THROTTLE_AT + 3.0)) / 10.0))
}

/// What a counter reads at `time` when it rises by one at `THROTTLE_AT` and every `period`
/// seconds before and after, and `before` of its rises are behind it at time 0; never below 0.
fn counted(time: f64, period: f64, before: i64) -> u64 {
    (((time - THROTTLE_AT) / period).floor() as i64 + 1 + before).max(0) as u64
}

/// The thermal block at `time`.
pub fn reading(time: f64) -> Thermal {
    let topology = topology();
    let cores = topology.cores();
    let temperatures: Vec<(u32, f64)> = cores
        .iter()
        .filter_map(|core| {
            let core_id = core.core_id?;
            let heat = core
                .cpus
                .iter()
                .map(|&cpu| lagged(cpu, time, LAG))
                .sum::<f64>()
                / core.cpus.len() as f64;
            let own = AMBIENT + SPAN * heat;
            let tj_max = f64::from(TJ_MAX);
            let celsius = if core_id == HOT_CORE {
                // The hot core reads TjMax exactly while it is throttled and no more than a
                // degree below it on the way there and back.
                let pulse = pulse(time);
                if pulse >= 1.0 {
                    tj_max
                } else {
                    (own + (tj_max - own) * pulse).round().min(tj_max - 1.0)
                }
            } else {
                // Sensors resolve whole degrees, as the `coretemp` driver's do.
                own.round()
            };
            Some((core_id, celsius))
        })
        .collect();
    let hottest = temperatures
        .iter()
        .map(|&(_, celsius)| celsius)
        .fold(f64::MIN, f64::max);
    let sensor = |label: String, place, celsius: f64| Sensor {
        label,
        place,
        celsius: Some(celsius as f32),
        critical: Some(TJ_MAX),
    };
    let mut sensors = vec![sensor(
        String::from("Package id 0"),
        SensorPlace::Package(0),
        hottest,
    )];
    sensors.extend(temperatures.iter().map(|&(core_id, celsius)| {
        sensor(
            format!("Core {core_id}"),
            SensorPlace::Core {
                package: 0,
                core_id,
            },
            celsius,
        )
    }));
    let hot_count = counted(time, CYCLE, THROTTLES_BEFORE);
    let core_throttles = cores
        .iter()
        .filter_map(|core| {
            let core_id = core.core_id?;
            Some(CoreThrottle {
                package: 0,
                core_id,
                count: if core_id == HOT_CORE { hot_count } else { 0 },
            })
        })
        .collect();
    let watts = (IDLE_WATTS
        + (0..CPUS)
            .map(|cpu| {
                let weight = if is_performance(cpu) {
                    P_WATTS
                } else {
                    E_WATTS
                };
                let now = demo_cpu(time, cpu);
                weight * proxy(&now, max_mhz(cpu))
            })
            .sum::<f64>()) as f32;
    Thermal {
        topology: Arc::new(topology),
        sensors,
        core_throttles,
        package_throttles: vec![PackageThrottle {
            package: 0,
            count: counted(time, PACKAGE_CYCLE, PACKAGE_THROTTLES_BEFORE),
        }],
        package_power: vec![PackagePower {
            package: 0,
            watts: Some(watts),
        }],
        package_watts: Measured::Known(watts),
        pass: time.to_bits(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::SOURCES;
    use super::*;
    use crate::model::{DemoOptions, demo_with};

    fn celsius(thermal: &Thermal, core_id: u32) -> f32 {
        thermal
            .sensors
            .iter()
            .find_map(|sensor| match sensor.place {
                SensorPlace::Core { core_id: id, .. } if id == core_id => sensor.celsius,
                _ => None,
            })
            .expect("the core has a sensor")
    }

    #[test]
    fn the_demo_topology_is_an_intel_hybrid_part_with_smt() {
        let topology = topology();
        assert_eq!(topology.cpus.len(), 16);
        assert_eq!(topology.packages(), [0]);
        let cores = topology.cores();
        assert_eq!(cores.len(), 12);
        assert_eq!(
            cores
                .iter()
                .filter_map(|core| core.core_id)
                .collect::<Vec<_>>(),
            [0, 4, 8, 12, 16, 17, 18, 19, 20, 21, 22, 23]
        );
        assert_eq!(cores[0].cpus, [0, 1]);
        assert_eq!(cores[2].cpus, [4, 5], "core 8 is CPUs 4 and 5");
        assert_eq!(cores[3].kind, CoreKind::Performance);
        assert_eq!(cores[4].cpus, [8]);
        assert_eq!(cores[4].kind, CoreKind::Efficiency);
        let clusters = topology.clusters();
        assert_eq!(
            clusters.iter().map(|c| c.cpus.clone()).collect::<Vec<_>>(),
            [
                vec![0, 1],
                vec![2, 3],
                vec![4, 5],
                vec![6, 7],
                vec![8, 9, 10, 11],
                vec![12, 13, 14, 15]
            ]
        );
        assert!(clusters.iter().all(|cluster| !cluster.low_power));
        assert_eq!(topology.l3_groups(), [(0..16).collect::<Vec<u32>>()]);
        assert_eq!(
            topology.place(3).and_then(|place| place.max_mhz),
            Some(4800.0)
        );
        assert_eq!(
            topology.place(12).and_then(|place| place.max_mhz),
            Some(3200.0)
        );
    }

    #[test]
    fn the_lag_is_the_steady_response_of_a_first_order_system() {
        for cpu in [0, 5, 9, 15] {
            // Without lag the harmonics give back the proxy of the demo's own CPU.
            for time in [-31.7, 0.0, 12.0, 55.5] {
                let direct = proxy(&demo_cpu(time, cpu), max_mhz(cpu));
                assert!(
                    (lagged(cpu, time, 0.0) - direct).abs() < 1e-4,
                    "cpu {cpu} at {time}"
                );
            }
            // With it, a numerical first-order system fed the same proxy, started long before,
            // lands on the closed form.
            let target = 40.0;
            let step = 0.01;
            let mut state = lagged(cpu, target - 90.0, LAG);
            let mut time = target - 90.0;
            while time < target {
                let input = proxy(&demo_cpu(time, cpu), max_mhz(cpu));
                state += (1.0 - (-step / LAG).exp()) * (input - state);
                time += step;
            }
            assert!(
                (state - lagged(cpu, target, LAG)).abs() < 0.01,
                "cpu {cpu}: {state} against {}",
                lagged(cpu, target, LAG)
            );
        }
    }

    #[test]
    fn package_sensor_reads_the_hottest_core_and_every_sensor_has_tjmax() {
        for step in 0..400 {
            let thermal = reading(f64::from(step) * 0.7 - 50.0);
            let hottest = thermal
                .topology
                .cores()
                .iter()
                .filter_map(|core| thermal.core_celsius(core))
                .fold(f32::MIN, f32::max);
            assert_eq!(thermal.package_celsius(0), Some(hottest), "step {step}");
            assert_eq!(thermal.sensors.len(), 13, "the package and twelve cores");
            assert!(
                thermal
                    .sensors
                    .iter()
                    .all(|sensor| sensor.critical == Some(100.0))
            );
            assert_eq!(thermal.tj_max(), Some(100.0));
            assert!(
                thermal
                    .sensors
                    .iter()
                    .filter_map(|sensor| sensor.celsius)
                    .all(|celsius| (30.0..=100.0).contains(&celsius)),
                "step {step}"
            );
        }
    }

    #[test]
    fn only_core_eight_reaches_tjmax_and_it_does_once_per_cycle() {
        let mut at_tjmax = 0;
        let mut others_below = true;
        for step in 0..8000 {
            let time = f64::from(step) * 0.05;
            let thermal = reading(time);
            if celsius(&thermal, HOT_CORE) >= 100.0 {
                at_tjmax += 1;
                assert!(
                    (THROTTLE_AT - 0.01..=THROTTLE_AT + 3.01).contains(&(time % CYCLE)),
                    "at {time}"
                );
            }
            others_below &= thermal
                .sensors
                .iter()
                .filter(|sensor| {
                    !matches!(
                        sensor.place,
                        SensorPlace::Core {
                            core_id: HOT_CORE,
                            ..
                        }
                    ) && sensor.place != SensorPlace::Package(0)
                })
                .all(|sensor| sensor.celsius.is_some_and(|celsius| celsius < 100.0));
        }
        assert!(others_below, "no other core is at TjMax");
        // 400 s is ten cycles; each holds TjMax for three seconds of 0.05 s steps.
        assert!((10 * 60..=10 * 62).contains(&at_tjmax), "{at_tjmax}");
    }

    #[test]
    fn the_hot_cores_throttle_count_rises_once_per_cycle_and_never_falls() {
        let count = |time: f64| {
            reading(time)
                .core_throttles
                .iter()
                .find(|throttle| throttle.core_id == HOT_CORE)
                .map(|throttle| throttle.count)
                .expect("core 8 has a counter")
        };
        assert_eq!(count(THROTTLE_AT - 0.001), count(THROTTLE_AT) - 1);
        assert_eq!(count(THROTTLE_AT), count(THROTTLE_AT + CYCLE - 0.001));
        assert_eq!(count(THROTTLE_AT + CYCLE), count(THROTTLE_AT) + 1);
        let mut last = count(-3000.0);
        let mut rises = 0;
        for step in 1..=8000 {
            let now = count(f64::from(step) * 0.5 - 3000.0);
            assert!(now >= last, "counts never decrease");
            rises += now - last;
            last = now;
        }
        assert_eq!(rises, 100, "4000 s is a hundred cycles");
        // Only the hot core ever throttles; the package does so every third cycle.
        let thermal = reading(500.0);
        assert!(
            thermal
                .core_throttles
                .iter()
                .filter(|throttle| throttle.core_id != HOT_CORE)
                .all(|throttle| throttle.count == 0)
        );
        let package = |time: f64| reading(time).package_throttles[0].count;
        assert_eq!(
            package(THROTTLE_AT + PACKAGE_CYCLE),
            package(THROTTLE_AT) + 1
        );
        assert_eq!(
            counted(-1.0e9, CYCLE, THROTTLES_BEFORE),
            0,
            "never negative"
        );
    }

    #[test]
    fn package_watts_follow_the_summed_proxy() {
        let thermal = reading(12.0);
        let expected = IDLE_WATTS
            + (0..CPUS)
                .map(|cpu| {
                    let weight = if cpu < 8 { P_WATTS } else { E_WATTS };
                    weight * proxy(&demo_cpu(12.0, cpu), max_mhz(cpu))
                })
                .sum::<f64>();
        let watts = thermal.package_power[0].watts.expect("a rate");
        assert!((f64::from(watts) - expected).abs() < 1e-4);
        assert_eq!(thermal.package_watts, Measured::Known(watts));
        assert_eq!(thermal.package_power[0].package, 0);
        for step in 0..300 {
            let watts = reading(f64::from(step) * 0.9).package_power[0]
                .watts
                .expect("a rate");
            assert!((IDLE_WATTS as f32..95.0).contains(&watts), "{watts}");
        }
    }

    #[test]
    fn the_demo_block_is_only_given_when_a_row_asks_for_it() {
        let asked = DemoOptions {
            thermal: true,
            ..DemoOptions::NONE
        };
        let with = demo_with(12.0, 16, asked);
        let thermal = with.thermal.as_ref().expect("the block");
        assert_eq!(thermal.pass, 12.0_f64.to_bits(), "numbered by its time");
        for source in SOURCES.iter() {
            assert!(with.sources.contains(source), "{}", source.name());
            assert!(!with.missing.contains(source));
        }
        assert!(demo_with(12.0, 16, DemoOptions::NONE).thermal.is_none());
        assert!(demo_with(12.0, 16, DemoOptions::NONE).sources.is_empty());
        let other = demo_with(13.0, 16, asked);
        assert_ne!(other.thermal.map(|t| t.pass), Some(thermal.pass));
    }
}
