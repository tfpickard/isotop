> **Planned, not yet implemented.** Specified in [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md) and tracked in issue [#43](https://github.com/tfpickard/isotop/issues/43).

# Planned: Horizon

The `horizon` view is specified as the OOM killer drawn as a Schwarzschild black hole: a round black shadow with a thin bright photon ring, an accretion disk whose far side is bent up over the top of the shadow and under its bottom, and processes riding circular orbits in the disk plane, ranked by `oom_score`, with the next victim on the innermost stable circular orbit and the safe processes far out. When the kernel's OOM killer really strikes, the victim leaves the ISCO, spirals in, stretches, reddens and dims, and freezes and fades at the horizon instead of crossing it, exactly as a distant observer would see. The geometry comes from [Math-Schwarzschild](Math-Schwarzschild); this page adds the design as written in issue #43 and says plainly what "memory pressure as mass" can and cannot mean. Nothing here is implemented: `isotop --list-views` does not list `horizon`.

![Reference mockup of the horizon view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/horizon.jpg)

*The reference mockup (`docs/mockups/horizon.jpg`), rendered in Python by `docs/mockups/code/horizon.py` from synthetic data. It is the visual target, not an isotop render.*

## The system being modelled

A **Schwarzschild black hole** is the simplest one: no spin, no charge, a horizon at $r = 2GM/c^2$ from which nothing returns, and a photon sphere at $1.5$ times that radius where light can orbit. Around it, gas with angular momentum settles into a thin disk in which each ring is in a nearly circular orbit and slowly spirals inward by friction, until it reaches the innermost stable circular orbit (ISCO) at $6GM/c^2$ and plunges. The disk is hot, and its light is bent by the hole so strongly that an observer near the disk's plane sees the far side of the disk lifted over the top of the shadow (the primary image) and the underside of the disk's far part as a second arc beneath it (the secondary image). Jean-Pierre Luminet computed the first such image in 1979, with a thin disk, by tracing light backwards from the observer and plotting it by hand and on a computer; the look was reused in the film *Interstellar* (whose black hole, Gargantua, is a spinning Kerr one, treated by James, von Tunzelmann, Franklin and Thorne in 2015), and the Event Horizon Telescope has since imaged the bright ring and dark shadow of M87* (2019). The ring is the lensed photon sphere; the shadow is the capture cross-section of radius $3\sqrt3\,GM/c^2$.

Why a black hole for the OOM killer. Linux's out-of-memory killer is a one-way surface of the machine: when allocation cannot be satisfied and reclaim fails, the kernel scores the processes with a *badness* heuristic (`/proc/<pid>/oom_score`, roughly each process's share of memory in thousandths, shifted by `oom_score_adj`) and kills one, and a process with `oom_score_adj = -1000` is exempt. The structure maps well: a ranking by how likely each process is to be consumed, an inner edge at which the next victim sits, and an event from which nothing comes back. It is a metaphor, and the page says below which parts of it carry data.

## Theory

The derivations are on [Math-Schwarzschild](Math-Schwarzschild); this is the summary of what the view uses, in units $G = c = M = 1$ (one world unit is one $M$).

| Quantity | Value | Where it appears in the view |
|---|---|---|
| Horizon | $2M$ | the radius no image reaches |
| Photon sphere | $3M$ | the (drawn) photon ring sits at $b_c$ |
| Marginally bound orbit | $4M$ | not drawn |
| ISCO | $6M$ | the next victim's orbit; inner edge of the disk |
| Critical impact parameter (shadow radius) | $b_c = 3\sqrt3\,M = 5.196\,M$ | the black disk; every secondary image has $b > b_c$ |
| Circular-orbit angular velocity | $\Omega = \sqrt{M/r^3}$ in coordinate time | orbital phase of every process |
| ISCO period | $2\pi\,6^{3/2}M = 92.3M$ | the mockup shows "period 92 M" for the victim |
| Radiative efficiency | $1 - \sqrt{8/9} = 5.72\,\%$ | disk brightness is proportional to the accretion rate |

**Kepler's law is exact here.** $\Omega = \sqrt{M/r^3}$ is Kepler's third law, in the distant observer's time, and the README is specified to say so. The proper time of an orbiting body runs slower by $\sqrt{1 - 3M/r}$.

