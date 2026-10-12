# View: Reef

Reef draws the machine as a coral reef under a sea sky. System services grow as coral colonies, one per cgroup,
with a polyp on a stalk for every process; your session's applications swim in schools whose speed follows their CPU;
containers are crabs scuttling on the sand; busy kernel threads drift as plankton. Size follows memory everywhere, a
glow follows CPU, I/O rises as bubbles and zombies float belly-up. It is the most decorative view of the five on this
page, and the page says so wherever it matters: almost nothing about *where* a creature is carries data.

![Reef: coral, schools of fish and crabs](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/reef.webp)

![Reef in motion](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/reef-motion.webp)

## The system being modelled

**A coral colony** is many small animals, the *polyps*, that share a skeleton of calcium carbonate and are linked by
living tissue; most reef-building species also host symbiotic algae (zooxanthellae) that photosynthesise for them.
A colony grows by budding new polyps and laying down skeleton, from millimetres to some centimetres a year depending on
the species, and a **reef** is the accumulated, cemented skeleton of many generations. Darwin's 1842 classification of
reefs as fringing, barrier or atoll followed from reading them as slowly rising structures built on slowly sinking
land. The part of that picture that matters for a metaphor of processes is the *colony*: individuals who share a
structure and a budget. A cgroup is the same kind of thing for processes, a node of the resource tree that a
service's processes share, and the reef gives every system service one.

**A school** is fish that swim together, each adjusting to its neighbours. Reynolds (1987) showed that the look of
flocks, herds and schools emerges from three local steering rules applied to every agent, which he called *boids*:

* **separation**, steer away from nearby mates so as not to crowd them;
* **alignment**, steer towards the mates' average heading;
* **cohesion**, steer towards the mates' average position.

Hunting and being hunted, the real reason fish school, are not modelled.

**Phyllotaxis.** Seeds in a sunflower head and scales on a pine cone sit on spirals. Vogel (1979) gave the simple
construction that reproduces a sunflower head: seed $k$ at distance $c\sqrt{k}$ from the centre and at angle
$k\cdot 137.5^\circ$, the *golden angle* $2\pi(1 - 1/\varphi)$, where $\varphi$ is the golden ratio. Because the golden ratio is the
most badly approximable irrational number, no two seeds ever line up in a ray and the density is uniform: the seeds
within distance $r$ number $r^2/c^2$, an area of $\pi c^2$ per seed. Reef uses it, through the shared
`layout::spiral`, to seat every member of a colony, school and gang.

![Reef: colony seating and school loops](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/reef-school.svg)

## Theory

### The school rule, as isotop writes it

Each fish carries a position $x$ and a velocity $v$ in three dimensions. Every frame, with $\Delta t \le 0.1$ s, the
state of the whole school taken *before any fish moves* (a synchronous update) gives

$$
\dot v_i = 0.4\,(g - x_i) \;+\; \sum_{\substack{j\ne i\\ 10^{-4} < d_{ij}^2 < 6}} \frac{1.5\,(x_i - x_j)}{d_{ij}^2} \;+\; 0.5\,(\bar v - v_i),
$$

with $g$ the school's moving target, $d_{ij}$ the distance between fish and $\bar v$ the mean velocity of the school
(including fish $i$). The speed is then clamped: $v \leftarrow v\,\dfrac{\operatorname{clamp}(\lvert v\rvert,\,0.3\,s,\,s)}{\lvert v\rvert}$
with the speed limit

$$
s(c) = 0.8 + 5\,\frac{c}{c + 40}\quad (c\text{ in percent of one core}),\qquad s = 0.3\ \text{for a zombie},
$$

so an idle fish is limited to 0.8 units per second and a fish at 100 % to 4.4. Position advances by $v\,\Delta t$ and height
is clamped between 1 and 13.5 (the surface is at 14).

Two properties follow directly.

