//! The die's floorplan: a pure function of the CPU topology (`Floorplan::of`), in cells of the
//! 192 by 96 grid the heat runs on. Nothing a sensor, a load or a process does moves a block.
//!
//! There are four generators, picked by the shape of the topology: Intel hybrid (P-cores in a
//! row over a ring of L3 slices, E-core clusters under them), Intel tiled (the same, with the
//! low-power E-cores in a tile of their own), AMD core complexes (an I/O die and a box per L3
//! group, its cores above and below the L3), and a generic core grid around a cache for anything
//! else. A hybrid part that names no cache gets two clusters of cores by kind (what macOS gives:
//! its topology is the core kinds). Logical CPUs map to blocks through the topology's cores, so
//! SMT siblings share a block. The layout is schematic: no size or position is to scale, and
//! only the blocks that are cores carry any measurement.

use std::collections::HashMap;

use crate::model::CoreKind;
use crate::model::thermal::{Core, Topology};

/// The grid the heat runs on, in cells.
pub const COLUMNS: usize = 192;
pub const ROWS: usize = 96;

/// A rectangle in cells: left, top, right and bottom, with x to the right and y down the plan.
pub type Rect = [f32; 4];

/// The part of the plan, left to right, that the compute blocks use, and the margin round it.
const COMPUTE: Rect = [31.0, 6.0, 133.0, 90.0];
const LEFT_EDGE: Rect = [3.0, 6.0, 27.0, 90.0];
const RIGHT_EDGE: Rect = [137.0, 6.0, 189.0, 90.0];
const GAP: f32 = 3.0;
/// The most P-core blocks in one row, and the most clusters or core blocks in one row of a box.
const ROW_OF_CORES: usize = 8;
const ROW_OF_CLUSTERS: usize = 4;

/// Which generator laid the plan out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    IntelHybrid,
    IntelTiled,
    Amd,
    /// Cores of two kinds in clusters, with no cache named (macOS).
    Clustered,
    Generic,
}

/// What a core block is, for its name and its heat weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Performance,
    Efficiency,
    /// An efficiency core of a tiled part's own tile, outside the compute tile's cache.
    LowPower,
    /// A core of a part with no kinds.
    Plain,
}

/// A physical core: the block its logical CPUs share. The only blocks that carry a reading.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub rect: Rect,
    /// The logical CPUs of the core (SMT siblings), sorted.
    pub cpus: Vec<u32>,
    pub package: Option<u32>,
    pub core_id: Option<u32>,
    pub kind: CoreKind,
    pub role: Role,
    /// `P2`, `E5`, `LP0` or `C3`: the number counts blocks of its role in plan order.
    pub name: String,
    /// Where the core's hot spot (and its sensor, if it has one) is, in cells.
    pub hot: [f32; 2],
    /// The group it is drawn in, if any.
    pub group: Option<usize>,
    /// The highest clock of its CPUs the topology knows, in MHz.
    pub max_mhz: Option<f32>,
}

/// What a group of core blocks is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKind {
    Efficiency,
    LowPower,
    /// A core complex: the cores that share one L3.
    Complex,
    /// All of the cores of a role, in the box the generic grid draws.
    Row,
}

/// A box drawn round some core blocks: an E-core cluster, a core complex.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub rect: Rect,
    pub name: String,
    pub kind: GroupKind,
    pub blocks: Vec<usize>,
}

/// A part of the die that has no source: a cache, the graphics, the system agent, the I/O die.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    L3,
    L2,
    Gpu,
    Agent,
    Io,
}

/// Schematic context: drawn, never measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Context {
    pub rect: Rect,
    pub part: Part,
    pub label: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Floorplan {
    pub family: Family,
    pub blocks: Vec<Block>,
    pub groups: Vec<Group>,
    pub context: Vec<Context>,
    /// The caption written under the die: `<family> <n>P + <m>E  schematic, not to scale`.
    pub caption: String,
    /// The package drawn: the lowest one the topology names, or None when it names none.
    pub package: Option<u32>,
    /// How many packages the machine has; the die shows one.
    pub packages: usize,
    by_cpu: HashMap<u32, usize>,
}

