//! The Die module view: the CPU die from above in false colour, an infrared floorplan. A heat
//! equation runs on a schematic floorplan of the cores, caches and clusters; what the sensors read
//! pulls the picture to the measurement, each process is a point over the core it last ran on,
//! and a core's outline flashes when its throttle count rises.
//!
//! What is measured and what is a model: the temperature at a sensor, the throttle counts, the
//! package power and each CPU's load and clock are measurements. The field between sensors is a
//! model (`heat`): a diffusion whose constants are the model's own, driven by the proxy
//! `busy * (clock / fmax)^3` and anchored to the sensors. With no temperature sensor the whole
//! field is that model, drawn in relative units under a bar that says so, with every core
//! hatched: it is never read as degrees. The floorplan (`floorplan`) is a function of the CPU
//! topology alone and is schematic.
//!
//! `readings` turns a sample into coverage, proxies, nudges, the 60 s series and throttle marks;
//! `heat` is the field; `paint` and `panel` draw it.

mod floorplan;
mod heat;
mod paint;
mod panel;
mod readings;

use floorplan::{COLUMNS, Floorplan, Role, ROWS};
use heat::{CELLS, Heat};
use paint::{Look, Pen, plane};
use panel::Row;
use readings::{Coverage, Mode, Readings, Regions, Watch};

use crate::model::thermal::Topology;
use crate::model::{Identity, Measured, Process, Snapshot};
use crate::palette::kind_color;
use crate::platform::Source;
use crate::render::{Card, Field, Point, Side, Stage, Tag, Thing, TourTarget};
use crate::simulation::{Rng, hash};
use crate::status::{missing_note, sources_note};
use crate::views::ViewModule;

/// The kind of thing a core block is, for picking and the inspector.
pub const CORE: &str = "die core";
/// Separates the die's per-process draws from every other use of the identity hash.
const SALT: u64 = 0x6469_6570_6f69;
/// The processes the Space tour visits, busiest first.
const TOUR: usize = 16;
/// A gap in the view's frames, in seconds, after which the field is settled again from the
/// current load instead of continuing from a picture that old.
const STALE: f32 = 3.0;

/// What is fixed by the topology: the floorplan and everything derived from it.
struct Layout {
    topology: Topology,
    plan: Floorplan,
    /// How each block's source is spread over its cells.
    shapes: Vec<Vec<(u32, f32)>>,
    /// The block under each cell of the grid, `u16::MAX` for none.
    cell_block: Vec<u16>,
    /// Kelvin a second per unit of proxy, calibrated so that one P-core at full proxy settles 40
    /// K above ambient.
    gain: f32,
}

impl Layout {
    fn new(topology: &Topology) -> Layout {
        let plan = Floorplan::of(topology);
        let shapes: Vec<Vec<(u32, f32)>> = plan
            .blocks
            .iter()
            .map(|block| heat::shape(block.rect, block.hot))
            .collect();
        let mut cell_block = vec![u16::MAX; CELLS];
        for (index, shape) in shapes.iter().enumerate() {
            for &(cell, _) in shape {
                cell_block[cell as usize] = u16::try_from(index).unwrap_or(u16::MAX - 1);
            }
        }
        let reference = plan
            .blocks
            .iter()
            .find(|block| matches!(block.role, Role::Performance | Role::Plain))
            .or(plan.blocks.first());
        Layout {
            topology: topology.clone(),
            gain: reference.map_or(1.0, |block| heat::gain(block.rect, block.hot)),
            plan,
            shapes,
            cell_block,
        }
    }
}

/// What a frame drew, kept for the legend and the cards.
#[derive(Default)]
struct Last {
    readings: Option<Readings>,
    /// The value shown at each block's hot spot, in the scale's own units.
    hot: Vec<f32>,
    /// The head of the legend: what is counted this sample.
    head: String,
}

