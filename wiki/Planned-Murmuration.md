# Planned: Murmuration

> **Planned, not yet implemented.** Specified in [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md) and tracked in issue [#51](https://github.com/tfpickard/isotop/issues/51).

A dusk sky over a dark treeline, and thousands of threads flying as one starling murmuration: a dark, shape-shifting cloud that folds into lobes and ribbons, with sheets seen edge-on as dark bands. Each thread is one bird, each process a sub-flock, a bird's speed is its thread's CPU use, and a memory-pressure spike or an OOM kill releases a falcon whose dive sends an agitation wave across the flock. This page is the design of that view, the biology and physics behind its one non-obvious rule (each bird follows its seven nearest neighbours, not everything within a radius), and what a stdlib port of the specified constants does.

![Reference mockup of the murmuration view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/murmuration.jpg)

*`docs/mockups/murmuration.jpg`, from the prototype `docs/mockups/code/murmuration.py` (3750 birds, a still from its animation). A visual target, not an isotop render.*

## 1. The system being modelled

A murmuration is the dusk display of European starlings (*Sturnus vulgaris*) before roosting: tens of thousands of birds flying as one body, turning in waves, splitting and re-merging, with no leader. A predator (a peregrine falcon, a sparrowhawk) attacking the flock produces dark bands that sweep across it, which Procaccini and colleagues (2011) measured as propagating waves of agitation. Ballerini and colleagues (2008) reconstructed the 3D positions of individual birds in flocks of hundreds to a few thousand and found that **each bird interacts with a fixed number of neighbours, about seven, regardless of how far away they are** (a *topological* interaction), not with all neighbours within a fixed distance (a *metric* interaction). Cavagna and colleagues (2010) found that velocity fluctuations in the flock are correlated over distances that scale with the size of the flock itself (scale-free correlations), a hallmark of a system poised near a critical point; the physics is reviewed by Cavagna and Giardina (2014).

For a process monitor the metaphor is easy and slightly mischievous: threads are birds, a process is a flock inside the flock, and a busy thread flies faster and surges to the front. The honest limit: the flock's *shape* is a simulation outcome, not a measurement of anything (section 5).

## 2. Theory

### 2.1 From boids to topological flocks

Craig Reynolds's *boids* (1987) steer each agent by three local rules: **separation** (do not crowd neighbours), **alignment** (match their heading) and **cohesion** (move toward their centre). Vicsek and colleagues (1995) reduced it to its physical core: self-propelled particles of constant speed that adopt the average heading of the neighbours within a radius, plus noise. As the noise falls (or the density rises) the system goes through a phase transition from disorder to collective motion, measured by the polarisation $\Phi = \lvert\sum_i \mathbf{v}_i\rvert / \sum_i \lvert \mathbf{v}_i \rvert$. See [Math-Flocking](Math-Flocking) for the family of models; the murmuration view's update is a Vicsek-style constant-speed model with Reynolds's three forces and *topological* neighbours.

### 2.2 Why topological: density robustness

Take a metric radius $r$. In a cloud of density $\rho$ the mean number of neighbours is $\tfrac43\pi r^3\rho$ and, for a Poisson cloud, the probability that a bird has none is $e^{-\frac43\pi r^3\rho}$. With the view's separation radius $r = 1.5$, a density of 1 per unit volume gives 14 neighbours on average, but at $\rho = 0.1$ a quarter of the birds (24 %) have no neighbour at all and at $\rho = 0.03$ two thirds do. A flock that opens up under attack, or whose edges are thin, would shed birds. A **fixed number** $k$ of nearest neighbours has the same degree for every bird at every density, so the interaction graph keeps its connectivity as the flock expands and contracts: **topological interaction is a density-robust rule**. That is the engineering argument for the 7; Ballerini et al. measured that real birds do it.

### 2.3 Closed islands: why $k = 7$ is not enough for cohesion

If $m = k + 1$ birds are each other's $k$ nearest neighbours, then every directed neighbour edge out of the group stays inside it. If no outside bird has any of them among its $k$ nearest, the group is a **closed island**: a connected component of the $k$-NN graph of size $k+1$. A flock built of tight clumps of 8, with $k = 7$, is exactly this. `docs/figures/murmuration-flock.py` (panel c) draws 12 clumps of 8 birds ($\sigma = 1$ inside a clump, clumps scattered with $\sigma = 14$): the $k$-NN graph has 12 components for $k = 3$ to 6, 11 at $k = 7$ (two clumps happened to overlap) and **1 at $k = 8$**. The brief's rule follows: alignment and separation use the 7 nearest (as starlings do), but **cohesion reaches 21 neighbours**, so an island would need at least 22 mutually closest birds. The prototype's own failure list says "cohesion over only the 7 nearest lets closed groups of eight split off as islands"; the derivation shows why.

### 2.4 The forces (the brief's constants, verified against the prototype)

Per step, with $\Delta t = 1$ and all rates per step:

| Term | Value |
|---|---|
| alignment | 0.30 toward the weighted mean velocity of the 7 nearest; same-process neighbours count double |
| cohesion | 0.02 toward the centroid of the **21** nearest |
| separation | 0.4 $\sum$ (away / $d^2$) over neighbours closer than 1.5 (distance floored at 0.3) |
| whole-flock pull | 0.0006 toward the flock centroid |
| roost | horizontal pull $0.004\,(r - 80)$ beyond $r = 80$; altitude band 35 to 110 with restoring force 0.01 |
| vertical damping | 0.10 |
| noise | Gaussian, $\sigma = 0.02$ |
| speed | after each step the velocity is rescaled to $1.0 + 0.3\,\text{busy}$ |
| falcon | flee radius 14, acceleration 0.9 |
| agitation | spreads as 0.93 times the neighbours' maximum when above 0.35, decays by 0.86 per step, adds 0.12 jitter |

The brief records three failures of the prototype not to repeat: a per-process centroid pull of 0.0012 or more splits the flock into separate balls; cohesion over only the 7 nearest lets closed groups of eight split off (above); vertical damping of 0.22 flattens the flock to a pancake that reads as a smear. Vertical damping is what makes sheets: birds fly level and turn by banking, so the flock spreads in the horizontal plane, and a sheet seen edge-on is a dark band.

### 2.5 The agitation wave

Each step a bird takes $a_i \leftarrow \max(0.86\,a_i,\ 0.93\max_{j \in N_7(i)} a_j)$ if the second term exceeds 0.35, else it decays. Along a chain of hops the amplitude falls by 0.93 per hop, so a wave from a source of amplitude 1 can advance only while $0.93^n > 0.35$, that is $n \le \ln 0.35/\ln 0.93 = 14.5$, **14 hops** per source. Birds within the flee radius of the falcon are re-set to 1 on every step, so a moving falcon is a moving line of sources; reach is therefore 14 hops beyond the falcon's *track*, not beyond one point. The number of hops times the spacing of the farthest-of-seven neighbour (about the 7th-neighbour distance) is a first estimate of the reach in distance.

### 2.6 The density rendering law

Each bird is drawn as a soft glow with peak alpha $1 - e^{-1.1 w}$, where $w = 1$ (or 2.6 when agitated). Compositing independent translucent layers multiplies their transmittances, so $n$ unit birds in the same pixel give transmittance $(e^{-1.1})^n = e^{-1.1n}$ and alpha $1 - e^{-1.1 n}$, **exactly the Beer-Lambert law for the summed weight**: one bird gives 0.667, two 0.889, three 0.963; a single agitated bird already gives 0.943. So blending splats is not an approximation of the brief's density law, it is that law, as long as the order of blending does not matter (it does not for this multiplicative form). Dense regions go black, thin ones grey.

## 3. What a port of the constants does (my simulation)

I ported the prototype into the stdlib (`docs/figures/murm_w8.py`, brute-force-equivalent grid search, the brief's constants, seed 11) and ran it at **1875 birds** (half of the prototype's 3750, the size the issue's connectivity test uses) for 260 steps. Results, with the caveat that this is one seed of a Python port, not the Rust view:

![Side view, connectivity, islands and the falcon's wave of the stdlib port](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/murmuration-flock.svg)

* **Connectivity.** The largest connected component of the 7-NN graph holds 100 % of the birds for the first 60 steps, then fluctuates between 0.79 and 1.00 as the flock splits and re-merges (0.985 at step 180, 0.822 at 200, **0.797 at step 260**). The 21-NN graph shows the same split at the end. The issue's test, "more than 95 % of birds after N steps", is therefore a statement about an instant in a fluctuating quantity; in this run it would pass at most steps up to 120 and again at 170 to 190, and fail at steps 80, 100, 130 to 160 and 200 to 260 (including 260). Real murmurations split and re-merge too, so this is a feature of the constants, but a test that samples one step is fragile (see section 8).
* **Polarisation** $\Phi$ starts at about 0.87, dips to 0.22 at step 100 (the flock turns) and recovers to 0.77 to 0.83.
* **Spacing.** At step 260 the mean distance to the 7th neighbour is 2.03 and to the 21st 2.89 world units.
* **Scale effect.** At one tenth size (375 birds, 11 clumps) the same constants broke the flock into 4 to 7 groups with the largest component at 42 to 55 % by step 260, so the issue's choice to run the connectivity test at half size is not a detail: the constants were tuned for thousands of birds.
* **Falcon.** Released at step 261 from the prototype's start point, it reaches the flock at +59 steps. At +59 18 birds are agitated (above 0.35); at +60 47, of which 6 are already beyond the flee radius 14; the count then grows to a peak of **1559 of 1875 (83 %) at +91**, with the farthest agitated bird 89 world units from the falcon at +99. So "agitation reaches beyond the flee radius" holds with a wide margin, but at these constants the wave is not a localised ripple: it floods most of the flock at weight 2.6, which would black out the whole cloud for a few seconds of view time. (Whether a weaker spread factor keeps a travelling band is untested.)

## 4. Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Bird | one thread, keyed `(Identity, tid)` (B7) | seeded with `Rng::for_identity(identity, tid ^ SALT)`; at most 8000 birds |
| Sub-flock | process | same-process neighbours count double in alignment |
| Bird speed | per-thread CPU (B7) | $\text{speed} = 1.0 + 0.3\,\text{clamp}(\text{cpu}/100, 0, 1)$; if a thread's `cpu` is `None` or the process has no thread list, the process mean `cpu / threads` is used and ` \| speed from process mean for N` goes into the legend |
| Falcon | `Snapshot.pressure[1]` (memory some avg10) $\ge$ 10, at most once per 8 s while it stays above; or `Snapshot.oom_kills` / the sum of `Unit.oom_kills` increasing | a chevron of two triangles, label `OOM` or `memory pressure` |
| Agitated bird weight | simulation state | weight 2.6 in the density law |
| New thread rises from the roost; exiting thread falls to it in 1.5 s | thread births and exits | then removed |
| Hover or selection | the process | its birds redrawn in `kind_color`, label `{name}  {n} threads` |
| Sky, treeline | none | **decorative**; the treeline carries no data; the pressure tint is the existing haze |
| Flock shape, lobes, ribbons | none | emergent simulation output |

Cap rule: with more than 8000 threads in total, process $p$ flies $\max(1, \text{round}(\text{threads}_p \cdot 8000/\text{total}))$ birds, chosen by lowest hash of `tid` so the selection is stable from sample to sample, and ` | 1 bird per K threads` goes into the legend.

## 5. Design: how isotop will realise it

* **Module and row.** `src/murmuration.rs` with `Murmuration` (`Default`), one registry row, no keys, no digit; hover, click, `/` and the camera keep their global behaviour. The row's `sources` list B7's threads and B4's counters (via S6), so threads are sampled only while this view is shown.
* **Neighbours.** `SpatialHash3` (see [Math-Spatial-Hashing](Math-Spatial-Hashing)) with cell size 2.0 and one `nearest(point, 21, ..)` per bird per step: the first 7 results drive alignment and separation, all 21 cohesion. Its doc comment advises a cell a little wider than the typical distance to the $k$-th neighbour; in the port above that distance is 2.89 at step 260, so a cell of 2.0 makes each query search at least two shells (an implementation cost to measure, not a bug).
* **Step.** `FixedStep::new(0.05, 6, 0.5)`: 20 prototype steps per second of wall clock, at most 6 per frame. While paused, time does not advance and the flock holds still. Headless warm-up: 15 s, 300 steps.
* **Roost.** At the world origin with $z$ up. Processes present on the first frame start as clumps in a loose ball, as in the prototype.
* **Falcon.** One falcon per release; entering the view after a gap longer than two sample intervals resets the baselines and releases nothing.
* **Rendering.** A new `Backdrop::Twilight = 4` added to `Sky::base` and `shaders.wgsl` together (the prototype's three-stop gradient, existing pressure tint applied on top; existing backdrops must stay byte-identical; test `twilight_does_not_change_existing_backdrops`). Birds are `Item::Glow` in `[12,10,16]`, sorted far to near, radius `clamp(0.45 * px_per_world_unit, 1.2, 6)` px, plus the pick-only splat from B9 (`Frame::pick_disc` exists in `src/render.rs`) carrying the process index. The treeline is opaque `NONE` triangles in `[16,14,22]` anchored to the bottom of the frame. `Scene` already has `highlight: Vec<Identity>`; the issue adds it to `Stage` if not exposed.
* **Camera.** `stage.positions[id]` is each process's centroid; `stage.bounds` is the fixed roost cylinder ($r = 90$, $z$ from 0 to 110, via `ring_bounds`) not the flock; the default pitch is `LOWEST_PITCH` (0.35 rad, about 20 degrees), clamp unchanged.
* **Legend.** ` bird = thread | flock = process (same-process neighbours count double) | speed = thread CPU | falcon = memory pressure or OOM kill | 7 topological neighbours`, with degradation notes from `status::missing_note`.
* **Dependencies** (ROADMAP index): P1, P2, S1, P4, B4, B7, B9, S2, S6. In the tree at commit `192299b`: P1, P2, S1, P4 (`SpatialHash3`, `Rng::normal`), S2, S6 and B9 are merged; **neither B4 (`Snapshot.oom_kills`) nor B7 (`Process.thread_list`) exists yet**.
* **Platforms.** Linux: full. macOS: **available but degraded**: threads and per-thread CPU come from B7's macOS path; there is no OOM-kill counter, so `missing` lists `oom kills` and the legend says so; the memory-pressure level (warn = 25, critical = 75) still releases the falcon; visual parity with Linux is expected.
* **No plugin system.** Compile-time module and `REGISTRY` row.

## 6. Reading the picture

* **Measured:** how many threads each process has (the number of birds in its sub-flock, up to the cap), each thread's CPU (its speed), when memory pressure or an OOM kill occurs (the falcon), and births and exits of threads (rising from and falling to the roost).
* **Derived:** the positions. They are the output of a seeded simulation, so the flock's lobes, ribbons, splits and merges **carry no information about the machine**; two runs with the same data give the same flock, but a different flock shape does not mean a different machine.
* **Decorative:** sky, treeline, the shape of the cloud.
* **What hover does:** it lights the birds of one process inside the flock, so the viewer can see that a process's threads stay clumped but not isolated (same-process neighbours count double, not infinitely).
* **Cannot infer:** which thread is which (individual birds are not labelled), CPU of a process from its sub-flock (speed differences are 30 % at most: $1.0$ to $1.3$ per step), or any relation between processes (no links are coupled to the flock).
* **Resolution and aliasing.** Birds are glows of 1.2 to 6 px; at 8000 birds on a 1080p frame the cloud saturates (dark) and the density law is the only readable quantity. The issue does not say whether positions are interpolated between the 20 Hz steps (`FixedStep::advance` returns the fraction); without interpolation three frames at 60 fps show the same state.
* **The falcon is an event marker:** it appears once per release, not continuously.

## 7. Try it

Nothing runs in isotop yet. When the view lands:

```sh
./target/release/isotop --demo --view murmuration --time 12 --width 1600 --output /tmp/murmuration.png --renderer cpu
```

(Tab and the `v` picker reach it; the headless run warms up 15 s of flock first.) Meanwhile `python3 docs/figures/murmuration-flock.py` (about 2.5 minutes) regenerates the simulation of section 3, and the model is `docs/figures/murm_w8.py`.

## 8. Where in the code

Existing: `simulation::SpatialHash3` (`nearest`, still marked `expect(dead_code)` until the first view uses it), `simulation::Rng::normal` and `for_identity`, `simulation::FixedStep` in `src/simulation.rs`; `Item::Glow`, `Frame::pick_disc`, `Stage`, `Backdrop`, `ring_bounds`, `LOWEST_PITCH` in `src/render.rs`; `Snapshot.pressure`, `Unit::oom_kills`, `Process.threads` in `src/model.rs`; `status::missing_note` in `src/status.rs`; `terminal::clean_text`. Planned: `src/murmuration.rs`, `Backdrop::Twilight` in `src/render.rs` and `src/shaders.wgsl` (and the constant in `src/render/shared.rs`), B4's `Snapshot.oom_kills`, B7's thread rows. The prototype is `docs/mockups/code/murmuration.py` (numpy, scipy `cKDTree`, Pillow; reference only).

## 9. Variations and open questions

* Make the connectivity test robust: measure the largest component's *time average* over a window, or its minimum over the last N steps, instead of one instant (the port above fluctuates between 0.79 and 1.0).
* Tune the agitation spread (0.93) and threshold (0.35) so the wave stays a band: measure the agitated fraction against spread factor.
* Compute the velocity correlation function of the simulated flock and see whether its correlation length scales with flock size, as in Cavagna et al. 2010.
* Replace the whole-flock pull by a per-process term at a small value and see how close one can get to 0.0012 before islands form.
* Use per-thread **names** to colour or arrange birds by role (workers, I/O threads).
* How does the 7 depend on the cohesion count of 21 for a flock of 8000?

## 10. References

* M. Ballerini et al., "Interaction ruling animal collective behavior depends on topological rather than metric distance: Evidence from a field study", *Proceedings of the National Academy of Sciences* 105 (2008) 1232-1237.
* A. Cavagna et al., "Scale-free correlations in starling flocks", *PNAS* 107 (2010) 11865-11870; A. Cavagna and I. Giardina, "Bird flocks as condensed matter", *Annual Review of Condensed Matter Physics* 5 (2014) 183-207.
* C. W. Reynolds, "Flocks, herds and schools: a distributed behavioral model", *Computer Graphics* 21(4) (SIGGRAPH 1987) 25-34.
* T. Vicsek et al., "Novel type of phase transition in a system of self-driven particles", *Physical Review Letters* 75 (1995) 1226-1229.
* H. Hildenbrandt, C. Carere and C. K. Hemelrijk, "Self-organized aerial displays of thousands of starlings: a model", *Behavioral Ecology* 21 (2010) 1349-1359.
* A. Procaccini et al., "Propagating waves in starling, *Sturnus vulgaris*, flocks under predation", *Animal Behaviour* 82 (2011) 759-765.
* I. D. Couzin et al., "Collective memory and spatial sorting in animal groups", *Journal of Theoretical Biology* 218 (2002) 1-11.
* Issue [#51](https://github.com/tfpickard/isotop/issues/51); `docs/briefs/views-2026-10.md` section 12; [docs/VIEW_CONVENTIONS.md](https://github.com/tfpickard/isotop/blob/main/docs/VIEW_CONVENTIONS.md). See also [Math-Flocking](Math-Flocking), [Math-Spatial-Hashing](Math-Spatial-Hashing), [Theory-Simulation-Toolkit](Theory-Simulation-Toolkit).
