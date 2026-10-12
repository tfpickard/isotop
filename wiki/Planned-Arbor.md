# Planned: Arbor

> **Planned, not yet implemented.** Specified in [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md) and tracked in issue [#54](https://github.com/tfpickard/isotop/issues/54).

The process tree drawn as a tree: PID 1 is the trunk, its descendants are the branches, and a branch is thick where its subtree uses a lot of memory. Leaves are threads, green when idle and shading through yellow to red when busy; blossoms burst on branch tips when processes start; leaves drift down when threads exit; the canopy sways in a wind set by CPU pressure; and below the soil the kernel threads of `kthreadd` are the root system. This page is the design, the botany that its geometry borrows (phyllotaxis, the pipe model, Leonardo's rule), and a worked example on a seeded synthetic tree that checks what the rules imply.

![Reference mockup of the arbor view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/arbor.jpg)

*`docs/mockups/arbor.jpg`, from the prototype `docs/mockups/code/arbor.py` (a mocked process tree with named processes). A visual target, not an isotop render.*

## 1. The system being modelled

### 1.1 The process tree

Every Unix process except the first has a parent; the relation is a tree rooted at PID 1 (`init`, or `systemd`). A process is created by `fork`, may replace its program with `exec`, and ends with `exit`; its parent collects its exit status with `wait`, and until it does the dead child is a **zombie** (state `Z`). If a parent dies first its children are re-parented, to PID 1 or to the nearest ancestor that declared itself a *subreaper*. Linux has a second tree under PID 2, `kthreadd`, the parent of the kernel threads; `/proc/<pid>/stat` carries the parent's pid. A process also has threads; the tree is a tree of processes, and threads hang on each process as a flat set. So a tree of processes is a *development*: a record of what spawned what, grown over time.

### 1.2 Why a tree is a good picture of it

A plant is the same kind of object: a structure built by repeated budding, in which what was made first stays where it is and new parts attach to old ones. Two real properties transfer almost without change. **New growth does not move old growth** (a branch never relocates when a new twig appears), which is exactly the project's stable-layout invariant. And a plant has a conservation law of its own, the one that ties branch thickness to what the branch supports (section 2.3).

## 2. Theory

### 2.1 Tidy trees, and why isotop does not use them

The standard way to draw a rooted tree is the **Reingold-Tilford** algorithm (1981, "Tidier drawings of trees", *IEEE Transactions on Software Engineering* SE-7), which places subtrees as compact contours with parents centred over their children; Walker (1990) made it linear in time for general trees. These layouts are optimised for compactness, and an optimum moves when the input changes: insert a child and its siblings, cousins and the whole subtree to the right shift. isotop requires **layouts to be stable** ([Theory-Stable-Layout](Theory-Stable-Layout)), so the specified view lays the tree out by *growth rules* instead of optimisation: each parent hands its children slots on a fixed pattern in the order it first sees them, and a placed child never moves.

### 2.2 Phyllotaxis and the golden angle

Leaves, scales and florets are arranged around a stem at a nearly constant angle between consecutive ones, and for a large fraction of plants that angle is the **golden angle** $360^\circ(1 - 1/\varphi) = 360^\circ/\varphi^2 = 137.5078\ldots^\circ$, $\varphi = (1+\sqrt5)/2$. Vogel (1979, "A better way to construct the sunflower head", *Mathematical Biosciences* 44) generated sunflower heads with $r = c\sqrt n$, $\theta = n \cdot 137.508^\circ$; Douady and Couder (1992, *Physical Review Letters* 68) showed the angle emerges from a physical growth process where each new primordium appears where the space is largest.

Why that angle? Place the $n$ slots at azimuths $\{n\alpha \bmod 360^\circ\}$. If $\alpha/360^\circ$ is a rational $p/q$ the slots collapse onto $q$ spokes and the gaps never close: 120 degrees gives three spokes forever, 144 degrees five. For an irrational $\alpha$ the three-gap (Steinhaus) theorem says $n$ points always cut the circle into gaps of at most three distinct lengths, and the gaps are most uniform when $\alpha/360^\circ$ has the most slowly converging continued fraction, $[0; 2, 1, 1, 1, \ldots]$, whose partial quotients are all 1: the golden angle is the "most irrational" turn. The figure shows the largest gap after $n$ slots: the golden turn closes it steadily (20.1 degrees at 21 slots, 7.7 at 55, 2.9 at 144, within 18 % of the ideal $360/n$ at those Fibonacci numbers), while 120 and 144 degree turns stall at 120 and 72.

A detail of the specification. The issue writes **137.5 degrees**; the repository's constant is `layout::GOLDEN_ANGLE = 2.399963` radians = 137.50783 degrees. With a literal 137.5 the azimuths repeat exactly after 144 slots ($137.5 \times 144 = 19800 = 55 \times 360$), so the 145th child of a parent would share an azimuth with the first (their attachment points still differ). The largest gap through 144 slots is nearly the same (2.5 against 2.9 degrees). `kthreadd` has hundreds of children in the mockup (173 kernel threads), so the choice is not purely academic; using the existing constant avoids it.

![Azimuth gaps, the specified skeleton on a synthetic tree, and child to parent radius ratios](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/arbor-skeleton.svg)

*`docs/figures/arbor-skeleton.py`. (a) Largest azimuth gap against slots used for four turn angles. (b) The issue's branch geometry on a seeded synthetic tree: child $k$ of a parent at azimuth $k \cdot 137.508^\circ$, elevation 50 degrees for the trunk's children and 40 degrees deeper, attachment at $0.35 + 0.6(1 - 0.8^{k+1})$ of the parent's length, length $14 \cdot 0.78^{\text{depth}}$, radius by Leonardo's rule; leaves, wind and roots omitted. The tree is a synthetic stand-in, not a machine. (c) Child-to-parent radius ratios.*

### 2.3 The pipe model and Leonardo's rule

Leonardo da Vinci noted in his notebooks that the branches of a tree at any height, taken together, are as thick as the trunk (stated as a paraphrase here). The quantitative form is that at a branching, parent diameter $d_p$ and child diameters $d_i$ obey

$$
d_p^{\alpha} = \sum_i d_i^{\alpha}, \qquad \alpha \approx 2 ,
$$

which says the **cross-sectional area is conserved**. The botanical explanation is the **pipe model** (Shinozaki, Yoda, Hozumi and Kira, 1964, *Japanese Journal of Ecology* 14): every unit of leaf is served by a "unit pipe" of constant cross-section running to the root, so the area of the stem at any point equals the number of pipes it carries, which is proportional to the foliage above it. Eloy (2011, *Physical Review Letters* 107, 258101) showed that the same rule follows from requiring a tree to resist the wind equally at every scale (self-similar wind-induced stress), which is why the brief says the wind in the view is a real one.

Blood vessels follow a different exponent. **Murray's law** (1926, *Proceedings of the National Academy of Sciences* 12) is $r_p^3 = \sum r_i^3$. It follows from minimising the sum of the viscous power cost of laminar flow and the metabolic cost of maintaining blood volume: by Hagen-Poiseuille the pumping power is $P_f = 8\mu L Q^2/(\pi r^4)$, the maintenance cost is $P_m = b\,\pi r^2 L$, and $dP/dr = 0$ gives $-32\mu Q^2/(\pi r^5) + 2 b \pi r = 0$, so $Q \propto r^3$; flow conservation $Q_p = \sum Q_i$ then gives the cube law. Area conservation (exponent 2) belongs to the plant, flow-optimised cost (exponent 3) to the vessel.

A consequence for the design, which I checked on the synthetic tree: **a tree built by Leonardo's rule can never satisfy Murray's law** except for an unbranched stem, because the $\ell^p$ norm decreases in $p$: $\left(\sum r_i^3\right)^{1/3} \le \left(\sum r_i^2\right)^{1/2}$ whenever $r_i \ge 0$. On the 17 branching nodes of the synthetic tree $\sum r_i^3 / r_p^3$ has median 0.51 (range 0.20 to 0.85), never above 1.

### 2.4 Branching statistics of a seeded tree

The synthetic tree of the figure (192 processes, a stand-in for the demo's default count; each new process attaches to an earlier parent with probability proportional to $(\text{children}+1)^{1.2}\,0.72^{\text{depth}}$, memory lognormal with median 20 MiB; seeded) has 146 leaves, maximum depth 5, and two hubs with 45 and 64 children while the other 44 interior nodes have 1 to 7. The child-to-parent radius ratio $r_c/r_p = \sqrt{A_c/A_p}$ has a median of 0.145 and a mean of 0.27, with most branches under a quarter of their parent's radius (panel c) and the two symmetric-split reference values 0.707 ($2^{-1/2}$, Leonardo) and 0.794 ($2^{-1/3}$, Murray) lying in a sparse tail. The model is mine and a different generator would give different numbers; the point is that **a real process tree is a very unbalanced tree** (a few hubs with many children, most nodes leaves), so most of the drawing is trunk-like hubs with thin twigs, which the real mockup also shows.

### 2.5 The wind

Each branch is a 2-D damped oscillator, $\ddot x + 2\zeta\omega\dot x + \omega^2 x = F(t)$, with $\zeta = 0.12$ and a frequency in hertz set by the issue's convention

$$
f = \text{clamp}\!\left(0.6\sqrt{r/0.18}\,/\sqrt{L/6},\ 0.15,\ 3\right),
$$

for branch radius $r$ and length $L$ in world units: it **falls with length and rises with thickness**, as the brief says. For $\zeta = 0.12$ the resonant amplification is $1/(2\zeta) = 4.2$. This formula is a convention, not beam mechanics: the first bending mode of a uniform cantilever of radius $r$ and length $L$ is $f_1 = \frac{1.875^2}{2\pi}\frac{r}{2L^2}\sqrt{E/\rho}$, which scales as $r/L^2$ and not as $\sqrt{r/L}$. Both have the right trend; the view uses the milder one and does not claim to compute a tree's natural frequency. The wind strength is `bounded(pressure[0], 20)` $= p/(p+20)$ with $p$ the CPU pressure (0.5 at 20 % pressure, 0.83 at 100 %). With the specified fixed step $H = 0.05$ s and a highest frequency of 3 Hz, a semi-implicit Euler integrator would be stable ($\omega H = 0.94 < 2$) with a frequency error of about $(\omega H)^2/24 = 3.7\,\%$ and 6.7 steps per period; the issue does not name the integrator.

## 3. Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Trunk | PID 1; if absent, the root of the largest subtree (`init-0` in the demo) | a process whose parent is not drawn attaches to the trunk |
| Branch cross-section | subtree memory (`Process.memory` summed over the subtree, from the `processes` slice) | radius $r = 1.6\sqrt{A/A_{trunk}}$ world units, floor 0.05; past each junction the area is own memory plus the children attached further out |
| Branch direction | parent-child order and `Identity.start` | golden-angle slot $k$ per parent (counter only increases): azimuth $k \times$ golden angle, attachment $0.35 + 0.6(1-0.8^{k+1})$ of the parent's length, elevation 50 degrees (trunk's children) or 40 degrees, length $14 \cdot 0.78^{depth}$; no position depends on CPU, memory or population |
| Roots | `Kind::Kernel` processes that are top-level or direct children of the trunk, with their subtrees | kernel threads have no RSS, so a root's cross-section is proportional to its **kernel-thread count** |
| Leaf | one per thread (B7) | at most 32 per twig (above that bucketed by tid, each leaf showing the bucket's mean CPU); scene cap 6000; colour from the prototype's `leaf_colour` stops (green at 0, yellow near 0.3, red at 1.0 busy); without B7 data, `Process.threads` leaves shaded by `cpu / threads` |
| Blossom | a process first seen by the scene less than 5 s ago | new `Stage::age(id)`; processes present on the first frame never blossom |
| Falling leaf | an exited thread (a tid that disappeared) | falls 4 s, rests on the soil 30 s, at most 400 falling or fallen |
| Bare grey twig | zombie (`Z`) | `[128,128,128]`, no leaves |
| Pale branch | an unreadable parent (`Snapshot.shadows`) | own memory 0, not pickable; legend suffix ` \| pale branch = a parent isotop can't read (run with sudo)` |
| Sway | `Snapshot.pressure[0]` (CPU some avg10) | damped oscillator per branch, 2.5 above |
| Soil, sky, stars, grass | none | **decorative** |

## 4. Design: how isotop will realise it

* **Module.** `src/arbor.rs`, `#[derive(Default)] struct Arbor`, one registry row: `Backdrop::Dusk`, no view-local keys, no digit; the row declares B7 through S6 and turns on S4's demo births, exits and thread churn. `draw(&mut Stage, &[&Process], &Snapshot) -> usize`.
* **Topology and cost.** Topology and rest geometry are rebuilt only when `snapshot.elapsed` changes; each frame runs only sway and projection. Branches are S3 camera-facing tapered ribbons with a rim light, split so that no triangle's screen bounding box exceeds about 64 x 64 px; a branch under 1 px becomes a `Line`.
* **Slots.** Each parent keeps a counter that only increases and assigns slots in the order it first sees its children, ties broken by `Identity.start`; a reparented process takes a new slot on its new parent; existing children never move.
* **Wind.** `simulation::FixedStep` with $H = 0.05$ s and at most 10 steps per frame; each branch's gust phase is seeded from its process identity and a child inherits its parent's displacement.
* **Soil.** A cut-away slab; roots are drawn after it with their depth clamped to just in front of its front face, so the slab never hides them. Camera pitch limits do not change.
* **Picking and text.** Leaves are two pickable triangles each, picking the owning process; `positions` holds the twig tips; `places` the trunk, the roots and the 6 heaviest branches; `notes` the subtree memory, process and thread counts, threads against leaves, the slot and the 3 busiest threads as `tid name cpu%`. All names pass `terminal::clean_text`.
* **Legend.** ` Trunk = PID 1 | branch area = subtree memory | leaves = threads: green idle, red busy | blossoms = started < 5 s | falling leaves = exited threads | roots = kernel threads | wind = CPU pressure N%`, with the degradation suffixes appended while they apply.
* **Headless.** With `--output --demo`, replay the demo at 1 s steps over the 8 s before `--time`, rendering every 0.05 s, so blossoms, falling leaves and sway are steady.
* **Dependencies** (ROADMAP index): P1, P2, S1, B7, S2, S3, S4, S6. In the tree at commit `192299b`: P1, P2, S1, S2, S4 and S6 are merged; **B7 (per-thread CPU) and S3 (ribbons) are not**.
* **Platforms.** Linux: full; pale branches appear when isotop runs without root. macOS: **degraded**. The tree, branch areas, leaves from thread counts, blossoms and falling leaves all work; without sudo, `launchd` is the pale trunk. Each gap adds a legend suffix and a status-strip name in S6's wording: ` | no kernel threads reported: no roots`, ` | no CPU pressure: still air`, ` | per-thread CPU unreadable for N: leaves use process CPU`.
* **No plugin system.** Compile-time module and `REGISTRY` row.

## 5. Reading the picture

* **Measured:** the process tree (who spawned whom, from the parent pid), memory per process, thread counts and per-thread CPU where B7 reads it, which processes are new (first seen), zombies, and CPU pressure.
* **Derived:** branch thickness (a sum over the subtree, so it is the subtree's memory, not the branch's own), the slot positions (a deterministic function of the order children were first seen, **not** of anything about the processes), the sway phase (seeded).
* **Decorative:** the soil, sky and stars, the exact curve of each branch, leaf shapes.
* **A thick branch means a heavy subtree, not a heavy process:** a thin shell process that spawned a huge compile is thick below it. The inspector's subtree memory and the twig's own figures separate them.
* **Cannot infer:** a process's age from its position (slot order is the order the *scene* saw children, not their start times, except for ties); the order of exec; CPU from branch colour (colours are on leaves, i.e. threads, not branches).
* **Kernel roots are counts, not bytes:** kernel threads have no RSS, so root thickness is the number of kernel threads in the subtree, in a different unit from the canopy.
* **Resolution:** with 192 processes, 941 threads in the mockup, most leaves are a few pixels; above the leaf cap (6000, per-twig 32) leaves stop being threads one-to-one and the legend and inspector say so.

## 6. Try it

Nothing runs in isotop yet. When the view lands:

```sh
./target/release/isotop --demo --view arbor --time 12 --width 1600 --output /tmp/arbor.png --renderer cpu
```

(Tab and the `v` picker reach it.) You can see the real tree it would draw with `pstree -p 1`, and regenerate the page's figure and statistics with `python3 docs/figures/arbor-skeleton.py`.

## 7. Where in the code

Existing: `Process.parent`, `Process.state`, `Process.threads`, `Process.memory`, `Process.started` and `Identity` in `src/model.rs`; `Kind::Kernel`; `Snapshot.shadows` (`platform::Shadow { pid, parent, name }`) in `src/platform/mod.rs`; `model::bounded` in `src/model.rs`; `layout::GOLDEN_ANGLE` and `layout::Seats` in `src/layout.rs`; `Stage::growth`, `Item::Beam`, `Frame::line` in `src/render.rs`; `simulation::FixedStep` and `Rng::for_identity` in `src/simulation.rs`; `Snapshot.life` and `life` events from B3 in `src/model.rs` and `src/platform/*/events.rs`. Planned: `src/arbor.rs`, `Stage::age`, B7's thread rows, S3's ribbons. The prototype is `docs/mockups/code/arbor.py` (numpy, Pillow; reference only).

## 8. Variations and open questions

* Replace the slot order "first seen" by the true start order (`Identity.start`) when the whole tree is present, and see whether stable layout is lost on churn.
* Murray-style trunk: radius by $\alpha = 3$ for the roots (kernel threads as a fluid network) and $\alpha = 2$ for the canopy; the inequality in 2.3 predicts roots would be thinner.
* Show real tree statistics from `/proc` over time: fan-out distribution, depth, fraction of leaves.
* Use `Snapshot.life` events (B3) for blossoms instead of first-seen age, so exec and fork storms show with exact times.
* Replace the "convention" wind frequency by the cantilever scaling and compare how the sway reads.
* The 145th-child azimuth repeat at literal 137.5 degrees: use the exact constant.

## 9. References

* E. M. Reingold and J. S. Tilford, "Tidier drawings of trees", *IEEE Transactions on Software Engineering* SE-7(2) (1981) 223-228; J. Q. Walker II, "A node-positioning algorithm for general trees", *Software: Practice and Experience* 20 (1990) 685-705.
* H. Vogel, "A better way to construct the sunflower head", *Mathematical Biosciences* 44 (1979) 179-182; S. Douady and Y. Couder, "Phyllotaxis as a physical self-organized growth process", *Physical Review Letters* 68 (1992) 2098-2101.
* A. Lindenmayer, "Mathematical models for cellular interactions in development I and II", *Journal of Theoretical Biology* 18 (1968) 280-315; P. Prusinkiewicz and A. Lindenmayer, *The Algorithmic Beauty of Plants* (Springer, 1990).
* K. Shinozaki, K. Yoda, K. Hozumi and T. Kira, "A quantitative analysis of plant form: the pipe model theory", *Japanese Journal of Ecology* 14 (1964) 97-105 and 133-139.
* C. Eloy, "Leonardo's rule, self-similarity, and wind-induced stresses in trees", *Physical Review Letters* 107 (2011) 258101.
* C. D. Murray, "The physiological principle of minimum work: I. The vascular system and the cost of blood volume", *Proceedings of the National Academy of Sciences* 12 (1926) 207-214.
* Issue [#54](https://github.com/tfpickard/isotop/issues/54); `docs/briefs/views-2026-10.md` section 15; [docs/VIEW_CONVENTIONS.md](https://github.com/tfpickard/isotop/blob/main/docs/VIEW_CONVENTIONS.md). See also [Theory-Stable-Layout](Theory-Stable-Layout), [Theory-Life-Events](Theory-Life-Events), [Theory-Text-Safety](Theory-Text-Safety).
