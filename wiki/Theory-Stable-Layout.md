# Theory: Stable layout

A process keeps its place. Its CPU can swing by an order of magnitude between two samples, its memory can
double, and a tenth of the machine can exit and be replaced, and the pebble, house, grave or star that stood
for it must not move. This page is about the machinery that makes that true: a golden-angle spiral that is a
pure function of a slot number, a seat allocator that hands slots out for life, a disc allocator that reserves
headroom so that growth rarely forces a move, and the test harness that fails any view that reshuffles. It
also derives *why* the golden angle is the right angle for placement by sequential insertion, with numbers
computed from the same arithmetic the code uses.

![A Vogel spiral at the golden angle against three wrong angles, and the 1-D discrepancy of the angle sequence](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/layout-vogel.svg)

*Top: 600 slots of `layout::spiral` at the golden angle, at 137.0 degrees, at 138.0 degrees and at 120
degrees (a third of a turn), dotted by arrival order. Bottom: the sequence discrepancy $N D^*_N$ of the
angles $\{n\alpha\}$ for $N=377$, against $\alpha$.*

## History

The arrangement of leaves, scales and florets was described long before it was explained. In 1837 the brothers Louis and Auguste Bravais (a botanist and a crystallographer) observed that successive organs around a stem are separated by about 137.5 degrees and tied the spiral counts to the Fibonacci sequence. The sequence itself comes from elsewhere: Leonardo of Pisa introduced it in the *Liber abaci* of 1202 as the answer to a problem about breeding rabbits, and no plant appears in the problem.

The mechanism came next. Hofmeister proposed in 1868 that a new primordium forms, one at a time, in the largest gap left at the growing tip; Schwendener's mechanical theory of 1878 put contact pressure between neighbours behind the same idea. Both state the rule that a new organ appears in the least crowded place; that doing so one at a time leads to the golden angle is what section 2 quantifies. Douady and Couder (1992) made that rule a laboratory experiment and a dynamical system: primordia that repel one another and appear at regular intervals settle on a divergence angle near 137.5 degrees, and on Fibonacci spiral counts, as the interval is shortened.

The formula isotop uses is Vogel's (1979): the $n$-th floret at radius $c\sqrt n$ and angle $n$ times the divergence angle. Computer graphics took it from there; Prusinkiewicz and Lindenmayer's *The Algorithmic Beauty of Plants* (1990) has a chapter on it, and Lindenmayer and Veen are recorded as having studied the spirals of sunflower seeds. The number-theoretic half of the argument is older than any of this. Hurwitz proved in 1891 that every irrational number has infinitely many rational approximations closer than $1/(\sqrt5\,q^2)$, that the constant cannot be improved, and that the golden ratio is the number that attains it; and the three-gap theorem (a conjecture of Steinhaus, proved independently by Sós, Surányi and Świerczkowski in 1958 and 1959) describes the partition of a circle by multiples of an angle.

The wish that a changing picture should not rearrange itself has its own literature. Misue, Eades, Lai and Sugiyama (1995) named the viewer's model of where things are the mental map and asked layout algorithms to preserve it. isotop's variant is stronger than most graph-drawing practice: place is not adjusted to preserve the map, it is computed from nothing else.

## 1. Why a visualization of changing data must not reshuffle

This section is general; it is not specific to isotop.

A static picture is read once. A picture that is redrawn every second from data that changes every second is
read *continuously*, and the reader's working memory is the thing that has to survive the redraw. Misue,
Eades, Lai and Sugiyama called that working memory the **mental map**: the user's internalised model of where
things are, and they argued that a dynamic layout should be adjusted so as to preserve it, even at the price
of a worse static layout (Misue et al. 1995). Anything a user does with a process view depends on place being
an identity:

* "the long thin thing at the left is the compiler" is a statement about a position;
* a selection, a hover tag, a tour stop or a search hit is found again by looking where it was;
* mouse input is cell-coarse (`target_near` searches a radius), so a target that drifts by a few pixels
  between the aim and the click is a missed click;
* the data itself is noise at the sampling rate: CPU is a smoothed rate recomputed every `--sample-ms`, so a
  layout *derived from* CPU would twitch every second for no reason the viewer could use.

The design rule of isotop (`AGENTS.md`, Invariants: "Layouts are stable") is therefore that **place is a
function of identity and history, never of the current measurements**. Measurements drive height, size,
colour and motion; they must not drive position. The rest of this page is how that is achieved and how it is
checked.