**Lensing by vertices.** The camera is orthographic: the observer is at infinity. Instead of tracing a ray per pixel, the view maps every vertex of the scene (a finely tessellated disk, process positions, orbit polylines, stars) to the screen position of its lensed image, then draws ordinary triangles and lines. For a point at radius $r$ whose direction from the hole makes angle $\psi$ with the line of sight, the primary image needs a photon that sweeps $\psi$ and the secondary image one that sweeps $2\pi - \psi$; each image lies at screen distance $b$ (the photon's impact parameter) from the shadow's centre along the projected direction. A table $b(r, \psi)$ is built by integrating $u'' = 3u^2 - u$ ($u = 1/r$) with a fourth-order Runge-Kutta method for a grid of $b$, and inverted at lookup time. Beloborodov's closed form, $1 - \cos\alpha = (1 - \cos\psi)(1 - 2M/r)$ with $b = r\sin\alpha/\sqrt{1 - 2M/r}$, is the specified oracle for primary images at $r \ge 6M$. I checked it against direct integration (details on the Math page): at $r = 10M$ it reproduces the exact sweep to $1.3\times10^{-4}$ at $\alpha = 60^\circ$ and to $7.7\times10^{-3}$ at $110^\circ$ (a sweep of 2.3 rad), so the specified test, $\psi$ from 0.1 to 2.0 at $r = 10M$ within 3 %, is comfortably satisfied.

**Disk flux and Doppler beaming.** The disk runs from $6M$ to about $30M$ with the Page-Thorne (1974) flux profile. Observed intensity is $I \propto F\,g^{4}$, with $g = 1/(1+z)$ and, following Luminet (1979),
$1 + z = (1 - 3M/r)^{-1/2}\left(1 + \sqrt{M/r^{3}}\,b\sin i\,\sin\alpha\right)$,
where $b\sin\alpha$ is the image's screen $x$ coordinate. The sign depends on how $\alpha$ is measured, and the brief says to check that the approaching side comes out brighter. For a ring at $6M$ seen at $80^\circ$ the approaching side is 30 times brighter than the receding side, at $10M$ 13 times, at $20M$ 6 times (middle panel below, no light bending). The colour temperature of the gas follows $T \propto F^{1/4}$ (Stefan-Boltzmann) times the redshift factor $g$.

![Effective potential, Doppler beaming of a disk ring, and the plunge](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/horizon-potential-redshift-plunge.svg)

*Left: $V = (1 - 2/r)(1 + L^2/r^2)$; the ISCO is the inflection of the $L^2 = 12$ curve and $L^2 = 16$ peaks at $E = 1$ at $r = 4$. Middle: relative intensity $g^4$ around a ring at $i = 80^\circ$ (0 degrees receding, 180 degrees approaching). Right: the plunge from the ISCO ($L = 0.95\,L_\mathrm{isco}$, kick 0.1): $\ln(r - 2M)$ falls with slope $-1/2$ in Schwarzschild time and $-1/4$ in the observer's retarded time. Script: `docs/figures/horizon-potential-redshift-plunge.py`.*

**The plunge, and which clock.** A body leaving the ISCO with slightly too little angular momentum and a small inward kick falls through the horizon in finite proper time, but a distant observer sees it freeze. Close to the horizon, $r - 2M \propto e^{-t/2M}$ in Schwarzschild time $t$ and $\propto e^{-u/4M}$ in the observer's retarded time $u = t - r_*$; the redshift grows as $e^{u/4M}$. The issue specifies a brightness $1.5\,(1 - 2/r)$ "which e-folds about every $4M$ (0.4 s)", with `plunge_track` integrated against coordinate time. Because $1 - 2/r \propto r - 2M$, that brightness e-folds every $2M$ of *Schwarzschild* time, not $4M$, and every $4M$ only if the plunge clock is the retarded time $u$. I measured both slopes by integration ($-0.4999$ and $-0.2500$ over $t \in [50, 58]$). The implementer should pick one clock on purpose: `t` gives 0.2 s at $10M$ per second, `u` gives 0.4 s. This is not a bug in the physics but an inconsistency in the specification, and I list it under open questions below too.

**The flux profile has a trap.** The Python prototype's `page_thorne` closed form uses $\sqrt3/3$ in front of the logarithm. Integrating the Page-Thorne expression directly gives $\sqrt3/2$ (derivation on the Math page): with $\sqrt3/2$ the ratio to the numerical integral is constant to five digits and the flux peaks at $r = 9.55M$; with $\sqrt3/3$ it peaks at $8.37M$ and is not proportional to the integral. The issue says to "port its physics and constants"; the port should check this coefficient (my finding from reading and testing the prototype; I have not checked Luminet's printed equation).

**The photon ring is drawn, not traced.** The thin ring just outside the shadow is a drawn annulus at $b_c + 0.02$ to $b_c + 0.12$ (at least 1.5 px, Doppler-modulated), not a computed image of the photon sphere; the README is specified to say so. The true photon ring is the infinite sequence of higher-order images, each narrower than the last by roughly $e^{-\pi} = 0.043$.

