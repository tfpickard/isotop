# View Murmuration

Thousands of threads flying as one starling murmuration over a dark treeline at dusk. Every thread is a bird and every process a sub-flock inside the flock; a bird's speed is its thread's CPU use; each bird steers by its seven nearest neighbours, however far away they are, with the threads of its own process counting double; and a memory-pressure spike or an OOM kill releases a falcon whose dive sends a wave of agitation across the flock, drawn as a dark band. The flock is drawn as the density of dark splats, so crowded regions go black and thin ones stay grey. The shape of the cloud is the output of a seeded simulation and carries no information about the machine; the number of birds, their speeds and the falcon do.

![Murmuration: a dark flock of thousands of birds in a dusk sky above a treeline](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/murmuration.webp)

*The README image of the view (`docs/media/murmuration.webp`), rendered from the demo workload.*

## The system being modelled

A murmuration is the dusk display of European starlings (*Sturnus vulgaris*) before they roost: tens of thousands of birds flying as one body, turning in waves, splitting and re-merging, with no leader. When a predator (a peregrine falcon, a sparrowhawk) attacks the flock, dark bands sweep across it, which Procaccini and colleagues (2011) measured as propagating waves of agitation. Ballerini and colleagues (2008) reconstructed the three-dimensional positions of individual birds in flocks of up to a few thousand and found that **each bird interacts with a fixed number of neighbours, six or seven, regardless of how far away they are**: a *topological* interaction, not a *metric* one with everything inside a fixed distance. Cavagna and colleagues (2010) found that velocity fluctuations are correlated over distances that scale with the size of the flock, a hallmark of a system poised near a critical point; the physics is reviewed by Cavagna and Giardina (2014).

For a process monitor the metaphor is easy and slightly mischievous: threads are birds, a process is a flock inside the flock, and a busy thread flies faster. The honest limit is that the flock's shape is a simulation outcome and not a measurement of anything.

## History

There is no in-repository theory page for this view; this history is written for the wiki.

**The word.** *Murmuration* is older than its birds. English has it from the late 14th century, by way of Middle French from the Latin *murmurare*, to murmur or mutter, and its older senses are all about murmuring. As a name for a flock of starlings it is a collective noun of the kind listed in medieval hunting books: the 1486 *Book of Saint Albans*, the first printed English collection of such terms, is commonly cited for it. One second-hand report quotes the *Oxford English Dictionary* for an earlier form, "a murmeracion of stares", in a text of about 1470 printed by Caxton in 1479 (unverified). The word names a sound, the roar of wings and the chatter of many birds at a roost. This view makes none.

**Boids.** Craig Reynolds, then at the Symbolics Graphics Division in Los Angeles, built a simulated flock in the mid 1980s and published it as "Flocks, herds, and schools: a distributed behavioral model" in *Computer Graphics* 21(4) (SIGGRAPH 1987) 25-34. Secondary sources date the program to 1986, the paper to 1987. His *boids* steered by three local rules: separation (do not crowd neighbours), alignment (match their heading) and cohesion (move toward their centre). No boid saw the whole flock. The rules reappear in the code of this view as `SEPARATION`, `ALIGNMENT` and `COHESION`.

**A physicist's reduction.** Tamas Vicsek and colleagues (1995) reduced it to its core: self-propelled particles of constant speed that adopt the mean heading of neighbours within a radius, plus noise. As the noise falls or the density rises, the system passes through a phase transition from disorder to collective motion. The view's constant-speed update, with each bird rescaled to its own speed after every step, is a descendant. Hildenbrandt, Carere and Hemelrijk (2010) built a model of thousands of starlings that is in the references below.

**STARFLAG.** In the mid 2000s a group in Rome, the STARFLAG project of the Institute for Complex Systems of the Italian National Research Council (ISC-CNR), filmed starling flocks and reconstructed each bird's position in three dimensions. Ballerini and colleagues published the result in 2008 (*Proceedings of the National Academy of Sciences* 105, 1232-1237): the interaction between birds is set by topological distance. The abstract speaks of six to seven neighbours; the round figure of seven comes from later summaries, and a later analysis led from Princeton found a balance near seven. The argument for a topological rule is robustness: a fixed number of neighbours keeps a flock connected as it opens up under attack, which a fixed radius does not (see Theory). Cavagna and colleagues followed with scale-free correlations (2010), and Procaccini and colleagues with the waves of agitation under predation (2011).

**What the code takes from it.** `NEIGHBOURS` = 7 steers alignment and separation, after Ballerini et al. Cohesion reaches `FAR_NEIGHBOURS` = 21, which is not from the biology: a flock built of tight clumps of eight would otherwise split into closed islands (see Theory).

