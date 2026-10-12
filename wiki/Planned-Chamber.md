> **Planned, not yet implemented.** Specified in [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md) and tracked in issue [#42](https://github.com/tfpickard/isotop/issues/42).

# Planned: Chamber

The `chamber` view is specified as a bubble chamber photograph that keeps developing: a round dark window with fiducial crosses and thin tracks of white bubbles. Each busy process circles its own stable home on a Larmor circle whose radius follows its CPU; processes that wind down curl into tight inward spirals; forks appear as V-shaped vertices, exits as small curling delta rays, and the local timer interrupt as a bundle of straight beam tracks. Tilting the camera shows that every track is a helix. This page is the physics of the instrument (track curvature, ionisation, delta rays, neutral decays) and then the design as written in issue #42. Nothing here is implemented yet: `isotop --list-views` does not list `chamber`.

![Reference mockup of the chamber view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/chamber.jpg)

*The reference mockup (`docs/mockups/chamber.jpg`), rendered in Python by `docs/mockups/code/chamber.py` from synthetic data. It is the visual target, not an isotop render.*

## The system being modelled

A **bubble chamber** is a vessel of liquid held just above its boiling point under pressure. Shortly before an accelerator pulse arrives, a piston expands the liquid and drops its pressure, leaving it superheated: it wants to boil but has no seed. A charged particle crossing it ionises atoms along its path; the freed electrons, and especially the energetic knock-on electrons called delta rays, deposit heat in tiny spots, and each spot nucleates a bubble. The bubbles grow for a few milliseconds while a flash photographs the chamber from several angles; then the liquid is recompressed and the cycle repeats. The photograph is a record of every charged track in the pulse, and a strong magnet around the vessel bends each track in proportion to its momentum. Donald Glaser invented the instrument in 1952 (Nobel Prize in Physics, 1960), and the large hydrogen chambers that Luis Alvarez's group built made the 1950s and 1960s the age of the bubble chamber. The Omega-minus hyperon was identified in a 1964 Brookhaven chamber photograph (Barnes et al.), and CERN's Gargamelle chamber recorded the first neutrino interactions without a charged lepton, the weak neutral currents, in 1973 (Hasert et al.).

What a physicist reads off such a photograph:

* a **charged** particle leaves a track; a **neutral** one leaves nothing, and shows itself only through what it does: a vertex out of nowhere, a "V" of two tracks (a V0);
* the **curvature** of a track gives its momentum, and the **sense** of the curl gives the sign of its charge;
* the **bubble density** along a track gives its ionisation, which at a given momentum depends on its mass, so that density plus curvature identifies the particle;
* a track that **loses energy** curls tighter until it stops: slow electrons make the small tight spirals;
* **delta rays** are short curling spurs that leave a track at its side, and a **vertex** is where something was produced or decayed.

That is also exactly the vocabulary of a process table: busy things that move, quiet things that do not, births and deaths at points, parents and children. The metaphor is therefore a mapping of structure, not of units: nothing about a process has a momentum or a charge in the physical sense, and the view says which measured quantity plays each role.

## Theory

### Track curvature: $p = 0.3\,B\,r$

A particle of charge $q$ and velocity $\mathbf{v}$ in a magnetic field $\mathbf{B}$ feels the Lorentz force $\mathbf{F} = q\,\mathbf{v} \times \mathbf{B}$. The force is perpendicular to the velocity, so it does no work: $\lvert\mathbf{p}\rvert$ and the speed stay constant and only the direction turns. For motion perpendicular to $\mathbf{B}$ the momentum vector rotates at the angular rate $\omega = v/r$ on a circle of radius $r$, so $\lvert d\mathbf{p}/dt\rvert = p\,v/r$, and equating it to the force $q v B$ gives

$$
p = q\,B\,r .
$$

This holds relativistically because $\mathbf{p} = \gamma m \mathbf{v}$ is the momentum that appears in $d\mathbf{p}/dt = \mathbf{F}$. In SI units $p$ is in kg m/s. To use particle-physics units, note that 1 GeV/$c$ is $10^{9}\,e\,\mathrm{V}/c = 5.344\times10^{-19}$ kg m/s, so for $q = e$

$$
p\,[\mathrm{GeV}/c] = \frac{e}{5.344\times10^{-19}}\,B\,r = 0.29979\;B\,[\mathrm{T}]\;r\,[\mathrm{m}] \approx 0.3\,B\,r .
$$

A 1 GeV/$c$ particle in a 1.5 T field circles with $r = 1/(0.3 \times 1.5) = 2.22$ m. Only the component of momentum perpendicular to $\mathbf{B}$ curls: with a dip angle $\lambda$ between the momentum and the plane perpendicular to $\mathbf{B}$, $r = p\cos\lambda/(0.3B)$, and the track is a helix of pitch $2\pi r \tan\lambda$ along $\mathbf{B}$. With $\mathbf{B}$ out of the page, a positive charge circulates clockwise and a negative one anticlockwise; the sense is the sign of the charge. The sagitta of a track segment of chord $L$ is $s \approx L^2/(8r) = 0.3\,B L^2/(8p)$, which is how a measurement of a short arc becomes a momentum.

![Radius against momentum for three fields, opposite curl for opposite charges, and a helix](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/chamber-larmor-helix.svg)

*Left: $r = p/(0.3B)$ for B = 0.5, 1.5 and 3.5 T. Middle: B out of the page; blue is q = +1 (clockwise), orange is q = -1 (anticlockwise); radius is proportional to $p$. Right: with a longitudinal momentum the circle becomes a helix. Script: `docs/figures/chamber-larmor-helix.py`.*

### Ionisation, and what a bubble counts

The mean energy a heavy charged particle (charge $e$, mass $M$) loses to ionisation per unit length is given by the Bethe formula,

$$
-\frac{dE}{dx} = K\,\frac{Z}{A}\,\frac{\rho}{\beta^{2}}\left[\frac{1}{2}\ln\frac{2 m_e c^{2}\beta^{2}\gamma^{2}\,T_{\max}}{I^{2}} - \beta^{2} - \frac{\delta}{2}\right],
$$

with $K = 0.307075$ MeV cm$^2$/mol, $T_{\max} = 2 m_e c^2\beta^2\gamma^2/(1 + 2\gamma m_e/M + (m_e/M)^2)$ the largest energy one collision can transfer to an electron, $I$ the mean excitation energy of the medium and $\delta$ the density-effect correction. Three regimes matter for a picture:

* At low speed the factor $1/\beta^2$ dominates: a slow particle spends longer near each atom and ionises more. This is why a slowing track ends in a dense, dark blob.
* Near $\beta\gamma \approx 3$ to $4$ the loss has a shallow minimum ("minimum ionising").
* At high speed the logarithm gives a slow relativistic rise, which in dense media saturates through $\delta$.

For liquid hydrogen ($Z/A = 0.99212$ mol/g, $I = 21.8$ eV, $\rho = 0.0708$ g/cm$^3$), and with $\delta$ left out, I computed a minimum of 4.06 MeV cm$^2$/g at $\beta\gamma = 3.5$, which is 0.288 MeV/cm. The bubbles are produced by the ionisation, so their number per unit length rises and falls with this curve (the proportionality to the *mean* loss is approximate: bubbles come from the energy deposited locally, and delta rays add more). A minimum-ionising track in liquid hydrogen carries of the order of ten bubbles per centimetre (unverified, from memory).

At **fixed momentum**, mass sets $\beta$ and so the density. At $p = 0.5$ GeV/$c$ the mean loss is 0.288, 0.438 and 0.905 MeV/cm for a pion, a kaon and a proton respectively: three times the proton, which is how the combination of curvature and bubble count separates particles.

![Bethe loss against beta gamma compared with 1/beta^2, and loss against momentum for pion, kaon and proton](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/chamber-bethe.svg)

*Left: the Bethe loss relative to its minimum (blue) and the view's specified bubble density $\min(1/\beta^2, 8)$ (orange, dashed). The clamped $1/\beta^2$ keeps the low-speed rise and drops the minimum and the relativistic rise. Right: loss against momentum for three masses. Script: `docs/figures/chamber-bethe.py`. Formula as printed; shell and density corrections left out.*

### Delta rays and spirals

A collision can hand an electron an energy up to $T_{\max}$; the spectrum of knock-on electrons falls roughly as $1/T^2$ (Rutherford scattering), so most are slow. A delta ray is a free electron: it is light, so $p = 0.3 B r$ gives a small radius, and it loses energy to the liquid until it stops. Because $r \propto p$ and $p$ falls along the path, the track is not a circle but a spiral that tightens as it goes. I integrated a 2 MeV electron in liquid hydrogen at 1.5 T with the electron form of the collision stopping power (radiative loss negligible, density correction omitted): it starts with $r = 0.55$ cm and stops after 6.8 cm of path. The right panel of the figure below integrates the *view's* specified track equations for a process whose CPU decays, and shows the same shape. This is the physical reason the view's "winding down" processes spiral inward without being scripted to.

![A delta ray spiralling to rest in liquid hydrogen, and a process track spiralling toward its home as its CPU falls](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/chamber-spirals.svg)

*Script: `docs/figures/chamber-spirals.py`. Left, physics (anticlockwise: negative charge, B out of the page). Right, the specified integration of issue #42 with CPU = 0.05 + 1.6 exp(-t/14 s) and 1 GiB.*

### Neutral decays: V0 and Armenteros-Podolanski

A neutral particle leaves no track. A neutral kaon $K^0_S$ (mass 497.611 MeV/$c^2$, $c\tau = 2.684$ cm) or a $\Lambda$ (1115.683 MeV/$c^2$, $c\tau = 7.89$ cm) flies unseen until it decays into two charged daughters at a point, and the photograph shows a "V" whose apex touches nothing. The probability that it has not decayed after a distance $L$ is $\exp\!\left(-L/(\beta\gamma c\tau)\right)$: at 2 GeV/$c$ the mean decay lengths are 10.8 cm for the kaon and 14.1 cm for the $\Lambda$. The two daughters leave the vertex with opposite charges, so they curl in opposite senses. In the two-body decay $M \to m_1 + m_2$ the daughters carry momentum $p^* = \sqrt{\left(M^2 - (m_1+m_2)^2\right)\left(M^2 - (m_1-m_2)^2\right)}/(2M)$ in the parent's rest frame: 206.0 MeV/$c$ for $K^0_S \to \pi^+\pi^-$ and 100.6 MeV/$c$ for $\Lambda \to p\pi^-$. The Armenteros-Podolanski plot uses two lab variables, $\alpha = (p_L^+ - p_L^-)/(p_L^+ + p_L^-)$ and the transverse momentum $q_T$ of one daughter relative to the parent direction. Each decay type fills an ellipse centred on $\alpha_0 = (m_1^2 - m_2^2)/M^2$ with $q_T$ at most $p^*$, so three clouds separate $K^0_S$, $\Lambda$ and $\bar\Lambda$ without knowing the parent's momentum.

![K0 decay into a V, the Armenteros-Podolanski plane and the survival probability of a neutral parent](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/chamber-v0.svg)

*Script: `docs/figures/chamber-v0.py`. Left: a 0.4 GeV/$c$ $K^0_S$ decaying at the origin in 1.5 T. Middle: the three ellipses at a parent momentum of 2 GeV/$c$. Right: survival probability, with the mean decay lengths. Masses and lifetimes from memory of the standard tables.*

## Mapping

The view is specified so that each visual element is a measurement or is declared decorative. The measured sources are `Process` and `Snapshot` fields; building block B4 is not on `main` yet.

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Guiding centre of a track (its home) | the process's stable seat, `layout::Seats` | sunflower point at radius $0.85R\sqrt{(\mathrm{seat}+0.5)/\mathrm{capacity}}$, $R = 40$ units; capacity is the next power of two at least $\max(256,\mathrm{span})$ |
| Whether there is a track | smoothed `Process.cpu` | below 0.10 cores the process is *neutral* and leaves none; the legend counts the neutrals |
| Larmor radius $r$ | CPU as the momentum $p$ | $r = 10.5\,p$ units per core, clamped to $[0.4,\,0.45R]$ |
| Sense of the curl (charge sign) | `Process.system_share` (utime/stime split, P5) | $+$ when the last interval's user time was at least its system time, otherwise $-$; when both are 0 the previous sign is kept |
| Speed along the track | CPU and memory | $\beta = p/\sqrt{p^2+m^2}$, $m = 1.2$ per GiB of `Process.memory`; speed $= 6\beta$ units/s |
| Bubble density per unit length | $\beta$ | $3.8\,\min(1/\beta^2,\,8)$ per unit |
| Helix pitch (visible when tilted) | memory growth rate $g$ in MiB/s since the previous sample | $dz/dt = K_Z\,\mathrm{sign}(g)\sqrt{\lvert g\rvert}$, $\lvert dz/dt\rvert \le 0.5\,v$, $z$ clamped to $\pm 0.25R$ |
| Fork vertex | fork events (`Snapshot::life`, B3) | child's track starts at the parent's track head; parent's heading kinks by 0.35 rad; a neutral parent gives a V0 |
| Exec marker | exec events | a ring that fades over 3 s |
| Delta rays at an exit | exit events and `Process.threads` at the last sample | one negative-curling spiral per thread, capped at 16 |
| Beam tracks | local timer interrupt delta (B4: `LOC` on x86, the timer row on arm64) | $\mathrm{round}(\Delta/4000)$ per tick, capped at 8 |
| Cosmic tracks | all other interrupts (B4) | one per 20,000, capped at 4, remainder carried |
| Bubble flash and fade | the sample tick | one expansion per sample; bubbles brightness $\sqrt{\mathrm{clamp}(\mathrm{age}/0.2,\,0.15,\,1)}\,e^{-\mathrm{age}/2.6}$; dropped at 12 s |
| Scan sheet (top 11 tracks: q, p, beta, r) | the same figures | listed through `stage.places`, follows the global `l` toggle |
| Film grain, flange, fiducial crosses, frame counter | none | **decorative** |

The figure below plots the mapping from CPU and memory to radius, speed and bubble density, and shows where each clamp binds: the radius clamp at 1.71 cores, and the density clamp below 0.45 cores for a 1 GiB process and below 1.81 cores for a 4 GiB process.

![Larmor radius, beta and bubble density against CPU for three memory sizes](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/chamber-mapping.svg)

*Script: `docs/figures/chamber-mapping.py`; grey band: neutral, below 0.10 cores.*

## Design

Everything in this section is the specification of issue #42; none of it exists in the source.

**Module and registry.** A new module `src/chamber.rs` with `#[derive(Default)] pub struct Chamber` implementing `views::ViewModule`, and one registry row: `Backdrop::Void`, pressure haze off, links drawn by the view (none), the generic birth and death flash off, default pitch top-down. It declares the sources B3 (`Source::LifeEvents`) and B4 through `sources`, so they are read only while the view is shown. There is no plugin system: views are compile-time modules.

**Dependencies.** Status at commit `192299b`:

| Item | Provides | Status |
|---|---|---|
| P1 (#17), P2 (#18) | registry row, picker, smoke list | merged |
| P5 (#21) | `Process.system_share` (utime/stime split), `syscall_io` | merged |
| B3 (#30) | fork, exec and exit events (`Snapshot::life`, `Source::LifeEvents`, `isotop-events` thread) | merged |
| S1 (#22), S2 (#23), S4 (#25), S6 (#27) | parity check, `FixedStep`, demo births and exits, source demand | merged |
| P3 (#19) | field primitive for the window backdrop | open |
| B4 (#31) | `/proc/interrupts`, `/proc/softirqs`, `oom_kill` | open |

**Simulation.** `simulation::FixedStep` at H = 1/120 s on wall-clock time. Each step turns the heading of every track by $-s\,v/r\,H$ ($s = \pm1$ the sign of the charge), advances the position by $v\,H$ along the heading, and relaxes the guiding centre toward the home at rate $K_\mathrm{home} = 0.6/\mathrm{s}$. Because the sign and radius are recomputed from the current CPU, a falling CPU shrinks $r$ and the track spirals in without any scripted path. At most 96 tracks are integrated, chosen by momentum; the legend counts the rest as `N over cap`. A test asserts the same path at 10 and at 60 frames per second.

**Stable homes.** Every admitted process, neutral ones included, gets a `layout::Seats` seat. The issue writes `pack::Seats` and `render::GOLDEN_ANGLE`; in the tree these are `layout::Seats` and `layout::GOLDEN_ANGLE`. The home is the sunflower (golden-angle) point of its seat, so a process keeps its place when CPU, memory or the population change.

**Forks, exits, execs.** On a fork the child's track starts at the parent's track head and relaxes to the child's home; the parent's heading kinks by 0.35 rad away from the child. If the parent was neutral the vertex has no incoming track: a V0. An exec draws a ring marker. An exit emits one delta ray per thread from `Process.threads` at the last sample, capped at 16, each a negative-curling spiral seeded from the `Identity` (`Rng::for_identity`).

**Bubbles.** At each sample tick bubbles form along the motion since the previous tick, using at most the last 2 s of motion. Each is a world-space white `Glow` of 1.5 to 3 px. At most 40,000 are kept, oldest dropped first; one window-sized `Glow` of 0.25 s is the expansion flash.

**Rendering and picking.** The backdrop is one P3 field (grain plus a liquid gradient), not triangles. The head of every charged track is a small `Sphere` whose pick is the process index; neutral processes are not drawn and cannot be picked. The view-local key `p` (through the registry key hook) switches the window to a printed look (paper grey, dark bubbles), and appears on the `?` help line. The global `t` key tilts the camera and so reveals the helices; it is unchanged.

**Degradation.**

* Linux: full. The event sources are whatever B3 reports (proc connector, taskstats, or sample diffing); the status line names the active ones.
* macOS: available, degraded. Life events come from sample diffing, and there are no interrupt counters, so there are no beam or cosmic tracks; the legend shows `no beam: macOS has no interrupt counters`. Live values are never synthesized.

**Demo.** With S4 churn on the registry row, at `--demo --time 12` the frame must contain the utime/stime split, interrupt deltas, a fork from a neutral parent, an exit of a process with 4 or more threads, and one process whose CPU decays through the neutral threshold.

**Legend** (specified): ` N tracks | M neutral | beam K = X LOC/s | <sources> | track = busy process circling its home | radius = CPU | curl + user, - system | bubbles ~ 1/beta^2 | V = fork | curls = exit`.

## Reading the picture

* **Momentum is CPU.** The radius is a scaled CPU load clamped to a fixed range; it is not an energy. The clamp at 18 units hides every difference above 1.7 cores.
* **Charge is a coarse dichotomy.** The sense of the curl says whether the last interval was user-dominated or system-dominated, and flips between samples when the two are close.
* **A missing track is information too.** The `M neutral` count includes every process below 0.10 cores, which on an ordinary machine is nearly all of them. Neutral parents of forks reveal themselves only by a vertex.
* **Bubble density is relative.** It encodes $\beta$, hence CPU relative to memory: a large idle process leaves a dense track, a small busy one a sparse track.
* **Delta ray counts are thread counts** at the last sample before the exit, not at the moment of death.
* **Timing is quantised by the sample interval.** Bubbles form once per tick over at most the last 2 s of motion; events between samples reach the view through B3 (exact on Linux with the proc connector, inferred by diffing otherwise).
* **Pitch is rare and small.** It is visible only when the camera is tilted and only for processes whose memory changes.

## Try it

None of these work yet: `chamber` is not registered.

```sh
# Once the view lands (issue #42).
./target/release/isotop --demo --view chamber --time 12 --width 1600 --output /tmp/chamber.png
./target/release/isotop --demo --view chamber --renderer cpu --width 1920 --benchmark 120
```

Keys planned: `p` toggles the printed (negative) look, `t` tilts, `l` toggles labels. The figures regenerate from the repository root with `python3 docs/figures/chamber-larmor-helix.py` and the other `chamber-*.py` scripts.

## Where in the code

* Existing: `src/views.rs` (`REGISTRY`, `ViewModule`), `src/simulation.rs` (`FixedStep`, `Rng::for_identity`), `src/layout.rs` (`Seats`, `GOLDEN_ANGLE`), `src/render.rs` (`Stage`, `Item::Glow`, `Item::Sphere`, `ring_bounds`), `src/model.rs` (`Process.system_share`, `Process.threads`, `Snapshot::life`), `src/platform/mod.rs` (`LifeEvent`, `Cause`, `ExitRecord`, `LifeQueue`), `src/platform/linux/events.rs` and `src/platform/macos/events.rs` (the `isotop-events` thread).
* Planned: `src/chamber.rs`, a registry row, building blocks P3 and B4.
* Prototype, reference only: `docs/mockups/code/chamber.py` (constants and integration scheme).

## Variations and open questions

* **A momentum measurement from the picture.** Fit a circle to a short arc of a drawn track and recover CPU from $r$: how well does the visual encoding survive the clamp and the sample interval?
* **A field that breathes.** B could follow the load average, shrinking all radii when the machine is busy.
* **Pair production.** A parent that exits at the moment two children are born could draw an $e^+e^-$ pair, the classic gamma-conversion spirals.
* **An identification plot.** A scan-sheet panel of bubble density against momentum, one point per track, would reproduce the particle-identification plot in which families fall on separate curves of constant mass.
* **Armenteros-Podolanski for forks.** Plot parent and child CPU in the same variables and see whether `make` children cluster like a decay.
* **Delta ray energy spectrum.** The view draws one ray per thread; a spectrum $\propto 1/T^2$ would make many short spurs and a few long ones.

## References

* D. A. Glaser, "Some effects of ionizing radiation on the formation of bubbles in liquids", *Physical Review* 87, 665 (1952).
* V. E. Barnes et al., "Observation of a hyperon with strangeness minus three", *Physical Review Letters* 12, 204 (1964).
* F. J. Hasert et al., "Observation of neutrino-like interactions without muon or electron in the Gargamelle neutrino experiment", *Physics Letters B* 46, 138 (1973).
* Particle Data Group, "Passage of particles through matter" (Review of Particle Physics): the Bethe formula, $T_{\max}$ and the electron stopping power used above.
* J. Podolanski, R. Armenteros, "Analysis of V-events", *Philosophical Magazine* 45, 13 (1954).
