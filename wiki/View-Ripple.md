# View: Ripple

The machine as a pond. Every process is a pebble, set among the pebbles of its cgroup like a handful thrown into the water; every busy pebble drives ripples at its own pitch, louder with more CPU; and a real two-dimensional wave equation, stepped on a grid of about 150 cells, carries those ripples across the open water so that they interfere with the ripples of the pebbles around them. The surface is shaded from the height field's own gradient, and hovering the water names the process that owns that patch of it. It is the view where the physics is closest to the surface: the wave equation is *solved*, not drawn.

![Ripple, a fresh render from the demo workload](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/ripple.webp)

![Ripple with its status lines, as it looks in the terminal](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/ripple.webp)

![Ripple in motion](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/ripple-motion.webp)

Select it with the key `3` or `--view ripple`. The equation, its lattice and its border are derived in [Math Wave Equation](Math-Wave-Equation); this page is about how a process table drives it.

## The system being modelled

A **ripple tank** is a shallow dish of water with vibrating dippers; it is the classroom apparatus for the interference of waves, and the thing the pond is a picture of. Two dippers beating at one frequency make a fixed pattern of lines where the waves cancel (nodes) and lines where they add; the lines are hyperbolas with the two dippers as foci, and for sources a distance $d$ apart there are $2\lfloor d/\lambda\rfloor + 1$ lines of maximum. With **different** frequencies there is no fixed pattern: the nodes drift at the beat frequency $\lvert f_1 - f_2\rvert$, and many sources of unrelated pitch superpose into a moving, band-limited random surface.