pub struct Die {
    layout: Option<Layout>,
    heat: Heat,
    clock: heat::Clock,
    /// The model with every nudge's offset added: what is drawn.
    shown: Vec<f32>,
    field: Field,
    regions: Regions,
    watch: Watch,
    running_max: Vec<f32>,
    mode: Mode,
    /// The animation time of the last draw, to tell a stale view.
    at: Option<f32>,
    /// Whether the field has been settled from the current load.
    settled: bool,
    last: Last,
}

impl Default for Die {
    fn default() -> Self {
        Die {
            layout: None,
            heat: Heat::new(0.0),
            clock: heat::Clock::default(),
            shown: Vec::new(),
            field: Field::new(
                hash::fnv1a_64_bytes(*b"die heat field"),
                COLUMNS,
                ROWS,
            ),
            regions: Regions::default(),
            watch: Watch::default(),
            running_max: Vec::new(),
            mode: Mode::default(),
            at: None,
            settled: false,
            last: Last::default(),
        }
    }
}

/// The key of a block's thing: its first CPU, which the topology fixes.
fn block_key(cpus: &[u32]) -> u64 {
    u64::from(cpus[0])
}

/// Where process `process` is drawn: the block it last ran on, else (macOS reports no CPU) a
/// block of its P or E cluster by its performance share, and the fractions of the block's width
/// and height its offset is, from a generator seeded by the process's identity so that it stays
/// put while the process stays on that core. None for a process that cannot be placed.
fn placement(plan: &Floorplan, process: &Process) -> Option<(usize, [f32; 2])> {
    let mut rng = Rng::for_identity(process.id, SALT);
    let offset = [0.12 + 0.76 * rng.unit(), 0.12 + 0.76 * rng.unit()];
    let pick = rng.unit();
    let block = match process.core {
        Some(cpu) => plan.block_of_cpu(cpu)?,
        None => {
            let role = if process.performance_share? >= 0.5 {
                Role::Performance
            } else {
                Role::Efficiency
            };
            let of_role: Vec<usize> = plan
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, block)| block.role == role)
                .map(|(index, _)| index)
                .collect();
            let pool: Vec<usize> = if of_role.is_empty() {
                (0..plan.blocks.len()).collect()
            } else {
                of_role
            };
            *pool.get((pick * pool.len() as f32) as usize)?
        }
    };
    Some((block, offset))
}

impl Die {
    /// Takes in the load of the measured power: the heat source follows the package's watts, when
    /// they are read.
    fn follow_power(&mut self, layout: &Layout, found: &Readings, elapsed: f64) {
        if let Some(&watts) = found.watts.known() {
            let model = readings::model_watts(&layout.plan, &found.proxies);
            self.watch.scale.take(watts, model, elapsed);
        }
    }

    /// Steps the field to this frame: sets the source from the load, the ambient and the nudges
    /// from the sensors, settles a cold start, and runs the whole steps the clock owes.
    fn settle(&mut self, layout: &Layout, found: &Readings, stage: &Stage, snapshot: &Snapshot) {
        if layout.plan.blocks.is_empty() {
            return;
        }
        let ambient = match found.mode {
            Mode::Relative => 0.0,
            Mode::Celsius => self
                .watch
                .series
                .ambient()
                .or(found.coolest.map(|coolest| coolest - readings::AMBIENT_MARGIN))
                .unwrap_or(readings::SCALE_LOW - readings::AMBIENT_MARGIN),
        };
        if found.mode != self.mode || self.at.is_some_and(|at| (stage.time - at).abs() > STALE) {
            self.settled = false;
        }
        self.mode = found.mode;
        self.heat.ambient = ambient;
        self.heat.source.fill(0.0);
        let scale = self.watch.scale.value();
        for ((block, shape), proxy) in layout
            .plan
            .blocks
            .iter()
            .zip(&layout.shapes)
            .zip(&found.proxies)
        {
            if let Some(proxy) = proxy {
                let amplitude = layout.gain * readings::weight(block) * scale * proxy;
                for &(cell, weight) in shape {
                    self.heat.source[cell as usize] += amplitude * weight;
                }
            }
        }
        if self.regions.update(&layout.plan, snapshot, found) {
            self.settled = false;
        }
        let (scheme, alpha) = (self.heat.scheme(), heat::alpha());
        if !self.settled {
            self.heat.field.fill(ambient);
            for _ in 0..(heat::PRE_ROLL / heat::STEP) as usize {
                self.heat.diffuse(scheme);
                for (nudge, offset) in self.regions.nudges.iter().zip(&mut self.regions.offsets) {
                    nudge.relax(&self.heat.field, offset, alpha);
                }
            }
            self.settled = true;
        }
        // A frame with no step (the first, a pause, a rewind) leaves the field where it was.
        if stage.dt > 0.0 {
            let (heat, regions) = (&mut self.heat, &mut self.regions);
            self.clock.advance(stage.time, || {
                heat.diffuse(scheme);
                for (nudge, offset) in regions.nudges.iter().zip(&mut regions.offsets) {
                    nudge.relax(&heat.field, offset, alpha);
                }
            });
        } else {
            self.clock.advance(stage.time, || {});
        }
        heat::compose(
            &self.heat.field,
            &self.regions.nudges,
            &self.regions.offsets,
            ambient,
            &mut self.shown,
        );
    }