fn width(rect: Rect) -> f32 {
    rect[2] - rect[0]
}

fn height(rect: Rect) -> f32 {
    rect[3] - rect[1]
}

/// `count` equal rectangles in rows of `columns` over `region`, row by row, `GAP` apart and from
/// its edge.
fn tile(region: Rect, count: usize, columns: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let columns = columns.clamp(1, count);
    let rows = count.div_ceil(columns);
    let cell = [
        (width(region) - GAP * (columns as f32 + 1.0)) / columns as f32,
        (height(region) - GAP * (rows as f32 + 1.0)) / rows as f32,
    ];
    (0..count)
        .map(|index| {
            let (column, row) = ((index % columns) as f32, (index / columns) as f32);
            let left = region[0] + GAP + column * (cell[0] + GAP);
            let top = region[1] + GAP + row * (cell[1] + GAP);
            [left, top, left + cell[0], top + cell[1]]
        })
        .collect()
}

/// A strip of cells, `slices` equal slices side by side, for the L3 ring.
fn strip(region: Rect, slices: usize) -> Vec<Rect> {
    let slice = width(region) / slices.max(1) as f32;
    (0..slices)
        .map(|index| {
            let left = region[0] + slice * index as f32;
            [left + 0.5, region[1], left + slice - 0.5, region[3]]
        })
        .collect()
}

struct Builder {
    blocks: Vec<Block>,
    groups: Vec<Group>,
    context: Vec<Context>,
    /// Blocks of each role placed so far, for the names.
    named: [usize; 4],
}

impl Builder {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            groups: Vec::new(),
            context: Vec::new(),
            named: [0; 4],
        }
    }

    fn part(&mut self, rect: Rect, part: Part, label: &'static str) {
        self.context.push(Context { rect, part, label });
    }

    /// Places `core` in `rect` and returns its block's index.
    fn core(&mut self, rect: Rect, core: &Core, role: Role) -> usize {
        let (slot, prefix, hot) = match role {
            Role::Performance => (0, "P", [0.42, 0.40]),
            Role::Efficiency => (1, "E", [0.5, 0.5]),
            Role::LowPower => (2, "LP", [0.5, 0.5]),
            Role::Plain => (3, "C", [0.42, 0.40]),
        };
        let name = format!("{prefix}{}", self.named[slot]);
        self.named[slot] += 1;
        self.blocks.push(Block {
            rect,
            cpus: core.cpus.clone(),
            package: core.package,
            core_id: core.core_id,
            kind: core.kind,
            role,
            name,
            hot: [
                rect[0] + width(rect) * hot[0],
                rect[1] + height(rect) * hot[1],
            ],
            group: None,
            max_mhz: None,
        });
        self.blocks.len() - 1
    }

    /// Draws a box named `name` round the blocks `members`.
    fn group(&mut self, rect: Rect, name: String, kind: GroupKind, members: Vec<usize>) {
        let index = self.groups.len();
        for &member in &members {
            self.blocks[member].group = Some(index);
        }
        self.groups.push(Group {
            rect,
            name,
            kind,
            blocks: members,
        });
    }

    /// A box of cores with its cache under (or between) them: used for clusters and complexes.
    fn boxed(
        &mut self,
        rect: Rect,
        cores: &[&Core],
        role: Role,
        name: String,
        kind: GroupKind,
        cache: Option<(Part, &'static str)>,
    ) {
        let reserve = if cache.is_some() { 9.0 } else { 0.0 };
        let region = [
            rect[0],
            rect[1] + 6.0,
            rect[2],
            rect[3] - reserve - if cache.is_some() { 1.0 } else { 0.0 },
        ];
        let columns = cores.len().clamp(1, ROW_OF_CLUSTERS);
        let members = tile(region, cores.len(), columns)
            .into_iter()
            .zip(cores)
            .map(|(cell, core)| self.core(cell, core, role))
            .collect();
        if let Some((part, label)) = cache {
            self.part(
                [rect[0] + 2.0, rect[3] - reserve, rect[2] - 2.0, rect[3] - 2.0],
                part,
                label,
            );
        }
        self.group(rect, name, kind, members);
    }
}

