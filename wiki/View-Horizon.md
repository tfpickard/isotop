# View Horizon

The OOM killer drawn as a Schwarzschild black hole: a round black shadow with a thin bright photon ring, an accretion disk whose far side is bent up over the top of the shadow and whose underside shows as a second arc below it, and processes riding circular orbits in the disk, ranked by `oom_score`, with the next victim on the innermost stable circular orbit and the processes the kernel never kills far out. When the kernel's OOM kill counter rises and a process vanishes, the victim leaves the ISCO, spirals in, stretches, reddens and dims, and freezes and fades short of the horizon instead of crossing it, as a distant observer would see it. The geometry is derived on [Math Schwarzschild](Math-Schwarzschild); this page says what the code draws, and what "memory pressure" can and cannot be in such a picture.

![Horizon: a lensed accretion disk bent over and under a black shadow, with coloured spheres for processes on orbits and one ringed in red](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/horizon.webp)

*The README image of the view (`docs/media/horizon.webp`), rendered from the demo workload.*

## The system being modelled

A **Schwarzschild black hole** is the simplest one: no spin, no charge, a horizon at $r = 2GM/c^{2}$ from which nothing returns, and a photon sphere at 1.5 times that radius where light can orbit. Gas with angular momentum settles into a thin disk around it, each ring on a nearly circular orbit, slowly spiralling inward until it reaches the innermost stable circular orbit (ISCO) at $6GM/c^{2}$ and plunges. The disk is hot, and the hole bends its light so strongly that an observer near the disk's plane sees the far side lifted over the top of the shadow (the *primary* image) and the underside of the far part as a second arc beneath it (the *secondary* image). The ring is the lensed photon sphere; the shadow is the hole's capture cross-section, of radius $3\sqrt3\,GM/c^{2}$.

**Why a black hole for the OOM killer.** Linux can promise more memory than it has. When allocation fails and reclaim cannot help, the kernel's out-of-memory killer scores the processes with a badness heuristic (`/proc/<pid>/oom_score`, roughly each process's share of memory in thousandths, shifted by `oom_score_adj`) and kills one; a process with `oom_score_adj` of -1000 is exempt. The structure maps well: a ranking by how likely each process is to be consumed, an inner edge where the next victim sits, and an event from which nothing comes back. It is a metaphor, and the sections below say which parts carry data.

## History