    /// The value on the colour scale at each block's hot spot, as shown.
    fn hot_values(&self, layout: &Layout) -> Vec<f32> {
        layout
            .plan
            .blocks
            .iter()
            .map(|block| self.shown[heat::cell_at(block.hot[0], block.hot[1]) as usize])
            .collect()
    }

    /// The rows of the panel: the package, the power, and one per group of cores.
    fn rows(&self, layout: &Layout, found: &Readings, hot: &[f32], time: f32) -> Vec<Row> {
        let mut rows = Vec::new();
        rows.push(match (found.mode, found.package_celsius) {
            (Mode::Celsius, Some(celsius)) => Row {
                level: Some(found.level(celsius)),
                text: format!("package {celsius:.0} C"),
            },
            (Mode::Celsius, None) => Row {
                level: None,
                text: "package n/a".to_owned(),
            },
            (Mode::Relative, _) => Row {
                level: None,
                text: "package n/a (no sensor)".to_owned(),
            },
        });
        rows.push(Row {
            level: None,
            text: match &found.watts {
                Measured::Known(watts) => format!("RAPL {watts:.1} W"),
                Measured::Unreadable => "RAPL n/a (root only)".to_owned(),
                Measured::Pending => "RAPL n/a (not read)".to_owned(),
            },
        });
        // One unit per box (a cluster, a core complex) and one per core that stands alone.
        let mut units: Vec<(String, Vec<usize>)> = Vec::new();
        for (index, block) in layout.plan.blocks.iter().enumerate() {
            if block.group.is_none() {
                units.push((block.name.clone(), vec![index]));
            }
        }
        for group in &layout.plan.groups {
            units.push((group.name.clone(), group.blocks.clone()));
        }
        units.sort_by_key(|(_, blocks)| blocks[0]);
        let room = panel::MOST_ROWS.saturating_sub(rows.len());
        let (shown, more) = if units.len() > room {
            (room.saturating_sub(1), units.len() - room + 1)
        } else {
            (units.len(), 0)
        };
        for (name, blocks) in units.iter().take(shown) {
            let value = blocks.iter().map(|&block| hot[block]).fold(f32::MIN, f32::max);
            let measured = blocks
                .iter()
                .all(|&block| found.coverage[block] != Coverage::Unknown);
            let hurt = blocks.iter().any(|&block| self.flashing(layout, block, time).is_some());
            let value_text = match (found.mode, measured) {
                (Mode::Celsius, true) => format!("{value:.0} C"),
                (Mode::Celsius, false) => format!("~{value:.0} C model"),
                (Mode::Relative, _) => {
                    format!("{:.0}% model", (found.level(value) * 100.0).clamp(0.0, 100.0))
                }
            };
            rows.push(Row {
                level: Some(found.level(value)),
                text: format!("{name} {value_text}{}", if hurt { " THROTTLE" } else { "" }),
            });
        }
        if more > 0 {
            rows.push(Row {
                level: None,
                text: format!("+{more} more"),
            });
        }
        rows
    }