/// How the cores of a part split, in plan order.
struct Cores<'a> {
    performance: Vec<&'a Core>,
    /// E-cores by cluster, clusters in order of their lowest CPU.
    efficiency: Vec<Vec<&'a Core>>,
    low_power: Vec<Vec<&'a Core>>,
}

impl Floorplan {
    /// The plan for `topology`. The die is one package's: the lowest the topology names (cores
    /// whose package is unknown count as in it). An empty topology gives an empty plan.
    pub fn of(topology: &Topology) -> Floorplan {
        let packages = topology.packages();
        let package = packages.first().copied();
        let cores: Vec<Core> = topology
            .cores()
            .into_iter()
            .filter(|core| core.package.is_none_or(|found| Some(found) == package))
            .collect();
        let mut builder = Builder::new();
        let family = if cores.is_empty() {
            Family::Generic
        } else {
            Self::lay_out(&mut builder, topology, &cores, package)
        };
        let mut plan = Floorplan {
            family,
            blocks: builder.blocks,
            groups: builder.groups,
            context: builder.context,
            caption: String::new(),
            package,
            packages: packages.len().max(1),
            by_cpu: HashMap::new(),
        };
        plan.by_cpu = plan
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(index, block)| block.cpus.iter().map(move |&cpu| (cpu, index)))
            .collect();
        for block in &mut plan.blocks {
            block.max_mhz = block
                .cpus
                .iter()
                .filter_map(|&cpu| topology.place(cpu).and_then(|place| place.max_mhz))
                .reduce(f32::max);
        }
        plan.caption = plan.caption();
        plan
    }

    /// The block whose core runs logical CPU `cpu`.
    pub fn block_of_cpu(&self, cpu: u32) -> Option<usize> {
        self.by_cpu.get(&cpu).copied()
    }

    /// The blocks of `role`.
    pub fn count(&self, role: Role) -> usize {
        self.blocks
            .iter()
            .filter(|block| block.role == role)
            .count()
    }

    fn caption(&self) -> String {
        let (performance, efficiency) = (
            self.count(Role::Performance),
            self.count(Role::Efficiency),
        );
        let low_power = self.count(Role::LowPower);
        let mut caption = match self.family {
            _ if self.blocks.is_empty() => "no CPU topology".to_owned(),
            Family::IntelHybrid => format!("Intel hybrid {performance}P + {efficiency}E"),
            Family::IntelTiled => {
                format!("Intel tiled {performance}P + {efficiency}E + {low_power}LP")
            }
            Family::Amd => format!(
                "AMD {} cores in {} core complexes",
                self.blocks.len(),
                self.groups.len()
            ),
            Family::Clustered => format!(
                "{}{performance}P + {efficiency}E clusters",
                if cfg!(target_os = "macos") {
                    "Apple "
                } else {
                    ""
                }
            ),
            Family::Generic => format!("{} cores", self.blocks.len()),
        };
        if self.packages > 1 {
            caption.push_str(&format!(" (one of {} packages)", self.packages));
        }
        caption.push_str("  schematic, not to scale");
        caption
    }

    /// Picks the generator for the shape of the topology and runs it.
    fn lay_out(
        builder: &mut Builder,
        topology: &Topology,
        cores: &[Core],
        package: Option<u32>,
    ) -> Family {
        let clusters = topology.clusters();
        let cluster_of = |core: &Core| {
            clusters
                .iter()
                .position(|cluster| cluster.cpus.contains(&core.cpus[0]))
        };
        let hybrid = cores.iter().any(|core| core.kind == CoreKind::Performance)
            && cores.iter().any(|core| core.kind == CoreKind::Efficiency);
        let any_l3 = topology.cpus.iter().any(|place| place.l3.is_some());
        let groups_of_l3: Vec<Vec<u32>> = topology
            .l3_groups()
            .into_iter()
            .filter(|group| {
                group.iter().any(|&cpu| {
                    topology
                        .place(cpu)
                        .is_some_and(|place| place.package.is_none_or(|found| Some(found) == package))
                })
            })
            .collect();
        let low_power = |core: &Core| cluster_of(core).is_some_and(|at| clusters[at].low_power);
        if hybrid {
            let efficient = |core: &&Core| !low_power(core) && core.kind == CoreKind::Efficiency;
            let split = Cores {
                performance: cores
                    .iter()
                    .filter(|core| !low_power(core) && core.kind != CoreKind::Efficiency)
                    .collect(),
                efficiency: by_cluster(cores.iter().filter(efficient), &cluster_of),
                low_power: by_cluster(cores.iter().filter(|core| low_power(core)), &cluster_of),
            };
            let tiled = !split.low_power.is_empty();
            if !any_l3 {
                clustered(builder, &split);
                Family::Clustered
            } else {
                hybrid_plan(builder, &split, tiled);
                if tiled {
                    Family::IntelTiled
                } else {
                    Family::IntelHybrid
                }
            }
        } else if groups_of_l3.len() >= 2 {
            amd_plan(builder, cores, &groups_of_l3);
            Family::Amd
        } else {
            generic_plan(builder, cores);
            Family::Generic
        }
    }
}

