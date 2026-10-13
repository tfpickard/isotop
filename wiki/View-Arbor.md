# View Arbor

The process tree drawn as a tree. PID 1 is the trunk and every other process is a branch on its parent; a branch is as thick as the memory of the whole subtree it carries (Leonardo's rule), leaves are threads, green when idle and shading through yellow to red as they work, blossoms open on processes that have just started, leaves drift down when threads exit, the canopy sways in a wind set by CPU pressure, and the kernel threads grow as roots under a cut-away slab of soil. Where a branch leaves its parent is fixed by a phyllotaxis slot, so nothing in the geometry depends on CPU, memory or how many processes there are.

![Arbor: a tree of processes on a slab of soil, leaves coloured by thread CPU, pale roots below the grass](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/arbor.webp)

*The README image of the view (`docs/media/arbor.webp`), rendered from the demo workload.*

## The system being modelled

**The process tree.** Every Unix process except the first has a parent, so the relation is a tree rooted at PID 1 (`init` or `systemd`; `launchd` on macOS). A process is made by `fork`, may replace its program with `exec`, and ends with `exit`; its parent collects the exit status with `wait`, and until it does the dead child is a *zombie* (state `Z`). If a parent dies first, its children are re-parented to PID 1 or to the nearest ancestor that declared itself a subreaper. Linux has a second tree under PID 2, `kthreadd`, the parent of the kernel threads. Threads hang on each process as a flat set, so the tree is a tree of processes, with the threads as a canopy on each twig. `/proc/<pid>/stat` carries the parent's pid; on macOS the parent comes from libproc.

**Why a tree is a good picture of it.** A plant is the same kind of object: a structure built by repeated budding, in which what was made first stays where it is and new parts attach to old ones. Two properties transfer almost unchanged. New growth does not move old growth (a branch never relocates when a twig appears), which is the project's stable-layout invariant ([Theory Stable Layout](Theory-Stable-Layout)). And a plant has a conservation law of its own, the one that ties the thickness of a branch to what it supports (see Theory below).

## History

The longer account lives in the repository: [docs/theory/arbor.md](https://github.com/tfpickard/isotop/blob/main/docs/theory/arbor.md).

**Trees are a habit, not a family.** A tree is a growth form, a plant that stands on its own wood, and plants invented it several separate times. The first tall plants were small: *Cooksonia* and its relatives, a few centimetres of bare forked stem, about 425 million years ago. *Wattieza*, whose stumps were found in the Gilboa quarry in New York State, is the oldest tree known in the fossil record, about 385 million years old: a hollow-trunked cladoxylopsid with a crown of frond-like branches and a shallow root system. Reports differ on how much older it is than the next tree, by roughly 15 to 25 million years.

**Archaeopteris** followed, and it is the ancestor of what the word usually means: true wood from secondary growth, branching in all directions, flat leaves and deep roots, forming forests of trees some tens of metres tall by the end of the Devonian. One hypothesis is that its roots, breaking rock and drawing down carbon dioxide, helped cool the Late Devonian climate. It remains a hypothesis. The Carboniferous coal forests (about 360 to 300 million years ago) were giant club-mosses and horsetail trees; a popular explanation of why they became coal, that nothing yet could rot lignin, was proposed in 2012 and has since been disputed.

**Branching rules.** Leonardo da Vinci's notebooks state that when a tree divides, the thickness of the daughters taken together equals the thickness of the parent. In 1964 Shinozaki and colleagues proposed the pipe model: a tree is a bundle of unit pipes, one per leaf-bearing twig, so a branch's cross-section tracks the leaf area it carries. In 2011 Christophe Eloy showed that the same rule follows from a different principle, equal risk of wind damage at every scale. The spiral arrangement of leaves and branches, *phyllotaxis*, was a puzzle for centuries: Karl Schimper and Alexander Braun found its Fibonacci regularities in the 1830s, the Bravais brothers measured the divergence angle near 137.5 degrees in 1837, and in 1992 Stephane Douady and Yves Couder reproduced the pattern with magnetized droplets, showing that it emerges when each new bud is placed where the earlier ones leave the most room.

**The tree as an idea.** Darwin sketched a branching tree in a notebook in 1837 beside the words "I think", and Haeckel drew his oaks of life in 1866. The process tree is the same figure with one difference: a process can die before its children, which are then adopted by PID 1, a sort of grafting. The only dated cultivation record on this page is the 1994 discovery of *Wollemia*, a conifer known only from fossils, alive in a gorge outside Sydney; no process has been reported to do that.

## Theory

### Tidy trees, and why isotop does not use them

The standard way to draw a rooted tree is the Reingold-Tilford algorithm (1981), which places subtrees as compact contours with parents centred over their children; Walker (1990) made it linear for general trees. These layouts optimise compactness, and an optimum moves when the input changes: insert a child and its siblings, cousins and whole subtrees shift. isotop requires stable layouts, so Arbor lays the tree out by growth rules instead of optimisation: each parent hands its children slots on a fixed pattern in the order it first sees them, and a placed child never moves.

### Phyllotaxis and the golden angle

Place the $n$ slots round a stem at azimuths $\{n\alpha \bmod 360^\circ\}$. If $\alpha/360^\circ$ is a rational $p/q$, the slots collapse onto $q$ spokes and the gaps never close. For an irrational $\alpha$ the three-gap theorem says $n$ points cut the circle into gaps of at most three lengths, and the gaps are most even when $\alpha/360^\circ$ has the most slowly converging continued fraction, $[0; 2, 1, 1, 1, \ldots]$: the golden angle

$$
\alpha = 360^\circ\left(1 - \frac{1}{\varphi}\right) = 137.5078\ldots^\circ, \qquad \varphi = \frac{1+\sqrt5}{2}.
$$

Vogel (1979) generated sunflower heads with $r = c\sqrt{n}$, $\theta = n\alpha$, and Douady and Couder (1992) showed the angle emerges from a physical growth process.

![Placing each new organ a fixed angle from the last: only near the golden angle do the points spread evenly](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/theory/figures/arbor-phyllotaxis.svg)

*Points at three fixed angles, radius proportional to the square root of the index. A rational angle leaves spokes. From the repository's theory page.*

The code uses the literal 137.5 degrees (`SLOT_ANGLE`, whose doc comment calls it the golden angle), not `layout::GOLDEN_ANGLE` (137.50783 degrees). With 137.5 exactly, $137.5 \times 144 = 19800 = 55 \times 360$, so the azimuth of slot 144 equals that of slot 0 and every later slot repeats one 144 slots earlier. The branches still leave at different points along the parent (`attachment`), so they do not coincide; but a parent with more than 144 children, such as `kthreadd` on a real machine, has azimuths that repeat.

### The pipe model and Leonardo's rule

At a branching, parent and child radii obey

$$
r_p^{2} = \sum_i r_i^{2},
$$

the conservation of cross-sectional area (Leonardo's rule). The botanical explanation is the pipe model (Shinozaki, Yoda, Hozumi and Kira, 1964): each unit of foliage is served by a unit pipe of constant cross-section, so the stem area at any point is the number of pipes it carries. A branch that splits into two equal halves has daughters of radius $r_p/\sqrt{2} \approx 0.707\,r_p$. Blood vessels follow a different exponent: Murray's law (1926), $r_p^3 = \sum r_i^3$, follows from minimising the viscous pumping power of laminar flow plus the metabolic cost of maintaining the blood volume. A tree built by Leonardo's rule can never satisfy Murray's law except for an unbranched stem, because the $\ell^p$ norm falls as $p$ rises: $\left(\sum r_i^3\right)^{1/3} \le \left(\sum r_i^2\right)^{1/2}$ for $r_i \ge 0$.

### A damped branch in a gust

Each branch is a damped oscillator pulled toward the lean the gust asks for:

$$
\ddot{x} = \omega^{2}\,(x_\mathrm{target} - x) - 2\zeta\omega\,\dot{x}, \qquad \zeta = 0.12 .
$$

Here $\omega = 2\pi f$ is the branch's natural angular frequency in radians per second and $\zeta$ its damping ratio. With $\zeta < 1$ the branch is underdamped: a step overshoots by $e^{-\pi\zeta/\sqrt{1-\zeta^2}} = 68\,\%$ and rings, and the resonant amplification is $1/(2\zeta) = 4.2$. The frequency in hertz is a convention, not beam mechanics:

$$
f = \operatorname{clamp}\!\left(0.6\sqrt{r/0.18}\,\Big/\sqrt{L/6},\ 0.15,\ 3\right),
$$

with branch radius $r$ and length $L$ in world units. It rises with thickness and falls with length, the right trend. The first bending mode of a uniform cantilever scales as $r/L^{2}$, not $\sqrt{r/L}$, so the view claims the trend and not a tree's actual natural frequency. For the demo's trunk ($r = 1.6$, $L = 14$) the formula gives 1.17 Hz; for a twig of radius 0.05 and length $14 \times 0.84^{5} = 5.85$ it gives 0.32 Hz, since the thin twig is long compared with its radius, and the 3 Hz ceiling is reached only by short thick stubs. Semi-implicit Euler at a fixed step of 0.05 s is stable while $\omega\,\Delta t < 2$; at 3 Hz the product is $2\pi \cdot 3 \cdot 0.05 = 0.94$.

![Daughter radii after an even split under Leonardo's rule and Murray's law, and the step response of a damped oscillator for three damping ratios](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/theory/figures/arbor-rules.svg)

*Left: Leonardo (areas add) against Murray (cubes add). Right: step responses; $\zeta = 0.12$ is `DAMPING` in `src/arbor.rs`. From the repository's theory page.*

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Trunk | PID 1, as a process or as a pale shadow (`Snapshot.shadows`, launchd on macOS without privilege); without PID 1, the top-level node whose tree holds the most nodes (ties to the lowest key) | a node whose parent is not drawn, or that sits in a cycle, hangs on the trunk |
| Branch cross-section | subtree memory: `Process.memory` summed over the subtree (the `memory` field, resident bytes) | radius $r = 1.6\sqrt{A/A_\mathrm{trunk}}$ world units (`R_TRUNK`), floor 0.05 (`RADIUS_FLOOR`); along a branch the area steps down at each junction (`Tree::levels`), so Leonardo's rule holds at every junction |
| Where a branch leaves its parent | order in which the parent first saw its children (a counter that only rises, ties by `Identity.start` then pid) | slot $k$: azimuth $k \times 137.5^\circ$, attachment at $0.35 + 0.6(1 - 0.8^{k+1})$ of the parent's length (0.47 for slot 0, 0.79 for slot 5, 0.94 for slot 20) |
| Branch length and angle | depth in the tree | length $14 \times 0.84^{d}$ (11.76 at depth 1, 8.30 at depth 3), leaves its parent 66 degrees off the axis for the trunk's children and 52 degrees for deeper ones (`ELEVATION_TRUNK`, `ELEVATION`) |
| Roots | processes of `Kind::Kernel` that are top-level or direct children of the trunk, with their subtrees | length $8 \times 0.72^{d}$ downward; the area of a root is the number of processes in it (one each), radius $0.88\sqrt{\text{share}}$ (`ROOT_COLLAR`, 0.55 of the trunk radius) |
| Root glow | `Process.cpu` of the root process | `bounded(cpu, 6)`, a saturating $c/(c+6)$ |
| Leaf | one per thread: `Process.thread_list` (`Source::Threads`), else `Process.threads` leaves | at most 32 per twig (`LEAVES_PER_TWIG`) and 6,000 in all (`LEAF_CAP`); threads past the cap share a leaf by `tid` bucket, showing the bucket's mean CPU |
| Leaf colour | thread CPU as a fraction of one core; a thread with no reading shows the process's `cpu / threads` | stops at 0 (74, 150, 72), 0.08, 0.3 (214, 196, 60), 0.6, 1.0 (220, 52, 44), linear between (`leaf_color`) |
| Blossom | a process the scene first saw less than 5 s ago (`Stage::age`, `BLOSSOM_SECONDS`) | up to 6 per process, opening and closing with a smoothstep; processes present on the first frame never blossom; roots do not |
| Falling leaf | a thread of the previous sample that is gone, or every leaf of a process that exited or became a zombie | falls 4 s (`FALL_SECONDS`), rests on the soil 30 s (`REST_SECONDS`), at most 400 at once (`FALLEN_CAP`) |
| Grey twig | zombie (`state == 'Z'`) with no children | colour (128, 128, 128) |
| Pale branch | a parent isotop cannot read (`Snapshot.shadows`) | colour (214, 218, 238), not pickable; the legend adds ` \| pale branch = a parent isotop can't read (run with sudo)` |
| Sway | `Snapshot.pressure[0]`, CPU pressure (10 s average) | `bounded(pressure, 20)` eased with a 1.5 s time constant (`WIND_EASING`) into the oscillators of the section above; roots do not sway |
| Soil slab, grass, sky, bark texture, leaf shapes, curve of each branch | none | decorative |

## How isotop realises it

**Module and registry row.** `src/arbor.rs` holds `Arbor` (a `#[derive(Default)]` struct implementing `views::ViewModule`) and one row at the end of `views::REGISTRY`: `Backdrop::Dusk`, `pitch: Some(0.48)` (the camera looks across the canopy), `flash: false`, `link_strokes: true`, `sources: Sources::of(&[Source::Threads])` (threads are sampled only while the view is shown), no keys, no digit (digits pick only the first ten rows). Its demo options are `churn`, `threads`, `thread_churn` and `unreadable_parent`, and its headless warm-up plays 8 s of demo at a 0.05 s step, so blossoms, falling leaves and sway are steady in a PNG. There is no plugin system: the view is compiled in.

**The tree** (`Tree::build`). Nodes are built from parent links; the trunk is chosen as in the Mapping table; kernel processes under the trunk go to the `Side::Root` side. `Slots::seat` gives each fresh child the next number of its parent's counter in start-time order; a child already seated there keeps its slot, and the counter never falls when a child leaves. `Tree::grow` then fixes every branch's rest geometry from slot, parent and a per-node seed (`Rng::for_identity` with `SALT`): the direction from the elevation and the azimuth, a bend toward the sky (roots: toward the ground) of 0.18 to 0.48 plus a small wobble, so `point(s) = base + length (direction s + bend s^2)`. Topology and rest geometry are rebuilt when the sample changes; each frame runs only the sway and the projection.

**Thickness** (`Tree::weigh`, `Weights::radius`, `Tree::stops`). On the crown a node's area is its resident memory; among the roots it is 1. Subtree sums run leaves-first. A branch's radius at its base is $1.6\sqrt{A/A_\mathrm{trunk}}$; going out it steps down at each child's attachment point by that child's subtree area, ending at its own area at the tip, each step blended by a ramp of 5 % of the branch (`RAMP`), and children whose junctions lie within 2 % of each other share one step (`JUNCTION_MERGE`), so a parent of hundreds of children is a handful of pieces.

**Drawing.** A branch wider than 2 px is a `Frame::tube` with 4, 6 or 8 sides (by width: over 5 px, over 14 px), shaded by `render::strokes::lit`; thinner ones are `Frame::ribbon`s with a floor of 1.2 px, so the CPU and GPU backends rasterize the same triangles. Leaves are two-triangle diamonds picking the owning process; blossoms are spheres with a halo `Item::Puff`. The soil is a checkered grass top and a four-layer cut-away side (`SOIL_HALF` 20, `SOIL_DEPTH` 10). The camera is fitted to a fixed cylinder (`FIT_RADIUS` 22, `FIT_TOP` 32), not to the moving canopy.

**Leaf budget** (`twig_cap`, `sampled_twigs`). The per-twig cap starts at 32 and is lowered for every twig alike until the scene holds at most 6,000 leaves; when even one leaf per process would pass 6,000, the 6,000 processes with the lowest hash of identity keep theirs, which is stable from sample to sample.

**Wind and time.** The oscillators step on `simulation::FixedStep` (`STEP` 0.05 s, `MAX_STEPS` 10, `MAX_GAP` 0.5 s). Each gust is a steady push along a heading of 0.35 rad plus two incommensurate sines whose phase is seeded per branch, so no two branches move alike and the motion does not repeat; a child's base moves with its parent's displacement, weighted by the square of its attachment fraction (`Sway::bases`). A full gust leans a branch by 9 % of its length (`LEAN`).

**Stable layout.** `Arbor::anchors` (test only) returns each twig's tip at rest, which depends on the tree alone; the view harness checks that no process moves when CPU and memory scale or a tenth of the population turns over.

**Picking, text and tour.** Twig tips go to `stage.positions`; `stage.places` names the trunk with its total memory, up to four roots with their kernel-thread counts and the six heaviest branches (`LABELLED_BRANCHES`); `stage.notes` for a lit process holds the subtree memory, process and thread counts, leaves drawn, the slot and the three busiest threads as `tid name cpu%`, and all names pass `terminal::clean_text`. The tour visits the 16 heaviest branches and 3 heaviest roots (`TOUR_BRANCHES`, `TOUR_ROOTS`).

**Legend.** ` Trunk = PID 1 | branch area = subtree memory | leaves = threads: green idle, red busy | blossoms = started < 5 s | falling leaves = exited threads | roots = kernel threads | wind = CPU pressure N%`, with suffixes appended while they apply: the pale-branch note, ` | no kernel threads reported: no roots`, the `status::missing_note` wording for CPU pressure ("still air") and threads ("leaves use process CPU"), and the count of processes whose per-thread CPU was unreadable.

**Platforms.** Linux: full; without root, other users' processes may be unreadable and appear as pale branches. macOS: the tree, areas, thread-count leaves, blossoms and falls work from libproc; without privilege `launchd` is the pale trunk. Where a platform reports no kernel threads or no CPU pressure, the legend says `no kernel threads reported: no roots` or `still air`; which of these applies on a given Mac was not run here (unverified).

## Reading the picture

* **Measured:** the process tree (who spawned whom, from the parent pid), memory per process, thread counts and per-thread CPU where the source is read, which processes are new (first seen), zombies, and CPU pressure.
* **Derived:** branch thickness (a sum over the subtree, so it is the subtree's memory, not the branch's own), slot positions (a deterministic function of the order in which children were first seen, not of anything about the processes), and the sway phase (seeded).
* **Decorative:** the soil, sky, grass, the exact curve of each branch, leaf shapes.
* **A thick branch is a heavy subtree, not a heavy process.** A thin shell that spawned a huge compile is thick below it. The inspector's subtree memory and the twig's own figures separate the two.
* **Cannot infer:** a process's age from its position (slot order is the order the scene saw children, apart from ties); the order of `exec`; CPU from branch colour (the colour is on the leaves, that is on threads).
* **Roots are counts, not bytes.** Kernel threads have no resident memory, so root thickness is the number of kernel processes in the subtree, in a different unit from the canopy.
* **Resolution.** With 192 processes most leaves are a few pixels. Above the leaf cap leaves stop being threads one to one, and the inspector's "threads, leaves" line shows the difference.

## Try it

Commands run with the release binary on Linux (x86_64, CPU renderer), repository root:

```sh
./target/release/isotop --list-views
./target/release/isotop --demo --view arbor --time 12 --width 1600 --output /tmp/arbor.png --renderer cpu
```

Output of the second command here: `Wrote /tmp/arbor.png (1600x900, 216 visible)`. The first lists eighteen views, `arbor` last (the order of `views::REGISTRY`). Live: `./target/release/isotop --view arbor`, or Tab and the `v` picker. `c` cycles the link mode, which draws sockets as hairlines between twig tips. `Space` visits processes in turn. `pstree -p 1` shows the real tree the demo imitates (not run here).

## Where in the code

| Item | Path and symbol |
|---|---|
| The view | [src/arbor.rs](https://github.com/tfpickard/isotop/blob/main/src/arbor.rs): `Arbor`, `Tree::build`, `Tree::grow`, `Slots`, `attachment`, `Weights`, `Tree::levels`, `leaf_specs`, `twig_cap`, `leaf_color`, `Sway`, `Oscillator`, `natural_frequency` |
| Registry row | [src/views.rs](https://github.com/tfpickard/isotop/blob/main/src/views.rs): `REGISTRY` row `arbor` |
| Strokes | [src/render/strokes.rs](https://github.com/tfpickard/isotop/blob/main/src/render/strokes.rs): `Frame::tube`, `Frame::ribbon` |
| Shared helpers | `simulation::FixedStep`, `Rng::for_identity`, `layout::GOLDEN_ANGLE` (not used for the slots), `Stage::age`, `model::bounded` |
| Unreadable parents | `Snapshot.shadows` (`platform::Shadow`) in [src/platform/mod.rs](https://github.com/tfpickard/isotop/blob/main/src/platform/mod.rs) |
| Repository theory page | [docs/theory/arbor.md](https://github.com/tfpickard/isotop/blob/main/docs/theory/arbor.md) |
| Prototype (reference only) | [docs/mockups/code/arbor.py](https://github.com/tfpickard/isotop/blob/main/docs/mockups/code/arbor.py) |

## Variations and open questions

* The design in the roadmap used lengths of $14 \times 0.78^{d}$ and elevations of 50 and 40 degrees; the implementation uses 0.84 and 66 and 52 degrees, which gives a broader, more open crown. The figure below was computed for the earlier numbers: its panel (a) (azimuth gaps) applies unchanged, panel (b) does not.
* The 145th child of a parent has the azimuth of the first, because `SLOT_ANGLE` is 137.5 degrees exactly. Using `layout::GOLDEN_ANGLE` removes the repeat; whether it changes the look of `kthreadd`'s roots is untried.
* Replace the slot order "first seen" by the true start order when the whole tree is present, and see whether stability survives churn.
* A Murray-style root system: radius by exponent 3 for the roots (kernel threads as a fluid network) and 2 for the canopy; the inequality above predicts thinner roots.
* Use `Snapshot.life` events for blossoms instead of first-seen age, so fork and `exec` storms show with exact times.
* Replace the convention frequency by the cantilever scaling $r/L^{2}$ and compare how the sway reads.
* A leaf that falls is not a grave. A process that exits drops all its leaves here and is buried, on its own terms, in [View Necropolis](View-Necropolis); the two views disagree only about which part of it was worth keeping.

![Azimuth gaps, a synthetic tree built to the earlier specification, and child to parent radius ratios](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/arbor-skeleton.svg)

*`docs/figures/arbor-skeleton.py`. (a) Largest azimuth gap against slots used for four turn angles. (b) A seeded synthetic tree with the design's earlier branch geometry (not the constants above). (c) Child to parent radius ratios. The tree is a stand-in, not a machine.*

## References

* E. M. Reingold and J. S. Tilford, "Tidier drawings of trees", *IEEE Transactions on Software Engineering* SE-7(2) (1981) 223-228; J. Q. Walker II, "A node-positioning algorithm for general trees", *Software: Practice and Experience* 20 (1990) 685-705.
* H. Vogel, "A better way to construct the sunflower head", *Mathematical Biosciences* 44 (1979) 179-182; S. Douady and Y. Couder, "Phyllotaxis as a physical self-organized growth process", *Physical Review Letters* 68 (1992) 2098-2101.
* K. Shinozaki, K. Yoda, K. Hozumi and T. Kira, "A quantitative analysis of plant form: the pipe model theory", *Japanese Journal of Ecology* 14 (1964) 97-105 and 133-139.
* C. Eloy, "Leonardo's rule, self-similarity, and wind-induced stresses in trees", *Physical Review Letters* 107 (2011) 258101.
* C. D. Murray, "The physiological principle of minimum work: I. The vascular system and the cost of blood volume", *Proceedings of the National Academy of Sciences* 12 (1926) 207-214.
* W. E. Stein et al., "Giant cladoxylopsid trees resolve the enigma of the Earth's earliest forest stumps at Gilboa", *Nature* 446 (2007) 904; B. Meyer-Berthaud, S. E. Scheckler and J. Wendt, "*Archaeopteris* is the earliest known modern tree", *Nature* 398 (1999) 700.
* P. Prusinkiewicz and A. Lindenmayer, *The Algorithmic Beauty of Plants* (Springer, 1990).
* See also [Theory Stable Layout](Theory-Stable-Layout), [Theory Life Events](Theory-Life-Events), [Theory Text Safety](Theory-Text-Safety), [Math Spatial Hashing](Math-Spatial-Hashing).
