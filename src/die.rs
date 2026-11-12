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

use floorplan::{COLUMNS, Floorplan, ROWS, Role};
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
            field: Field::new(hash::fnv1a_64_bytes(*b"die heat field"), COLUMNS, ROWS),
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
                .or(found
                    .coolest
                    .map(|coolest| coolest - readings::AMBIENT_MARGIN))
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
            let value = blocks
                .iter()
                .map(|&block| hot[block])
                .fold(f32::MIN, f32::max);
            let measured = blocks
                .iter()
                .all(|&block| found.coverage[block] != Coverage::Unknown);
            let hurt = blocks
                .iter()
                .any(|&block| self.flashing(layout, block, time).is_some());
            let value_text = match (found.mode, measured) {
                (Mode::Celsius, true) => format!("{value:.0} C"),
                (Mode::Celsius, false) => format!("~{value:.0} C model"),
                (Mode::Relative, _) => {
                    format!(
                        "{:.0}% model",
                        (found.level(value) * 100.0).clamp(0.0, 100.0)
                    )
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
        snapshot
            .thermal
            .as_ref()?
            .sensors
            .iter()
            .find_map(|sensor| match sensor.place {
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
            missing_note(
                snapshot,
                Source::LastCpu,
                "points sit on their P or E cluster",
            ),
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
            notes.push_str(&format!(
                " | package {} of {} drawn",
                layout.plan.package.unwrap_or(0),
                layout.plan.packages
            ));
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
            Some(Mode::Relative) | None => {
                "ironbow = relative heat modelled from load (not degrees)"
            }
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
            if block.cpus.len() > 1 {
                " with SMT"
            } else {
                ""
            },
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
                cpu.busy.map_or_else(
                    || "busy n/a".to_owned(),
                    |busy| format!("busy {:.0}%", busy * 100.0),
                )
            })
            .collect();
        let clock = block
            .cpus
            .iter()
            .filter_map(|&id| cpu_of(id).and_then(|cpu| cpu.mhz))
            .reduce(f32::max);
        lines.push(format!(
            "{}  clock {}",
            if busy.is_empty() {
                "load not read".to_owned()
            } else {
                busy.join(" ")
            },
            clock.map_or_else(|| "n/a".to_owned(), |mhz| format!("{mhz:.0} MHz"))
        ));
        lines.push(match found.proxies[index] {
            Some(proxy) => format!("heat source: busy x (clock / fmax)^3 = {proxy:.2}"),
            None => "no heat source: no load reading".to_owned(),
        });
        let core = block.core();
        let count = snapshot.thermal.as_ref().and_then(|thermal| {
            thermal
                .core_throttle(&core)
                .zip(core.package.zip(core.core_id))
        });
        lines.push(match count {
            Some((count, key)) => {
                let seen = self.watch.cores.get(&key).map_or(0, |marked| marked.seen);
                format!("throttled {count} times since boot, {seen} seen while watching")
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
        let chosen = stage
            .thing
            .filter(|thing| thing.kind == CORE)
            .and_then(|thing| {
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
            paint::flash(
                &mut pen,
                [0.0, 0.0, COLUMNS as f32, ROWS as f32],
                time - marked.since,
            );
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
        stage.places.push((
            plane(COLUMNS as f32 / 2.0, ROWS as f32 + 8.0, 0.0),
            plan.caption.clone(),
        ));
        stage.bounds.extend([
            plane(-8.0, -8.0, 0.0),
            plane(260.0, -8.0, 0.0),
            plane(260.0, ROWS as f32 + 12.0, 0.0),
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
        stage.tour.extend(
            busiest
                .into_iter()
                .take(TOUR)
                .map(|(cpu, id, position)| TourTarget {
                    id,
                    position,
                    weight: cpu,
                }),
        );
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{churned, rescaled};
    use crate::model::thermal::{CoreThrottle, SensorPlace};
    use crate::model::{CoreKind, DemoOptions, demo_with};
    use crate::platform::Sources;
    use crate::render::{Camera, Color, Frame, Item, NONE, RenderInput, Scene, THING, Target};
    use crate::views::view;

    /// The colour a throttle flash is drawn in.
    const FLASH: Color = [110, 240, 255];

    fn demo(time: f64) -> Snapshot {
        demo_with(
            time,
            192,
            DemoOptions {
                thermal: true,
                ..DemoOptions::NONE
            },
        )
    }

    /// A 640 by 360 input for the die at `time`.
    fn input<'a>(snapshot: &'a Snapshot, camera: &'a Camera, time: f32) -> RenderInput<'a> {
        let mut input = RenderInput::small(snapshot, view("die"), camera, time);
        (input.width, input.height) = (640, 360);
        input
    }

    /// The die's frame at `time`, fitted and rasterized.
    fn draw(scene: &mut Scene, snapshot: &Snapshot, time: f32) -> Frame {
        let mut camera = Camera {
            pitch: 1.05,
            ..Camera::default()
        };
        scene.render(&input(snapshot, &camera, time));
        scene.fit(&mut camera, 640, 360);
        let mut frame = scene.render(&input(snapshot, &camera, time));
        frame.rasterize();
        frame
    }

    /// A scene that has watched the demo from 9 s to 12 s, a sample a second.
    fn watched() -> (Scene, Snapshot, Frame) {
        let mut scene = Scene::new();
        let mut last = None;
        for second in 9..=12 {
            let snapshot = demo(f64::from(second));
            let frame = draw(&mut scene, &snapshot, second as f32);
            last = Some((snapshot, frame));
        }
        let (snapshot, frame) = last.expect("drew four samples");
        (scene, snapshot, frame)
    }

    fn die(scene: &Scene) -> &Die {
        scene.module::<Die>(view("die")).expect("the die has drawn")
    }

    #[test]
    fn die_renders_the_same_frame_twice_from_the_same_inputs() {
        let run = || {
            let mut scene = Scene::new();
            let mut frame = None;
            for second in 9..=12 {
                frame = Some(draw(&mut scene, &demo(f64::from(second)), second as f32));
            }
            frame.expect("drew four samples")
        };
        let (first, second) = (run(), run());
        assert_eq!(first.pixels, second.pixels);
        assert_eq!(first.picks, second.picks);
        assert!(
            first.pixels.iter().any(|&byte| byte > 40),
            "and it drew something"
        );
    }

    #[test]
    fn the_field_is_the_same_at_ten_and_sixty_frames_per_second() {
        // The demo's sample held for 2.95 s, drawn through the scene at each rate; the end is
        // half a step past a whole number of them, so rounding the frame times cannot tip one.
        let run = |frame: f32| {
            let mut scene = Scene::new();
            let snapshot = demo(12.0);
            let camera = Camera::default();
            let render = |scene: &mut Scene, time: f32| {
                scene.render(&RenderInput::small(&snapshot, view("die"), &camera, time));
            };
            let mut index = 0;
            while 12.0 + frame * ((index + 1) as f32) < 14.95 {
                render(&mut scene, 12.0 + frame * index as f32);
                index += 1;
            }
            render(&mut scene, 14.95);
            let die = die(&scene);
            (die.heat.field.clone(), die.shown.clone())
        };
        let (ten, sixty) = (run(0.1), run(1.0 / 60.0));
        assert_eq!(ten.0, sixty.0, "the model is the same, bit for bit");
        assert_eq!(ten.1, sixty.1, "and so is what is shown");
        let warm = ten.0.iter().copied().fold(f32::MIN, f32::max);
        assert!(warm > 50.0, "and the die is warm: {warm}");
    }

    #[test]
    fn a_throttle_count_increment_flashes_the_core_outline() {
        let mut scene = Scene::new();
        let camera = Camera::default();
        let mut seen = Vec::new();
        for tenth in 200..=270 {
            let time = tenth as f32 / 10.0;
            let snapshot = demo(f64::from(time.floor()));
            let frame = scene.render(&RenderInput::small(&snapshot, view("die"), &camera, time));
            let flashing = frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Triangle(_, color, _) if *color == FLASH))
                .count();
            let die = die(&scene);
            let layout = die.layout.as_ref().expect("layout");
            let hurt: Vec<&str> = (0..layout.plan.blocks.len())
                .filter(|&block| die.flashing(layout, block, time).is_some())
                .map(|block| layout.plan.blocks[block].name.as_str())
                .collect();
            seen.push((time, flashing, hurt.join(",")));
        }
        for (time, flashing, hurt) in &seen {
            // The hot core's count rises at 24 s, and its outline flashes for 1.4 s from the
            // first frame that shows the new count.
            if (24.0..=25.2).contains(time) {
                assert!(*flashing > 0, "{time}: no flash drawn");
                assert_eq!(hurt, "P2", "{time}: the demo's hot core, P2, is the one");
            } else if *time < 24.0 || *time >= 25.6 {
                assert_eq!(*flashing, 0, "{time}: nothing flashes");
                assert_eq!(hurt, "", "{time}");
            }
        }
        // The legend counts what was seen, and the strip says so.
        let legend = scene.legend(view("die"), &demo(27.0));
        assert!(legend.contains("throttles: P2 x1"), "{legend}");
    }

    #[test]
    fn a_count_that_is_missing_or_falls_flashes_nothing() {
        let mut scene = Scene::new();
        let camera = Camera::default();
        let hurt = |scene: &mut Scene, snapshot: &Snapshot, time: f32| {
            scene.render(&RenderInput::small(snapshot, view("die"), &camera, time));
            let die = die(scene);
            let layout = die.layout.as_ref().expect("layout");
            (0..layout.plan.blocks.len())
                .filter(|&block| die.flashing(layout, block, time).is_some())
                .count()
        };
        let with = |count: Option<u64>, second: f64| {
            let mut snapshot = demo(second);
            let thermal = snapshot.thermal.as_mut().expect("thermal");
            thermal.core_throttles = count
                .map(|count| {
                    vec![CoreThrottle {
                        package: 0,
                        core_id: 8,
                        count,
                    }]
                })
                .unwrap_or_default();
            thermal.pass = second as u64 + 1;
            snapshot
        };
        assert_eq!(hurt(&mut scene, &with(Some(50), 1.0), 1.0), 0);
        assert_eq!(
            hurt(&mut scene, &with(Some(10), 2.0), 2.0),
            0,
            "a count that falls"
        );
        assert_eq!(
            hurt(&mut scene, &with(None, 3.0), 3.0),
            0,
            "a counter that is not there"
        );
        assert_eq!(
            hurt(&mut scene, &with(Some(10), 4.0), 4.0),
            0,
            "and back, from nothing"
        );
        assert_eq!(
            hurt(&mut scene, &with(Some(11), 5.0), 5.0),
            1,
            "a rise is a throttle"
        );
    }

    /// The demo with its sensors gone, as a Linux machine without hwmon reports it.
    fn without_sensors(time: f64) -> Snapshot {
        let mut snapshot = demo(time);
        let thermal = snapshot.thermal.as_mut().expect("thermal");
        thermal.sensors.clear();
        thermal.core_throttles.clear();
        thermal.package_throttles.clear();
        thermal.package_power.clear();
        thermal.package_watts = Measured::Unreadable;
        snapshot.sources = Sources::from(Source::Topology);
        snapshot.missing = Sources::of(&[
            Source::Temperature,
            Source::ThrottleCounts,
            Source::PackagePower,
        ]);
        snapshot
    }

    #[test]
    fn missing_sensors_show_relative_heat_not_degrees() {
        let mut scene = Scene::new();
        let mut last = None;
        for second in 9..=12 {
            let snapshot = without_sensors(f64::from(second));
            let frame = draw(&mut scene, &snapshot, second as f32);
            last = Some((snapshot, frame));
        }
        let (snapshot, frame) = last.expect("drew four samples");
        let die = die(&scene);
        let found = die.last.readings.as_ref().expect("readings");
        assert_eq!(found.mode, Mode::Relative);
        assert!(
            found
                .coverage
                .iter()
                .all(|coverage| *coverage == Coverage::Unknown)
        );
        // Every core is hatched, and the model has heat in it: a field, not a blank die.
        let looks: Vec<Look> = found
            .coverage
            .iter()
            .zip(&found.proxies)
            .map(|(&coverage, &proxy)| Look::of(coverage, proxy))
            .collect();
        assert!(looks.iter().all(|look| *look == Look::Model), "{looks:?}");
        assert!(die.heat.peak() > 10.0, "the model's own heat");
        // Nothing the view writes calls a number degrees: no tick, no row, no title.
        let texts: Vec<&str> = scene
            .tags
            .iter()
            .map(|tag| tag.text.as_str())
            .chain(scene.places.iter().map(|(_, text)| text.as_str()))
            .collect();
        for text in &texts {
            let digits_then_unit = text
                .split_whitespace()
                .zip(text.split_whitespace().skip(1))
                .any(|(number, unit)| {
                    number.trim_start_matches('~').parse::<f32>().is_ok() && unit == "C"
                });
            assert!(
                !digits_then_unit && !text.contains("TjMax"),
                "degrees in {text:?}"
            );
        }
        assert!(texts.contains(&"hot") && texts.contains(&"cold"));
        assert!(texts.iter().any(|text| text.contains("model")));
        // The strip names the missing sources in the notes' wording, then says what the field is.
        let legend = scene.legend(view("die"), &snapshot);
        assert!(
            legend.contains(" | no temperature: field is model only"),
            "{legend}"
        );
        assert!(
            legend.contains(" | no throttle counts: no throttle flashes"),
            "{legend}"
        );
        assert!(
            legend.contains(" | no package power: RAPL n/a (root only), heat source unscaled"),
            "{legend}"
        );
        assert!(legend.contains("relative heat, no degrees"), "{legend}");
        assert!(
            legend.contains("ironbow = relative heat modelled from load (not degrees)"),
            "{legend}"
        );
        assert!(legend.contains("hatched = no sensor"), "{legend}");
        assert!(
            !legend.contains("package n/a ") || !legend.contains(" C"),
            "{legend}"
        );
        // The picture differs from the same sample with sensors: the hatch, and the scale.
        let with = {
            let mut scene = Scene::new();
            let mut frame = None;
            for second in 9..=12 {
                frame = Some(draw(&mut scene, &demo(f64::from(second)), second as f32));
            }
            frame.expect("drew four samples")
        };
        assert_ne!(frame.pixels, with.pixels);
        // The bar is the model's own: its scale is 0 to 75 units, not 30 degrees and up.
        assert_eq!(found.scale(), (0.0, readings::RELATIVE_SPAN));
    }

    #[test]
    fn a_core_with_no_load_reading_and_no_sensor_is_drawn_blank_not_cold() {
        let mut snapshot = without_sensors(12.0);
        for cpu in snapshot.cpus.iter_mut().filter(|cpu| cpu.id < 2) {
            cpu.busy = None;
        }
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 12.0);
        let die = die(&scene);
        let found = die.last.readings.as_ref().expect("readings");
        assert_eq!(found.proxies[0], None, "P0 has no load reading");
        assert!(found.proxies[1].is_some());
        let cold = crate::palette::ironbow(0.0);
        let layout = die.layout.as_ref().expect("layout");
        let block = &layout.plan.blocks[0];
        let cell = |x: f32, y: f32| heat::cell_at(x, y) as usize;
        let inside = [block.hot, [block.rect[0] + 1.0, block.rect[1] + 1.0]];
        for [x, y] in inside {
            let color = die.field.colors[cell(x, y)];
            assert_ne!(color, cold, "not painted as the coldest colour");
        }
        let blank: Vec<Color> = inside
            .iter()
            .map(|&[x, y]| die.field.colors[cell(x, y)])
            .collect();
        assert!(
            blank
                .iter()
                .all(|color| *color == [24, 22, 32] || *color == [64, 64, 84])
        );
    }

    #[test]
    fn process_points_keep_their_places_when_cpu_memory_or_population_change() {
        let positions = |snapshot: &Snapshot| {
            let mut scene = Scene::new();
            draw(&mut scene, snapshot, 12.0);
            scene.positions.clone()
        };
        let base = demo(12.0);
        let before = positions(&base);
        assert!(
            before.len() > 150,
            "the demo's processes are drawn: {}",
            before.len()
        );
        let heavier = positions(&rescaled(&base));
        assert_eq!(before.len(), heavier.len());
        for (id, place) in &before {
            assert_eq!(
                heavier.get(id),
                Some(place),
                "pid {} moved with its CPU or memory",
                id.pid
            );
        }
        let (changed, gone) = churned(&base);
        let after = positions(&changed);
        for (id, place) in &before {
            if !gone.contains(id) {
                assert_eq!(
                    after.get(id),
                    Some(place),
                    "pid {} moved when others came and went",
                    id.pid
                );
            }
        }
        // A process that last ran on another core is on that core's block, the same way over.
        let mut moved = base.clone();
        let mover = moved.processes[3].id;
        let core = moved.processes[3].core.expect("the demo gives a last CPU");
        moved.processes[3].core = Some((core + 8) % 16);
        let after = positions(&moved);
        assert_ne!(after[&mover], before[&mover]);
        let steady = before
            .keys()
            .filter(|&&id| id != mover)
            .all(|id| after[id] == before[id]);
        assert!(steady, "nobody else moved");
    }

    #[test]
    fn a_process_with_no_last_cpu_sits_on_its_p_or_e_cluster_by_performance_share() {
        use crate::model::testing::process;
        // The shape macOS gives: kinds, no caches, and no last CPU for any process.
        let cpus = (0..6)
            .map(|number| crate::model::thermal::CpuPlace {
                cpu: number,
                package: Some(0),
                die: None,
                core_id: Some(number),
                cluster: Some(u32::from(number < 2)),
                siblings: vec![number],
                l2: None,
                l3: None,
                max_mhz: None,
                kind: if number < 2 {
                    CoreKind::Efficiency
                } else {
                    CoreKind::Performance
                },
            })
            .collect();
        let plan = Floorplan::of(&Topology { cpus });
        let at = |share: Option<f32>, core: Option<u32>| {
            let mut one = process(7, 1);
            one.core = core;
            one.performance_share = share;
            placement(&plan, &one)
        };
        let (block, offset) = at(Some(0.9), None).expect("placed on a P cluster");
        assert_eq!(plan.blocks[block].role, Role::Performance);
        assert!(
            offset
                .iter()
                .all(|fraction| (0.12..0.88).contains(fraction))
        );
        let (block, _) = at(Some(0.1), None).expect("placed on an E cluster");
        assert_eq!(plan.blocks[block].role, Role::Efficiency);
        assert_eq!(at(None, None), None, "no CPU and no share: not placed");
        assert_eq!(
            at(None, Some(99)),
            None,
            "a CPU the topology does not know: not placed"
        );
        // The same identity lands on the same block and offset every time.
        assert_eq!(at(Some(0.9), None), at(Some(0.9), None));
        // A last CPU wins over the share.
        let (block, _) = at(Some(0.9), Some(0)).expect("placed by its CPU");
        assert_eq!(plan.blocks[block].cpus, [0]);
    }

    #[test]
    fn a_core_block_picks_as_a_thing_and_its_card_tells_what_is_measured() {
        let (scene, snapshot, frame) = watched();
        assert_eq!(frame.things.len(), 12, "one thing per core block");
        let picked: Vec<Thing> = frame
            .picks
            .iter()
            .filter(|&&pick| pick != NONE && pick & THING != 0)
            .filter_map(|&pick| match frame.target(pick) {
                Some(Target::Thing(thing)) => Some(thing),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        assert!(
            picked.len() >= 8,
            "most blocks have a pixel: {}",
            picked.len()
        );
        assert!(picked.iter().all(|thing| thing.kind == CORE));
        let p2 = Thing { kind: CORE, key: 4 };
        let card = scene.card(view("die"), p2, &snapshot).expect("P2's card");
        assert_eq!(card.title, "P2 core");
        let text = card.lines.join("\n");
        assert!(
            text.contains("performance core with SMT, cpu4 cpu5"),
            "{text}"
        );
        assert!(
            text.contains("measured by its own sensor (Core 8)"),
            "{text}"
        );
        assert!(
            text.contains("heat source: busy x (clock / fmax)^3 ="),
            "{text}"
        );
        assert!(
            text.contains("throttled 90 times since boot, 0 seen while watching"),
            "{text}"
        );
        // A core no sensor speaks for says that the colour is the model's.
        let mut bare = snapshot.clone();
        bare.thermal
            .as_mut()
            .expect("thermal")
            .sensors
            .retain(|sensor| {
                sensor.place
                    != SensorPlace::Core {
                        package: 0,
                        core_id: 8,
                    }
            });
        let mut scene = Scene::new();
        draw(&mut scene, &bare, 12.0);
        let card = scene.card(view("die"), p2, &bare).expect("P2's card");
        assert!(
            card.lines.join("\n").contains("no sensor reads this core"),
            "{card:?}"
        );
        // A thing the die did not draw has no card.
        let stray = Thing {
            kind: CORE,
            key: 999,
        };
        assert_eq!(scene.card(view("die"), stray, &bare), None);
        assert_eq!(
            scene.card(
                view("die"),
                Thing {
                    kind: "grave",
                    key: 4
                },
                &bare
            ),
            None
        );
    }

    #[test]
    fn a_hostile_sensor_label_reaches_the_card_raw_for_the_app_to_clean() {
        let mut snapshot = demo(12.0);
        for sensor in &mut snapshot.thermal.as_mut().expect("thermal").sensors {
            sensor.label = format!("\x1b]0;pwned\x07{}", sensor.label);
        }
        let mut scene = Scene::new();
        draw(&mut scene, &snapshot, 12.0);
        let thing = Thing { kind: CORE, key: 4 };
        let card = scene.card(view("die"), thing, &snapshot).expect("card");
        assert!(
            card.lines.iter().any(|line| line.contains("pwned")),
            "it is the app that cleans text"
        );
        // Nothing the view writes into the frame itself comes from a sensor.
        let texts: Vec<&str> = scene.tags.iter().map(|tag| tag.text.as_str()).collect();
        assert!(
            texts.iter().all(|text| !text.contains("pwned")),
            "{texts:?}"
        );
    }

    #[test]
    fn the_legend_names_every_mapping_and_what_the_demo_reads() {
        let (scene, snapshot, _) = watched();
        let legend = scene.legend(view("die"), &snapshot);
        assert!(legend.starts_with(" package "), "{legend}");
        assert!(legend.contains(" C | RAPL "), "{legend}");
        for mapping in [
            "ironbow = heat: sensor where one reads, model between",
            "source = busy x (clock / fmax)^3",
            "points = processes at their last CPU, size = CPU",
            "cyan outline = throttle",
            "hatched = no sensor",
            " | sources: cpu topology, temperature, throttle counts, package power",
        ] {
            assert!(legend.contains(mapping), "{mapping:?} in {legend}");
        }
        assert!(
            !legend.contains(" | no "),
            "the demo reads every source: {legend}"
        );
        // Without a clock the source is busy alone, and without a last CPU the points move.
        let mut bare = snapshot.clone();
        bare.missing = Sources::of(&[Source::CpuClock, Source::LastCpu, Source::Topology]);
        let legend = scene.legend(view("die"), &bare);
        assert!(
            legend.contains(" | no cpu clock: heat source is busy alone"),
            "{legend}"
        );
        assert!(
            legend.contains(" | no last cpu: points sit on their P or E cluster"),
            "{legend}"
        );
        assert!(
            legend.contains(" | no cpu topology: floorplan from the CPU list"),
            "{legend}"
        );
    }

    #[test]
    fn the_field_is_one_draw_and_the_frame_stays_small_however_many_processes_run() {
        let (_, _, frame) = watched();
        let fields = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Field { .. }))
            .count();
        assert_eq!(
            fields, 2,
            "one field of two triangles, never a pair per cell"
        );
        let spheres = frame
            .items
            .iter()
            .filter(|item| matches!(item, Item::Sphere { .. }))
            .count();
        assert_eq!(spheres, 192, "a sphere a process");
        assert!(frame.items.len() < 9000, "{} items", frame.items.len());
    }

    #[test]
    fn the_floorplan_does_not_move_with_the_load_or_the_population() {
        let plans = |snapshot: &Snapshot| {
            let mut scene = Scene::new();
            draw(&mut scene, snapshot, 12.0);
            die(&scene).layout.as_ref().expect("layout").plan.clone()
        };
        let base = demo(12.0);
        let plan = plans(&base);
        assert_eq!(plan, plans(&rescaled(&base)));
        assert_eq!(plan, plans(&churned(&base).0));
        let mut busy = base.clone();
        for cpu in &mut busy.cpus {
            cpu.busy = Some(1.0);
        }
        assert_eq!(plan, plans(&busy), "a whole die of load changes no block");
        let mut quiet = base;
        quiet.cpus.iter_mut().for_each(|cpu| cpu.busy = Some(0.0));
        assert_eq!(plan, plans(&quiet));
    }

    #[test]
    fn the_tour_visits_the_busiest_points_with_the_places_they_are_drawn_at() {
        let (scene, snapshot, _) = watched();
        assert_eq!(scene.tour.len(), TOUR);
        assert!(
            scene
                .tour
                .windows(2)
                .all(|pair| pair[0].weight >= pair[1].weight)
        );
        for stop in &scene.tour {
            assert_eq!(scene.positions.get(&stop.id), Some(&stop.position));
        }
        assert!(
            scene.tour[0].weight > 10.0,
            "the demo has a process that is busy"
        );
        assert_eq!(scene.tour.len(), TOUR.min(snapshot.processes.len()));
    }

    #[test]
    fn an_empty_machine_draws_an_empty_die_and_says_it_is_reading_the_topology() {
        let mut scene = Scene::new();
        let empty = Snapshot::default();
        let frame = draw(&mut scene, &empty, 1.0);
        assert!(frame.things.is_empty(), "no core to pick");
        assert!(scene.positions.is_empty());
        assert_eq!(scene.visible, 0);
        assert!(
            scene
                .places
                .iter()
                .any(|(_, text)| text.starts_with("no CPU topology")),
            "the caption says what it has"
        );
    }

    #[test]
    fn rewinding_and_pausing_do_not_step_the_field_but_a_long_gap_settles_it_again() {
        let mut scene = Scene::new();
        let camera = Camera::default();
        let snapshot = demo(12.0);
        for tenth in 0..30 {
            let time = 12.0 + tenth as f32 / 10.0;
            scene.render(&RenderInput::small(&snapshot, view("die"), &camera, time));
        }
        let held = die(&scene).heat.field.clone();
        // A frame at an earlier time (a rewind) takes no step, and the same time again takes none.
        scene.render(&RenderInput::small(&snapshot, view("die"), &camera, 14.0));
        assert_eq!(die(&scene).heat.field, held);
        let paused = RenderInput {
            paused: true,
            ..RenderInput::small(&snapshot, view("die"), &camera, 14.3)
        };
        scene.render(&paused);
        assert_eq!(
            die(&scene).heat.field,
            held,
            "a paused frame takes none either"
        );
        // A view shown again after a long gap is settled from the load it finds, not continued
        // from a picture that old; under the same load it settles where it had settled.
        scene.render(&RenderInput::small(&snapshot, view("die"), &camera, 60.0));
        let settled = die(&scene).heat.field.clone();
        let peak = |field: &[f32]| field.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            (peak(&settled) - peak(&held)).abs() < 3.0,
            "{} against {}",
            peak(&settled),
            peak(&held)
        );
    }
}