    /// The age of the flash on block `block` at `time` if it is flashing.
    fn flashing(&self, layout: &Layout, block: usize, time: f32) -> Option<f32> {
        let block = &layout.plan.blocks[block];
        let marked = self.watch.cores.get(&(block.package?, block.core_id?))?;
        let age = time - marked.since;
        (0.0..readings::FLASH).contains(&age).then_some(age)
    }

    /// The legend's head: what this sample counts, ahead of the notes for what is missing.
    fn head(&self, layout: &Layout, found: &Readings) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(match (found.mode, found.package_celsius) {
            (Mode::Celsius, Some(celsius)) => format!("package {celsius:.0} C"),
            (Mode::Celsius, None) => "sensors read, package n/a".to_owned(),
            (Mode::Relative, _) => "relative heat, no degrees".to_owned(),
        });
        match &found.watts {
            Measured::Known(watts) => parts.push(format!("RAPL {watts:.1} W")),
            Measured::Unreadable => parts.push("RAPL n/a (root only)".to_owned()),
            Measured::Pending => {}
        }
        let mut throttled: Vec<(usize, u64)> = layout
            .plan
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| {
                let marked = self.watch.cores.get(&(block.package?, block.core_id?))?;
                Some((index, marked.seen))
            })
            .collect();
        throttled.sort_unstable();
        if !throttled.is_empty() {
            let names: Vec<String> = throttled
                .iter()
                .map(|&(index, seen)| format!("{} x{seen}", layout.plan.blocks[index].name))
                .collect();
            parts.push(format!("throttles: {}", names.join(", ")));
        } else if let Some(package) = layout.plan.package
            && let Some(marked) = self.watch.packages.get(&package)
        {
            parts.push(format!("package throttled x{}", marked.seen));
        }
        format!(" {}", parts.join(" | "))
    }

    /// The label of block `block`'s own sensor and its raw text, for the card: the driver's name
    /// of it, untrusted.
    fn sensor_label(snapshot: &Snapshot, block: &floorplan::Block) -> Option<String> {
        snapshot.thermal.as_ref()?.sensors.iter().find_map(|sensor| match sensor.place {
            crate::model::thermal::SensorPlace::Core { package, core_id }
                if Some(core_id) == block.core_id
                    && block.package.is_none_or(|found| found == package) =>
            {
                Some(sensor.label.clone())
            }
            _ => None,
        })
    }
}