/// `cores` grouped by the cluster `cluster_of` names for them, clusters in the order their first
/// core appears (cores are ordered by their lowest CPU); a core with no known cluster stands
/// alone.
fn by_cluster<'a>(
    cores: impl Iterator<Item = &'a Core>,
    cluster_of: &impl Fn(&Core) -> Option<usize>,
) -> Vec<Vec<&'a Core>> {
    let mut clusters: Vec<(Option<usize>, Vec<&'a Core>)> = Vec::new();
    for core in cores {
        let key = cluster_of(core);
        let open = key.and_then(|key| clusters.iter().position(|(other, _)| *other == Some(key)));
        match open {
            Some(at) => clusters[at].1.push(core),
            None => clusters.push((key, vec![core])),
        }
    }
    clusters.into_iter().map(|(_, members)| members).collect()
}

/// Intel hybrid and tiled: the P-core row, the L3 ring, the E-core clusters, the iGPU on the
/// right and the system agent (with the low-power tile of a tiled part) on the left.
fn hybrid_plan(builder: &mut Builder, split: &Cores, tiled: bool) {
    let p_rows = split.performance.len().div_ceil(ROW_OF_CORES).clamp(1, 3);
    let p_height = [34.0, 44.0, 52.0][p_rows - 1];
    let p_region = [COMPUTE[0], COMPUTE[1], COMPUTE[2], COMPUTE[1] + p_height];
    for (cell, core) in tile(p_region, split.performance.len(), ROW_OF_CORES)
        .into_iter()
        .zip(&split.performance)
    {
        builder.core(cell, core, Role::Performance);
    }
    let ring_top = if split.performance.is_empty() {
        COMPUTE[1]
    } else {
        p_region[3] + GAP
    };
    let ring = [COMPUTE[0], ring_top, COMPUTE[2], ring_top + 11.0];
    let slices = split.performance.len().clamp(4, ROW_OF_CORES);
    for cell in strip(ring, slices) {
        builder.part(cell, Part::L3, "");
    }
    let region = [COMPUTE[0], ring[3] + GAP, COMPUTE[2], COMPUTE[3]];
    let clusters = split.efficiency.len();
    for (index, (cell, members)) in tile(region, clusters, ROW_OF_CLUSTERS)
        .into_iter()
        .zip(&split.efficiency)
        .enumerate()
    {
        builder.boxed(
            cell,
            members,
            Role::Efficiency,
            format!("E-cluster {index}"),
            GroupKind::Efficiency,
            Some((Part::L2, "L2")),
        );
    }
    builder.part(RIGHT_EDGE, Part::Gpu, "iGPU");
    if tiled {
        let agent = [LEFT_EDGE[0], LEFT_EDGE[1], LEFT_EDGE[2], 50.0];
        let tile_rect = [LEFT_EDGE[0], 54.0, LEFT_EDGE[2], LEFT_EDGE[3]];
        builder.part(agent, Part::Agent, "SoC");
        let members: Vec<&Core> = split.low_power.iter().flatten().copied().collect();
        builder.boxed(
            tile_rect,
            &members,
            Role::LowPower,
            "LP E-cluster".to_owned(),
            GroupKind::LowPower,
            None,
        );
    } else {
        builder.part(LEFT_EDGE, Part::Agent, "SA");
    }
}

