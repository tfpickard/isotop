# View: Orbit

The process tree as an observatory. A process with children is a **star** and its children are **planets** circling it on Keplerian ellipses with the parent at one focus; a planet with children of its own is a star in turn, with moons. Bodies are sized by memory (the volume of a star carries the whole subtree's), ringed by threads, haloed by CPU and GPU use, and coloured by what kind of process they are. Branches that hang off a top-level process (`init`, `kthreadd`) become separate solar systems packed around the middle of the scene, and the whole galaxy turns slowly about its memory-weighted barycentre. It is the second view (`2`).

![Orbit, a fresh render from the demo workload](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/orbit.webp)

![Orbit with its status lines, as it looks in the terminal](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/orbit.webp)

Orbit shares its layout with [Flow](View-Flow): Flow calls the same `systems::layout` with the motion held at zero.

The honest summary, before the detail: the **shape** of this view is real (the tree, the sizes, the rings, the glows) and its **motion** uses the right physics (Kepler's equation, the third law for the period), but the **orbits themselves**, their radii, eccentricities, orientations and the order of the planets on them, are packing and decoration. Nothing is attracted to anything. This page separates the two.

## The system being modelled

A hierarchical gravitational system: the Sun with its planets, a planet with its moons, a star with its planets and a distant companion. Hierarchy is what makes the picture work. The process table is a tree (parent and child by `ppid`), and a tree of dominance is what a hierarchical orbital system is: each body moves under the pull of the one it orbits and, to first approximation, ignores the rest. The approximation that makes the Solar System analysable (each planet moves on a Kepler orbit about the Sun, perturbed only slightly by the others) is the approximation Orbit uses everywhere, to the point of the limit: every child is a **test particle** that does not affect its parent or its siblings.

The metaphor works for process data in a real sense: memory in the subtree is a plausible measure of how "heavy" a parent is, and a parent with a heavy tree has many children whose lives it dominates. It fails in another real sense: processes do not attract, children do not feel their parent's mass, and a child can outweigh its parent (a shell that launches a 20 GiB process). The view says what it says without pretending otherwise.

## History

The thing Orbit draws is the Keplerian system, and the machine that shows it, the orrery.

* Before Kepler the planets rode circles. Ptolemy's epicycles (see [Ptolemy](View-Ptolemy)) and Copernicus's heliocentric rearrangement of 1543 both kept them, and Kepler's *Mysterium cosmographicum* (1596) still tried to nest the five Platonic solids between the planetary spheres.
* Kepler's first two laws appear in *Astronomia nova* (1609), worked out from Tycho Brahe's observations of Mars. The third, $T^2 \propto a^3$, is in *Harmonices mundi* (1619); several sources give the date of the discovery as 15 May 1618. How he arrived at it is described in the literature as somewhat mysterious.
* Newton's *Principia* (1687) derived all three from an inverse-square attraction, which turned a description of the planets into a consequence of a force. The restriction Orbit uses, bodies that feel their parent and nothing else, is the test-particle limit of that derivation.
* Mechanical models of the heavens are older than the laws. The Antikythera mechanism, recovered from a wreck off the Greek island in 1901, dates from the second or first century BCE and gears the Sun and Moon with some precision. Planetaria with clockwork planets were built in the seventeenth century (Huygens's of 1682 is often cited, `(unverified)` here).
* The word *orrery* is English and comes from a title. About 1710 the instrument maker John Rowley of London made a copy of a design by George Graham for Charles Boyle, 4th Earl of Orrery, and the device took his name. Graham had also had a copy made for Prince Eugene of Savoy. A history-of-science museum in Oxford notes that Boyle performed no seminal experiments and has no scientific discovery to his name; his fame rested on his collection of instruments. The name survived the more accurate machines that followed. One source calls the maker James Rowley and another John, and the dates given for Boyle's birth differ by two years (1674 or 1676).
* Orbit keeps the orrery's sins: radii and periods are chosen so the machine fits on a table, not to scale, and the planets never perturb each other.

## Theory

### Kepler's laws

Kepler (*Astronomia nova*, 1609; *Harmonices mundi*, 1619) found, from Tycho's observations of Mars and the planets, that

1. each planet moves on an **ellipse with the Sun at one focus**;
2. the line from the Sun to the planet sweeps **equal areas in equal times**;
3. the square of the period is proportional to the cube of the semi-major axis, $T^2 \propto a^3$.

Newton (*Principia*, 1687) derived all three from an inverse-square attraction. For a body of negligible mass in orbit about a mass $M$, with $\mu = GM$:

$$
T = 2\pi\sqrt{\frac{a^3}{\mu}}, \qquad v^2 = \mu\Big(\frac{2}{r} - \frac1a\Big) \quad(\text{vis-viva}).
$$

For an ellipse of semi-major axis $a$ and eccentricity $e$ with the central body at a focus, the distance runs between periapsis $a(1-e)$ and apoapsis $a(1+e)$. From vis-viva the speeds there are $v_p = \sqrt{\mu(1+e)/(a(1-e))}$ and $v_a = \sqrt{\mu(1-e)/(a(1+e))}$: their ratio $(1+e)/(1-e)$ is $1.14$ at the smallest eccentricity isotop draws and $1.56$ at the largest.

### Kepler's equation

Let $b = a\sqrt{1-e^2}$ and let $E$ be the **eccentric anomaly**: the angle, at the centre of the ellipse, of the point's projection on the circumscribing circle of radius $a$. Then the position relative to the focus is

$$
x = a(\cos E - e), \qquad y = b\sin E, \qquad r = a(1 - e\cos E),
$$

and the area swept from periapsis is the focal sector of the ellipse, $\tfrac12 ab\,(E - e\sin E)$ (an affine image of a circular sector minus a triangle). The second law says that area grows linearly in time, and the whole ellipse $\pi ab$ is swept in one period, so with the **mean anomaly** $M = 2\pi (t-\tau)/T$,

$$
M = E - e\sin E .
$$

There is no closed form for $E(M)$. Newton's method on $f(E) = E - e\sin E - M$ with $f' = 1 - e\cos E$ converges quadratically from $E_0 = M$ for the small $e$ used here.

### Packing bodies on an ellipse

Put $n$ test bodies on one ellipse **evenly spaced in $M$**. They then keep their order for ever (they all have the same period, so $M$ differences are constant), but Kepler's second law bunches them: spatial spacing is $v\,\Delta t$, smallest at apoapsis where $v_a = n a\sqrt{(1-e)/(1+e)}$ with $n = 2\pi/T$. So a shell of $n$ bodies each needing an arc $s_i$ fits if $\sum s_i \le 2\pi a\sqrt{(1-e)/(1+e)}$ (the circumference *as if the whole orbit were travelled at the apoapsis speed*). That is exactly the `capacity` in `shells`.

Ellipses with the same $e$ and the same orientation $\omega$ and different $a$ are **scaled copies** of each other about the common focus: $r(\theta) = a\,(1-e^2)/(1 + e\cos(\theta-\omega))$ is proportional to $a$ at every $\theta$. They never cross, so shells of different $a$ cannot collide, however the bodies move.

### The third law as a layout knob

For orbits about a mass $M$: $T \propto a^{3/2} M^{-1/2}$. The Orbit view uses it with $M$ the parent's **subtree memory**, normalised so that $a = 3$ about a 512 MiB parent has $T = 30$ s:

$$
T(a, M) = \mathrm{clamp}\!\Big(30\,\Big(\frac{a}{3}\Big)^{3/2}\Big/\sqrt{M/512\,\mathrm{MiB}},\ 8\ \mathrm{s},\ 600\ \mathrm{s}\Big),\qquad M \ge 1\,\mathrm{MiB}.
$$

### Hohmann and other transfers: not used

Hohmann (1925) found the least-energy transfer between two circular coplanar orbits: an ellipse tangent to both, with semi-major axis $(r_1+r_2)/2$ and two burns. It would be the right model for a *newborn planet moving from its parent's position to its shell*; isotop does not do it. A newborn flies out in a straight line scaled by a cubic ease-out (below). Nothing in the code changes a body's orbit once it has one.

## Mapping

| Visual element | Measured quantity (platform source) | Transform | Status |
|---|---|---|---|
| Body (sphere) | one process | impostor of radius `mass_radius(subtree memory) * grow` | measured |
| Parent and child | `ppid` | child orbits parent at one focus; children of a top-level process with children of their own become separate systems | measured (macOS: the "responsible" app) |
| Body radius | resident memory of the **whole subtree** the body anchors (`weigh`; RSS on Linux, footprint on macOS), eased | $0.24\sqrt[3]{\mathrm{MiB}}$ clamped to 0.25 to 4.5 world units: volume proportional to memory | measured; saturates at 6.4 GiB |
| Colour | kind: kernel slate, system teal, session amber, container violet (cgroup path markers, `PF_KTHREAD`; macOS by uid and path); zombie, stopped, blocked override | `kind_color`: base times $0.86 + 0.24\,j$ with $j$ from the pid | measured kind; the brightness jitter is decorative |
| Saturn rings | thread count (`/proc/<pid>/stat` field 20) | none below 4; then 1, 2, 3, 4 rings at 4, 16, 64, 256 threads (one more per fourfold); ring $k$ at radius $r(1.45 + 0.28k)$, in a plane tilted 0.45 rad towards the viewer | measured count; the ring's azimuth (hash of the pid) is decorative |
| Warm glow | CPU | when above 1%: radius $\mathrm{core}\,(2.2 + 2\,u/(u+50))$, strength $0.3 + 0.6\,u/(u+40)$ | measured |
| Comet trail | CPU | when above 3%: $2 + \lfloor 14\,u/(u+40)\rfloor$ segments back along the orbit, in steps of $\min(0.6/a, 0.25)$ of mean anomaly | measured |
| Green glow and halo | NVIDIA per-process GPU memory (NVML) | glow radius $\mathrm{core}\,(2.6 + 0.5\ln(1+\mathrm{MiB}/64))$ and a ring at 1.9 radii | measured; absent on macOS |
| Orbit semi-major axis $a$ | none | from packing: `shells` | **decorative** (layout) |
| Eccentricity $e$, orientation $\omega$ | none | $e = 0.06 + 0.16\,u$, $u$ from `knuth(parent pid)`; $\omega = 2\pi\cdot$ other bits | **decorative** |
| Orbital period | parent's subtree memory and $a$ | the third-law formula above, clamped | derived: heavier tree gives faster children, but clamped and normalised |
| Which shell, which phase | pid order | branches first, then by `Identity`; evenly spaced in $M$ from the shell's start | **decorative** (arrival order) |
| System position | none | `Galaxy::place`: golden-angle spiral, largest first; then the galaxy turns | **decorative** |
| Galaxy turn | enclosed subtree memory | `galactic_period`, 120 to 900 s | derived, clamped |
| Arcs and beams | socket links, connections leaving the machine | as in the other views | measured |
| Spawn and exit | a process appearing or leaving | a child flies out of its parent over 0.9 s; an exit leaves an expanding ring (1.2 s) | measured events |

## How isotop realises it

Orbit is a `ViewModule`, the `Orbits` struct in [`src/views/orbit.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/orbit.rs); its registry row in `views::REGISTRY` builds it with a closure. It owns a `Galaxy` (where the systems stand and how they turn) and draws what `systems::layout` returns; the layout itself, shared with Flow, is in [`src/views/systems.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/systems.rs). `Orbits::draw` calls `systems::hubs`, then `systems::layout`, then `paint`, then `Layout::report` (tour stops, titles and the popup line of the memory a system holds). `Scene` holds none of this state.

**The tree.** `layout` finds each process's parent among the admitted ones. A process with a parent in the set becomes that parent's child **if it is a leaf or its parent itself has a parent**; otherwise it becomes a root. So a branch hanging off a top-level process (a service with children under `init`) is its own system, and the leaves directly under a top-level process stay in orbit around it, which is why kernel threads form one ringed system around `kthreadd`. Children are ordered by (is a leaf, `Identity`): branches take the inner shells, then leaves in pid order. Depth is capped by `MAX_DEPTH` = 3, so four levels are drawn; deeper processes count as collapsed.

**Sizes.** `weigh` sums memory and process count over each subtree (`Layout::mass` for stars). `radii` follow from `mass_radius`; `reach` multiplies the radius by `ring_scale(threads)` = $1.55 + 0.28(\text{rings}-1)$ so that neighbours clear the rings. `measure` computes each body's `extent`: a leaf's is its `reach`; a parent's is the `reach` returned by `shells` over its children's extents.

**Shells.** `elements(pid)` hashes the **parent's** pid into $e \in [0.06, 0.22]$ and $\omega$, shared by all that parent's shells. `shells(body, sizes, e)` then packs the children: the first shell's semi-major axis puts periapsis at $a(1-e) = \mathrm{reach}_\mathrm{body} + \max(\text{extent}) + 0.6 + 1$; bodies fill a shell in order while $\sum (2\,\mathrm{extent}_i + 0.6) \le 2\pi a\sqrt{(1-e)/(1+e)}$; the slack is shared out so bodies are evenly spaced in mean anomaly; the next shell's $a$ grows so that the **periapsis** gap is $\mathrm{extent}_\mathrm{widest\ here} + \mathrm{extent}_\mathrm{widest\ after} + 0.6$ (divided by $1-e$ for $a$). Siblings on a shell share one ellipse; each shell starts 0.5 rad of mean anomaly further round than the one inside it (`shell * 0.5`). The test `kepler_solution_and_shells_keep_bodies_apart` checks Newton's solution, periapsis at $a(1-e)$ and the gaps.

**Systems.** `Galaxy::place` needs a disc of radius `extent + SYSTEM_GAP` (2.5) per root and reserves 1.08 times that; a system keeps its place while its need stays within its reservation and is otherwise placed again with `layout::vacant`, the first point on a golden-angle spiral (step $\max(0.4R, 1)$) where its disc clears all the others; the largest are placed first, ties by `Identity`. Systems whose root is hidden by focus or `--limit` keep their places (test `focusing_and_clearing_focus_keeps_every_orbit_system_in_place`).

**Motion.** `Walk::branch` positions each child at `mean = phase + 2π·motion/period(a, parent_mass)` and solves Kepler's equation by `eccentric_anomaly` (six Newton steps from $E=M$). The child's orbit centre is the parent's *drawn* position, so a moon follows its planet. A newborn's drawn position is the parent's plus `(settled - target) * child_grow` with the cubic ease-out of `simulation::growth` over 0.9 s.

**The galaxy.** `Galaxy::revolve` turns the systems about the memory-weighted centroid of the drawn systems' centres, taken when systems come or go. Systems whose annuli $[d - R, d + R]$ about that point overlap form a **band**, which turns rigidly (so no two members can collide). Bands are ordered from the inside; each has angular speed $2\pi/T_\mathrm{gal}$ with `galactic_period(radius, enclosed)` $= \mathrm{clamp}\big(240\,(r/30)^{3/2}/\sqrt{M_\mathrm{enc}/4\,\mathrm{GiB}},\ 120,\ 900\big)$ seconds, and an outer band never turns faster than the one inside it. Phase accumulates, so a band changing speed never makes a system jump; the galaxy turns only between consecutive frames and by at most `MAX_TURN_STEP` = 2 s of motion, so a pause, a rewind or a view switch never spins it (test `the_galaxy_turns_at_one_frame_per_second_but_not_across_view_switches`).

**Hubs.** `systems::hubs` makes a hollow star (`HUB_START` identity, an outline, no measurement) for a parent that isotop cannot read (on macOS without root, `launchd`), so its readable children circle it instead of each standing alone; the legend adds "hollow star = a parent isotop can't read (run with sudo)". A hub neither dies nor splashes.

**Time.** Orbit passes `stage.time` as the `motion` of the layout and steps nothing with `stage.dt`. Orbits and the galaxy therefore run at the frame clock and stop when it stops.

**Drawing.** Orbit ellipses are polylines through `Frame::trace`, one per distinct (centre, $a$) (deduplicated at 1/8 unit), tinted by depth; a spoke joins a parent to a child that has children. CPU trails are lines along the orbit; bodies are sphere impostors with the thread rings as 48-segment lines in a tilted plane. Lines clear the pick buffer where they draw, so the spheres, which are drawn after the lines, are what is clicked; `positions[id]` is the settled target, so labels and the tour do not chase a flying newborn.

**CPU/GPU.** Spheres, glows, lines: nothing special; the shared display list keeps parity.

**Stability.** The row waives no property: `known_unstable` is empty for Orbit. The harness checks the **home** of each process, the centre of its system, which `Orbits` returns from `ViewModule::anchors` (test-only), because the orbit a body travels follows memory and its siblings by design (radii follow memory, the radii size the extents, the extents size the discs). A system is placed again only when its extent outgrows the reservation, which is `HEADROOM` = 1.3 of what it needs, and that absorbs the harness's changes to memory and population.

**What degrades.** On macOS: no GPU halos (the legend leaves out "green = NVIDIA GPU"), no `D` state, hollow stars for unreadable parents, parents by "responsible app". Where cgroups are missing, kind falls back to the platform's own classification. No optional source is declared.

## Reading the picture

What the demo render shows (`--demo --view orbit --width 1600 --output ... --renderer cpu`): twelve solar systems, one per demo family (a head process with fifteen children), arranged on a golden-angle spiral with the biggest in the middle. Each has a central star, one or two elliptical shells with its planets spaced round them, planets of visibly different sizes, Saturn-style rings on the more threaded ones, orange-yellow glows (and short comet trails) on the busy ones, thin green ellipses where the demo's fake GPU processes are, and cyan arcs and pink beams crossing between systems. Orbits of different systems have different eccentricities (0.064 to 0.208 for the heads). The PNG carries no labels (they are terminal text laid over the image), so naming a body takes a hover in the terminal.

**You can infer**

* *Who is whose child*, from which star a planet circles, and who has the deepest subtrees, from the number of moons.
* *Memory*, from the radius (cube root: 5.5 times the width for 1 GiB over 6 MiB), and how much a star's whole system holds, since a star's size is its subtree's.
* *Thread counts*, as ring counts.
* *CPU*, from the glow and the trail; GPU use, from green.
* *Relative period by shell*: inner shells turn faster, as Kepler says, and heavier parents turn their children faster, within the clamps.

**You cannot infer**

* *Anything from orbital radius.* $a$ is the packing of siblings (their sizes), not a measurement; a large $a$ means many or big siblings inside it.
* *Anything from eccentricity or orientation.* A hash of the parent's pid.
* *Anything from the angular position of a planet.* Evenly spaced by construction; the shell's phase is arrival order.
* *The speed of a planet as CPU.* Motion is orbital mechanics, not activity. Activity is the glow.
* *Speed differences among kernel threads.* `kthreadd` holds no resident memory, so its tree's mass is floored to 1 MiB and every shell has the clamped period 600 s (see the table): the system turns once in ten minutes whatever it does. The same happens to any near-empty parent.
* *Mass in the physical sense.* The "mass" is a proxy (memory), the third law is only a layout rule.
* *Dynamical stability.* Nothing perturbs anything; nothing can collide inside a system or between systems, by construction.

**Period table.** $T$ in seconds, raw and then clamped to the displayed range:

| parent subtree memory | $a = 3$ | $a = 8$ | $a = 15$ | $a = 30$ |
|---|---|---|---|---|
| none (floor 1 MiB) | 679 → 600 | 2956 → 600 | 7590 → 600 | 21466 → 600 |
| 16 MiB | 170 | 739 → 600 | 1897 → 600 | 5367 → 600 |
| 64 MiB | 85 | 370 | 949 → 600 | 2683 → 600 |
| 512 MiB | 30 | 131 | 335 | 949 → 600 |
| 4 GiB | 10.6 | 46 | 119 | 335 |
| 6 GiB (a demo family) | 8.7 | 37.7 | 96.8 | 274 |

For the demo's twelve families (subtree 4.2 to 6.6 GiB, shells at $a \approx 10$ to $21$), I computed 52 to 158 s per revolution (a Python port of `shells`, `period` and the placement, whose layout I compared against the render by eye: the same twelve systems with one or two shells each in the same arrangement; the sub-pixel numbers are from the port, not the binary). A system with no memory to speak of, such as kernel threads, sits on the 600 s clamp in every shell.

![A system and the period law](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/orbit-system.svg)

**Resolution.** Orbit sizes are in world units, drawn at the camera's zoom; a 6 MiB process has radius 0.44 and is at least 3.5 pixels (`MIN_BODY_PX`) for glows; the sphere itself can be a few pixels wide on a large machine. Of the hundreds of processes in a real session only four levels are drawn.

## Try it

```sh
./target/release/isotop --demo --view orbit
./target/release/isotop --demo --view orbit --width 1600 --output orbit.png --renderer cpu
./target/release/isotop --demo --view orbit --time 40 --output orbit40.png   # the same layout, later in the planets' motion
./target/release/isotop --view orbit
```

Keys: `2` selects it; click a body for its inspector (memory of the system it anchors, socket counts, command line, parent, cgroup); `f` focuses a body and its descendants (the system turns about itself); `l` toggles labels; `c` cycles links. The hollow-star case needs macOS without root.

## Where in the code

* [`src/views/orbit.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/orbit.rs): `Orbits`, `Orbits::draw`, `paint`, `gpu_note`; tests `focusing_and_clearing_focus_keeps_every_orbit_system_in_place`, `leaves_stay_in_orbit_and_systems_keep_clear_of_each_other`.
* [`src/views/systems.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/systems.rs) (shared with Flow): `layout`, `Layout`, `Layout::report`, `Galaxy`, `Galaxy::place`, `Galaxy::revolve`, `Walk::branch`, `hubs`, `weigh`, `measure`, `shells`, `elements`, `period`, `galactic_period`, `eccentric_anomaly`, `Orbit` (`point`, `at`), `rings`, `ring_scale`, `whole_roots`, constants `MAX_DEPTH`, `ORBIT_GAP`, `SYSTEM_GAP`, `HEADROOM`, `MAX_TURN_STEP`; tests `kepler_solution_and_shells_keep_bodies_apart`, `sizes_span_memory_and_stars_carry_their_systems_mass`, `overlapping_rings_turn_together_and_detached_systems_turn_slower`, `the_galaxy_turns_at_one_frame_per_second_but_not_across_view_switches`.
* [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs): `mass_radius`, `MIN_BODY_PX`, `Frame::trace`.
* [`src/layout.rs`](https://github.com/tfpickard/isotop/blob/main/src/layout.rs): `vacant`, `spiral`, `GOLDEN_ANGLE`.
* [`src/palette.rs`](https://github.com/tfpickard/isotop/blob/main/src/palette.rs): `kind_color`, `KERNEL`, `SYSTEM`, `SESSION`, `CONTAINER`.
* [`src/simulation.rs`](https://github.com/tfpickard/isotop/blob/main/src/simulation.rs): `growth`.
* [`src/views.rs`](https://github.com/tfpickard/isotop/blob/main/src/views.rs): the `orbit` row (`LinkRule::All`, `Backdrop::Space`); the legend is `Orbits::legend`.
* Figure script: `docs/figures/orbit-system.py`.

## Variations and open questions

* **A real transfer for births.** Give a newborn a Hohmann-style transfer ellipse from the parent's surface to its shell, so the arrival is a Kepler arc and not a straight line.
* **Meaningful $a$.** Put a measured quantity on the semi-major axis instead of the packing: wall-clock age (`Process::started` exists), or CPU time, so that old processes orbit far. The shells would then be a time axis.
* **Real $N$-body.** Integrate the system with a symplectic scheme (leapfrog, Wisdom-Holman) with memory as the mass. The hierarchy would stay stable only if inner periods are much shorter than outer ones; what is the right "stability radius" for a tree of processes? Mardling and Aarseth's criterion for hierarchical triples is the place to start.
* **Resonances.** Choose shell radii so that adjacent periods are in small integer ratios (2:1, 3:2). Real systems lock into them; here it would be a pure visual rhyme.
* **Flat rotation curve.** The galaxy's angular speed falls outwards as a Keplerian one; real galaxies' rotation curves are flat (Rubin and Ford 1970). A flat curve is also easy to implement and would stop the outer systems lagging.
* **Open:** the clamp to 600 s makes the third law invisible for most real systems. Is a logarithmic compression of $\sqrt{M}$ a better way to keep the ordering legible?

## References

* J. Kepler, *Astronomia nova* (1609) and *Harmonices mundi* (1619).
* I. Newton, *Philosophiae naturalis principia mathematica* (1687).
* W. Hohmann, *Die Erreichbarkeit der Himmelskörper* (Oldenbourg, 1925).
* J. M. A. Danby, *Fundamentals of Celestial Mechanics*, 2nd ed. (Willmann-Bell, 1988).
* C. D. Murray and S. F. Dermott, *Solar System Dynamics* (Cambridge University Press, 1999).
* R. A. Mardling and S. J. Aarseth, "Tidal interactions in star cluster simulations", *Monthly Notices of the Royal Astronomical Society* 321 (2001), 398 (unverified page range).
* V. C. Rubin and W. K. Ford Jr., "Rotation of the Andromeda Nebula from a spectroscopic survey of emission regions", *The Astrophysical Journal* 159 (1970), 379-403.
* [Theory Stable Layout](Theory-Stable-Layout) for the layout invariant that the waived properties qualify.