1. **The cohesion term is not cohesion in Reynolds's sense.** It pulls each fish to the school's *target*, a point
   that circles the school's home, not to the mates' centroid. Separation is a $1/d$ repulsion inside a radius of
   $\sqrt 6 = 2.45$ units (the force has magnitude $1.5/d$ along the line between the pair). Alignment is
   to the mean velocity of the *whole* school, with no neighbourhood.
2. **The school's centre is an undamped oscillator.** Separation forces are equal and opposite for each pair and alignment
   terms sum to zero over the school, so the mean position $X$ obeys $\ddot X = 0.4\,(g - X)$ exactly, apart from the speed
   and height clamps: angular frequency $\sqrt{0.4} = 0.63$ rad/s, period 9.9 s, no damping. A school therefore swings
   back and forth *through* its target rather than settling on it; the clamp bounds the swing but does not damp it. (Explicit Euler
   integration adds a little energy, a factor of $\sqrt{1 + 0.4\,\Delta t^2} \approx 1.002$ in amplitude per 0.1-s step,
   which the speed limit absorbs.) A run of the rule for ten fish, in the figure above, gives swings of the centre past
   the target of 2.5 units at 3 % CPU and 3.4 units at 100 % in 40 s, and a faster, wider school at higher CPU. That run is a replica of
   the rule written for this page, not the renderer.

The target itself is at $g(t) = \bigl(h_x + 0.4\,\rho\cos(0.05\,t + \phi),\ h_y + 0.4\,\rho\sin(0.05\,t + \phi),\
5 + 2.5\sin(0.07\,t + \phi)\bigr)$ with $h$ the colony's home, $\rho$ its room and $\phi$ a hash of its name: a slow
circle with a period of $2\pi/0.05 = 126$ s and a bob with a period of 90 s.

### Seats and domes

Member $k$ of a colony (its *seat*, stable for the life of the process) sits at distance $1.6\sqrt{k + 0.5}$ and angle
$k\cdot 2.399963 + \phi$ rad from the colony's home. A colony with seat span $m$ reserves a disc of radius
$1.6\sqrt{m} + 2$, which the packer `Discs` places at 1.15 times that. A polyp's stalk is
$1.5 + 5\,(1 - d/\text{room})$ tall, so the colony is a dome: the centre is tallest, the rim 1.5. Crabs use
spacing 1.3 and shuffle along the tangent.

## Mapping

| Visual element | Measured quantity (platform source) | Transform |
|---|---|---|
| Habitat | Process kind and cgroup path (or process group): `"{Kind:?} {path}"` | One colony, school or gang per habitat; kernel threads have none |
| Coral colony | A system-kind cgroup (`Kind::System`) | Palette of 5 chosen by a hash of the name |
| Polyp | A process in the colony | One sphere on a two-segment stalk from the colony's base |
| Polyp size | `Process::memory` | $(0.18 + 0.32\,\texttt{mass\_radius}) \le 0.75$ with `mass_radius` $= 0.24\sqrt[3]{\text{MiB}}$ in $[0.25, 4.5]$, eased in when born |
| Polyp glow | `Process::cpu` above 1 % | Radius $\text{size}\cdot\text{zoom}\,(2.2 + 2\,c/(c+50))$, strength $0.3 + 0.5\,c/(c+40)$; the polyp also breathes $\pm 8\,\%$ |
| Polyp position | Seat (stable) | Vogel spiral, spacing 1.6; stalk height $1.5 + 5(1 - d/\text{room})$ |
| School | A session cgroup (`Kind::Session`) | Palette of 6 by hash |
| Fish | A process in the school | Two spheres, a tail wagging at $4 + 3\,\lvert v\rvert$ rad/s, an eye |
| Fish length | `Process::memory` | $\bigl(1 + 2.2\,m/(m+300)\bigr)$ with $m$ in MiB, eased in when born; girth 0.26 of length |
| Fish speed limit | `Process::cpu` | $0.8 + 5\,c/(c+40)$ units/s |
| Fish glow | `Process::cpu` above 5 % | Strength $0.2 + 0.4\,c/(c+50)$ |
| Fish position | None: the steering rule above | Decorative |
| Crab | A process in a container cgroup (`Kind::Container`) | One per process, shell violet |
| Crab size | `Process::memory` | $(0.35 + 0.5\,m/(m+200))$ with $m$ in MiB |
| Crab pace | `Process::cpu` | $0.4 + 2\,c/(c+40)$; stride $0.9\sin(t\cdot\text{pace} + \text{seat})$ along the tangent |
| Plankton | A kernel thread | Position from a hash of the pid plus slow drift ($0.07$, $0.05$, $0.11$ rad/s); below 0.5 % CPU only a faint glint, not pickable; busier ones a pickable mote |
| Bubbles | `Process::io_rate` above 4096 B/s (storage bytes, read plus write; `/proc/<pid>/io`, `proc_pid_rusage`) | $1 + \lfloor 3\,b/(b + 1\,\text{MiB/s})\rfloor$ glows rising from the creature to the surface once per 4 s |
| Zombie | Process state `Z` | Grey; a fish floats belly-up towards the surface at speed 0.3 |
| Seabed, caustics, kelp, light shafts, sky | none | Decorative |