That second case is Ripple. Why a pond for processes? Because the machine is a *medium shared by oscillators*: every process wakes, works and sleeps at its own rhythm, and what one does is felt, through the scheduler, caches and memory bus, by the others near it. A pond makes that coupling a literal field. It is a metaphor with an honest limitation, stated plainly: **nothing in the operating system propagates between processes at 14 cells per second.** The coupling between pebbles is the equation's, not the machine's; see [Reading the picture](#reading-the-picture).

The physical wave medium is a membrane, not water. Real surface waves are dispersive (Lamb 1932); the membrane's $\omega = c\lvert\mathbf{k}\rvert$ plus a small restoring term is chosen for simplicity and cost.

## History

The thing Ripple draws is a wave medium disturbed by many sources, and the ways people have made such media visible.

* In his Royal Society lecture of November 1801, Thomas Young presented an analogy between interfering light, sound and water waves, and showed the water case in a shallow tank. Interference was a hard sell: Newton's reputation counted for more than a demonstration, and Young published the double-slit form of his argument only in 1807.
* Ernst Chladni's *Entdeckungen uber die Theorie des Klanges* (1787) introduced the technique of fixing a plate, scattering sand on it and drawing a bow along its edge: the plate rings at one of its natural frequencies and the sand collects on the nodal lines. The patterns are still called Chladni figures.
* Chladni lectured in Paris in 1808, and the Academy announced on 27 February 1809 a prize of 3,000 francs for a mathematical theory of the figures, with the deadline of 1 October 1811 (the story that Napoleon was in the audience and sponsored the prize is the usual telling; the dates of the visit vary by source, 1808 or 1809). Sophie Germain was the only entrant in the first round and her derivation contained errors; Lagrange supplied a missing term in 1813; she received the prize on her third attempt, in 1816. Histories add that her theory was still incomplete.
* The wave equation itself is older than either: d'Alembert's equation for the vibrating string dates from the 1740s (`(unverified)` as to the year), and its two-dimensional form for membranes and plates is what the Paris problem needed.
* The ripple tank became standard school apparatus in the twentieth century: a shallow glass dish, a lamp above and a screen below, with a motor-driven bar or dippers as sources. Dates of its adoption in curricula are not given here.
* Ripple's medium is a membrane with a small restoring term and a damping sponge at its border, not water. Real surface waves are dispersive; the pond is a teaching tank in that respect and in no other.

## Theory

The pond integrates the damped Klein-Gordon equation with forcing, derived and analysed on [Math Wave Equation](Math-Wave-Equation):

$$
h_{tt} = c^2\nabla^2 h - \gamma(\mathbf{x})\,h_t - \kappa\,h + F(\mathbf{x},t),
\qquad c = 14\ \text{cells/s},\ \gamma_0 = 0.35\ \mathrm{s^{-1}},\ \kappa = 0.5\ \mathrm{s^{-2}} .
$$

The results that matter for this view:

* **A pebble is a point source.** Driven at $\omega = 2\pi f$ it makes a Hankel wave whose far field falls as $r^{-1/2}e^{-\alpha r}$, $\alpha \approx \gamma/2c = 0.0126$ per cell: the e-folding amplitude length is 79 cells, over half the pond's long side. *Measured* on the exact scheme: $\alpha = 0.0122$.
* **The pitch band is 0.35 to 1.25 Hz**, wavelengths 42 to 11 cells ($k = 0.149$ to $0.559$ rad per cell). Phase speed is $1.056\,c$ and group speed $0.947\,c$ at the low end (the $\kappa$ term), $1.004\,c$ and $0.996\,c$ at the high end. The lattice adds at most $-1\%$ phase error and $-3\%$ group error at 11 cells.
* **Many incommensurate sources make a Gaussian field.** The surface at a point is a sum of $N$ sinusoids of independent frequency and phase and amplitudes $A_i(r)$; for large $N$ its distribution tends to a Gaussian by the central limit theorem, with variance $\sum_i A_i^2/2$. The shading, which reads the *gradient* of the field, therefore looks like marbling rather than rings, as soon as more than a handful of processes are busy.
* **A cluster is partly coherent.** Pitch is an independent uniform draw on $[0.35, 1.25]$ Hz per pebble. The probability that two pebbles' pitches lie within $\delta$ is $2\delta/W - (\delta/W)^2$ with $W = 0.9$ Hz; for $\delta = 0.02$ Hz that is 4.4%, so a cluster of 16 busy pebbles (120 pairs) is expected to hold about five pairs with a beat period of at least 50 s. Those pairs make quasi-stationary nodal lines, for as long as both stay busy.
* **Energy goes as amplitude squared.** The forcing amplitude saturates in CPU (below), so the *power* that a pebble radiates goes as $(u/(u+40))^2$.

![Single source at 0.8 Hz: the field and its radial envelope](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/ripple-single-source.svg)

## Mapping

Realness contract: each visual element is a measurement, derived from one, or declared decorative.

| Visual element | Measured quantity (source) | Transform | Status |
|---|---|---|---|
| Pebble (sphere) | one process | `kind_color` (kernel slate, system teal, session amber, container violet; zombie, stopped and blocked override) | measured (cgroup path, `PF_KTHREAD`; macOS by app and uid) |
| Pebble radius | resident memory (`Process::memory`: RSS on Linux, memory footprint on macOS), eased | `mass_radius`: $0.24\,\sqrt[3]{\text{MiB}}$, clamped to 0.25 to 4.5 world units | measured; volume proportional to memory (1 GiB is 5.5 times as wide as 6 MiB) |
| Cluster | `Process::group` (the service or scope of the cgroup; on macOS the `.app` bundle) | one sunflower spiral per group, slot $k$ at $s\sqrt{k+\tfrac12}$ and $k$ golden angles | measured grouping; slot order is first-come |
| Ripple amplitude | process CPU in percent of one core (counter delta, $\tau = 1.5$ s in `measure`, then $\tau = 0.3$ s in `Scene::ease`) | forcing $900\,u/(u+40)$ on the velocity; only if $u > 0.3$ | measured, saturating (half of full at 40%) |
| Ripple pitch and phase | none | $f = 0.35 + 0.9\,\mathrm{knuth}(\text{pid})_{31:24}/255$ Hz; phase from bits 8 to 23 | **decorative**: a stable per-pid fingerprint |
| Drip | a process appearing | $+30$ impulse on the velocity at its pebble | measured event: first seen in a sample (`Scene::ease` notes the birth); the view reads no life-event source |
| Splash | a process exiting | $-90$ impulse where its pebble last was, plus the shared exit ring | measured event: gone from the next sample |
| Swell | pressure stall information, `some avg10` of CPU and I/O (Linux) | amplitude $0.01 + 0.12\,p/(p+20)$, $p$ = CPU + I/O percent, of three travelling sines | measured; **not** memory pressure; on macOS the CPU and I/O slots read zero, so it rests at 0.01 |
| Warm glow | CPU | when $u > 1$: radius $\mathrm{core}\,(2.2 + 2\,u/(u+50))$, strength $0.3 + 0.6\,u/(u+40)$ | measured |
| Green glow | NVIDIA per-process GPU memory (NVML) | $\mathrm{core}\,(2.6 + 0.5\ln(1 + \text{MiB}/64))$ | measured; absent on macOS |
| Water tint | the nearest process | 8% of the owner's kind colour, over a fixed deep blue | derived (a flood-fill territory), see below |
| Glint | slope of the surface | specular $(\mathbf{n}\cdot\mathbf{h})^{48}$ towards moonlight, capped at 0.6 | derived from the field |
| Foam | speed of the surface $\lvert h_t\rvert$ | $\min(0.05\lvert h_t\rvert, 1)$ | derived from the field |
| Cyan arcs, pink beams | socket links; connections leaving the machine | as in Orbit; they lie on the water | measured |
| Sky tint | pressure stall (all three) | haze colour per resource | measured |

Nothing in the table is a quantity the wave field *measures about the machine*; the field is derived.

## How isotop realises it

Ripple is a `ViewModule` in [`src/views/ripple.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/ripple.rs): the `Ripple` struct owns an optional `Wave` from [`src/medium.rs`](https://github.com/tfpickard/isotop/blob/main/src/medium.rs), the `clusters` and the `pebbles`, and its registry row in `views::REGISTRY` builds it with a closure. `Scene` keeps none of it. Each frame, in `Ripple::draw`:

1. **`Ripple::scatter` places the pebbles.** `Ripple` keeps `clusters` (one per group name) and `pebbles` (identity to group and slot). A newcomer takes the lowest free slot of its group, so a pebble keeps its slot for life and a freed slot goes to the next arrival. The spiral spacing satisfies $s \ge (2r_{\max} + 1)/1.4$ with $r_{\max}$ the cluster's largest pebble (`PEBBLE_GAP` = 1); when it must grow it grows to $1.15$ times the need and **never shrinks**, so a cluster only ever spreads. Each pebble is jittered by up to $0.1\,s$ from a hash of its pid. Adjacent spiral slots sit at least about 1.55 spacings apart (*measured* over 200 slots), so the 1.4 divisor leaves room for the jitter. A cluster of extent $E$ reserves open water $1.1(E + 3)$ in radius (`POND_GAP` = 3) and is placed with `layout::vacant` (the golden-angle search used by Orbit's systems); it moves only when its reach outgrows its reservation. Slots and clusters are reserved for every sampled process, drawn or not, so focus and `--limit` never move anything (tests `focusing_and_clearing_focus_keeps_every_ripple_cluster_in_place` and `processes_beyond_the_limit_keep_their_ripple_place`).
2. **`footprint` makes the grid.** `render::cover` calls `Grid::covering` over the pebbles plus a margin of $8 + 0.08\times$ extent world units, with 150 cells on the longer side, so the grid is always about 150 cells long and the pond crosses in 150/14 = 10.7 s of wall-clock whatever the machine looks like. World-space wave speed is $14\times$ the cell size. If the new grid is `render::similar` to the old (same columns and rows, cell within 5%, origin within two cells) the `Wave` is kept; otherwise a fresh, still pond starts.
3. **Events become impulses.** Every point in `Stage::splashes` (where processes left since the last frame, which the scene notes for every view) calls `wave.drive(point, -90)`, every process in `Stage::newborn` `wave.drive(point, 30)`.
4. **Sources.** Each process with CPU above 0.3% becomes a source $(\text{at},\ 900\,u/(u+40),\ 2\pi f,\ \varphi)$. `wave.advance(time, ...)` calls `force` before each substep, which `drive`s each source by $\text{loudness}\cdot\sin(\omega t + \varphi)\cdot\Delta t$. The substepping rule keeps the Courant number at or below 0.5 (see the math page). Time is `stage.time`, not `stage.dt`: `Wave::advance` clamps the elapsed time to between 0 and 0.25 s itself, and when the clock does not advance (elapsed 0) the water does not move.
5. **Territories.** `territories` floods the grid breadth-first from every pebble, larger pebbles claiming shared cells first, so each node belongs to its nearest pebble in the **four-neighbour (Manhattan) metric**; borders are therefore diamond-shaped, not circular.
6. **Surface.** The drawn height is $0.09\,h + \text{swell}\cdot(\sin_1 + 0.7\sin_2 + 0.5\sin_3)/2.2$ world units; the three sines are plane waves of fixed wavenumber and time rates 0.9, 0.7, 1.3 that do not come from the wave solver.
7. **Shading.** Each node's normal comes from central differences of the *drawn* heights; shade is diffuse (light from the side of the view) plus a $(\mathbf{n}\cdot\mathbf{h})^{48}$ glint plus foam, blended towards moonlight by at most 0.6 so the highlights never add up to white (test `ripple_water_stays_dim_however_hard_it_is_stirred`: no water triangle above 190 in any channel, even at 400% CPU everywhere and pressure 90). Each cell is two triangles whose colour is the average of their three nodes, so a cell never shows two shades.
8. **Draw.** The grid is $\approx 2\cdot150\cdot(\text{rows}-1)$ `Item::Triangle`s (`Frame::facet`) with pick index `owners[a]`, then each pebble as a sphere impostor with its glows. Both rasterizers draw the same triangles, so there is no CPU/GPU parity special case.

**Picking.** The water facets carry the owner's index, so the pointer over water selects the owning process (cell-coarse, at the node `a` of the cell, not the pixel's nearest pebble); a pebble's own sphere pick is nearer and wins. Nothing is a `Thing`.

**Determinism.** Source pitch and phase hash the pid only; pebble jitter hashes the pid; the swell uses `time`. There is no random stream. Headless runs (`--output`, `--benchmark`) settle first: the row's `SETTLE` warm-up (`Phase::Settle`, 80 frames 0.05 s apart, `When::Output`) ends at `--time`, so the water has been stirred for four seconds. Without it a one-frame render would show a flat pond.

**When a source is missing.** The row declares no optional source. Without NVIDIA GPU memory there are no green glows; without CPU and I/O pressure (macOS) the swell stays at its resting 0.01.

**Stability.** The row waives no property. A cluster is laid out with 15% more spacing than its largest pebble needs, so pebbles can grow in place (the test `doubling_every_pebble_leaves_them_in_place_and_clear_of_each_other` doubles every pebble's memory) and the cluster spreads, and is placed again, only when a pebble would touch a neighbour or the cluster outgrows its reserved water (`pebbles_that_outgrow_their_cluster_spread_it_out_without_touching`).

## Reading the picture

What I saw rendering the demo at the default time (`--demo --view ripple --width 1600 --output ... --renderer cpu`): a rhombus of dark teal water (a rectangular grid seen isometrically), twelve loose clusters of sixteen pebbles in four colours, and a field of marbled bands in the water with bright glints on the slopes. The headless render settles for four seconds. There are no clean concentric rings; there are about 130 busy pebbles of unrelated pitch, and the field is their sum. Busy pebbles carry amber glows; a few carry green rings; faint cyan arcs link some pairs.

**You can infer**

* *Which pebbles are busy*, from the glow and from a local standing disturbance of the water around them (amplitude falls as $r^{-1/2}$ from the pebble).
* *Memory*, as pebble volume (radius is the cube root: a 5-fold width difference is a 125-fold difference in memory).
* *Cluster membership*, from the spiral grouping, and the cgroup name from hovering.
* *Births and exits*, as a drop (an outgoing ring that grows at 14 cells per second) or a splash (an inverted ring), for a few seconds until the damping removes them.
* *Overall load*, from the swell: a rougher whole surface means CPU and I/O pressure.

**You cannot infer**

* *Which process made which wave from the wave alone.* Pitch is a hash; two pebbles of the same pitch are an accident. The water tint says only whose territory a patch is, which is the nearest pebble, not the source of the waves there.
* *Any causal link.* The ripples that cross open water between two clusters are the equation's propagation, not a measured influence of one process on another. A pebble does not slow down because water arrived. Interference here is arithmetic.
* *The absolute scale.* The cell size follows the layout, so world distance is meaningless for waves; cells are the unit. The border within about 10 cells is damped (reflecting 20% to 46% of the amplitude at normal incidence), and the layout always keeps the pebbles outside it.
* *A process under 0.3% CPU.* It drives nothing; it is a pebble on the water and nothing else. Sub-threshold activity is invisible.
* *Anything about the order of the slots.* It is arrival order.
* *Memory pressure.* The swell is CPU plus I/O pressure; the memory term of the pressure is in the sky haze, not the water.

**Resolution and aliasing.** The water mesh has 150 nodes on the long side; the shortest pebble wavelength (11 cells) has 11 samples per wavelength, which the shading resolves; nothing is excited near the 2-cell Nyquist limit because the forcing footprint (a raised cosine) has no energy there. The pressure swell is a different, slower field laid over the solved one: its wavelengths are tens of world units and its rates under 1.3 rad per second.

**Worked example.** A busy process at 100% CPU has loudness $900\cdot 100/140 = 643$ and, at 0.8 Hz, makes a steady near-field amplitude of about 2.7 height units (*measured* with the exact scheme), falling to 1.2 at 10 cells. The drawn height is $0.09\times$ that: 0.24 and 0.11 world units, smaller than the 0.25 to 4.5 pebble radii, and the crest term, $\mathrm{clamp}(1.8\,h_{\mathrm{drawn}}, -0.5, 1)$, reaches 0.43. At 10% CPU the loudness is 180 and everything is 0.28 times as large.

## Try it

```sh
./target/release/isotop --demo --view ripple
./target/release/isotop --demo --view ripple --width 1600 --output ripple.png --renderer cpu
./target/release/isotop --demo --view ripple --width 1600 --output ripple12.png --time 12
./target/release/isotop --view ripple          # live; hover the water to name its owner
```

Keys while it is shown: `3` selects it; hover over water names the owning process; `c` cycles links; `[`/`]` step through retained snapshots (the water is never rewound: `Wave::advance` clamps a negative elapsed time to zero); Space pauses. In a live run pressure comes from `/proc/pressure/{cpu,io}`; to see the swell move, load the CPU (`stress-ng --cpu 8` or a parallel build).

## Where in the code

* [`src/views/ripple.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/ripple.rs): `Ripple`, `Ripple::draw`, `Ripple::scatter`, `Cluster`, `System`, `territories`, `announce`, `footprint`, constants `PEBBLE_GAP`, `POND_GAP`, `RIPPLE_DEEP`, `RIPPLE_CREST`, `RIPPLE_MOON`, `RIPPLE_SHINE`; tests `pebbles_cluster_by_cgroup_without_touching_and_stay_put`, `ripples_answer_to_activity`, `ripple_water_stays_dim_however_hard_it_is_stirred`.
* [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs): `cover`, `similar`, `glows`, `mass_radius`, `Frame::facet`, `Stage::splashes`, `Stage::newborn`.
* [`src/medium.rs`](https://github.com/tfpickard/isotop/blob/main/src/medium.rs): `Wave`, `Grid`.
* [`src/layout.rs`](https://github.com/tfpickard/isotop/blob/main/src/layout.rs): `spiral`, `vacant`, `spin`, `GOLDEN_ANGLE`.
* [`src/views.rs`](https://github.com/tfpickard/isotop/blob/main/src/views.rs): the `ripple` row (`SETTLE`, `LinkRule::All`); the legend is `Ripple::legend`.
* [`src/palette.rs`](https://github.com/tfpickard/isotop/blob/main/src/palette.rs): `kind_color`.
* Figure script: `docs/figures/ripple-single-source.py`.

## Variations and open questions

* **Make the pitch mean something.** Replace the pid hash by a measured frequency: the dominant period of the process's own CPU history (the shared `History` already holds a minute), so a periodic worker rings at its real rhythm and two workers on the same cron schedule are coherent for real. Then interference stops being decoration.
* **Let the water push back.** Feed the local surface slope into a pebble's drift, so busy clusters shoulder each other apart. It would break the stable layout rule; as an *offset drawn but not stored* it would not.
* **Anisotropic water.** `c` differing along the two grid axes would make a sheet in which rings become ellipses; map the axes to user and kernel time.
* **Locate coherent pairs.** Highlight pairs within 0.02 Hz, which beat for at least 50 s, and see whether the nodal lines between them are visible at the display's resolution.
* **A proper boundary.** The sponge reflects up to 46%; a perfectly matched layer would make the pond effectively infinite (see the variations on the math page).
* **Resonance.** The pond is not a resonator, because the sponge eats the reflections. An ideal pond with closed edges, 150 cells on its long side at $c = 14$ cells per second, would have a gravest mode at $c/2L = 0.047$ Hz, a period of 21.4 s, far below hearing; Chladni's plates sang because they were small and stiff.
* **Open:** how many busy pebbles before marbling dominates? The Gaussian-field argument says a handful; a statistical test of the field's kurtosis against $N$ would show it.

## References

* H. Lamb, *Hydrodynamics*, 6th ed. (Cambridge University Press, 1932).
* E. Hecht, *Optics*, 5th ed. (Addison-Wesley, 2017) for two-source interference and nodal lines.
* M. Born and E. Wolf, *Principles of Optics*, 7th ed. (Cambridge University Press, 1999).
* The numerical analysis is cited on [Math Wave Equation](Math-Wave-Equation#references).
