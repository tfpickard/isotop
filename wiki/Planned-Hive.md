> **Planned, not yet implemented.** Specified in [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md) and tracked in issue [#46](https://github.com/tfpickard/isotop/issues/46).

# Planned: Hive

The `hive` view is specified as an apiary: one wooden hive per cgroup, with frames pulled up and standing behind it, one frame per process. Each frame is a honeycomb with an amber honey arch along the top, a band of orange pollen, a pearly brood nest in the middle and wax-capped cells; one cell stands for a fixed amount of proportional set size (PSS), and the cells are coloured by *kind* of memory: honey is anonymous memory the process made itself, pollen is file-backed memory gathered from outside, royal jelly is shared memory, capped honey is memory sealed away in swap, and brood is memory that has just grown. Bees crawl on the frames, one per thread, scurrying at a speed set by that thread's CPU; one larger bee per frame is the thread group leader, the queen. This page is the geometry of the honeycomb (including the exact statement of Hales's proof of the honeycomb conjecture), the Linux memory accounting that supplies the counts, and then the design as written in issue #46. Nothing here is implemented: `isotop --list-views` does not list `hive`.

![Reference mockup of the hive view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/hive.jpg)

*The reference mockup (`docs/mockups/hive.jpg`), rendered in Python by `docs/mockups/code/hive.py` from synthetic data. It is the visual target, not an isotop render.*

## The system being modelled

A colony of honey bees (*Apis mellifera*) lives in wax comb, and a beekeeper's **Langstroth hive** (L. L. Langstroth's movable-frame hive, 1852) holds the comb in removable wooden frames hung side by side in boxes: a frame can be lifted out and examined, which is exactly what the view does with a process. A full brood frame has a recognisable arrangement. The queen lays eggs in the **brood nest** in the middle of the frame; around it is a band of **pollen**, the bees' protein, stored in cells of the colours of the flowers it came from; above the nest is an arch of **honey**, with the cells at the top, the cells the bees are finished with, **capped** with a wax lid. The brood itself is capped too while the larva pupates. Worker cells are about 5 mm across, and the whole arrangement is a standard photograph in beekeeping manuals: the nest warm and central, the stores on the cold edge.