## How isotop realises it

**Habitats and placement.** `Reef::draw` in `src/reef.rs` builds a habitat for every sampled non-kernel process and
assigns `Seats` (a seat per process within its habitat) and `Discs` (a home per habitat). Both are reserved for every *sampled*
process, drawn or not: focusing a subtree or lowering `--limit` frees no seat and no home, and a hidden fish waits where
it swam and resumes from there, so clearing the focus puts everything back and the framing does not change. The camera
fit is a fixed square of half-width `reach = max(homes.reach() + 6, 16)` and height $0.6\times 14$.

**Schools.** `Reef::school` runs the rule above for the school's drawn members, from a `Swimmer { position, velocity }`
kept per `Identity`. A new fish hatches at the target plus a hash-derived offset ($\pm 2$, $\pm 2$, $\pm 1$) from its pid, with a unit heading
from the school's phase. A fish whose process disappears from the sample is dropped. The step is `Reef`'s own clamp, $\Delta t = \operatorname{clamp}(\text{time} - \text{last}, 0, 0.1)$,
not `stage.dt`. Neighbour search is a plain double loop over the school (quadratic), which is fine at school sizes. No spatial hash is used here.

**Corals and crabs** are placed, not simulated: positions are functions of the seat and `stage.time`
(a $0.15\sin(0.6\,t + \text{seat})$ sway for polyps, the stride for crabs).

**Seabed.** A grid of between 60 and 160 quads per side, height $0.25\sin(0.31x)\cos(0.23y)$, lit by a caustic term
$(\tfrac12 + \tfrac12\,\sin(\dots)\sin(\dots))^3$ built from two travelling sines; 28 kelp strands of 12 segments and 6 faint
light shafts. All deterministic functions of position and time. None of it is data.