impl ViewModule for Die {
    fn record(&mut self, snapshot: &Snapshot, time: f32) {
        // The first package the topology names is the one the die draws.
        let package = snapshot
            .thermal
            .as_ref()
            .and_then(|thermal| thermal.topology.packages().first().copied());
        self.watch.ingest(snapshot, time, package);
    }

    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let topology = readings::topology_of(snapshot);
        if self
            .layout
            .as_ref()
            .is_none_or(|layout| layout.topology != *topology)
        {
            let layout = Layout::new(&topology);
            self.running_max = vec![0.0; layout.plan.blocks.len()];
            self.regions = Regions::default();
            self.settled = false;
            self.layout = Some(layout);
        }
        let layout = self.layout.take().expect("the layout was just built");
        let found = Readings::of(&layout.plan, snapshot, &mut self.running_max);
        self.watch.ingest(snapshot, stage.time, layout.plan.package);
        self.follow_power(&layout, &found, snapshot.elapsed);
        self.settle(&layout, &found, stage, snapshot);
        let drawn = self.paint(&layout, &found, stage, processes);
        self.at = Some(stage.time);
        self.last.head = self.head(&layout, &found);
        self.last.hot = self.hot_values(&layout);
        self.last.readings = Some(found);
        self.layout = Some(layout);
        drawn
    }

    fn legend(&self, snapshot: &Snapshot) -> String {
        let mut notes = String::new();
        for note in [
            missing_note(snapshot, Source::Topology, "floorplan from the CPU list"),
            missing_note(snapshot, Source::Temperature, "field is model only"),
            missing_note(snapshot, Source::ThrottleCounts, "no throttle flashes"),
            missing_note(
                snapshot,
                Source::PackagePower,
                "RAPL n/a (root only), heat source unscaled",
            ),
            missing_note(snapshot, Source::CpuClock, "heat source is busy alone"),
            missing_note(snapshot, Source::LastCpu, "points sit on their P or E cluster"),
        ]
        .into_iter()
        .flatten()
        {
            notes.push_str(&note);
        }
        let readings = self.last.readings.as_ref();
        if readings.is_some_and(|found| found.unmapped_ccds > 0) {
            notes.push_str(" | CCD sensors not placed on cores: cache groups do not say");
        }
        if let Some(layout) = &self.layout
            && layout.plan.packages > 1
        {
            notes.push_str(&format!(" | package {} of {} drawn", layout.plan.package.unwrap_or(0), layout.plan.packages));
        }
        if let Some(sources) = sources_note(snapshot) {
            notes.push_str(&sources);
        }
        let head = if self.last.head.is_empty() {
            " reading the CPU topology"
        } else {
            &self.last.head
        };
        let colour = match readings.map(|found| found.mode) {
            Some(Mode::Relative) | None => "ironbow = relative heat, model only (not degrees)",
            Some(Mode::Celsius) => "ironbow = heat: sensor where one reads, model between",
        };
        format!(
            "{head}{notes} | {colour} | source = busy x (clock / fmax)^3 | points = processes at their last CPU, size = CPU | cyan outline = throttle | hatched = no sensor"
        )
    }

    fn card(&self, thing: Thing, snapshot: &Snapshot) -> Option<Card> {
        if thing.kind != CORE {
            return None;
        }
        let layout = self.layout.as_ref()?;
        let found = self.last.readings.as_ref()?;
        let index = layout
            .plan
            .blocks
            .iter()
            .position(|block| block_key(&block.cpus) == thing.key)?;
        let block = &layout.plan.blocks[index];
        let cpus: Vec<String> = block.cpus.iter().map(|cpu| format!("cpu{cpu}")).collect();
        let mut lines = vec![format!(
            "{} core{}, {}",
            crate::palette::core_name(block.kind),
            if block.cpus.len() > 1 { " with SMT" } else { "" },
            cpus.join(" ")
        )];
        let hot = self.last.hot.get(index).copied().unwrap_or(0.0);
        lines.push(match found.coverage[index] {
            Coverage::Own(celsius) => match Self::sensor_label(snapshot, block) {
                Some(label) => format!("{celsius:.0} C, measured by its own sensor ({label})"),
                None => format!("{celsius:.0} C, measured by its own sensor"),
            },
            Coverage::Ccd(number, celsius) => format!(
                "{celsius:.0} C, one sensor for all of CCD {number}; the split between its cores is the model's"
            ),
            Coverage::Package(celsius) => format!(
                "{celsius:.0} C, the package sensor: one reading for the whole die, the split is the model's"
            ),
            Coverage::Unknown => match found.mode {
                Mode::Celsius => format!("no sensor reads this core: ~{hot:.0} C is the model's"),
                Mode::Relative => format!(
                    "no sensor: {:.0}% of the scale is the model's relative heat, not degrees",
                    (found.level(hot) * 100.0).clamp(0.0, 100.0)
                ),
            },
        });
        let cpu_of = |id: u32| snapshot.cpus.iter().find(|cpu| cpu.id == id);
        let busy: Vec<String> = block
            .cpus
            .iter()
            .filter_map(|&id| cpu_of(id))
            .map(|cpu| {
                cpu.busy
                    .map_or_else(|| "busy n/a".to_owned(), |busy| format!("busy {:.0}%", busy * 100.0))
            })
            .collect();
        let clock = block
            .cpus
            .iter()
            .filter_map(|&id| cpu_of(id).and_then(|cpu| cpu.mhz))
            .reduce(f32::max);
        lines.push(format!(
            "{}  clock {}",
            if busy.is_empty() { "load not read".to_owned() } else { busy.join(" ") },
            clock.map_or_else(|| "n/a".to_owned(), |mhz| format!("{mhz:.0} MHz"))
        ));
        lines.push(match found.proxies[index] {
            Some(proxy) => format!("heat source: busy x (clock / fmax)^3 = {proxy:.2}"),
            None => "no heat source: no load reading".to_owned(),
        });
        let count = snapshot.thermal.as_ref().and_then(|thermal| {
            thermal.core_throttles.iter().find(|throttle| {
                Some(throttle.core_id) == block.core_id
                    && block.package.is_none_or(|found| found == throttle.package)
            })
        });
        lines.push(match count {
            Some(throttle) => {
                let seen = self
                    .watch
                    .cores
                    .get(&(throttle.package, throttle.core_id))
                    .map_or(0, |marked| marked.seen);
                format!(
                    "throttled {} times since boot, {seen} seen while watching",
                    throttle.count
                )
            }
            None => "throttle count n/a".to_owned(),
        });
        let [x, y] = [block.rect[2], block.rect[1]];
        Some(Card {
            title: format!("{} core", block.name),
            lines,
            at: plane(x, y, 0.0),
        })
    }

    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