**What "memory pressure as mass" could honestly mean.** The view does not use a mass: $M = 1$ is a unit. Mass sets the scale of a Schwarzschild hole and nothing else, and no process quantity is a mass. The measured quantities enter in three different ways:

* **Memory pressure is an accretion rate.** The disk's luminosity is $\eta\,\dot M c^2$ with $\eta = 5.72\,\%$, so brightness proportional to `memory some avg10` is brightness proportional to how hard the machine is being consumed. That is a sound reading of a stall percentage; it is not an amount of memory. Full pressure flares the disk; the specified gain is $0.45 + 0.9\cdot(\mathrm{pressure}/100)$.
* **`oom_score` is a rank, not a radius.** Radii come from the *rank* of the score, so the kernel's absolute scale does not matter; the radius $6 + 17.5\,((\mathrm{rank}-1)/27)^{0.8}$ is a chosen function, not a measurement.
* **The horizon is an epistemic edge.** The frozen image of a killed process is the *last sample* of it: after a kill, isotop's information about the process stops, and the picture shows that stop as an asymptotic approach. This is true of every isotop view, and the horizon is its honest drawing.

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Orbit radius of a process | rank by `oom_score` (building block B6, open) among non-kernel processes with a readable score and `oom_score_adj` above -1000; ties by `Identity` | the top 28 sit at $r = 6 + 17.5\,((\mathrm{rank}-1)/27)^{0.8}$ in $M$; rank 1 is the ISCO; two neighbours swap only if their scores differ by 5 or more; radius changes ease with $\tau = 1.5$ s, at most 2 M/s |
| Unkillable band (`oom_score_adj = -1000`) | `oom_score_adj` (B6) | a band from 25.7 to 27.3 M at golden-angle phases, at most 12 drawn, labelled `unkillable` |
| Orbital phase and speed | none (decoration of the true law) | $\Omega = \sqrt{1/r^3}$, time at 10 M per wall-clock second, `stage.dt` clamped to 0.5 s; initial phase from `Rng::for_identity` |
| Body size | `Process.memory` | radius $\mathrm{clamp}(0.12\,\sqrt[3]{\mathrm{MiB}},\,0.18,\,0.9)\,M$ |
| Body colour | `Process.kind` | `palette::kind_color` |
| Disk brightness | memory pressure `Snapshot.pressure[1]` | gain $0.45 + 0.9\cdot\mathrm{pressure}/100$ |
| Disk colour and brightness pattern | the physics of the metric (Doppler and gravitational redshift) | $I = \mathrm{gain}\,F\,g^4\,\mathrm{tex}\,\mathrm{bb}(T)$, $T = 7200\,\mathrm{K}\,(F^{1/4}g)^{1.55}$ (the exponent 1.55 is a stylised widening), colour through the blackbody table B5 |
| OOM kill | `oom_kill` counter in `/proc/vmstat` (B4; macOS has none), else the sum of `Unit.oom_kills` (cgroup `memory.events`, already in `Snapshot.units`) | a vanished body stays as a ghost for 2.5 s; if the counter rose, the best-ranked ghost plunges |
| Plunge | the kill event | eased to 6 M, then `plunge_track` ($L = 0.95\,L_\mathrm{isco}$, kick 0.1); brightness $1.5\,(1 - 2/r)$; colour toward [235, 60, 30]; $r \ge 2.0006$; removed below brightness 0.02 |
| Other exits | exit events | fade in place over 1.2 s |
| Disk surface texture (value noise in the co-rotating frame), glare `Glow`s, background stars, the photon ring annulus | none | **decorative** |

Inspector notes (specified): `oom_score N | oom_score_adj N`, `rank R of K`, `orbit r = X.XX M | period P M (S s)`.

## Design

Everything below is the specification of issue #43. None of it exists.

**Modules.** `src/horizon.rs` holds a `#[derive(Default)] Horizon` implementing `views::ViewModule`, with one registry row; `src/horizon/lens.rs` holds the pure physics, with no drawing, in the style of `medium.rs`: the photon table, Page-Thorne flux, $1 + z$, and `plunge_track`. One world unit is $M = 1$. There is no plugin system; the module is compiled in.

**Dependencies.** Status at commit `192299b`:

| Item | Provides | Status |
|---|---|---|
| P1 (#17), P2 (#18) | registry row, picker, smoke list | merged |
| S1 (#22) | CPU/GPU parity check and tolerance | merged |
| S4 (#25) | opt-in demo exits and respawns (the kill cycle) | merged |
| S6 (#27) | source demand, wording of missing sources | merged |
| B4 (#31) | `/proc/vmstat` `oom_kill` and its demo value | open |
| B5 (#32) | blackbody colour table | open |
| B6 (#33) | `oom_score`, `oom_score_adj` with demo values | open |

**Lens table.** RK4 on $u'' = 3u^2 - u$ with $h = 0.002$ and the prototype's grid of $b$; 360 log-spaced radii from 2.0005 to 150, by 721 values of $\psi$. Built in a `OnceLock` on the first draw, not at startup. Tests named by the issue include `lens_table_matches_beloborodov_at_ten_m` (psi from 0.1 to 2.0, within 3 %), `lens_table_matches_a_direct_photon_integration` (0.5 %), `secondary_images_always_lie_outside_the_shadow`, `orbital_angular_velocity_matches_kepler`, `a_plunging_image_dims_monotonically_and_never_reaches_the_horizon` and `the_approaching_side_of_the_disk_is_brighter_and_bluer`.

**Rendering.** Existing items only (`Triangle`, `Sphere`, `Glow`, `Beam`, `Star`); no rasterizer or shader change, so CPU and GPU draw the same lensed geometry and picking comes for free. The backdrop is `Backdrop::Void` with the pressure haze left on. About 1,500 lensed stars on a shell at 40 M, both images, the brightest 40 as `Glow`s, coloured through B5 with a fixed seed. The disk is flat-shaded facets of about 4 px (the secondary image at half the density), capped at 120,000 triangles, relensed only when inclination, zoom or frame size changes and recoloured every frame; facets darker than 2/255 are skipped. Colour per channel is $255\cdot\mathrm{srgb}(1 - e^{-0.36 I^{1.3}})$. Draw order, nearest last: secondary disk, the shadow (a fan of radius $b_c$, pick `NONE`), the photon ring, the primary disk, the bodies. The ISCO and the unkillable band are dashed lensed `Beam` polylines. Bodies are `Sphere`s whose pick is the process index; the secondary image is drawn at 60 % brightness with the same pick.

**Camera.** Inclination is a piecewise-linear map of the global camera pitch: `LOWEST_PITCH` gives 86 degrees, `ISOMETRIC` gives 80 degrees (the default, as in the mockup), $\pi/2$ gives 0 degrees (face-on). Camera rotation shifts the disk azimuth, and pixels per M follow the zoom. There are no view-local keys: PgUp/PgDn, e/Q, right-drag and zoom drive the camera, each a pure function of the global camera. `Camera::ground` becomes `pub(crate)` so that lensed screen points can be unprojected for `positions`, `places` and `bounds`.

**Orbits and the kill.** The details are in the mapping table. A killed process stays as a ghost for 2.5 s; if the kill counter rose inside that window, the best-ranked ghost plunges. A new `Stage.own_exits: bool` (default false), set on horizon's registry row, makes `Scene::render` skip its generic exit ring for this view only (a test, `other_views_still_draw_exit_rings`, guards the others).

**Demo and warm-up.** With S4 options on the row, the demo's kill cycle has a period of 30 s: the highest-index non-kernel process at index 134 or below gets score 950, exits at phase 20 s (when the demo `oom_kill` counter increments), and respawns with a new start at 26 s. The headless warm-up (80 frames) is on the registry row. Parity frames: `--time 12` and `--time 20.5` (mid-plunge).

**Degradation.**

* Linux: full. A process whose score is unreadable is excluded and the legend shows `N unranked`. With no kill counter at all the legend reads `no oom_kill counter: exits not classified`.
* macOS: available, degraded. No `oom_score`, so processes are ranked by memory footprint; no unkillable band and no plunges (all exits fade); the lensing, disk and pressure work. Legend note: `macOS: ranked by memory footprint; no oom_score or kill count`. The view is never skipped.

**Legend** (specified): `next victim NAME SCORE | oom_kill N (vmstat|cgroups) | orbit radius = oom_score rank (ISCO 6M = next victim) | disk brightness = memory pressure | colour = Doppler + gravitational redshift | far orbit = oom_score_adj -1000 | i = 80 deg`.

## Reading the picture

* **Measured:** the ranking by `oom_score`, memory pressure, the existence and time of an OOM kill, process memory and kind.
* **Derived from the metric, not measured:** the shape of the disk, the shadow, the doubled images, the Doppler brightening and every colour in the disk. They are the correct pictures of an *imagined* disk and carry no information about the machine; the `tex` noise and glare are decorative.
* **Radius is rank.** A body at $6M$ has the highest score, not a score of any particular size; two bodies at $6M$ and $7M$ may differ by one point or by hundreds.
* **The victim is a prediction.** The ISCO process is the kernel's most likely victim by its own heuristic; the real victim may differ (cgroup-level OOM kills, `oom_score_adj` changed since, memory cgroups with `oom.group`). When the counter rises and the best-ranked ghost plunges, the view is asserting a match that the counter alone does not prove.
* **Freezing is the point.** The plunge's fading is the observer's view; the process stopped existing at once. Nothing about the dimming time is a property of the kill.
* **The ring is a drawing, the shadow is exact** (radius $b_c$ for a Schwarzschild hole), and Kerr spin is not modelled (out of scope).
* **Resolution:** at most 28 ranked bodies are drawn; the rest are collapsed and counted.

## Try it

None of these work yet: `horizon` is not registered.

```sh
# Once the view lands (issue #43).
./target/release/isotop --demo --view horizon --time 12 --width 1600 --output /tmp/horizon.png
./target/release/isotop --demo --view horizon --time 20.5 --width 1600 --output /tmp/plunge.png
./target/release/isotop --demo --view horizon --renderer cpu --width 1920 --benchmark 120
```

The figure regenerates from the repository root with `python3 docs/figures/horizon-potential-redshift-plunge.py`; the light-bending figure and the maths are on [Math-Schwarzschild](Math-Schwarzschild).

## Where in the code

* Existing: `src/views.rs` (`REGISTRY`, `ViewModule`), `src/simulation.rs` (`Rng::for_identity`), `src/layout.rs` (`GOLDEN_ANGLE`), `src/render.rs` (`Stage`, `Camera`, `Item`, `Backdrop`), `src/palette.rs` (`kind_color`), `src/model.rs` (`Snapshot.pressure`, `Snapshot.units`, `Unit.oom_kills`, `Process.memory`, `Process.kind`).
* Planned: `src/horizon.rs`, `src/horizon/lens.rs`, building blocks B4, B5, B6 (their modules are not on `main`), `Stage.own_exits`.
* Prototype, reference only: `docs/mockups/code/horizon.py` (`LensTable`, `beloborodov_b`, `page_thorne`, `one_plus_z`, `plunge_track`, `rank_radius`).

## Variations and open questions

* **Which plunge clock?** Decide between Schwarzschild time (e-folding $2M$, 0.2 s) and retarded time (e-folding $4M$, 0.4 s) for the brightness decay, and document it.
* **Check the Page-Thorne coefficient** before porting (see above), and add a test that integrates the flux numerically.
* **Kerr.** A spinning hole moves the ISCO from $6M$ (non-rotating) to $M$ (maximal, prograde) and shifts and flattens the shadow; the issue puts it out of scope. A spin parameter taken from, say, the hottest process's CPU would be arbitrary, which is exactly the problem.
* **A real tidal stretch.** The victim is stretched radially by a stylised streak; the geodesic-deviation tidal stretch $2GM\ell/r^3$ (Math page) could set its length, so larger processes stretch more.
* **Hawking radiation.** A black hole's temperature falls as $1/M$; a hole that shrinks as it eats (the victim's mass is removed) is a different physical model, and the memory freed by the kill is the natural analogue. Nothing in the specification does this.
* **Time to OOM.** The trend of the pressure could predict the next kill and move the victim from the ISCO to a marginally bound orbit as the pressure rises.
* **Photon ring images.** Draw the first higher-order image at its true $b$ and width from the lens table instead of the annulus.

## References

* J.-P. Luminet, "Image of a spherical black hole with thin accretion disk", *Astronomy & Astrophysics* 75, 228 (1979).
* D. N. Page, K. S. Thorne, "Disk-accretion onto a black hole. I. Time-averaged structure of accretion disk", *The Astrophysical Journal* 191, 499 (1974).
* A. M. Beloborodov, "Gravitational bending of light near compact objects", *The Astrophysical Journal* 566, L85 (2002).
* O. James, E. von Tunzelmann, P. Franklin, K. S. Thorne, "Gravitational lensing by spinning black holes in astrophysics, and in the movie Interstellar", *Classical and Quantum Gravity* 32, 065001 (2015).
* The Event Horizon Telescope Collaboration, "First M87 Event Horizon Telescope results. I. The shadow of the supermassive black hole", *The Astrophysical Journal Letters* 875, L1 (2019).
* C. W. Misner, K. S. Thorne, J. A. Wheeler, *Gravitation*, W. H. Freeman (1973).
* The Linux kernel documentation for `/proc` (`oom_score`, `oom_score_adj`) and `/proc/vmstat` (`oom_kill`).