## Theory

### Topological against metric neighbours

Take a metric radius $r$. In a cloud of density $\rho$ the mean number of neighbours is $\tfrac43\pi r^{3}\rho$ and, for a Poisson cloud, the probability that a bird has none is $e^{-\frac43\pi r^{3}\rho}$. With the view's separation radius $r = 1.5$ and $\rho = 1$ per unit volume a bird has 14 neighbours on average, but at $\rho = 0.1$ about 24 % of birds have none at all. A flock that opens up under attack, or whose edges are thin, sheds birds. A fixed number $k$ of nearest neighbours gives every bird the same degree at every density, so the interaction graph keeps its connectivity as the flock expands and contracts. That is the engineering argument for seven; Ballerini et al. measured that real birds do it. [Math Flocking](Math-Flocking) covers the family of models; the view is a constant-speed Vicsek-style model with Reynolds's three forces and topological neighbours.

### Closed islands: why cohesion reaches 21

If $m = k + 1$ birds are each other's $k$ nearest neighbours and no outside bird has any of them among its $k$ nearest, the group is a connected component of the $k$-nearest-neighbour graph of size $k+1$: a closed island. A flock built of tight clumps of eight with $k = 7$ is exactly this. So alignment and separation use the seven nearest, as starlings do, but cohesion pulls toward the centroid of the 21 nearest, and an island would need at least 22 mutually closest birds.