There is a price. A layout that never moves anything must place a newcomer *around* the incumbents, so it
cannot be as dense as one computed from scratch, and a body that outgrows its room must either overlap its
neighbours or be moved once. Section 4 puts numbers on both costs.

## 2. Theory: sunflower packing by sequential insertion

This section is mathematics, independent of the code. The code is in section 3.

### 2.1 Vogel's model and the square root

Place point $n=0,1,2,\dots$ at polar coordinates

$$
r_n = c\sqrt{n+n_0},\qquad \theta_n = 2\pi\alpha\, n + \theta_0 ,
$$

with a spacing $c$, an offset $n_0\ge0$, a phase $\theta_0$ and a *divergence angle* $2\pi\alpha$ (this is the
model of Vogel 1979 for the florets of a sunflower head; it is the fixed-divergence limit of the growth
process analysed by Douady and Couder 1992). The radius law is forced by wanting a uniform density. The
annulus between $r_n$ and $r_{n+1}$ has area

$$
\pi\,(r_{n+1}^2-r_n^2)=\pi c^2\,\bigl((n+1+n_0)-(n+n_0)\bigr)=\pi c^2 ,
$$

independent of $n$: every point owns the same area $\pi c^2$, so the density is $1/(\pi c^2)$ everywhere.
A perfect hexagonal lattice of that density has nearest-neighbour distance
$d_{\rm hex}=\sqrt{2A/\sqrt3}$ with $A=\pi c^2$, that is

$$
d_{\rm hex}=c\sqrt{2\pi/\sqrt3}=1.9046\,c ,
$$

and no arrangement of that density can have a larger minimum distance. This is the yardstick for everything
below.

The only freedom left is $\alpha$. Its job is to stop successive points, and points a few steps apart, from
standing in each other's radial shadow.

### 2.2 Rational angles collapse to rays

If $\alpha=p/q$ in lowest terms, point $n+q$ has the same angle as point $n$ and sits at radius
$c\bigl(\sqrt{n+q+n_0}-\sqrt{n+n_0}\bigr)\approx cq/(2\sqrt n)$ further out. The picture is $q$ straight rays
(the 120 degree panel of the figure is $q=3$), the points on a ray crowd together as $n$ grows, and the
measured minimum spacing at $N=1000$ is $0.047c$, 2.5 percent of the ideal $1.9046c$ derived below. An
$\alpha$ near a rational with a small denominator is nearly as bad, because the rays merely rotate slowly into
spiral arms.

### 2.3 The three-gap theorem and why the golden ratio is the most irrational number

Look at the *angles alone*: the $N$ numbers $\{n\alpha\}$ for $n<N$ on the circle $\mathbb R/\mathbb Z$. The
**three-gap theorem** (Steinhaus' conjecture, proved independently by Sós 1958, Surányi 1958 and Świerczkowski
1959) says that these points cut the circle into arcs of **at most three distinct lengths**, and if there are
three, the longest is the sum of the other two. The lengths and the transition values of $N$ are read off the
continued fraction $\alpha=[0;a_1,a_2,a_3,\dots]$ with convergents $p_k/q_k$: the ratio of the longest to the
shortest gap grows with the partial quotients $a_k$.

Adding point $N$ splits one of the longest gaps. The question for sequential insertion is how uneven the
partition can get at *any* $N$, not only at lucky ones. Measured over every $N\le1500$, the supremum of
(longest gap)/(shortest gap) is

| $\alpha$ | continued fraction | $\sup_N$ longest/shortest |
|---|---|---|
| $1/\varphi^2=0.381966$ (the golden angle) | $[0;2,1,1,1,\dots]$ | $2.6180339892$, which is $\varphi^2$ |
| $\sqrt2-1$ | $[0;2,2,2,\dots]$ | $3.4142$ ($=2+\sqrt2$) |
| $(\sqrt{13}-3)/2$ | $[0;3,3,3,\dots]$ | $4.3028$ |
| $e-2$ | $[0;1,2,1,1,4,\dots]$ | $9.52$ |
| $\pi-3$ | $[0;7,15,1,292,\dots]$ | $293.6$ (the quotient 292 at $N\approx114$) |