/// Cores of two kinds in two clusters, with no cache to name and a graphics part: macOS.
fn clustered(builder: &mut Builder, split: &Cores) {
    let efficiency: Vec<&Core> = split.efficiency.iter().flatten().copied().collect();
    let p_region = [COMPUTE[0], COMPUTE[1], COMPUTE[2], 58.0];
    let e_region = [COMPUTE[0], 62.0, COMPUTE[2], COMPUTE[3]];
    for (name, region, cores, role) in [
        ("P cluster", p_region, &split.performance, Role::Performance),
        ("E cluster", e_region, &efficiency, Role::Efficiency),
    ] {
        let members: Vec<usize> = tile(region, cores.len(), ROW_OF_CORES.min(6))
            .into_iter()
            .zip(cores)
            .map(|(cell, core)| builder.core(cell, core, role))
            .collect();
        builder.group(region, name.to_owned(), GroupKind::Row, members);
    }
    builder.part(LEFT_EDGE, Part::L3, "SLC");
    builder.part(RIGHT_EDGE, Part::Gpu, "GPU");
}

/// AMD: an I/O die on the left and one box per L3 group, its cores above and below the cache.
fn amd_plan(builder: &mut Builder, cores: &[Core], groups: &[Vec<u32>]) {
    builder.part(LEFT_EDGE, Part::Io, "IOD");
    let region = [34.0, 6.0, 189.0, 90.0];
    let columns = groups.len().clamp(1, ROW_OF_CLUSTERS);
    for (index, (rect, group)) in tile(region, groups.len(), columns)
        .into_iter()
        .zip(groups)
        .enumerate()
    {
        let members: Vec<&Core> = cores
            .iter()
            .filter(|core| core.cpus.iter().all(|cpu| group.contains(cpu)))
            .collect();
        let top = members.len().div_ceil(2);
        let strip_height = 10.0;
        let middle = (rect[1] + rect[3]) / 2.0;
        let cache = [rect[0] + 2.0, middle - strip_height / 2.0, rect[2] - 2.0, middle + strip_height / 2.0];
        let above = [rect[0], rect[1] + 6.0, rect[2], cache[1] - 1.0];
        let below = [rect[0], cache[3] + 1.0, rect[2], rect[3] - 1.0];
        let mut placed = Vec::new();
        for (region, slice) in [(above, &members[..top]), (below, &members[top..])] {
            for (cell, core) in tile(region, slice.len(), slice.len().max(1))
                .into_iter()
                .zip(slice)
            {
                placed.push(builder.core(cell, core, Role::Plain));
            }
        }
        builder.part(cache, Part::L3, "L3");
        builder.group(rect, format!("CCD {index}"), GroupKind::Complex, placed);
    }
}