**Stable layout.** The harness records known failures for Reef (issue
[#119](https://github.com/tfpickard/isotop/issues/119)): fish swim at CPU-dependent speeds and crabs scuttle at CPU-dependent
paces and no anchor is declared, so scaling CPU moves them by up to about 1.7 units; and in one churn case the separation
steering among school mates changes the paths of the remaining fish. Coral polyps, which are seated, are stable.

**CPU and GPU.** Spheres, lines, beams, facets and glows only; no new primitive. The row uses the `Sea` backdrop and a
four-second settle before a headless frame.

## Reading the picture

* **Membership and size are the data; position and motion are not.** Which colony, school or gang a creature belongs to
  is its cgroup and kind. How big it is is its memory. How it glows is its CPU. Where it is within the cluster, and where a fish
  is at an instant, is the seat or the steering rule: it says nothing.
* **Speed is a limit, not a reading.** A fast school is a busy one in the sense that its speed limit is high, but the
  fish are following a rule, not their processes' threads.
* **Schools do not rest.** An undamped oscillator never settles; even idle schools swim, slowly. A school that holds
  still is one with no drawn members or one paused.
* **Bubbles are storage I/O only**, written to or read from block devices; network and page-cache I/O raise none.
* **The reef has no memory of its own.** Nothing grows or bleaches over time: a polyp's size is the process's memory now (eased), and
  a service that ran hot yesterday leaves no skeleton. The only growth is the ease-in (`sprout`) of a newborn.
* **The cluster layout is a stable arrangement, not a map.** Colonies sit where the packer put them first and keep their place;
  the distance between two colonies means nothing.
* **A hidden creature is still there.** Colonies keep room for hidden processes, so a tight `--limit` leaves empty water.

## Try it

```sh
./target/release/isotop --demo --view reef --width 1600 --output reef.png --renderer cpu
./target/release/isotop --demo --view reef --time 40 --output reef40.png
./target/release/isotop --view reef        # live; press 9 in a session
```

Headless PNGs simulate four seconds first so the fish have left their hatching point. Click a creature to inspect
its process; `f` focuses a subtree and the rest of the reef keeps its places. In the demo, a polyp's glow pulses with the process's demo CPU
and the schools are the demo's session cgroups.

## Where in the code

* [`src/reef.rs`](https://github.com/tfpickard/isotop/blob/main/src/reef.rs): `Reef::draw`, `school`, `crab`, `polyp`, `draw_seabed`, `habitat`; constants `DEPTH`, `CORALS`, `FISH`, `ZOMBIE`.
* [`src/layout.rs`](https://github.com/tfpickard/isotop/blob/main/src/layout.rs): `Seats`, `Discs`, `spiral`, `spin`, `GOLDEN_ANGLE`, `vacant`.
* [`src/palette.rs`](https://github.com/tfpickard/isotop/blob/main/src/palette.rs): `CONTAINER`, `tint`; [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs): `mass_radius`; [`src/simulation.rs`](https://github.com/tfpickard/isotop/blob/main/src/simulation.rs): `hash`, `sprout`.
* [`src/views.rs`](https://github.com/tfpickard/isotop/blob/main/src/views.rs): `reef_legend`, the `reef` row (`SETTLE`, `known_unstable`).
* Figure: `docs/figures/reef-school.py`.

AGENTS.md lists `reef.rs` among the module views; its row is `Hooks::BuiltIn` (`BuiltIn::Reef`).

## Variations and open questions

* **Anchors for fish and crabs** (issue #119): declare each seat and school target so the harness can hold them to account.
* **Bleaching.** Real corals expel their algae under thermal stress and turn white. Fade a colony towards white as its cgroup's memory
  or CPU pressure (the PSI figure Cells reads) rises, and let it recover.
* **Accretion.** Make a polyp's height the integral of its CPU time (calcification), so a long-lived busy service builds a tall reef
  and a short-lived one leaves nothing; that makes the reef a record rather than a snapshot.
* **Real cohesion.** Replace the pull to the target with the pull to the school's centroid, or to the $k$ nearest neighbours with
  `SpatialHash3`, and measure with a polarisation order parameter as [Coop](View-Coop) does for its flocks
  ([Math: Flocking](Math-Flocking)). The school would stop being a driven oscillator and would drift as a group.
* **Damping.** Add a drag term and the school will settle on its target; the question is whether a still school reads as healthy or dead.
* **Spawning events.** Release a cloud of particles when a service forks many children at once.
* **Open:** what should the reef do with processes that are in two cgroups' worth of ancestry? Today the leaf cgroup is the habitat.

## References

* C. W. Reynolds, "Flocks, herds and schools: A distributed behavioral model", Computer Graphics (SIGGRAPH '87
  Proceedings) 21(4), 1987, pp. 25-34.
* H. Vogel, "A better way to construct the sunflower head", Mathematical Biosciences 44, 1979, pp. 179-189.
* C. Darwin, *The Structure and Distribution of Coral Reefs*, 1842.
* Related pages: [Math: Flocking](Math-Flocking), [Math: Spatial hashing](Math-Spatial-Hashing),
  [Theory: Stable layout](Theory-Stable-Layout), [Theory: Determinism](Theory-Determinism), [View: Cells](View-Cells), [View: Coop](View-Coop).