Three things from the hive reappear in the view as structure, not as data: the **cells** (a fixed, countable unit, here a quantum of memory), the **frames** (the bounded container of a single process's share), and the **bees** (individual actors, here threads, whose liveliness shows how busy the frame is). The metaphor is good for memory for a simple reason: memory is counted in discrete pages, accounted by kind, and shared between processes in a way that a frame of comb makes visible. It is a metaphor: a cell is not a page, and the placement of honey above brood is a convention of the beekeeper's picture that the view borrows (see [Reading the picture](#reading-the-picture)).

## History

Beekeepers had long kept bees in hives from which the comb could not be removed whole without destroying it. The movable-frame hive was reached more than once. François Huber designed a hive of hinged frames in 1789 (the "leaf" or "book" hive), and Johann Dzierzon's work in Germany preceded Langstroth's. The Reverend Lorenzo Langstroth, in Pennsylvania, found in 1851 that bees fill a gap smaller than about 6.4 mm (a quarter inch) with propolis and a gap larger than about 9.5 mm (three eighths of an inch) with comb, so that a space between the two is left alone; the term "bee space" came later. Hanging each frame with a bee space around it made the frame removable without tearing the colony, and his US Patent 9300 of 5 October 1852 survived many attempts to challenge it on grounds of prior art. The frame the view draws is his.

The bees' own communication was worked out in the twentieth century. Karl von Frisch showed that a forager returning to the hive dances on the comb, and that the dance carries the direction and distance of the food; his *Die Tänze der Bienen* appeared in 1946. He shared the 1973 Nobel Prize in Physiology or Medicine with Konrad Lorenz and Nikolaas Tinbergen. The citation speaks of discoveries concerning the organisation and elicitation of individual and social behaviour patterns; the dance is not named in it.

The dance language was disputed. From 1967 Adrian Wenner argued that recruited bees find food by odour alone and not by reading a dance. The dispute is usually said to have settled by about the time of the Nobel Prize, and in 2005 Riley and colleagues followed individual recruited bees with harmonic radar and published in *Nature* that bees read the dance and also compensate for wind drift on the way. Tania Munz's account of the argument, *The Dancing Bees* (University of Chicago Press, 2016) (publication details unverified), argues that von Frisch's interpretation largely held.

The geometry of the comb has a separate history, told in the section above: the hexagon, D'Arcy Thompson's argument from foam, and the proof of the honeycomb conjecture by Hales, published in 2001.

## Theory

### Why hexagons: the honeycomb conjecture

Honey bees tile the plane with hexagonal cells. The observation that this uses the least wall for a given cell area is ancient: Pappus of Alexandria wrote in the fourth century that bees know that the hexagon holds more honey for the same material than the square or the triangle. Among the three regular polygons that tile the plane the claim is a short calculation. A regular $n$-gon of unit area has perimeter $P = 2\sqrt{n\tan(\pi/n)}$, which is 4.559 for the triangle, 4.000 for the square and 3.722 for the hexagon (and tends to the circle's $2\sqrt\pi = 3.545$, which does not tile). The left panel of the figure below plots it.

The **honeycomb conjecture** is the much stronger statement that no partition of the plane into cells of equal area does better than the regular hexagonal one, whatever the shapes of the cells and however their walls curve. It was proved by Thomas Hales in 2001 (*The honeycomb conjecture*, *Discrete & Computational Geometry* 25, 1-22). The statement, as I recall it (paraphrased, unverified in its exact wording): for a locally finite graph $\Gamma$ in the plane made of smooth curves, such that the complement has infinitely many bounded components all of unit area, the perimeter of $\Gamma$ per unit area, in the limit of large discs, is at least $\sqrt[4]{12} = 1.861$. A hexagon of unit area has perimeter 3.722 and each wall is shared by two cells, so each cell pays $3.722/2 = 1.861$: equality holds for the regular hexagonal tiling. Hales's proof has a periodic version on a torus and rests on a *hexagonal isoperimetric inequality*, a lower bound on the perimeter of a single cell in terms of its area and its number of sides, with a correction for the curvature of the walls. The equal-area condition matters: with cells of unequal areas the statement is not the same problem.

Why bees build hexagons is a separate question. D'Arcy Thompson argued in *On Growth and Form* (1917) that the geometry is mechanical, not computational: wax softened by the warmth of the bees' bodies behaves like a foam, and where three walls meet the surface tension pulls them to angles of 120 degrees (Plateau's rule for soap films), which is the angle of a hexagonal tiling. Whether the bees build hexagons directly or the wax flows into them, the mathematics of the result is Hales's.

![Wall per unit area of regular polygons, one frame of the specified comb layout, and the cell unit as the largest frame grows](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/hive-honeycomb-frame.svg)

*Left: perimeter of a unit-area regular polygon; yellow marks the three that tile the plane. Middle: one 64 x 32 frame filled with the arrangement of the prototype's `Comb.layout` (brood in the centre, pollen around it, royal jelly below, honey in an arch above, capped honey on its rim) from a made-up split of 1,160 cells; the colours are mine, not the specification's. Right: the cell unit rule with its hysteresis; see Design. Script: `docs/figures/hive-honeycomb-frame.py`.*

**Geometry used by the view.** In an offset ("brick") layout of pointy-topped hexagons with cell width $w$ (flat to flat), the columns are $w$ apart and the rows $\tfrac{\sqrt3}{2}w$ apart, every other row shifted by $w/2$; the hexagon's circumradius is $w/\sqrt3$ and its area $\tfrac{\sqrt3}{2}w^2 = 0.866\,w^2$. With rows of 64 and 63 cells alternately (as the prototype's `Comb` builds them) a 64 x 32 comb has 2,032 cells; with 64 cells in every row it would have 2,048.

### Memory accounting: RSS, PSS and the rollup

A process's **resident set size** (RSS) counts every resident page that is mapped, so a page shared by $n$ processes (a shared library, a shared memory segment, a copy-on-write page after `fork`) is counted in full in each of them, and summing RSS over processes counts shared pages many times. The **proportional set size** (PSS) divides each resident page by the number of processes that map it:

$$
\mathrm{PSS}_p = \sum_{\text{pages } q \text{ mapped by } p} \frac{1}{n_q},
$$

so that $\sum_p \mathrm{PSS}_p$ counts each mapped page exactly once. Linux exposes it per mapping in `/proc/<pid>/smaps` and, summed over the whole address space, in `/proc/<pid>/smaps_rollup` (Linux 4.14 and later). The rollup contains, among others:

| Field | Meaning |
|---|---|
| `Rss` | resident pages, each counted in full |
| `Pss` | the proportional share, defined above |
| `Pss_Anon`, `Pss_File`, `Pss_Shmem` | the PSS split by kind: anonymous memory (heap, stack), file-backed pages, and shared memory (`shmem`, `tmpfs`, `memfd`); the split needs Linux 5.3 or later (per the issue) |
| `Shared_Clean`, `Shared_Dirty`, `Private_Clean`, `Private_Dirty` | resident pages by sharing and by whether they differ from their backing store |
| `Swap`, `SwapPss` | pages swapped out, and their proportional share |

A frame's cells are $\mathrm{Pss\_Anon} + \mathrm{Pss\_File} + \mathrm{Pss\_Shmem} + \mathrm{SwapPss}$ divided by the cell unit. Because PSS is *relative to the other processes*, a process's frame can change without the process doing anything: when a second process starts sharing a library, the first one's `Pss_File` falls. Reading `smaps_rollup` walks the process's page tables, so its cost grows with the address space, and it is permitted only for processes the reader may ptrace (the same user, or with privilege); both facts shape the design: the rollup is read only for the 64 processes with the largest RSS, on the background thread, with a time budget, and an unreadable process is *sealed*, not drawn empty.

**The cell unit.** The scene uses one cell unit $u$, a power of two in bytes at least 64 KiB, chosen so that the largest drawn frame has at most about 2,000 cells. Each of the five components is rounded to the nearest cell independently, so a frame is off by at most half a cell per component, three cells in total, from its true byte count. When the largest frame grows the unit doubles, and every other frame's cell count halves; the specification keeps the unit while the largest frame lies between 700 and 2,000 cells, so it never flips back and forth (the right panel of the figure above).

### Bees: threads, queens and dances

In Linux every thread is a task with its own id (`tid`) and its own CPU time, and the thread whose `tid` equals the `pid` is the thread group leader. The view draws one bee per thread, with speed depending on the thread's CPU, and gives the leader a larger body and the queen's role. The role is positional: a real colony has exactly one queen and thousands of workers, whereas every process has one leader.

The bees' real communication is the **waggle dance**, decoded by Karl von Frisch (Nobel Prize 1973; *The Dance Language and Orientation of Bees*): a forager returning from food runs a figure of eight on the vertical comb, and the angle of the straight, waggling run from the vertical encodes the direction of the food relative to the sun's azimuth, and its duration encodes the distance, roughly proportional to it. The dance is a compact code for "where is the resource and how far", which is a good thing for a memory view to dream about, but nothing in the specification encodes information in the bees' motion; their positions are a random walk.

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Hive | the cgroup of a process, as named by `cells::cell_name` | at most 8 hives, ranked by the sum of `Process.memory` over all of the hive's processes; a shown hive is replaced only by a challenger 25 % larger |
| Frame | a process; the 3 largest by `Process.memory` in each hive | ties by `Identity`; a process keeps its slot while shown; the rest are counted in the hive's label (`+N more`) |
| One cell | a fixed amount of PSS | one scene-wide unit, a power of two of at least 64 KiB, such that the largest frame has at most 2,000 cells |
| Honey cells | `Pss_Anon` from `smaps_rollup` (building block B6) | bytes / unit, rounded to the nearest cell |
| Pollen cells | `Pss_File` | the same |
| Royal jelly cells | `Pss_Shmem` | the same |
| Capped honey cells | `SwapPss` | the same |
| Brood cells | growth of `Process.memory` since the previous sample | $\max(0,\ \mathrm{memory}_{\mathrm{now}} - \mathrm{memory}_{\mathrm{previous}})$ in cells, carved out of honey; reverts when growth stops |
| Unsplit frame | `Process.memory` (RSS; footprint on macOS) | one pale-nectar kind, used when no split is available |
| Sealed frame | `Measured::Unreadable` rollup | a uniform wax cap, no cells and no bees, labelled `sealed: smaps_rollup unreadable` |
| Bee | a thread (B7: `Process.thread_list`) | at most 200 per frame; overflow labelled `+N bees` from `Process.threads` |
| Bee speed | per-thread CPU | $0.25 + 9\sqrt{\mathrm{cpu\ fraction}}$ cells/s; a thread below 3 % stands still in a cluster beside the brood |
| Queen | the thread with `leader == true` (tid == pid) | 1.37 times larger, a mark in the kind colour, at most 0.6 cells/s |
| Where each kind sits in the frame | none | **convention**: brood in the centre, pollen around it, royal jelly below, honey above in an arch, capped honey on its rim; the counts are real, the arrangement is documented as a convention |
| Wood, lid, lift beam, wings, cell highlights, larva size (4 stages by rank in the brood order), bee positions | none | **decorative** |

## Design

Everything below is the specification of issue #46. None of it exists in the source.

**Module and registry.** A single new module `src/hive.rs` with a `#[derive(Default)] Hive` implementing `views::ViewModule`, plus one registry row, `ViewInfo::new("hive", "HIVE", Backdrop::Dusk, build)` with `build` a `views::Build` closure, whose `sources` are `Source::MemoryDetail` (B6) and `Source::Threads` (B7), so that both are read only while the hive is shown. The scene builds the view from its row and owns its state. There is no plugin system; the module is compiled in. There are no view-local keys and the hives are not pickable; the bee triangles pick their process.

**Dependencies.** Status in the current source tree:

| Item | Provides | Status |
|---|---|---|
| P1 (#17), P2 (#18), S1 (#22), S2 (#23), S6 (#27) | registry row, picker, parity check, `FixedStep`, source demand | merged |
| P4 (#20) | simulation utilities | merged |
| B6 (#33) | `Process.memory_detail` (`Measured<Rollup>`), `Source::MemoryDetail`, the `memory split` gap, demo (`DemoOptions::oom_and_memory_detail`) | merged |
| B7 (#34) | `Process.thread_list` (`Measured<ThreadList>`), `Source::Threads`, demo (`DemoOptions::threads`) | merged |
| P3 (#19) | field primitive with four-corner (vertical) placement (`Placement::Corners` in `src/render/field.rs`) | merged |

Every prerequisite is on `main`; the view itself is what is missing.

**Hives, frames, cells.**

* One hive per `cells::cell_name(process)` (`src/cells.rs`, currently private; the issue makes it `pub(crate)`), labelled with `cells::short`. Hives are placed by `layout::Discs` keyed by hive name (the issue writes `pack::Discs`).
* Frames per hive: the top 3 by `Process.memory`. Frame $j$ sits back by $0.5 + 0.4j$ and sideways by $0.1j$ hive units, lifted to the hive top plus $0.5 + 0.78j$, and measures $4.4 \times 2.04$ units (ported from the prototype's `frame_rect`).
* What a frame shows depends on the state of `Process.memory_detail`: a split rollup draws the comb by kind; a rollup without the split (kernels before 5.3) draws PSS as one pale-nectar kind plus `SwapPss` capped; an unreadable rollup draws the sealed frame; a pending rollup (outside the 64 largest, or not reached yet) draws the unsplit frame from `Process.memory`, a measured quantity and not a substitute; on macOS every frame is unsplit and the legend says ` | no memory detail: cells = memory footprint`.
* The comb is a fixed 64 x 32 offset hexagonal grid (the issue says 2,048 cells; a straight port of the prototype's alternating 64- and 63-cell rows gives 2,032, which is still above the 2,000 cells plus 3 cells of rounding the unit rule allows, so a frame never overflows).
* Unit rule: the target is the smallest power of two of at least 64 KiB with the largest drawn, non-sealed frame at most 2,000 cells; the current unit is kept while $700u \le \mathrm{largest} \le 2000u$, or while the unit is 64 KiB and the largest frame is below 700 cells.
* Brood is read from the module's `ViewModule::record`, which keeps `(previous, latest)` memory per `Identity` on every sample in every view, deduplicated on `snapshot.elapsed`, so brood is correct on the first frame after a view switch.
* The layout of kinds is a port of `Comb.layout`: brood centred at $(w/2,\ 0.58h)$ with $x$ scaled by $1/1.55$, then pollen around it, royal jelly below, then honey and capped honey in an arch metric (cells above the nest count as near, cells below as far); the priority order of each kind is precomputed once per comb with ties broken by cell index. I reproduced it in the middle panel above.

**Bees.** At most `min(200, Process.threads)` per frame; listed threads come first. Bees step through `FixedStep::new(0.05, 10, 0.5)` (20 Hz) with interpolation. A bee without a per-thread CPU reading (the first observation, threads beyond an incomplete list, pending or unreadable thread lists) is never given a speed: it stands in the idle cluster and the legend counts it. Randomness is one `const SALT: u64` with `Rng::for_identity(process.id, SALT ^ key)`, where the key hashes `tid` and `start` for a listed thread.

**Rendering.** Each comb is a field (P3) on a vertical four-corner quad at 8 texels per cell pitch, with the frame's process as owner, rebuilt only when its cell kinds or the unit change. A bee is at most 8 triangles with no glow; wings only when the bee is at least 6 px long. Hive bodies use the shaded cuboid helper (`geometry::cuboid`; the issue calls it `coop::cuboid`, which is where it was before it moved). The ground slab is made of quads. Backdrop `Dusk`. `stage.positions` holds the frame centres so death rings work; `stage.bounds` is the box around the shown hives, recomputed only when the shown set changes. Cost is bounded by 8 hives x 3 frames whatever the process count.

**Legend** (specified, in the conventions' order): ` 1 cell = <unit> | H hives (+X), F frames, U unsplit, C counted, B bees (S without thread CPU)`, then the `memory detail` and `memory split` notes, then the sources note, then ` | hive = cgroup | frame = process (top 3 by <MEMORY_LABEL>) | honey = Pss_Anon, pollen = Pss_File, royal jelly = Pss_Shmem, capped = SwapPss, brood = <MEMORY_LABEL> growth | bee = thread, speed = thread CPU, queen = leader`. The inspector notes list MiB per component, the thread count and the busiest thread's name (raw; the popup path cleans it).

**Headless warm-up and demo.** Samples at `--time - 2` and `--time - 1` go through the record hook, then the bee clock is stepped from `--time - 10` to `--time` without rasterizing. The demo takes B6's synthesized rollups (the 64 largest by RSS, one unreadable) and B7's threads; brood comes from the demo's existing memory oscillation. If no frame grows at `--time 12`, the implementer picks the nearest later reference time that has brood.

**Degradation.**

* Linux: full mapping; kernels before 5.3 draw PSS as one kind and show the `memory split` note.
* macOS: available, degraded. Hives group by app, every frame is unsplit and is never drawn sealed; brood works from footprint growth; per-thread CPU comes from B7 for readable processes; the bees of unreadable processes stand in the idle cluster and are counted.

**Out of scope** (per the issue): the brief's *shared combs* mode (the full `smaps` every 15 s, with one communal comb per shared library), pickable hives, view-local keys, and any change to B6's or B7's selection and caps. B6 reads rollups for the 64 largest processes by RSS within a 50 ms budget; B7 caps a pass at 8,192 threads and a process at 1,024 (both from the building-block issues).

## Reading the picture

* **Cell counts are measured; their arrangement is not.** The number of honey, pollen, jelly, capped and brood cells in a frame follows the rollup (within half a cell per kind). Where those cells sit follows the beekeeper's convention. A honey cell at the top of the arch is no older or newer than one at its edge.
* **A unit change changes every frame at once.** When the largest frame outgrows 2,000 cells the unit doubles and every comb halves its cell count; the legend shows the unit, and one cell is only comparable between frames within one scene.
* **PSS is relative.** A frame can change because *another* process started or stopped sharing a library. Honey (`Pss_Anon`) is the least relative: anonymous pages are mostly private; pollen (`Pss_File`) and royal jelly (`Pss_Shmem`) are the most.
* **Brood is growth, not birth.** It is `max(0, RSS_now - RSS_before)` between two samples: resident memory that is new. It does not say the growth is permanent, and it reverts to honey when growth stops.
* **Unsplit, sealed and counted are three different things.** An unsplit frame has a measured total but no kinds; a sealed frame could not be read at all (typically another user's process); a counted process is in a shown hive but outside its top 3.
* **Bees are threads, capped at 200.** A process with 1,000 threads shows 200 and a label for the other 800. Idle bees stand in a cluster, and so do bees whose CPU is not known; the legend counts the latter.
* **The queen is the leader**, not a unique entity: every process has one.

## Try it

None of these work yet: `hive` is not registered.

```sh
# Once the view lands (issue #46).
./target/release/isotop --demo --view hive --time 12 --width 1600 --output /tmp/hive.png
./target/release/isotop --demo --view hive --renderer cpu --width 1920 --benchmark 120
```

The figure regenerates from the repository root with `python3 docs/figures/hive-honeycomb-frame.py`. On a live Linux machine, the quantities the view will draw can be read by hand from `/proc/<pid>/smaps_rollup`.

## Where in the code

* Existing: `src/views.rs` (`REGISTRY`, `ViewModule`), `src/cells.rs` (`cell_name`, `short`, currently private), `src/layout.rs` (`Discs`), `src/geometry.rs` (`cuboid`), `src/simulation.rs` (`FixedStep`, `Rng::for_identity`), `src/render.rs` (`Stage`), `src/model.rs` (`Process.memory`, `Process.threads`, `Process.cgroup`), `src/platform/mod.rs` (`MEMORY_LABEL`, `GROUP_LABEL`).
* Also existing, which the design uses: B6 and B7 (`Process.memory_detail`, `Process.thread_list`, `Source::MemoryDetail`, `Source::Threads`) and `src/render/field.rs` (P3).
* Planned: `src/hive.rs` and a registry row.
* Prototype, reference only: `docs/mockups/code/hive.py` (`Comb.layout`, `draw_cell`, `draw_bee`, `frame_rect`, `frame_geometry`).

## Variations and open questions

* **Shared combs.** The brief's optional mode: resolve `Pss_File` by backing file from `/proc/<pid>/smaps` every 15 s, so each shared library becomes a communal comb whose cells are coloured by the processes sharing it.
* **A waggle dance.** Let a thread that migrates between CPUs or NUMA nodes dance: the angle for the direction of the node, the duration for the distance. The data (`Process.core`, per-thread CPU) partly exists.
* **The same cgroup twice.** [View-Coop](View-Coop) draws a cgroup as a henhouse of hens; this view draws it as a hive of bees. The two contain the same processes and different animals, and neither page takes a position.
* **Swarming.** A fork storm (a container starting) is the colony reproducing; a new hive could be founded by a swarm of bees leaving an old one, using the life events of building block B3.
* **A temperature.** A brood nest is kept warm; building block B12 (thermal sensors) could tint the frames of a cgroup by the temperature of the cores it runs on.
* **Cell unit as information.** The cell count is a quantised measurement; each component loses at most half a cell. A per-frame display of the rounding error would show how much of the picture is quantisation.
* **The 2,032 versus 2,048 question** (see Design): decide whether the comb is rectangular, and keep the unit-rule bound consistent with it.

## References

* T. C. Hales, "The honeycomb conjecture", *Discrete & Computational Geometry* 25, 1-22 (2001).
* D. W. Thompson, *On Growth and Form*, Cambridge University Press (1917).
* Pappus of Alexandria, *Synagoge (Collection)*, Book V (fourth century).
* K. von Frisch, *The Dance Language and Orientation of Bees*, Harvard University Press (1967).
* L. L. Langstroth, *A Practical Treatise on the Hive and the Honey-Bee* (1853).
* The Linux kernel documentation, `Documentation/filesystems/proc.rst`: `/proc/<pid>/smaps` and `smaps_rollup`.