impl Die {
    /// Draws the package, the field, the outlines, the panel, the labels and the process points.
    fn paint(
        &mut self,
        layout: &Layout,
        found: &Readings,
        stage: &mut Stage,
        processes: &[&Process],
    ) -> usize {
        let time = stage.time;
        let plan = &layout.plan;
        let looks: Vec<Look> = found
            .coverage
            .iter()
            .zip(&found.proxies)
            .map(|(&coverage, &proxy)| Look::of(coverage, proxy))
            .collect();
        let hot = self.hot_values(layout);
        let rows = self.rows(layout, found, &hot, time);
        let mut pen = Pen {
            frame: &mut *stage.frame,
            camera: stage.camera,
        };
        paint::package(&mut pen);
        let picks: Vec<u32> = plan
            .blocks
            .iter()
            .map(|block| {
                pen.frame.thing_pick(Thing {
                    kind: CORE,
                    key: block_key(&block.cpus),
                })
            })
            .collect();
        paint::colour_field(
            &mut self.field,
            &self.shown,
            |value| found.level(value),
            &layout.cell_block,
            &looks,
            &picks,
        );
        paint::field(&mut pen, &self.field);
        let chosen = stage.thing.filter(|thing| thing.kind == CORE).and_then(|thing| {
            plan.blocks
                .iter()
                .position(|block| block_key(&block.cpus) == thing.key)
        });
        paint::outlines(&mut pen, plan, &found.coverage, chosen);
        for (index, block) in plan.blocks.iter().enumerate() {
            if let Some(age) = self.flashing(layout, index, time) {
                paint::flash(&mut pen, block.rect, age);
            }
        }
        if let Some(package) = plan.package
            && let Some(marked) = self.watch.packages.get(&package)
            && (0.0..readings::FLASH).contains(&(time - marked.since))
        {
            paint::flash(&mut pen, [0.0, 0.0, COLUMNS as f32, ROWS as f32], time - marked.since);
        }
        panel::draw(
            &mut pen,
            stage.tags,
            &panel::Input {
                mode: found.mode,
                scale: found.scale(),
                tj_max: found.tj_max,
                rows: &rows,
                series: &self.watch.series,
                package_title: match found.mode {
                    Mode::Celsius => "package C, last 60 s".to_owned(),
                    Mode::Relative => "package C: no sensor".to_owned(),
                },
                power_title: match found.watts {
                    Measured::Known(_) => "RAPL W, last 60 s".to_owned(),
                    _ => "RAPL W: n/a".to_owned(),
                },
            },
        );
        // The names of the blocks, the boxes and the schematic parts: tags, which the app cleans
        // and places where there is room.
        for block in &plan.blocks {
            if block.group.is_none() {
                stage.tags.push(Tag {
                    at: plane(block.rect[0] + 0.5, block.rect[1] + 2.0, 0.0),
                    side: Side::Right,
                    text: block.name.clone(),
                });
            }
        }
        for group in &plan.groups {
            stage.tags.push(Tag {
                at: plane(group.rect[0] + 0.5, group.rect[1] + 2.0, 0.0),
                side: Side::Right,
                text: group.name.clone(),
            });
        }
        for part in plan.context.iter().filter(|part| !part.label.is_empty()) {
            stage.tags.push(Tag {
                at: plane(part.rect[0] + 0.5, part.rect[1] + 2.0, 0.0),
                side: Side::Right,
                text: part.label.to_owned(),
            });
        }
        stage.places.push((plane(COLUMNS as f32 / 2.0, ROWS as f32 + 8.0, 0.0), plan.caption.clone()));
        stage.bounds.extend([
            plane(-8.0, -8.0, 0.0),
            plane(270.0, -8.0, 0.0),
            plane(270.0, ROWS as f32 + 12.0, 0.0),
            plane(-8.0, ROWS as f32 + 12.0, 0.0),
        ]);
        self.points(layout, stage, processes)
    }