(The golden entry reaches its limit to nine digits, $2.6180339892$ against $\varphi^2=2.6180339887$, at
$N=1376$. Computed with exact enumeration in `float64`; a stdlib script is in the figure source.) The golden
angle has every partial quotient equal to 1, the smallest possible, and that is exactly what keeps the
partition within a factor of $\varphi^2$ of uniform at every prefix. This is the "sequential insertion"
property: the layout is good after every addition, not only when the count is a Fibonacci number.

The same fact in the language of discrepancy: the star discrepancy $D^*_N$ of $\{n\alpha\}$ is
$O(\log N/N)$ when the partial quotients are bounded, and in general $N D^*_N$ is governed by the sum of the
partial quotients up to the scale $N$, so it is smallest for $\alpha$ with all quotients 1 (Weyl 1916 for
equidistribution; Ostrowski 1922 and Kuipers and Niederreiter 1974 for the discrepancy). In one dimension $D^*_N$ of a sorted sample is exactly
$\frac1{2N}+\max_i\lvert x_{(i)}-\frac{2i-1}{2N}\rvert$. The bottom panel of the figure plots $N D^*_N$ for
$N=377$. Computed values:

| $N$ | golden | $\sqrt2-1$ | $\pi-3$ | $137.0^\circ/360^\circ$ | $1/3+10^{-4}$ |
|---|---|---|---|---|---|
| 144 | 1.000 | 1.842 | 3.897 | 2.400 | 45.97 |
| 233 | 1.445 | 2.601 | 1.986 | 2.328 | 72.93 |
| 377 | 1.000 | 2.225 | 6.281 | 2.836 | 112.16 |

At $N=144$ and $N=377$ the golden sequence has $N D^*_N=1$ (it is 1.44 at 89 and 233). One is the least any
point set containing the origin can have, since a point at 0 alone forces $D^*_N\ge1/N$; equal spacing
attains it. So at those $N$ the golden angles are as uniform as a lattice of angles. Near rational angles
the discrepancy grows in towers proportional to $N$.

### 2.4 From angle to packing: Hurwitz's constant sets the minimum spacing

The gap statistics concern angles. Packing is about two-dimensional distances, and there is a short argument
that connects them. Take the pair $(n,n+q)$. Their angles differ by $2\pi\lVert q\alpha\rVert$, where
$\lVert x\rVert$ is the distance from $x$ to the nearest integer; their radii differ by about
$cq/(2\sqrt n)$ (for $q\ll n$). Write $\kappa=q\lVert q\alpha\rVert$. At radius $r=c\sqrt n$ the squared
distance between them is

$$
d^2\;\approx\;\Bigl(2\pi c\sqrt n\,\frac{\kappa}{q}\Bigr)^2+\Bigl(\frac{cq}{2\sqrt n}\Bigr)^2 .
$$

Put $t=q/\sqrt n$. Then $d^2/c^2=(2\pi\kappa/t)^2+t^2/4$, which is minimised at $t^2=4\pi\kappa$ with value

$$
d^2_{\min}=2\pi\kappa\,c^2 .
$$

So whenever the sequence has a $q$ with small $\kappa=q\lVert q\alpha\rVert$ near the right scale
$q\approx\sqrt{4\pi\kappa n}$, two points in the same neighbourhood are at most $c\sqrt{2\pi\kappa}$ apart.
**Hurwitz's theorem** (1891) states that for every irrational $\alpha$ there are infinitely many $q$ with
$\kappa<1/\sqrt5$, and that $1/\sqrt5$ cannot be improved; the golden ratio is the number that attains it.
Hence, for large $N$, the minimum spacing of *any* Vogel spiral is at most

$$
d_{\min}\le c\sqrt{\frac{2\pi}{\sqrt5}}=1.6763\,c=0.880\,d_{\rm hex},
$$

and the golden angle is the one angle for which the bound is essentially reached (all its $\kappa_q$ stay near
$1/\sqrt5$ and none is smaller). This is a derivation sketch, not a theorem about finite $N$: the pairs
$(n,n+q)$ are not the only close pairs, and the integer $q$ near the optimal scale need not exist for every
$n$. The measurement is the check. Minimum nearest-neighbour distance over the outer four fifths of
$N=1000$ points (offset $n_0=0.5$), as a fraction of $d_{\rm hex}$:

| angle | $\alpha$ | $d_{\min}/c$ | $d_{\min}/d_{\rm hex}$ |
|---|---|---|---|
| golden, 137.5078 deg | 0.381966 | **1.675** | **0.880** |
| $\sqrt2-1$ (silver) | 0.414214 | 1.490 | 0.782 |
| $e-2$ | 0.718282 | 1.208 | 0.634 |
| 137.3 deg | 0.381389 | 1.100 | 0.578 |
| 137.0 deg | 0.380556 | 1.048 | 0.550 |
| 137.6 deg | 0.382222 | 0.974 | 0.512 |
| 138.0 deg | 0.383333 | 0.964 | 0.506 |
| $\pi-3$ | 0.141593 | 0.819 | 0.430 |
| 120 deg | 0.333333 | 0.047 | 0.025 |

The golden angle measures 1.675 against the predicted 1.676. Every other angle in the table loses minimum
spacing relative to it: 11 percent for the silver angle, 28 percent for $e-2$, 34 to 42 percent for the four
angles within half a degree of the golden angle, 51 percent for $\pi-3$ and 97 percent for a third of a turn.

### 2.5 Fibonacci parastichies

The convergents of the golden angle are ratios of consecutive Fibonacci numbers ($1/2$, $1/3$, $2/5$, $3/8$,
$5/13$, $\dots$), so the nearly-aligned pairs $(n,n+q)$ have $q$ a Fibonacci number. The visible families of
spirals ("parastichies") in the picture are therefore counted by consecutive Fibonacci numbers, 8 and 13 near
the middle, 21 and 34 further out. This is why seeds and pine cones show Fibonacci counts, and it is not a
coincidence that the same constant appears in the placement and in the picture.

## 3. How isotop realises it

### 3.1 The spiral: a pure function of a slot number

`layout::spiral(spacing, offset, slot, phase) -> (distance, angle)` returns exactly Vogel's law:

$$
\text{distance}=\text{spacing}\cdot\sqrt{\text{slot}+\text{offset}},\qquad
\text{angle}=\text{slot}\cdot\texttt{GOLDEN\_ANGLE}+\text{phase},
$$

with `GOLDEN_ANGLE = 2.399_963` (`f32` radians; $2\pi(1-1/\varphi)=2.39996323$, so the literal is low by
$2.3\times10^{-7}$ rad, relative error $9.6\times10^{-8}$). Slot 0 sits at the centre when the offset is 0;
the offset keeps slot 0 off the centre when something else stands there (callers use 0.5 or 1.0). The error of
the literal accumulates linearly in the slot, $2.3\times10^{-7}\cdot k$ rad, which is $0.013^\circ$ at slot
1000; no view gets near the slot counts where it would matter.

The property that makes stability possible is that **a slot's position depends on nothing but the slot, the
spacing, the offset and the phase**. Adding slot $N+1$ cannot move slots $0..N$. Appending is free.

`layout::spin(name) -> f32` supplies the phase: the FNV-1a 32-bit hash of the name, its top 24 bits scaled to
$[0,2\pi)$. Clusters thus do not all share one spiral orientation (which would line their arms up), and each
cluster's orientation is a function of its *name*, so it survives restarts.

### 3.2 `Seats`: a slot for life

```text
Seats { taken: group -> (seat -> Identity), seats: Identity -> (group, seat) }
```

`Seats::assign(members)` takes the current members as `(Identity, group)` pairs and

1. releases every seat whose holder left or changed group;
2. gives each newcomer the **lowest free seat of its group**: `(0..=taken.len()).find(free)`, and one of
   $\texttt{len}+1$ candidates is always free by pigeonhole (hence the `expect`);
3. visits newcomers in `Identity` order (`pid`, then `start`; `Identity` derives `Ord`), so the assignment
   does not depend on `HashMap` iteration order (see [Determinism](Theory-Determinism)).

A member keeps its seat for life. A freed seat is reused by the next newcomer of that group, so the numbers
stay dense and `span(group)`, one past the highest seat in use, bounds the spiral's radius. Seat numbers
carry no meaning beyond arrival order within a group; they are not ranks. The seat becomes a position as
`spiral(spacing, offset, seat, spin(group))` in Cells (`spiral(spacing, 1.0, ..)`), Reef
(`spiral(1.3, 0.5, ..)` for fish, `spiral(1.6, 0.5, ..)` for crabs), Globe (`spiral(0.9, 0.5, ..)`, and
`GOLDEN_ANGLE` directly), and Necropolis uses `Seats` for its zombie pits. Cortex seats its neurons with `Seats`
too. Ripple has its own copy of the same slot logic, with `cluster.slots` instead of `Seats`
(`Ripple::scatter` in `src/views/ripple.rs`), and the City has a third mechanism below.