The longer account lives in the repository: [docs/theory/horizon.md](https://github.com/tfpickard/isotop/blob/main/docs/theory/horizon.md).

**Dark stars.** The idea that something could be too heavy for light to leave is older than relativity. In 1783 John Michell, and in 1796 Pierre-Simon Laplace, reasoned with Newtonian gravity and light as fast particles: enough mass packed into a small enough radius has an escape velocity above $c$. The argument was dropped when light was shown to be a wave, and it was right for the wrong reasons.

**Schwarzschild.** Einstein published the field equations in November 1915. Within weeks Karl Schwarzschild, serving in the German army on the Russian front, found the exact solution for the space around a non-rotating mass and sent it to Einstein. He died of an autoimmune disease in May 1916. For decades the surface at $r = 2M$ was treated as a true singularity; Arthur Eddington (1924) and later David Finkelstein (1958) showed that only the coordinates misbehave there, and that an infalling observer crosses it without noticing.

**Collapse.** In 1939 Robert Oppenheimer and Hartland Snyder computed the collapse of a pressureless cloud and found both pictures at once: the infalling matter reaches the centre in finite proper time, while a distant observer sees it slow and freeze at the horizon. That is the plunge this view draws. The phrase "black hole" was popularized by John Wheeler in a 1967 lecture, replacing "gravitationally completely collapsed object".

**Disks.** Matter falling into a compact object has angular momentum and settles into a disk. Shakura and Sunyaev (1973), Novikov and Thorne (1973) and Page and Thorne (1974) worked out what a thin relativistic disk radiates, and Cygnus X-1, a bright X-ray binary, became the first strong black-hole candidate in the early 1970s.

**Pictures.** In 1978 the French astrophysicist Jean-Pierre Luminet computed what a thin disk around a Schwarzschild hole looks like to a distant observer, on an IBM 7040 at the Paris-Meudon Observatory. The machine had no display, so he plotted the picture by hand with India ink, putting dots more densely where the numbers were brighter. It was published in 1979 and contains the crescent-and-ring structure this view reproduces. The Event Horizon Telescope imaged the shadow of M87\* in 2019 and of Sagittarius A\* in 2022.

**The OOM killer.** Each Linux process has an `oom_score`, higher meaning likelier to be chosen, and the administrator can bias it with `oom_score_adj`, from -1000 (never kill) to +1000. That interface arrived in Linux 2.6.36 (2010) in place of the older, coarser `oom_adj`, which was a power-of-two bit shift. The ranking this view draws is a policy that kernel developers chose and have revised.

## Theory

Units are $G = c = M = 1$, so one world unit is one gravitational radius $GM/c^{2}$ (`M` below). The horizon is at $r = 2$, the photon sphere at 3 and the ISCO at 6.

### Why an ISCO

A massive particle in the Schwarzschild metric obeys

$$
\left(\frac{dr}{d\tau}\right)^{2} = E^{2} - \left(1 - \frac{2}{r}\right)\left(1 + \frac{L^{2}}{r^{2}}\right),
$$

with conserved energy $E$ and angular momentum $L$ per unit mass, and $\tau$ the particle's proper time. Write $V(r) = (1 - 2/r)(1 + L^{2}/r^{2})$. Circular orbits sit at $V' = 0$ and are stable if $V'' > 0$. $V' = 0$ gives $L^{2} = r^{2}/(r-3)$, which exists only for $r > 3$; setting $V'' = 0$ as well, where the minimum is about to disappear, gives $r^{2} - 6r = 0$, so $r = 6$, with $L^{2} = 12$ and $E^{2} = 8/9$. A particle falling from rest at infinity to the ISCO has radiated $1 - \sqrt{8/9} \approx 5.7\,\%$ of its rest mass, the radiative efficiency of a Schwarzschild disk, against well under 1 % for nuclear fusion. Inside $r = 6$ no circular orbit survives a nudge.

![Effective potential for three values of angular momentum](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/theory/figures/horizon-potential.svg)

*The effective potential for three angular momenta. For $L^{2} = 16$ there is a stable minimum and an unstable maximum; as $L^{2}$ falls to 12 they merge into the inflection at $r = 6$. From the repository's theory page.*

### Light bending

Photons follow null geodesics. With $u = 1/r$ and the azimuth $\varphi$ as the parameter,

$$
\frac{d^{2}u}{d\varphi^{2}} + u = 3u^{2}
$$

(Newtonian light would have no right-hand side and run straight). A photon reaching a distant observer with impact parameter $b$ has $du/d\varphi = 1/b$ at $u = 0$. The critical value is $b_c = 3\sqrt3 \approx 5.196$ (`lens::SHADOW_RADIUS`): below it the photon crosses the horizon, above it the photon turns and escapes. The disk of $b < b_c$ on the screen is the shadow. Photons just above $b_c$ wind round the photon sphere before leaving, which is the thin bright ring; since the photon sphere's potential is $V_\gamma = (1 - 2/r)/r^{2}$, with its maximum at $r = 3$, its impact parameter is $r/\sqrt{1 - 2/r}$ evaluated at 3, which is $3\sqrt3$. A disk point is imaged twice: the primary image by a photon that sweeps the angle $\psi$ between the point's direction and the line of sight, the secondary by one that sweeps $2\pi - \psi$ round the other side.

![Light rays traced back from the observer around a black hole](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/theory/figures/horizon-rays.svg)

*Photons traced back from a distant observer on the right (the same equation and Runge-Kutta step as `lens::trace_photon`): below $3\sqrt3$ they spiral into the horizon, above it they are bent and escape. From the repository's theory page.*

### The disk, redshift and beaming

For a thin disk in the Novikov-Thorne model the flux at radius $r$ is zero at the ISCO, peaks near $9.5\,M$ and falls as $r^{-3}$ far out; `lens::flux` evaluates a closed form after Luminet (1979) derived from the Page-Thorne integral. Light from a patch at radius $r$ reaches the observer with

$$
1 + z = \left(1 - \frac{3}{r}\right)^{-1/2}\left(1 + \Omega\, x \sin i\right), \qquad \Omega = r^{-3/2},
$$

where the first factor is gravitational redshift plus the transverse Doppler effect of the orbit, the second the line-of-sight Doppler shift, $x$ is the horizontal screen position of the image in $M$ and $i$ the inclination from face-on. Observed bolometric intensity goes as $F g^{4}$ with $g = 1/(1+z)$: Liouville's theorem makes $I_\nu/\nu^{3}$ invariant along a ray, which gives $g^{3}$ at each frequency, and integrating over the shifted frequencies adds one more $g$. That is why the approaching side is so much brighter. Even before the line-of-sight term, the factor $(1 - 3/r)^{-1/2}$ is $\sqrt2$ at the ISCO, so light from the inner edge arrives at $(1/\sqrt2)^{4} = 25\,\%$ of its emitted brightness. The colour temperature is $T = 7200\,\mathrm{K}\,(F^{1/4} g)^{1.55}$ (`DISK_KELVIN`, `KELVIN_EXPONENT`; the exponent widens the colour ramp and is stylised).

### Orbits and clocks

The coordinate-time angular velocity of a circular orbit is $\Omega = r^{-3/2}$, identical to Kepler's, and the period is $2\pi r^{3/2}$ in $M$: 92.3 $M$ at the ISCO. The view runs time at 10 $M$ per second (`M_PER_SECOND`), so a body on the ISCO goes round in 9.2 s and one at the outer edge of the ranked disk ($r = 23.5$) in 71.6 s: the inner bodies outrun the outer ones by $(23.5/6)^{3/2} \approx 7.8$.

### A body falling in

For a body leaving the ISCO the coordinate time $t$ (a distant clock) and the proper time $\tau$ disagree: $dt/d\tau = E/(1 - 2/r)$ diverges as $r \to 2$, so the body never reaches the horizon in finite $t$. Adding the climb time of the light, the tortoise coordinate $r_* = r + 2\ln(r/2 - 1)$, the observer sees the image freeze just outside the horizon, redden, and dim as $1.5\,(1 - 2/r)$. Which clock the dimming is counted in matters by a factor of two. Near the horizon $1 - 2/r \propto r - 2 \propto e^{-t/2}$ in Schwarzschild time, but $\propto e^{-u/4}$ in the observer's retarded time $u = t - r_*$. The code uses the retarded time (`PlungeTrack::seen_at`): the displayed light falls by a factor of $e$ every $4\,M$, which at 10 $M$ per second is 0.4 s. `lens::PLUNGE_FLOOR` = 2.0006 is the radius where the drawn image stops.

![Effective potential, Doppler beaming of a ring, and the plunge in two clocks](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/horizon-potential-redshift-plunge.svg)

*Left: $V = (1-2/r)(1+L^{2}/r^{2})$. Middle: relative intensity $g^{4}$ around a ring at $i = 80^{\circ}$ (no light bending). Right: the plunge from the ISCO ($L = 0.95\,L_\mathrm{isco}$, kick 0.1): $\ln(r - 2M)$ falls with slope $-1/2$ in Schwarzschild time and $-1/4$ in the observer's retarded time. Script: `docs/figures/horizon-potential-redshift-plunge.py`.*

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Orbit radius of a process | rank by `oom_score` among processes that are not kernel threads or zombies, with a readable score (`Source::Oom`); ties by `Identity` | the top 28 ranks (`RANKED`) sit at $r = 6 + 17.5\,(k/27)^{0.8}$ for rank $k = 0$ (the ISCO) to 27 (radii 6.0, 7.25, 8.18, 10.54, 15.75 and 23.5 at $k = 0, 1, 2, 5, 13$ and 27); two neighbours swap only if their scores differ by at least 5 (`SWAP_MARGIN`); the radius eases with a 1.5 s time constant, at most 2 M/s |
| Unkillable band | `oom_score_adj` of -1000 | radius from the identity hash between 25.7 and 27.3 M (`BAND`), at most 12 drawn, labelled `unkillable` |
| Orbital phase and speed | none: the true law, decoration | $\Omega = r^{-3/2}$, 10 M per second of the animation clock; the starting phase from `Rng::for_identity` |
| Body size | `Process.memory` | radius $\mathrm{clamp}(0.12\,\sqrt[3]{\mathrm{MiB}},\,0.18,\,0.9)$ M |
| Body colour | `Process.kind` | `palette::kind_color` |
| Next victim's ring | rank 0 | a red ring of 28 hairline segments round the body, colour (235, 60, 30) |
| Disk brightness | memory pressure `Snapshot.pressure[1]` (10 s average) | gain $0.45 + 0.9\,p/100$ (0.45 to 1.35), the reading eased with a 0.6 s time constant |
| Disk colour and brightness pattern | the physics of the metric | $I = \mathrm{gain}\,F\,g^{4}\,\mathrm{texture}$, colour from `blackbody::color` at $T$ above; colour per channel through $1 - e^{-0.36 I^{1.3}}$ |
| OOM kill | `Snapshot.oom_kills` (`oom_kill` in `/proc/vmstat`), else the sum of `Unit.oom_kills` over cgroups | a rise of the counter claims the best-ranked ghost that vanished within `GHOST_WINDOW` (2.5 s) + `BACKGROUND_PERIOD` (2 s) + one sample interval |
| Plunge | the claimed kill | eases to the ISCO in 0.8 s, then follows `PlungeTrack` ($L = 0.95\,L_\mathrm{isco}$, kick 0.1); brightness $1.5\,(1 - 2/r)$; colour toward (235, 60, 30); a radial streak stretches the body; removed below brightness 0.02 |
| Other exits | exit with no counter rise | the body fades where it stood over 1.2 s (`FADE_SECONDS`) |
| Inclination | the camera's pitch | 86 degrees at the lowest pitch, 80 at the default isometric pitch, 0 (face-on) at top-down, linear between |
| Disk texture, glares, 1,500 stars, the photon ring, the shadow, the dashed ISCO and band guides | none | decorative (the ISCO and band guides are positions of the true orbits, but carry no per-process data) |

## How isotop realises it

**Modules.** `src/horizon.rs` holds `Horizon` (the `ViewModule`: `Roster`, the inclination map, drawing order, legend, popup lines); `src/horizon/lens.rs` is pure physics with no drawing (`LensTable`, `Observer`, `flux`, `one_plus_z`, `disk_emission`, `angular_velocity`, `period`, `PlungeTrack`, `plunge_brightness`); `src/horizon/orbits.rs` is the population without drawing (`Ranking`, `Orbits`, `Ghost`, `rank_radius`, `band_radius`); `src/horizon/disk.rs` cuts the disk, sky, shadow and ring into triangles. The registry row sets `links: LinkRule::DrawnByView` (no sockets are drawn), `flash: false` (the view draws its own exits, so the scene's expanding ring would say it twice), `sources: Sources::of(&[Source::Oom, Source::OomKills])` (scores and the counter are read only while the view is shown), `Backdrop::Void`, demo options `oom_and_memory_detail`, `kernel_counters` and `oom_cycle`, and a headless warm-up that replays 8 s of demo samples. No keys, no digit. There is no plugin system: the view is compiled in.

**Lensing is done on vertices, not pixels.** `LensTable::shared` (built once, on first use, about a fifth of a second on every core in a release build) traces about 1,800 photon paths backwards from the observer by fourth-order Runge-Kutta on the orbit equation at $h = 0.002$ rad, for impact parameters crowding towards $b_c$ from both sides. For each of 360 log-spaced radii (`RADII`, from 2.0005 to 150) it records the sweep at which each photon crosses that radius, inbound and, for photons that turn, outbound; inverting that gives the impact parameter of the ray that joins a disk point at radius $r$ and sweep $\psi$ to the observer, on a grid of 721 sweeps from 0 to $\pi$ (a quarter degree). The deflection of light is an elliptic integral with no elementary closed form, so the table replaces a per-pixel solve, and it depends on fixed constants only, so it is the same on every machine. `disk.rs` then draws ordinary triangles between the images of mesh vertices, so the CPU and GPU rasterizers draw the same frame. The primary image agrees with Beloborodov's closed form (2002) to a few percent at $r \ge 6$, which a test checks.

**The disk.** Facets of about 4 px (`FACET_PIXELS`), the secondary image at half the density (its sides longer by $\sqrt2$), at most 120,000 triangles in both images (`TRIANGLE_CAP`); relensed only when the inclination or scale changes and recoloured every frame. The disk runs from the ISCO (`DISK_INNER` 6) to 30 M (`DISK_OUTER`) and is tapered to nothing over its last 10 M (`disk_flux`). Draw order, back to front: sky, secondary image, shadow (a fan of radius $b_c$, pick `NONE`), photon ring, primary image, the dashed guides, glares, the bodies. The photon ring is a drawn annulus from $b_c + 0.02$ to $b_c + 0.12$ (`RING_OFFSET` 0.07, `RING_WIDTH` 0.10, never thinner than 1.5 px), coloured and brightened by the Doppler and gravitational shift of matter at 7 M (so brighter and bluer on the approaching side), not a computed image of the photon sphere: the true ring is the infinite series of higher-order images, each narrower than the last by roughly $e^{-\pi} = 0.043$. A body's secondary image is drawn at 0.6 of its radius and tinted by 0.6, and only when it lies at least 1.2 M outside the shadow's edge.

**Rank, not value, sets the radius.** `orbits::Ranking` orders processes by `oom_score` and `rank_radius` spaces the ranks, so the kernel's absolute scale does not matter and the radius is a chosen function, not a measurement. Two bodies at 6 M and 7.25 M may differ by one point or by hundreds.

**A kill needs corroboration, and the victim is inferred.** The counter says that a kill happened, never whose. A plunge starts when the counter rises and a process vanishes near it in time. The counter reaches the frame loop from the background thread, so the exit and the rise may land in different samples: `Orbits::classify` keeps vanished processes as fading ghosts and matches a rise before or after an exit within `claim_window()`; the best-ranked ghost in that window becomes the victim, so an unrelated exit close to a real kill can be the one shown plunging. A process that exits with no rise does not plunge. A restart of the counter cannot flood the ghosts: one sample adds at most 8 rises (`MOST_RISES`).

**Camera.** Pitch maps to inclination (`inclination`); camera rotation shifts the disk's azimuth; zoom sets pixels per $M$. PgUp, PgDn, right-drag and zoom drive the camera; nothing is view-local. The camera's bounds follow the disk's extent.

**Demo kill cycle.** `DemoOptions::oom_cycle` runs a 30 s cycle: `worker-134` has score 950 and is the next victim, exits at 20 s into the cycle (the demo `oom_kill` counter rises then) and starts again at 26 s (`DEMO_KILL_PERIOD`, `DEMO_KILL_AT`, `DEMO_RESPAWN_AT` in `src/model.rs`).

**Inspector and labels.** The popup of a body shows `oom_score N | oom_score_adj N`, `rank R of K` (or `unkillable: the kernel never picks it`) and `orbit r = X.XX M | period P M (S s)`. The five best ranks and one unkillable body are labelled; the tour visits bodies best-ranked first.

**Legend.** ` next victim NAME SCORE`, then ` | oom_kill N (vmstat)` or `(cgroups)` when a counter exists, with `status::missing_note` wording appended while it applies (`no oom score: ranked by memory footprint`, `no oom kills: exits not classified` or `kills counted from cgroups`, `no memory pressure: disk at its base brightness`, ` | N unranked`), then ` | orbit radius = oom_score rank (ISCO 6M = next victim) | disk brightness = memory pressure | colour = Doppler + gravitational redshift | far orbit = oom_score_adj -1000 | i = 80 deg` (the inclination in degrees).

**Platforms.** Linux: full. A process whose score cannot be read is excluded and counted as unranked. macOS has no `oom_score`: the sampler offers jetsam priorities only as root, and a score without an adjustment is not treated as a Linux score, so the roster is ranked by memory footprint, there is no unkillable band, and without a kill counter no exit is classified as a kill (all fade). The lensing, disk and pressure still work. This was read from the code; it was not run on a Mac.

## Reading the picture

* **Measured:** the ranking by `oom_score`, memory pressure, the existence and time of an OOM kill (to the counter's resolution), process memory and kind.
* **Derived from the metric, not measured:** the shape of the disk, the shadow, the doubled images, the Doppler brightening and every colour in the disk. They are the correct pictures of an imagined disk and carry no information about the machine. The texture and glare are decorative.
* **Radius is rank.** A body at 6 M has the highest score, not a score of any particular size.
* **The victim is an inference.** The ISCO process is the kernel's likeliest victim by its own heuristic; the real victim may differ (cgroup-level kills, `oom_score_adj` changed since, `oom.group`). When the counter rises and the best-ranked ghost plunges, the view asserts a match that the counter alone does not prove.
* **Freezing is the observer's view.** The process stopped existing at once; the dimming time is not a property of the kill.
* **The ring is a drawing, the shadow is exact** (radius $b_c$ for a Schwarzschild hole). Nothing spins, so there is no frame dragging.
* **Brighter and bluer on the left.** The matter on that side approaches (the disk turns counterclockwise as seen from above). The disk wraps over the top of the shadow even at 80 degrees inclination: that is the primary image of the far side, not a drawing error.
* **Resolution.** At most 28 ranked bodies (plus up to 12 in the band) are drawn.

## Try it

Run here with the release binary (Linux, CPU renderer):

```sh
./target/release/isotop --demo --view horizon --time 12 --width 1600 --output /tmp/horizon.png --renderer cpu
./target/release/isotop --demo --view horizon --time 20.5 --width 1600 --output /tmp/plunge.png --renderer cpu
./target/release/isotop --demo --view horizon --width 1600 --renderer cpu --benchmark 30
```

Output of the first: `Wrote /tmp/horizon.png (1600x900, 31 visible)`; the second wrote the same line for `plunge.png` (the frame is the plunge in progress; I did not inspect it frame by frame). The benchmark printed `30 frames | 1600x900 | 31 visible | 38.07 ms/frame | 26.3 renderer fps on cpu (excludes terminal transport)`, measured on a shared sandbox while other builds were running, so read it as an upper bound. Live: `./target/release/isotop --view horizon` or Tab and the `v` picker (the sixteenth row, no digit). Not run: `--renderer gpu`, and any macOS run.

## Where in the code

| Item | Path and symbol |
|---|---|
| The view | [src/horizon.rs](https://github.com/tfpickard/isotop/blob/main/src/horizon.rs): `Horizon`, `inclination`, `roster`, `kill_counter`, `body_radius` |
| Physics | [src/horizon/lens.rs](https://github.com/tfpickard/isotop/blob/main/src/horizon/lens.rs): `LensTable`, `Observer`, `flux`, `disk_flux`, `one_plus_z`, `PlungeTrack`, `plunge_brightness`, `beloborodov_impact_parameter` |
| Population | [src/horizon/orbits.rs](https://github.com/tfpickard/isotop/blob/main/src/horizon/orbits.rs): `Ranking`, `Orbits`, `Orbits::classify`, `rank_radius`, `band_radius` |
| Mesh | [src/horizon/disk.rs](https://github.com/tfpickard/isotop/blob/main/src/horizon/disk.rs): `Disk`, `Turbulence`, `SkyField`, `draw_shadow`, `draw_ring` |
| Registry row | [src/views.rs](https://github.com/tfpickard/isotop/blob/main/src/views.rs): `REGISTRY` row `horizon` |
| Sources | `Source::Oom`, `Source::OomKills`; `platform/linux/memory.rs`, `platform/linux/counters.rs` |
| Repository theory page | [docs/theory/horizon.md](https://github.com/tfpickard/isotop/blob/main/docs/theory/horizon.md) |
| Prototype (reference only) | [docs/mockups/code/horizon.py](https://github.com/tfpickard/isotop/blob/main/docs/mockups/code/horizon.py) |

## Variations and open questions

* **A trap in the prototype.** The Python prototype's `page_thorne` writes the coefficient of the logarithm as $\sqrt3/3$; integrating the Page-Thorne expression directly gives $\sqrt3/2$. With $\sqrt3/2$ the flux peaks near $9.55\,M$ and is proportional to the integral; with $\sqrt3/3$ it peaks near $8.4\,M$ and rises linearly from the ISCO instead of quadratically. `lens::flux` uses $\sqrt3/2$, pinned by a test against the integral.
* **Departures from the roadmap design.** The design specified a plunge clock without choosing; the code chose the observer's retarded time, so the light falls by $e$ every $4\,M$. The design's `Stage.own_exits` flag does not exist: the row's `flash: false` does that job. The design's unkillable band at golden-angle phases is a radius from an identity hash.
* **Kerr.** A spinning hole moves the ISCO from $6M$ to $M$ (maximal, prograde) and shifts and flattens the shadow. A spin taken from, say, the hottest process's CPU would be arbitrary, which is exactly the problem.
* **A real tidal stretch.** The victim is stretched by a stylised streak; the geodesic-deviation stretch $2GM\ell/r^{3}$ could set its length, so larger processes would stretch more.
* **Hawking radiation.** A black hole's temperature falls as $1/M$; a hole that shrinks as it eats, with the memory freed by the kill as the analogue, is a different model. Nothing here does it.
* **Time to OOM.** The trend of the pressure could predict the next kill and move the victim from the ISCO towards a marginally bound orbit as it rises.
* **Photon ring images.** Draw the first higher-order image at its true $b$ and width from the lens table instead of the annulus.
* **One kill, four renderings.** This view draws an OOM kill as a plunge that never arrives; [View Necropolis](View-Necropolis) records it as a stone with a cause of death; [View Coop](View-Coop) sends a fox; [View Cortex](View-Cortex) shows dust.

## References

* J.-P. Luminet, "Image of a spherical black hole with thin accretion disk", *Astronomy & Astrophysics* 75, 228 (1979).
* D. N. Page and K. S. Thorne, "Disk-accretion onto a black hole. I. Time-averaged structure of accretion disk", *The Astrophysical Journal* 191, 499 (1974).
* A. M. Beloborodov, "Gravitational bending of light near compact objects", *The Astrophysical Journal* 566, L85 (2002).
* O. James, E. von Tunzelmann, P. Franklin and K. S. Thorne, "Gravitational lensing by spinning black holes in astrophysics, and in the movie Interstellar", *Classical and Quantum Gravity* 32, 065001 (2015).
* The Event Horizon Telescope Collaboration, "First M87 Event Horizon Telescope results. I. The shadow of the supermassive black hole", *The Astrophysical Journal Letters* 875, L1 (2019).
* C. W. Misner, K. S. Thorne and J. A. Wheeler, *Gravitation*, W. H. Freeman (1973).
* The Linux kernel documentation for `/proc` (`oom_score`, `oom_score_adj`) and `/proc/vmstat` (`oom_kill`).
* See also [Math Schwarzschild](Math-Schwarzschild), [Theory Background Sampler and Demand](Theory-Background-Sampler-and-Demand).