    /// The process points: returns how many were drawn.
    fn points(&self, layout: &Layout, stage: &mut Stage, processes: &[&Process]) -> usize {
        let plan = &layout.plan;
        let mut pen = Pen {
            frame: &mut *stage.frame,
            camera: stage.camera,
        };
        let mut drawn = 0;
        let mut busiest: Vec<(f32, Identity, Point)> = Vec::new();
        for (index, process) in processes.iter().enumerate() {
            let Some((block, offset)) = placement(plan, process) else {
                continue;
            };
            let rect = plan.blocks[block].rect;
            let at = [
                rect[0] + (rect[2] - rect[0]) * offset[0],
                rect[1] + (rect[3] - rect[1]) * offset[1],
            ];
            let radius = 0.1 + 0.2 * (process.cpu / 100.0).clamp(0.0, 1.0).sqrt();
            let selected = stage.highlight.contains(&process.id);
            paint::point(
                &mut pen,
                at,
                radius,
                paint::point_color(kind_color(process), process.cpu),
                index as u32,
                selected,
            );
            let position = paint::point_position(at);
            stage.positions.insert(process.id, position);
            busiest.push((process.cpu, process.id, position));
            if selected {
                let name = &plan.blocks[block].name;
                stage.popup.push((
                    process.id,
                    match process.core {
                        Some(cpu) => format!("last ran on cpu{cpu}, drawn on {name}"),
                        None => format!(
                            "no last CPU here: drawn on a core of its {} cluster by performance share",
                            if plan.blocks[block].role == Role::Performance { "P" } else { "E" }
                        ),
                    },
                ));
            }
            drawn += 1;
        }
        busiest.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        stage.tour.extend(busiest.into_iter().take(TOUR).map(|(cpu, id, position)| TourTarget {
            id,
            position,
            weight: cpu,
        }));
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_die_names_its_thing_kind() {
        assert_eq!(CORE, "die core");
    }

    #[test]
    #[ignore]
    fn dump() {
        use crate::model::{DemoOptions, demo_with};
        use crate::render::{Camera, RenderInput, Scene};
        use crate::views::view;
        let mut scene = Scene::new();
        let snapshot = demo_with(12.0, 192, DemoOptions { thermal: true, ..DemoOptions::NONE });
        let camera = Camera::default();
        for step in 0..5 {
            let input = RenderInput::small(&snapshot, view("die"), &camera, 12.0 + step as f32 * 0.1);
            scene.render(&input);
        }
        let die = scene.module::<Die>(view("die")).expect("die");
        let layout = die.layout.as_ref().expect("layout");
        for (i, block) in layout.plan.blocks.iter().enumerate() {
            let cell = heat::cell_at(block.hot[0], block.hot[1]) as usize;
            eprintln!("{} cov {:?} proxy {:?} model {:.1} shown {:.1}", block.name, die.last.readings.as_ref().unwrap().coverage[i], die.last.readings.as_ref().unwrap().proxies[i], die.heat.field[cell], die.shown[cell]);
        }
        eprintln!("offsets {:?} ambient {}", die.regions.offsets, die.heat.ambient);
    }
}