**Hidden processes keep their seats.** Reef, Cells, Coop, Globe and Ripple reserve seats and places
for *every sampled process, drawn or not* (the comments in `Ripple::scatter` and in `cells.rs`, `reef.rs`,
`coop/layout.rs` and `globe.rs` say so), and the Orbit and Flow systems keep a hidden root's place in `Galaxy::place`
(`src/views/systems.rs`). Focusing a subtree or lowering `--limit` hides processes without freeing their seats, so
clearing the focus brings everything back where it was (Cores has the same test for its marbles). This is the same rule as stability under churn, one
level up: the *view of the data* (focus, limit) must not move anything either. The commits
"Keep hidden processes' Reef and Cells seats and places" (#106) and "Keep Ripple, Strata, Globe, Orbit, Flow
and Cores layouts when focus or --limit hides processes" (#105) established it.

### 3.3 `vacant` and `Discs`: first fit on the spiral, with reservation

`layout::vacant(placed, radius)` finds the first point on a golden-angle spiral out from the origin where a
disc of `radius` clears every placed `(centre, radius)` disc:

```text
step = max(0.4 * radius, 1.0)
for k in 0.. :   c_k = spiral(step, 0, k, 0)      # origin outwards
    if for all placed (o, r):  |c_k - o| >= radius + r:  return c_k
```

It is a **first-fit** online algorithm. Termination is guaranteed because $|c_k|=\text{step}\sqrt k$ grows
without bound while the placed set is bounded. The spiral supplies a canonical order of candidate sites that
sweeps the plane evenly, so the result is compact and independent of any container order.

`Discs` wraps it for named discs (cgroups, flocks). `Discs::arrange(needs: [(name, radius)])`:

* **keeps** every placed disc whose current need is no larger than the radius it **reserved**
  (`r <= reserved`); a disc that shrank keeps its old reservation, so the reservation is a high-water mark;
* **drops** discs that left, or that grew beyond their reservation;
* places the *pending* discs (new or dropped), **largest first, ties by name**, each at
  `vacant(other_discs, reserved)` with `reserved = radius * 1.15`.

The 15 percent is the **reservation**: headroom bought at placement so that ordinary growth does not force a
move. Other stable placements use the same trick with their own constants: Ripple's clusters reserve
`reach * 1.1` (plus `POND_GAP`) and grow their spacing by 1.15 when a pebble outgrows it; the Orbit and Flow
systems reserve `need * HEADROOM` with `HEADROOM = 1.3` in `src/views/systems.rs` (the comment there says 30
percent holds what the view harness does to a system: memory by half to double, a tenth of its processes leaving
and as many arriving, and that a system which outgrows it is placed again). Cells reserves a `Room` per cell, and
its dish radius carries 5 percent (`needed * 1.05`). `Discs::reach()` is the radius around the origin that holds every disc, for fitting the camera.

### 3.4 The City: square shells

`City::place` in `src/views/city.rs` is the third mechanism. A process gets a plot, a (district, slot) pair, from its
group's districts: the lowest free slot among the group's 16 (a 4 by 4 block, 4 world units apart), else a
new district. `district_origin(index)` enumerates the quadrant in **square shells**: with
$s=\lfloor\sqrt{\text{index}}\rfloor$ and $o=\text{index}-s^2$ the district sits at $(s,o)$ if $o\le s$ else
$(2s-o,\,s)$, scaled by 23 world units:

```text
 y
 3 | 15 14 13 12      shell 3 (indices 9..15) is filled before shell 4
 2 |  8  7  6 11
 1 |  3  2  5 10
 0 |  0  1  4  9
   +--------------- x
```

Index $i$ lives in shell $\lfloor\sqrt i\rfloor$, each square number $n^2$ completes an $n\times n$ block, and
no earlier district ever moves. The cost is a growing, roughly square city; the benefit is that `plots` plus
`occupancy` are all the state the layout needs. (The test `city_does_not_relocate_when_resources_or_population_change`
in `src/views/city.rs` pins it.)

### 3.5 What the guarantee costs

Two prices, computed with a faithful port of `vacant` (radii log-normal, $\sigma=0.6$, base radius 2, 40
seeded streams per row, Python, `float64`). **Density.** The packing fraction $\sum r^2/R^2$ of the discs in
the circle of radius $R=$ `reach`:

| discs | arrival order | largest first (offline greedy) | arrival order, 1.15 reservation |
|---|---|---|---|
| 10 | 0.345 +- 0.080 | 0.462 +- 0.075 | 0.260 +- 0.062 |
| 30 | 0.406 +- 0.083 | 0.621 +- 0.055 | 0.309 +- 0.062 |
| 100 | 0.459 +- 0.066 | 0.703 +- 0.022 | 0.348 +- 0.050 |

Placing in arrival order rather than largest first costs a third of the density at 100 discs, and the
reservation takes a quarter more ($1.15^2=1.32$ in area). `Discs` sorts only the *pending* set, which is the
only part of the order it is free to choose. For scale, the densest packing of equal discs in the plane is
$\pi/\sqrt{12}=0.907$.

**Moves.** A reservation is hysteresis. Model a disc's log-radius as a driftless random walk with per-sample
step $\sigma$; the reservation is a barrier $\ln h$ above the radius at placement, and each crossing re-places
the disc and raises the barrier to $\ln h$ above the new radius, so the barrier climbs a ladder of rungs
$\ln h$ apart. The number of re-placements in $T$ samples is then about $M_T/\ln h$, where $M_T$ is the
running maximum, $E[M_T]=\sigma\sqrt{2T/\pi}$ for a Brownian motion. A simulation of 3600 samples (an hour at
the default `--sample-ms 1000`), mean of 60 seeded runs, steps $\sigma\,\mathcal N(0,1)$ drawn with the same
splitmix64 and Box-Muller as `Rng::normal`:

| headroom $h$ | area overhead $h^2$ | moves per hour, $\sigma=0.05$ | moves per hour, $\sigma=0.10$ |
|---|---|---|---|
| 1.05 | 1.10 | 29.3 | 41.7 |
| **1.15** (`Discs`) | **1.32** | **13.6** | **23.2** |
| 1.30 | 1.69 | 7.5 | 14.3 |
| 1.50 | 2.25 | 5.0 | 9.8 |

The ladder estimate $M_T/\ln h$ over-predicts these by 20 to 30 percent at $h=1.15$ (17.1 and 34.3), because a
real crossing overshoots the barrier and the new rung is measured from the overshoot. The shape is the
point: moves fall roughly as $1/\ln h$ while the area cost grows as $h^2$. Process memory is not a driftless
random walk (it plateaus and jumps), so this is a model of the trade-off, not telemetry; the constants
1.15 and 1.1 in the code were chosen by eye, and the 1.3 of `HEADROOM` against the harness's perturbation.

## 4. How the harness enforces it

`harness.rs` (test-only) runs **every registered view** through the same six properties
(`views::Property`): `Deterministic`, `NonEmpty`, `ValidPicks`, `ResourceChange`, `Churn`, `EmptyMachine`.
The two stability properties are:

* **Resource change.** `rescaled(snapshot)` multiplies each process's CPU by $0.25\cdot16^{u}$ and its memory
  by $0.5\cdot4^{u}$, with $u\in[0,1)$ drawn from `Rng::for_identity(process.id, SALT)`: CPU is scaled
  log-uniformly in $[0.25,4]$, memory in $[0.5,2]$, each fixed by the process's identity. Cgroup totals are
  re-tallied.
* **Churn.** `churned(snapshot)` removes `len/10` **leaf** processes (not a parent, not a holder of a lock
  anybody waits for, chosen by `hash_identity` order) and starts as many new ones, pids from 900000, as
  siblings of survivors in their group and cgroup, with a fresh `Identity`. Leaves only: an exiting parent
  re-parents its children, which is a change of *structure*, not of population.

A run is a headless `--output` run at the row's `contract.time` (12 s unless the view needs a fuller history;
a row names a later one where needed, as Ptolemy's 300 s), width 160, 192 demo processes, CPU rasterizer: the row's warm-up, a first frame, then 12
frames 0.25 s apart, with the changed sample arriving as the sample of time + 1 s. The steady run and the
changed run are compared **frame for frame at the same times**, so whatever moves by itself (orbits, walking
chickens) moves alike in both. For every process present in both, the distance in the *ground plane*,
$\sqrt{\Delta x^2+\Delta y^2}$, must not exceed `contract.tolerance` (1 world unit by default); height is
excluded because several views draw a measurement with it. Where creatures wander, the view supplies
**anchors**: `ViewModule::anchors` returns the places a body keeps to (Coop's seats and houses, Cores' lane
starts, Reef's school targets and crab seats), which `Scene::anchors` hands to the harness. A view with nothing to compare fails as `Broken`, and a panic is also `Broken`;
neither can be excused.

`known_unstable` records the debt of views that fail a property, each with its issue. **It is empty on every
registered row.** The six views that predate the harness (Orbit, Flow, Ripple, Cores, Cells, Reef) were listed
there with the issues [#114](https://github.com/tfpickard/isotop/issues/114) to
[#119](https://github.com/tfpickard/isotop/issues/119), and the change that moved every view onto
`ViewModule` ([#143](https://github.com/tfpickard/isotop/pull/143)) fixed each layout and dropped the entries:
the Orbit and Flow systems got `HEADROOM`, Cells a reserved `Room` per cell, and Cores and Reef declare the
places that do not wander as anchors. All eighteen registered views pass all six properties; Matrix draws journal
lines, not processes (`draws_processes: false` on its row), and is exempt from the stability properties. The
harness reports a property that starts to pass on a listed row ("`NowPasses`": drop the entry and close the
issue), accepts no entry for a new view, and never excuses `Broken`. The listing mechanism is exercised only by
test rows in `views::testing` (`SHUFFLED`, whose second row lists a churn failure and a resource-change failure
it does not have), and `the_harness_detects_a_view_that_reshuffles` proves the test has teeth against a
deliberately rank-seated test view.

## 5. Reading the picture

* **What stability promises.** Position is a function of identity and arrival history. It does *not*
  promise that position means anything: a seat number is arrival order, a spiral radius is not a ranking, and
  two processes close together are not related unless the view says so (Cells and Coop put a cgroup's members
  together *by construction*, which is information; the spiral inside the group is not).
* **Holes.** A freed seat stays empty until a newcomer of the same group arrives, so a long-running machine
  shows gaps that record exits. Absence of a thing is information here.
* **Re-placement is visible.** When a disc outgrows its reservation it jumps once. A jump in an otherwise
  still view is therefore a statement about growth, not about the process list.
* **Camera is separate.** Stable world positions do not mean a stable *screen*: the camera fit follows
  `stage.bounds`, and views whose content moves declare a fixed envelope so that the fit does not chase it.
* **Not a proof of non-overlap.** `vacant` guarantees clearance at placement, for the reserved radius. A
  disc that grows within its reservation stays inside its own reserved disc by construction; a body drawn
  larger than the radius it asked for would overlap a neighbour, and that is a bug of the view.

## 6. Try it

```sh
# The same process stays where it is while the data changes under it (any view that passes the harness)
cargo test every_view_satisfies_the_view_contract -- --nocapture   # prints pass / exempt / known unstable per property
cargo test seats_survive_departures_and_discs_survive_small_growth
cargo test city_does_not_relocate_when_resources_or_population_change

# A visual: the city at two demo times, same plots
./target/release/isotop --demo --view city --time 12 --width 1200 --renderer cpu --output a.png
./target/release/isotop --demo --view city --time 40 --width 1200 --renderer cpu --output b.png
```

Regenerate the figure with `python3 docs/figures/layout-vogel.py` from the repository root.

## 7. Where in the code

* [`src/layout.rs`](https://github.com/tfpickard/isotop/blob/main/src/layout.rs): `GOLDEN_ANGLE`, `spiral`,
  `vacant`, `spin`, `Seats` (`assign`, `seat`, `span`), `Discs` (`arrange`, `reserved`, `reach`), and the test
  `layout_spiral_places_the_same_points_as_before` that pins their bits.
* [`src/views/city.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/city.rs): `City::place`,
  `district_origin`.
  [`src/views/systems.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/systems.rs): `Galaxy::place`,
  `HEADROOM` (Orbit and Flow).
  [`src/views/ripple.rs`](https://github.com/tfpickard/isotop/blob/main/src/views/ripple.rs): `Ripple::scatter`.
* [`src/cells.rs`](https://github.com/tfpickard/isotop/blob/main/src/cells.rs),
  [`src/reef.rs`](https://github.com/tfpickard/isotop/blob/main/src/reef.rs),
  [`src/coop/layout.rs`](https://github.com/tfpickard/isotop/blob/main/src/coop/layout.rs),
  [`src/globe.rs`](https://github.com/tfpickard/isotop/blob/main/src/globe.rs),
  [`src/necropolis.rs`](https://github.com/tfpickard/isotop/blob/main/src/necropolis.rs),
  [`src/cortex.rs`](https://github.com/tfpickard/isotop/blob/main/src/cortex.rs): the users of `Seats` and `Discs`.
* [`src/harness.rs`](https://github.com/tfpickard/isotop/blob/main/src/harness.rs): `rescaled`, `churned`,
  `Session`, `Runs`, `stayed`, `every_view_satisfies_the_view_contract`.
* [`src/views.rs`](https://github.com/tfpickard/isotop/blob/main/src/views.rs): `Contract`, `Property`,
  `Unstable`, `ViewModule::anchors`, the registry rows' `known_unstable` (empty today).

`docs/VIEW_CONVENTIONS.md` lists `render::vacant` and `render::GOLDEN_ANGLE` among the shared helpers "today";
in the code they live in `layout.rs`. The code wins.

## 8. Variations and open questions

* **Rank-free versus rank-aware seats.** Ptolemy keeps its top ten processes on spheres whose *ranks*
  shuffle while the spheres stay put (`wanderers_keep_their_spheres_while_ranks_shuffle_inside_the_top_ten`).
  Is there a seating scheme that lets rank show without moving anything?
* **Shrinking reservations.** `Discs` never lowers a reservation. A slow decay (re-place a disc downwards
  when its need has stayed below half its reservation for a minute) would reclaim area at the price of one
  more move per episode. The model in section 3.5 predicts how often.
* **Offline compaction on demand.** A key that re-packs everything largest-first would give the 0.70
  density of the table in one deliberate reshuffle, with an animation. The mental-map literature suggests
  tweening between the two layouts.
* **A discrepancy-optimal slot order.** Seats are handed out lowest-first, which on the spiral fills from the
  centre. Filling by the order that minimises the largest empty circle at every prefix is a different
  problem (and the golden sequence is already near-optimal for it).
* **Higher dimensions.** Arbor, a 3-D view, advances the azimuth of successive slots by the golden angle
  (`src/arbor.rs`). A 3-D analogue of the spiral for placing points in a volume has no single golden counterpart;
  the generalised golden ratio sequences of the Kronecker type (the "$R_d$ sequence") are a candidate
  (unverified; not in the tree).
* **Holes with names.** A freed seat records an exit and nothing else. Necropolis records an exit with a stone,
  a minute and a cause. The two do not share a representation, and nobody has asked them to.
* **Two sequences compared.** Replace `GOLDEN_ANGLE` in a scratch build with the silver angle and render Cells:
  the table in section 2.4 predicts an 11 percent smaller minimum spacing.

## 9. References

* Vogel, H. 1979. "A better way to construct the sunflower head." *Mathematical Biosciences* 44: 179-189.
* Douady, S. and Couder, Y. 1992. "Phyllotaxis as a physical self-organized growth process." *Physical Review
  Letters* 68: 2098-2101.
* Prusinkiewicz, P. and Lindenmayer, A. 1990. *The Algorithmic Beauty of Plants.* Springer. (Chapter 4 on
  phyllotaxis.)
* Hurwitz, A. 1891. "Ueber die angenaeherte Darstellung der Irrationalzahlen durch rationale Brueche."
  *Mathematische Annalen* 39: 279-284.
* Sós, V. T. 1958. "On the distribution mod 1 of the sequence $n\alpha$." *Annales Universitatis Scientiarum
  Budapestinensis, Sectio Mathematica* 1: 127-134. Swierczkowski, S. 1959. "On successive settings of an arc
  on the circumference of a circle." *Fundamenta Mathematicae* 46: 187-189.
* Weyl, H. 1916. "Ueber die Gleichverteilung von Zahlen mod. Eins." *Mathematische Annalen* 77: 313-352.
* Kuipers, L. and Niederreiter, H. 1974. *Uniform Distribution of Sequences.* Wiley.
* Khinchin, A. Ya. 1964. *Continued Fractions.* University of Chicago Press.
* Misue, K., Eades, P., Lai, W. and Sugiyama, K. 1995. "Layout adjustment and the mental map." *Journal of
  Visual Languages and Computing* 6(2): 183-210.