![Side view, connectivity, islands and a falcon's wave in a Python port of the prototype](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/murmuration-flock.svg)

*`docs/figures/murmuration-flock.py`, a stdlib Python port of the prototype's constants (1,875 birds, one seed). It is not the Rust view; panel (c) shows the 7-neighbour graph of twelve clumps of eight split into 12 components for $k$ = 3 to 6 and joined into one at $k = 8$.*

### The forces

Per step, in units per step (the step is 0.05 s of animation time, and a bird moves its velocity each step):

| Term | Value | Constant |
|---|---|---|
| alignment | 0.30 toward the weighted mean velocity of the 7 nearest; same-process neighbours weigh 2 | `ALIGNMENT` |
| cohesion | 0.02 toward the centroid of the 21 nearest | `COHESION` |
| separation | 0.4 $\sum$ (away / $d^{2}$) over those of the 7 nearest closer than 1.5, distance floored at 0.3 | `SEPARATION`, `SEPARATION_RADIUS`, `SEPARATION_FLOOR` |
| whole-flock pull | 0.0006 toward the flock's centroid | `FLOCK_PULL` |
| roost | horizontal pull $0.004\,(r - 80)$ beyond $r = 80$; altitude band 35 to 110 with restoring force 0.01 | `ROOST_PULL`, `ROOST_RADIUS`, `FLOOR_ALTITUDE`, `CEILING_ALTITUDE`, `ALTITUDE_PULL` |
| vertical damping | 0.10 of the vertical velocity | `VERTICAL_DAMPING` |
| noise | Gaussian, $\sigma = 0.02$ per axis | `NOISE` |
| speed | after each step the velocity is rescaled to $1.0 + 0.3\,\mathrm{busy}$ | `roster` |
| falcon | flee radius 14, acceleration 0.9 / max(distance, 0.5) | `FLEE_RADIUS`, `FLEE`, `FLEE_FLOOR` |
| agitation | 0.93 times the largest of the 7 neighbours' agitation if that exceeds 0.35; decays by 0.86; adds jitter 0.12 | `SPREAD`, `SPREAD_FLOOR`, `DECAY`, `JITTER` |

The source records three failures of the prototype it must not repeat: a per-process centroid pull of 0.0012 or more splits the flock into separate balls (so only the whole flock is pulled in); cohesion over only the seven nearest lets closed groups of eight split off; and a vertical damping of 0.22 flattens the flock to a pancake that reads as a smear. Vertical damping is what makes sheets: birds fly level and turn by banking, so the flock spreads in the horizontal plane, and a sheet seen edge-on is a dark band.

### The agitation wave

Each step a bird takes $a_i \leftarrow \max\bigl(0.86\,a_i,\ 0.93\max_{j \in N_7(i)} a_j\bigr)$ if the second term exceeds 0.35, else it decays. Along a chain of hops the amplitude falls by 0.93 per hop, so a wave from a source of amplitude 1 advances only while $0.93^{n} > 0.35$, that is $n \le \ln 0.35/\ln 0.93 = 14.5$: 14 hops per source. Birds within the flee radius of the falcon are reset to 1 on every step, so a moving falcon is a moving line of sources, and the reach is 14 hops beyond its track, not beyond one point.

### The density law

Each bird is a soft splat with peak alpha $1 - e^{-1.1\,w}$, $w = 1 + 1.6\,a$ for agitation $a$ in $[0, 1]$, so $w$ runs from 1 to 2.6. Compositing independent translucent layers multiplies their transmittances, so $n$ unit birds in one pixel give transmittance $(e^{-1.1})^{n}$ and alpha $1 - e^{-1.1n}$, exactly the Beer-Lambert law for the summed weight: one bird gives 0.667, two 0.889, three 0.963, and a single fully agitated bird already 0.943. Blending splats is not an approximation of the density law, it is that law, as long as the order of blending does not matter, and for this multiplicative form it does not. Dense regions go black, thin ones grey. AGENTS.md notes why `Item::Puff` is used and not stacked `Item::Glow`: the CPU blend truncates where the GPU's rounds, so many stacked glows drift apart, while the puff rounds on both.

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Bird | one thread: `Process.thread_list` (`Source::Threads`), keyed by process identity and `tid` | at most 8,000 birds (`BIRD_CAP`); a process without a thread list flies `Process.threads` birds keyed apart from real threads, matched to a list that arrives later by position |
| Sub-flock | process | same-process neighbours count double in alignment |
| Bird speed | per-thread CPU; if a thread's reading is unknown, the process's `cpu / threads` | $1.0 + 0.3\,\mathrm{clamp}(\mathrm{cpu}/100, 0, 1)$ units per step, so 20 to 26 units per second at 20 steps per second |
| Cap | total threads above 8,000 | a process flies $\max(1, \mathrm{round}(\mathrm{threads}\cdot 8000/\mathrm{total}))$ birds, the threads with the lowest hash of `tid`, stable from sample to sample |
| Falcon (memory) | `Snapshot.pressure[1]` (memory, 10 s average) at or above 10 % (`PRESSURE_FALCON`) | at most one falcon per 160 steps (8 s) while it stays above; label `memory pressure` |
| Falcon (OOM) | a rise of `Snapshot.oom_kills`, else of the sum of `Unit.oom_kills` | waits for the falcon in flight to leave; label `OOM` |
| Agitated bird | simulation state | splat weight up to 2.6 |
| New thread | a thread that appeared | rises from the roost at its foot until it reaches the altitude band (35), where it joins the flock |
| Exited thread | a thread that vanished | falls to the roost over 30 steps (1.5 s) and is removed |
| Hover or selection | the process | its birds redrawn in `palette::kind_color` at radius ×1.15; label `{name}  {n} threads`; inspector lines of threads and flying birds and the three busiest threads |
| Tour and camera targets | each process's flock | `stage.positions` is the centroid of its birds; the tour visits the 12 largest flocks (`TOUR_TARGETS`) |
| Sky, treeline, the shape of the cloud | none | decorative; the treeline stands for the roost and carries no data; the pressure tint is the scene's haze |

## How isotop realises it

**Module and row.** `src/murmuration.rs` holds `Murmuration` (`Default`, implementing `views::ViewModule`) and one row at the end of `views::REGISTRY`: `Backdrop::Twilight`, `links: LinkRule::DrawnByView` (the flock draws no links), `flash: false` (a thread that exits falls to the roost instead), `pitch: Some(LOWEST_PITCH)` and a `face` that leaves the camera's rotation and pitch as they are (a flock flies high above the ground, so a tour or search centres the flock, not the ground below it), `sources: Sources::of(&[Source::Threads, Source::OomKills])` (threads and the kill counter are read only while the view is shown), demo options `threads` and `kernel_counters`, and a headless warm-up that plays 15 s of demo at a 0.05 s step, so the flock has settled from its clumps and the demo's samples have set off the falcon. No keys, no digit. There is no plugin system: the view is compiled in.

**The flock** (`Flock`) is pure simulation with no camera or frame. Each step (`Flock::step`) releases a falcon if one is due, advances rising birds and falling birds, then `steer` rebuilds a `simulation::SpatialHash3` over the flying birds and asks it for the 22 nearest points of each bird (itself and 21 others), so seven neighbours drive alignment and separation and all 21 drive cohesion. The hash answers exactly whatever its cell, so `Grid::adapt` resizes the cell (starting at 2.0, kept between 1 and 8) to the mean distance of a bird to its 21st neighbour when it is more than a quarter off, which changes the cost of a query and never a bird's path. New velocities are computed from the old ones for all birds and then applied together, so the order of birds does not matter.

**Clock.** `simulation::FixedStep` with `STEP` 0.05 s (20 steps a second, each a step of the prototype with $\Delta t = 1$), `MAX_STEPS` 6 and `MAX_GAP` 0.5 s. `FixedStep::advance` returns the fraction of the next step the frame lies in, and the view draws every bird at its position plus that fraction of its velocity, so the motion is smooth at any frame rate; a test checks that the flock follows the same path at 10 and at 60 frames per second. While paused (`stage.dt` zero) the flock holds still.

**Roster.** `roster` builds the birds the processes ask for, ordered by key, with the cap above. A bird keeps its place while its key is asked for again (`pair`, `Flock::reroster`); on the very first roster birds start in a clump per process (a point of a loose ball, centred by `clump_centre`, spread 14), later births rise from the roost.

**Falcon.** `release_falcon` starts the falcon at (-140, 40, 70) from the flock's centroid, flying at 2.6 units per step toward it, for at most 130 steps (6.5 s). One falcon at a time: a pressure falcon waits 160 steps after the last, an OOM falcon waits for the one in flight to leave. A view entered after a gap longer than twice the sample interval resets its baselines and releases nothing for what happened while it was away (`Murmuration::observe`).

**Drawing.** Each bird is an `Item::Puff` splat of colour (12, 10, 16), radius 0.85 world units kept between 1.2 and 6 px, sorted far to near. A lit process's birds are drawn again in its kind colour with strength $1 - e^{-2.2}$. Picking uses pick-only `Item::Pick` discs of at least 2 px carrying the process index, so the nearest bird has the last word. The falcon is a chevron of two triangles (red-pink, (235, 70, 80)) that picks nothing, with its label. The treeline is opaque triangles in (16, 14, 22) along the bottom, from two sine ridges scaled to the frame, picking nothing. The camera is fitted to the fixed roost cylinder (`FIT_RADIUS` 90, altitude 0 to 110), not to the moving flock.

**Stable layout.** A flock moves by itself, so the harness checks the place each process keeps at the start: the clump centre, which depends only on the process (`Murmuration::anchors`, test only).

**Legend.** ` bird = thread | flock = process (same-process neighbours count double) | speed = thread CPU | falcon = memory pressure or OOM kill | 7 topological neighbours`, preceded by ` 1 bird per K threads` above the cap, ` speed from process mean for N` for birds whose thread's CPU is unknown, and the `status::missing_note` wording for threads ("speed from process mean") and, without any kill counter, for the kill counter ("falcon from memory pressure only").

**Platforms.** Linux: full. On macOS the thread reader and the memory-pressure source are the platform's own; there is no kernel OOM counter on macOS, so the legend says the falcon comes from memory pressure only. I read this from the code and did not run it on a Mac (unverified).

## Reading the picture

* **Measured:** how many threads each process has (the size of its sub-flock, up to the cap), each thread's CPU (its speed), when memory pressure rises or an OOM kill occurs (the falcon), and births and exits of threads (rising from and falling to the roost).
* **Derived:** the positions. They come from a seeded simulation, so the lobes, ribbons, splits and merges of the flock carry no information about the machine; two runs with the same data give the same flock, but a different shape does not mean a different machine. The wave of agitation is derived too: it is the simulation's response to the falcon.
* **Decorative:** the sky, the treeline, the shape of the cloud.
* **A flock splits.** The source's own test says so: over eight fixed seeds the largest connected component of the 7-neighbour graph holds nearly every bird after 100 steps (5 s), and after 260 steps, as the flock turns about the roost and sheds lobes, most of them; in 80 seeds, 5 held more than 95 % at 260 steps and the median was 84 %. Real murmurations split and re-merge too.
* **Hover** lights the birds of one process inside the flock, so a viewer can see that a process's threads stay clumped but not isolated: same-process neighbours count double, not infinitely.
* **Cannot infer:** which thread is which (birds are not labelled), a process's CPU from its sub-flock (speed differs by at most 30 %), or any relation between processes (no link is coupled to the flock).
* **Resolution.** Birds are splats of 1.2 to 6 px; at 8,000 birds on a 1080p frame the cloud saturates, and the density law is the only readable quantity.
* **The falcon is an event marker.** It appears once per release, not continuously.

## Try it

Run here with the release binary (Linux, CPU renderer):

```sh
./target/release/isotop --demo --view murmuration --time 12 --width 1600 --output /tmp/murmuration.png --renderer cpu
./target/release/isotop --demo --view murmuration --width 1600 --renderer cpu --benchmark 30
```

Output of the first: `Wrote /tmp/murmuration.png (1600x900, 192 visible)`. The benchmark printed `30 frames | 1600x900 | 192 visible | 22.64 ms/frame | 44.2 renderer fps on cpu (excludes terminal transport)`, measured on a shared sandbox while other builds ran. Live: `./target/release/isotop --view murmuration`, or Tab and the `v` picker (seventeenth row, no digit); click a bird to light its process. Not run: the GPU renderer, a live non-demo session, and a macOS run.

## Where in the code

| Item | Path and symbol |
|---|---|
| The view | [src/murmuration.rs](https://github.com/tfpickard/isotop/blob/main/src/murmuration.rs): `Murmuration`, `Flock`, `Flock::steer`, `roster`, `Grid::adapt`, `roost_pull`, `paint_treeline` |
| Registry row | [src/views.rs](https://github.com/tfpickard/isotop/blob/main/src/views.rs): `REGISTRY` row `murmuration` |
| Neighbours | `simulation::SpatialHash3` (`nearest`, `rebuild`) in [src/simulation.rs](https://github.com/tfpickard/isotop/blob/main/src/simulation.rs); [Math Spatial Hashing](Math-Spatial-Hashing) |
| Clock | `simulation::FixedStep`; [Theory Simulation Toolkit](Theory-Simulation-Toolkit) |
| Drawing | `Item::Puff`, `Item::Pick`, `Backdrop::Twilight` in [src/render.rs](https://github.com/tfpickard/isotop/blob/main/src/render.rs) |
| Sources | `Source::Threads`, `Source::OomKills`; `Snapshot.pressure`, `Snapshot.oom_kills` |
| Prototype (reference only) | [docs/mockups/code/murmuration.py](https://github.com/tfpickard/isotop/blob/main/docs/mockups/code/murmuration.py) |

## Variations and open questions

* The roadmap sketched birds as `Item::Glow`s and the cell of the spatial hash as fixed at 2.0; the code draws `Item::Puff`s (which round on both backends) and lets the cell follow the flock's density. It also draws birds at an interpolated position between steps, which the roadmap left open.
* Tune the agitation spread (0.93) and threshold (0.35) so the wave stays a travelling band rather than flooding the flock; measure the agitated fraction against the spread factor. A Python port with the same constants flooded most of a 1,875-bird flock within about 90 steps of the falcon's arrival (a port, not the Rust view).
* Compute the velocity correlation function of the simulated flock and see whether its correlation length scales with the flock's size, as Cavagna et al. found for real flocks.
* Replace the whole-flock pull by a small per-process term and see how close to 0.0012 islands begin to form.
* Colour or arrange birds by thread name (workers, I/O threads).
* How does the choice of 21 for cohesion depend on the flock's size at 8,000 birds?
* [View Coop](View-Coop) is the other view in which processes flock. It uses a metric Vicsek radius of 4.0 world units (`NEIGHBOURHOOD` in `src/coop.rs`), keeps its flocks in henhouses, and sends a fox when the kernel's OOM killer acts; here a single falcon is released by the same event, and the birds are not kept.

## References

* M. Ballerini et al., "Interaction ruling animal collective behavior depends on topological rather than metric distance: Evidence from a field study", *Proceedings of the National Academy of Sciences* 105 (2008) 1232-1237.
* A. Cavagna et al., "Scale-free correlations in starling flocks", *PNAS* 107 (2010) 11865-11870; A. Cavagna and I. Giardina, "Bird flocks as condensed matter", *Annual Review of Condensed Matter Physics* 5 (2014) 183-207.
* C. W. Reynolds, "Flocks, herds and schools: a distributed behavioral model", *Computer Graphics* 21(4) (SIGGRAPH 1987) 25-34.
* T. Vicsek et al., "Novel type of phase transition in a system of self-driven particles", *Physical Review Letters* 75 (1995) 1226-1229.
* H. Hildenbrandt, C. Carere and C. K. Hemelrijk, "Self-organized aerial displays of thousands of starlings: a model", *Behavioral Ecology* 21 (2010) 1349-1359.
* A. Procaccini et al., "Propagating waves in starling, *Sturnus vulgaris*, flocks under predation", *Animal Behaviour* 82 (2011) 759-765.
* I. D. Couzin et al., "Collective memory and spatial sorting in animal groups", *Journal of Theoretical Biology* 218 (2002) 1-11.
* *The Book of Saint Albans* (printed 1486), for the list of collective nouns.
* See also [Math Flocking](Math-Flocking), [Math Spatial Hashing](Math-Spatial-Hashing), [Theory Simulation Toolkit](Theory-Simulation-Toolkit).