/// Anything else: the cores in a grid, a cache across the middle.
fn generic_plan(builder: &mut Builder, cores: &[Core]) {
    let region = [3.0, 6.0, 189.0, 90.0];
    let columns = ((cores.len() as f32 * 2.0).sqrt().ceil() as usize).max(1);
    let rows = cores.len().div_ceil(columns);
    let top_rows = rows.div_ceil(2);
    let top = (top_rows * columns).min(cores.len());
    let strip_height = 12.0;
    let middle = (region[1] + region[3]) / 2.0;
    let cache = if rows > 1 {
        [region[0], middle - strip_height / 2.0, region[2], middle + strip_height / 2.0]
    } else {
        [region[0], region[3] - strip_height, region[2], region[3]]
    };
    let above = [region[0], region[1], region[2], if rows > 1 { cache[1] - 1.0 } else { cache[1] - 1.0 }];
    let below = [region[0], cache[3] + 1.0, region[2], region[3]];
    for (region, slice) in [(above, &cores[..top]), (below, &cores[top..])] {
        for (cell, core) in tile(region, slice.len(), columns).into_iter().zip(slice) {
            builder.core(cell, core, Role::Plain);
        }
    }
    builder.part(cache, Part::L3, "cache");
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::thermal::CpuPlace;

    /// A CPU with the given place, for the topologies below.
    #[expect(clippy::too_many_arguments, reason = "one literal of a test fixture")]
    pub(crate) fn cpu(
        cpu: u32,
        core_id: u32,
        siblings: &[u32],
        l2: Option<&[u32]>,
        l3: Option<&[u32]>,
        kind: CoreKind,
        package: u32,
    ) -> CpuPlace {
        CpuPlace {
            cpu,
            package: Some(package),
            die: Some(0),
            core_id: Some(core_id),
            cluster: None,
            siblings: siblings.to_vec(),
            l2: l2.map(<[u32]>::to_vec),
            l3: l3.map(<[u32]>::to_vec),
            max_mhz: Some(4800.0),
            kind,
        }
    }

    /// An Intel hybrid part: `p` SMT P-cores on CPUs 0 to 2p, then E-core clusters of four.
    pub(crate) fn hybrid(p: u32, e_clusters: u32, with_l3: bool) -> Topology {
        let threads = 2 * p;
        let l3: Vec<u32> = (0..threads + 4 * e_clusters).collect();
        let l3 = with_l3.then_some(&l3[..]);
        let mut cpus = Vec::new();
        for core in 0..p {
            let pair = [2 * core, 2 * core + 1];
            for thread in pair {
                cpus.push(cpu(thread, 4 * core, &pair, Some(&pair), l3, CoreKind::Performance, 0));
            }
        }
        for cluster in 0..e_clusters {
            let first = threads + 4 * cluster;
            let group: Vec<u32> = (first..first + 4).collect();
            for thread in &group {
                cpus.push(cpu(
                    *thread,
                    4 * p + (thread - threads),
                    &[*thread],
                    Some(&group),
                    l3,
                    CoreKind::Efficiency,
                    0,
                ));
            }
        }
        Topology { cpus }
    }

    /// An AMD part: `ccds` complexes of eight SMT-less cores, one L3 each.
    pub(crate) fn amd(ccds: u32) -> Topology {
        let mut cpus = Vec::new();
        for ccd in 0..ccds {
            let group: Vec<u32> = (8 * ccd..8 * ccd + 8).collect();
            for &thread in &group {
                cpus.push(cpu(
                    thread,
                    thread % 8,
                    &[thread],
                    Some(&[thread]),
                    Some(&group),
                    CoreKind::Unknown,
                    0,
                ));
            }
        }
        Topology { cpus }
    }

    fn rects_overlap(a: Rect, b: Rect) -> bool {
        a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
    }

    fn inside_the_die(rect: Rect) -> bool {
        rect[0] >= 0.0 && rect[1] >= 0.0 && rect[2] <= COLUMNS as f32 && rect[3] <= ROWS as f32
    }

    /// No core block overlaps another, and every block lies on the die.
    fn assert_sane(plan: &Floorplan) {
        for (index, block) in plan.blocks.iter().enumerate() {
            assert!(inside_the_die(block.rect), "{} is off the die", block.name);
            assert!(width(block.rect) > 0.5 && height(block.rect) > 0.5);
            for other in &plan.blocks[index + 1..] {
                assert!(
                    !rects_overlap(block.rect, other.rect),
                    "{} overlaps {}",
                    block.name,
                    other.name
                );
            }
            assert!(
                block.hot[0] > block.rect[0]
                    && block.hot[0] < block.rect[2]
                    && block.hot[1] > block.rect[1]
                    && block.hot[1] < block.rect[3],
                "{}'s hot spot is inside it",
                block.name
            );
        }
    }

    #[test]
    fn intel_hybrid_topology_with_smt_maps_siblings_to_one_block() {
        let plan = Floorplan::of(&hybrid(4, 2, true));
        assert_eq!(plan.family, Family::IntelHybrid);
        assert_eq!(plan.blocks.len(), 12, "four P-cores and eight E-cores");
        assert_eq!(plan.count(Role::Performance), 4);
        assert_eq!(plan.count(Role::Efficiency), 8);
        for pair in [[0, 1], [2, 3], [4, 5], [6, 7]] {
            let [a, b] = pair.map(|cpu| plan.block_of_cpu(cpu).expect("a P-core CPU"));
            assert_eq!(a, b, "SMT siblings {pair:?} share a block");
            assert_eq!(plan.blocks[a].cpus, pair);
        }
        assert_ne!(plan.block_of_cpu(0), plan.block_of_cpu(2));
        let names: Vec<&str> = plan
            .blocks
            .iter()
            .map(|block| block.name.as_str())
            .collect();
        assert_eq!(&names[..5], ["P0", "P1", "P2", "P3", "E0"]);
        assert_eq!(plan.groups.len(), 2, "one box per E-core cluster");
        assert_eq!(plan.groups[0].blocks.len(), 4);
        assert_eq!(plan.block_of_cpu(99), None);
        assert_eq!(
            plan.caption,
            "Intel hybrid 4P + 8E  schematic, not to scale"
        );
        assert_sane(&plan);
    }

    #[test]
    fn amd_topology_with_two_ccds_groups_eight_cores_around_each_l3() {
        let plan = Floorplan::of(&amd(2));
        assert_eq!(plan.family, Family::Amd);
        assert_eq!(plan.groups.len(), 2);
        for group in &plan.groups {
            assert_eq!(group.kind, GroupKind::Complex);
            assert_eq!(group.blocks.len(), 8, "eight cores to a complex");
            // The L3 strip lies between the two rows of cores: some cores above it and some below.
            let cache = plan
                .context
                .iter()
                .find(|part| {
                    part.part == Part::L3
                        && part.rect[0] >= group.rect[0]
                        && part.rect[2] <= group.rect[2]
                        && part.rect[1] >= group.rect[1]
                        && part.rect[3] <= group.rect[3]
                })
                .expect("each complex has its L3");
            let above = group
                .blocks
                .iter()
                .filter(|&&block| plan.blocks[block].rect[3] <= cache.rect[1])
                .count();
            let below = group
                .blocks
                .iter()
                .filter(|&&block| plan.blocks[block].rect[1] >= cache.rect[3])
                .count();
            assert_eq!((above, below), (4, 4), "four cores each side of the L3");
        }
        assert!(
            plan.context
                .iter()
                .any(|part| part.part == Part::Io && part.label == "IOD"),
            "the I/O die is drawn"
        );
        assert_eq!(plan.blocks.len(), 16);
        assert!(plan.caption.starts_with("AMD 16 cores in 2 core complexes"));
        assert_sane(&plan);
    }

    /// A tiled part: four P-cores, one cluster of four E-cores on the compute tile, and two
    /// low-power E-cores that share an L2 and have no L3.
    fn tiled() -> Topology {
        let mut topology = hybrid(4, 1, true);
        let l3: Vec<u32> = (0..12).collect();
        for place in &mut topology.cpus {
            place.l3 = Some(l3.clone());
        }
        let low = [12, 13];
        for cpu_number in low {
            topology.cpus.push(cpu(
                cpu_number,
                100 + cpu_number,
                &[cpu_number],
                Some(&low),
                None,
                CoreKind::Efficiency,
                0,
            ));
        }
        topology
    }

    #[test]
    fn a_tiled_part_puts_its_lp_e_cores_in_a_separate_cluster() {
        let plan = Floorplan::of(&tiled());
        assert_eq!(plan.family, Family::IntelTiled);
        assert_eq!(plan.count(Role::Performance), 4);
        assert_eq!(plan.count(Role::Efficiency), 4);
        assert_eq!(plan.count(Role::LowPower), 2);
        let low: Vec<&Group> = plan
            .groups
            .iter()
            .filter(|group| group.kind == GroupKind::LowPower)
            .collect();
        assert_eq!(low.len(), 1, "one tile for the low-power cores");
        assert_eq!(low[0].blocks.len(), 2);
        let compute = plan
            .groups
            .iter()
            .find(|group| group.kind == GroupKind::Efficiency)
            .expect("the compute tile's own cluster");
        assert!(
            low[0].blocks.iter().all(|block| !compute.blocks.contains(block)),
            "the low-power cores are not in the compute tile's cluster"
        );
        assert!(
            rects_overlap(low[0].rect, compute.rect) == false,
            "and sit in a tile of their own"
        );
        assert!(plan.caption.starts_with("Intel tiled 4P + 4E + 2LP"));
        assert_sane(&plan);
    }

    #[test]
    fn unknown_topology_falls_back_to_a_core_grid_around_a_cache() {
        // Eight cores with no kinds, no caches and no package: nothing says what they are.
        let cpus = (0..8)
            .map(|number| CpuPlace {
                cpu: number,
                package: None,
                die: None,
                core_id: None,
                cluster: None,
                siblings: Vec::new(),
                l2: None,
                l3: None,
                max_mhz: None,
                kind: CoreKind::Unknown,
            })
            .collect();
        let plan = Floorplan::of(&Topology { cpus });
        assert_eq!(plan.family, Family::Generic);
        assert_eq!(plan.blocks.len(), 8, "every CPU is a core of its own");
        let cache = plan
            .context
            .iter()
            .find(|part| part.part == Part::L3)
            .expect("a cache");
        let above = plan
            .blocks
            .iter()
            .filter(|block| block.rect[3] <= cache.rect[1])
            .count();
        let below = plan
            .blocks
            .iter()
            .filter(|block| block.rect[1] >= cache.rect[3])
            .count();
        assert_eq!(
            (above, below),
            (4, 4),
            "the cores stand above and below the cache"
        );
        assert_eq!(plan.caption, "8 cores  schematic, not to scale");
        assert_sane(&plan);
        // Nothing at all is an empty plan, not a panic.
        let empty = Floorplan::of(&Topology::default());
        assert!(empty.blocks.is_empty() && empty.context.is_empty());
        assert_eq!(empty.caption, "no CPU topology  schematic, not to scale");
    }

    #[test]
    fn apple_core_kinds_build_p_and_e_clusters() {
        // The shape `platform::macos::logic::topology` gives: the efficiency cores first, one
        // logical CPU to a core, a cluster per kind, no L2, no L3 and no clocks.
        let cpus = (0..10)
            .map(|number| CpuPlace {
                cpu: number,
                package: Some(0),
                die: None,
                core_id: Some(number),
                cluster: Some(if number < 2 { 1 } else { 0 }),
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
        assert_eq!(plan.family, Family::Clustered);
        assert_eq!(plan.groups.len(), 2, "a P cluster and an E cluster");
        let performance = plan.groups.iter().find(|group| group.name == "P cluster");
        let efficiency = plan.groups.iter().find(|group| group.name == "E cluster");
        assert_eq!(performance.map(|group| group.blocks.len()), Some(8));
        assert_eq!(efficiency.map(|group| group.blocks.len()), Some(2));
        assert_eq!(plan.count(Role::Performance), 8);
        assert_eq!(plan.count(Role::Efficiency), 2);
        assert!(plan.caption.contains("8P + 2E clusters"));
        assert_sane(&plan);
    }

    #[test]
    fn the_floorplan_is_a_function_of_the_topology_alone() {
        let once = Floorplan::of(&hybrid(6, 2, true));
        assert_eq!(once, Floorplan::of(&hybrid(6, 2, true)));
        assert_ne!(once, Floorplan::of(&hybrid(4, 2, true)));
        // Only the machine's first package is drawn, and the caption says so.
        let mut two = hybrid(4, 1, true);
        let mut second = two.clone();
        for place in &mut second.cpus {
            place.package = Some(1);
            place.cpu += 100;
            place.siblings = place.siblings.iter().map(|cpu| cpu + 100).collect();
            place.l2 = place.l2.take().map(|l2| l2.iter().map(|cpu| cpu + 100).collect());
            place.l3 = place.l3.take().map(|l3| l3.iter().map(|cpu| cpu + 100).collect());
        }
        two.cpus.extend(second.cpus);
        let plan = Floorplan::of(&two);
        assert_eq!(plan.blocks.len(), 8, "four P-cores and four E-cores of package 0");
        assert_eq!(plan.packages, 2);
        assert!(plan.caption.contains("one of 2 packages"));
    }
}
