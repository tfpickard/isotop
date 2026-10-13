# Flocking

Birds, fish and chickens that copy their neighbours' headings move together; add noise and, past a threshold, they
stop. The smallest model of that is Vicsek's (1995), and it is the engine of the [Coop](View-Coop): every process is a chicken, every cgroup a
flock, and the heading noise is driven by how erratic the process's CPU is and how stalled the machine is. This page states the Vicsek model
precisely, places it among the other classic flocking models (Reynolds's boids, Couzin's zones, Toner and Tu's hydrodynamics), shows by simulation what its order parameter does
at the sizes isotop uses, and says exactly where [Coop](View-Coop) and [Reef](View-Reef) depart from the textbook.

![Order parameter against heading noise, for the 1995 rule and for the Coop yard](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/flocking-vicsek-order.svg)

*Panel A: $\varphi$ against $\eta$ for the 1995 rule on a periodic square (blue) and for the Coop yard's rule (orange). Panel B: the
noise `coop.rs` assigns to a CPU pressure $\psi$ and CPU variation CV. Panel C: panel A read through panel B. Computed by
`docs/figures/flocking-vicsek-order.py` with the Python standard library; the yard series re-implements `Yard::step`
and follows the rule, not the Rust generator's bits.*

## History

The longer account lives in the repository: [docs/theory/coop.md](https://github.com/tfpickard/isotop/blob/main/docs/theory/coop.md).

**The pecking order.** The earliest quantitative study of a group that orders itself is of hens. The Norwegian zoologist Thorleif Schjelderup-Ebbe described the dominance hierarchy of domestic fowl in his doctoral dissertation of 1921. Sources differ on whether his own word was *Hackliste* or *Hackordnung*; the English phrase "pecking order" is first recorded in a 1927 translation of a book by Friedrich Alverdes. Popular accounts have him keeping notebooks on his family's hens from the age of ten; the primary text was not checked for this page.

**Boids.** Craig Reynolds built his *boids* ("bird-oid objects") in 1986 and published "Flocks, herds and schools: a distributed behavioral model" at SIGGRAPH in 1987. Three local steering rules (separation, alignment, cohesion) were enough to make a computer flock look alive; the aim was animation, not measurement. The model was first used in the short *Stanley and Stella in: Breaking the Ice* (1987; one source says 1988) and, in modified form, for the bat swarms and penguin armies of *Batman Returns* (1992).

**A phase transition.** In 1995 Tamas Vicsek, Andras Czirok, Eshel Ben-Jacob, Inon Cohen and Ofer Shochet published a deliberately minimal model in *Physical Review Letters*: particles of fixed speed, each taking the mean heading of its neighbours plus noise. They reported a transition from disorder to collective motion as the noise fell. The result was surprising because the equilibrium XY model, which has the same alignment rule, cannot order in two dimensions (the Mermin-Wagner theorem). John Toner and Yuhai Tu explained in the same year, with a hydrodynamic theory, why moving particles escape the theorem. Later work on larger systems (Gregoire and Chate 2004; Chate and co-workers 2008) found that with angular noise the transition is in fact discontinuous, with travelling high-density bands near threshold: the continuous-looking transition of the small systems of the original paper is read as a finite-size effect.

**Real birds.** Couzin and co-workers (2002) gave agents concentric zones of repulsion, orientation and attraction and found swarm, torus and parallel-motion states. The StarFlag project reconstructed the three-dimensional positions of starlings over Rome, and Ballerini and co-workers (2008) concluded that each bird interacts with a fixed number of neighbours, about seven, rather than with all within a radius. Cavagna and co-workers (2010) measured scale-free correlations in the same flocks. The topological rule is why the Murmuration view aligns birds with their seven nearest neighbours.

## Theory

### The Vicsek model

$N$ point particles move at constant speed $v_0$ in the plane. At each discrete step $t \to t+1$ every particle sets its heading to the mean heading of all particles
within distance $r$ of it (itself included), adds a random angle, and steps:

$$
\theta_i(t+1) = \operatorname{atan2}\!\Bigl(\sum_{j:\,|x_j - x_i| < r} \sin\theta_j(t),\ \sum_{j:\,|x_j - x_i| < r} \cos\theta_j(t)\Bigr) + \Delta\theta_i,\qquad \Delta\theta_i \sim U\!\left[-\tfrac{\eta}{2},\tfrac{\eta}{2}\right],
$$

$$
x_i(t+1) = x_i(t) + v_0\,(\cos\theta_i(t+1),\ \sin\theta_i(t+1))\,\Delta t .
$$

The update is *synchronous*: every new heading is computed from the old state. The state of order is measured by

$$
\varphi = \frac{1}{N v_0}\,\Bigl|\sum_i \mathbf v_i\Bigr| \in [0,1],
$$

which is 1 when all particles move together and, for independent random headings, has expectation
$\sqrt{\pi}/(2\sqrt N) \approx 0.886/\sqrt N$ (the mean length of a two-dimensional random walk of $N$ unit steps, $\sqrt{\pi N}/2$, divided by $N$). That finite-size floor is the dotted line in the figure: 0.055 for $N = 256$, 0.114 for $N = 60$.

The control parameters are the noise $\eta \in [0, 2\pi]$ (here the *width* of the uniform kick), the density $\rho = N/L^2$ and the
dimensionless step $v_0\Delta t/r$. Vicsek, Czirok, Ben-Jacob, Cohen and Shochet (1995) reported a continuous
transition, $\varphi \propto (\eta_c - \eta)^{\beta}$ with $\beta \approx 0.45$, from a disordered phase (random motion) to
a phase with long-range order, in two dimensions. Later work on much larger systems (Gregoire and Chate, 2004; Chate, Ginelli, Gregoire and Raynaud, 2008) found
that, with this angular noise, the transition is *discontinuous*: just below threshold the ordered liquid coexists
with high-density travelling bands. The finite systems of the original paper and of isotop see a smooth crossover that
looks continuous; the figure's curves are of this kind. A model in which the noise is added to the *vector sum* instead of the angle (vectorial noise) behaves differently
in detail (see Chate et al., 2008).

### Why order in two dimensions is surprising

The equilibrium XY model (spins on a lattice, aligning with neighbours) has the same alignment rule but, by the Mermin-Wagner theorem, no
long-range order in two dimensions at any non-zero temperature: thermal fluctuations of the Goldstone mode destroy it. The Vicsek flock does order in two dimensions. Toner and Tu (1995, 1998)
showed why with a hydrodynamic theory: the particles *move*, so the density and velocity fields are coupled by advection. In the form usually quoted, the velocity field $\mathbf v(\mathbf r, t)$ and density $\rho$ obey

$$
\partial_t\rho + \nabla\!\cdot(\rho\mathbf v) = 0,
$$

$$
\partial_t \mathbf v + \lambda_1(\mathbf v\!\cdot\!\nabla)\mathbf v + \lambda_2(\nabla\!\cdot\!\mathbf v)\mathbf v + \lambda_3\nabla|\mathbf v|^2
= \alpha\mathbf v - \beta|\mathbf v|^2\mathbf v - \nabla P + D_B\nabla(\nabla\!\cdot\!\mathbf v) + D_T\nabla^2\mathbf v + D_2(\mathbf v\!\cdot\!\nabla)^2\mathbf v + \mathbf f,
$$

with a noise $\mathbf f$. The terms $\alpha\mathbf v - \beta|\mathbf v|^2\mathbf v$ make $|\mathbf v|$ settle to $\sqrt{\alpha/\beta}$ (the ordered phase); the advective $\lambda_1$ term, absent in equilibrium
systems, stabilises the order against long-wavelength fluctuations, and in two dimensions the exponents are known exactly (scaling exponents $\zeta = 3/5$, $\chi = -1/5$, $z = 6/5$). Two further consequences recorded in the
literature: anomalously large (*giant*) density fluctuations in the ordered phase, and fast, long-range propagation of
disturbances. Neither is something isotop's flocks of tens of chickens can show.

### Other classic models

* **Boids** (Reynolds, 1987) gave each simulated bird three steering rules applied to its neighbours: *separation* (steer away from crowding), *alignment* (steer towards the average heading), *cohesion* (steer towards the average position). It
  has inertia and individual forces rather than instant heading changes, and no noise term: the target was convincing animation.
* **Couzin et al. (2002)** put three concentric *zones* around each agent: repulsion (innermost), orientation, attraction (outermost). Varying the zone widths produces four collective states: a swarm, a torus (milling), dynamic parallel motion, highly parallel motion, with hysteresis
  between them.
* **Metric versus topological interaction.** Ballerini et al. (2008) found that starlings in large flocks keep interacting with a *fixed number* of nearest neighbours (about seven), whatever the distance, rather than with all neighbours within a radius. Cavagna et al. (2010) measured scale-free
  correlations in such flocks. The [Murmuration](Planned-Murmuration) view uses seven-neighbour alignment for that reason.

## What the code does

### Coop: the Vicsek rule with additions (`Yard::step` in `src/coop.rs`)

Foragers are the chickens that are not roosting, feeding, brooding, waiting, frozen, stuck or zombie (`Role::Foraging`). One step, $h = 0.05$ s (a fixed 20 Hz clock,
`FixedStep::new(H, MAX_STEPS = 10, MAX_GAP = 0.5)`, so the frame rate does not change the dynamics):

1. Every chicken draws a kick once per step whatever it is doing, `rng.signed() * 0.5` (uniform in $[-\tfrac12, \tfrac12)$), so the draws never depend on update order. A forager's new heading is
   `atan2` of the sum of unit headings of **same-flock** foragers within `NEIGHBOURHOOD = 4.0` units (itself included), found through a [spatial hash](Math-Spatial-Hashing) and summed in identity order, plus `chicken.noise * draw`: exactly the 1995 kick of width $\eta$ = `noise`.
2. Three extra terms are added to the *vector sum before the angle is taken*: a **peer pull**, for each socket peer $k$ that is also foraging, $w\,(\cos\theta_k + \tfrac12\hat{\mathbf u}_{ik})$; a **home pull** of $0.6\min((d - \text{range})/4, 1)$ towards the henhouse when the chicken is farther than `home.range` from it; and a **flee push** of $1.5\,(1 - d/6)$ away from every fox eye within 6 units.
3. All headings are applied together, then positions advance by `speed * h` along the new heading. Speed is *per chicken*: $v = 0.6 + 3.4\,c/(c + 25)$ units per second for CPU $c$ percent.
4. A Jacobi *separation* pass pushes overlapping bodies apart (a forager against a forager shares the overlap; against a planted or walking chicken it takes all of it), then `confine` pushes foragers out of house footprints and reflects them off the fence.

The noise has three parts, summed and capped at a full turn:

$$
\eta = \min\Bigl(\,0.5 + 0.5\min(\mathrm{CV}, 2) + 5.5\,\frac{\psi}{\psi + 15},\ 2\pi\Bigr),
$$

with $\mathrm{CV}$ the coefficient of variation of the process's CPU over its last 30 samples (0 below 3 samples or a mean under 0.5 %) and $\psi$ the system CPU pressure (the `some` average over 10 s, percent). A steady, unloaded machine ($\mathrm{CV} = 0$, $\psi = 0$) has $\eta = 0.5$: per-step kicks of $\pm 14^\circ$. At $\psi = 20\%$ it is 3.64, at 40 % it is 4.5. Without pressure readings the pressure term is dropped.
The constants were calibrated by the test `flocks_order_at_low_noise_and_dissolve_past_critical_noise` (60 foragers: $\varphi > 0.8$ at $\psi = 0$ and $< 0.4$ at $\psi = 40$).

**Differences from the 1995 model, all visible in the figure.** (a) The fence reflects rather than wraps. (b) $N$ is small (a flock is the members of one cgroup; the test uses 60). (c) The interaction radius is large compared with a step: at the minimum speed
$v\,h/r = 0.6 \times 0.05/4 = 0.0075$, a quarter of Vicsek's $0.03$, rising to 0.029 at 25 % CPU and 0.05 at the speed ceiling of 4. So the neighbour graph changes slowly, and
a low-CPU flock behaves like an XY model on a nearly frozen random geometric graph with a slowly mixing neighbourhood. (d) Speeds are heterogeneous, so $\varphi$ is defined as $|\sum\mathbf v|/\sum|\mathbf v|$, a speed-weighted
version (`Yard::order`; the original port of [tfpickard/chicken](https://github.com/tfpickard/chicken) summed speed magnitudes and was always 1). (e) Flocks do not interact in the alignment rule, only through separation, so the yard is many independent flocks.

### What the simulation says

For the 1995 rule on a $6.4 \times 6.4$ periodic square with 256 particles ($\rho = 6.25$, $r = 1$, $v_0 = 0.03$, 1000 steps, the last 500 averaged) $\varphi$ falls from 1 at $\eta = 0$ through 0.82 at 2, 0.49 at 3.5 and 0.19 at 4.5 to the
floor of 0.054 at $2\pi$: a rounded crossover centred near $\eta \approx 4$. For the Coop yard rule with 60 foragers (six seeds, last 15 s averaged) the order is 0.97 at $\eta = 0.5$, 0.77 at 2, 0.46 at 3.5, 0.24 at 4.5 and the floor 0.113 at $2\pi$.
Read through the noise formula (panel C), a steady flock holds $\varphi \approx 0.97$ at $\psi = 0$, about 0.45 at $\psi = 20\%$ and about 0.24 at $\psi = 40\%$; a process whose CPU is erratic (CV = 2) starts at $\eta = 1.5$ and is scattered
by $\psi \approx 20\%$. *There is no sharp transition at these sizes*; the legend's "phi" is a smooth index of how coordinated the foragers are. The test's thresholds (0.8 and 0.4) sit on the two sides of the crossover.

![Snapshots of the yard flock and the periodic box at three noise levels](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/flocking-snapshots.svg)

*Top: the yard rule at $\eta = 0.5, 3.64, 4.5$ after 60 s; bottom: the periodic box at $\eta = 1, 4, 5.5$. Colour is heading angle, so an aligned flock is one colour. Script: `docs/figures/flocking-snapshots.py`.*

### Reef: a boid with a global neighbourhood (`Reef::school` in `src/reef.rs`)

Fish of one habitat form a school steered by three terms per frame: a pull towards the school's wandering target (`0.4` times the offset), a *separation* push $1.5\,\mathbf d/|\mathbf d|^2$ from each school-mate nearer than $\sqrt 6$ units, and an *alignment* pull
$0.5\,(\bar{\mathbf v} - \mathbf v)$ towards the mean velocity $\bar{\mathbf v}$ of the *whole school*. There is no neighbourhood radius, so the alignment is all-to-all (a mean-field limit: it cannot show a Vicsek-type transition, and the school always aligns), there is no noise, the speed is clamped between 0.3 and 1 of
$0.8 + 5\,c/(c + 40)$ units per second, and the step uses the frame's own time step, clamped to 0.1 s (`dt` in `Reef::draw`), rather than a fixed clock, so the school's path depends on the frame rate.

### Murmuration: the topological rule (`src/murmuration.rs`)

The Murmuration view ([Murmuration](Planned-Murmuration)) uses the topological rule: one bird per thread, `Flock` stepped at a fixed 20 Hz (`STEP = 0.05`, through `FixedStep`), neighbours found with `SpatialHash3::nearest`. Alignment (`ALIGNMENT = 0.30`) pulls a bird's velocity towards the weighted mean velocity of its `NEIGHBOURS = 7` nearest, with same-process neighbours counting double; cohesion (`COHESION = 0.02`) pulls it towards the centroid of its `FAR_NEIGHBOURS = 21` nearest; separation (`SEPARATION = 0.4` inside `SEPARATION_RADIUS = 1.5`) pushes close neighbours apart; a roost and a falcon complete the forces. The comments in the source record three failures of the prototype that bear on this page: a per-process centroid pull of 0.0012 or more splits the flock into balls (so `FLOCK_PULL = 0.0006` pulls only the whole flock in); cohesion over only seven neighbours lets closed groups of eight split off as islands (hence 21); and a vertical damping of 0.22 flattens the flock into a pancake (`VERTICAL_DAMPING = 0.10`). The flock is thinned to `BIRD_CAP = 8000` birds. Unlike Coop it has no order parameter in its legend.

## Reading the picture

* **Measured:** which chickens belong to which flock (cgroup), their speed (CPU), their noise (CPU variation and system pressure), their pull (socket traffic or co-activity). **Derived:** the headings, which are the output of the dynamics and not data; $\varphi$ is a *derived* order of that simulation.
* A coherent flock means the *noise inputs are low*, not that the processes are doing the same thing: the headings are made up. The picture is an indicator of CPU stability (low CV) and low CPU pressure in disguise.
* $\varphi$ in the legend is for the whole yard: with many flocks pointing in unrelated directions it is small even when each flock is perfectly ordered (the test `chickens_align_only_with_their_own_flock`: two flocks, each 0.98, yard under 0.2). The inspector gives the per-flock value.
* With $N$ foragers the floor is about $0.886/\sqrt N$; a flock of four can show $\varphi \approx 0.4$ by chance.

## Try it

```sh
./target/release/isotop --demo --view coop --width 1600 --output /tmp/coop.png --renderer cpu
python3 docs/figures/flocking-vicsek-order.py      # about five minutes; deterministic
python3 docs/figures/flocking-snapshots.py
```

In a live coop (`--view coop`) select a chicken: the inspector line `flock X: phi ... (N foraging)` is the per-flock order.

## Where in the code

* `src/coop.rs`: `Yard::step`, `separate`, `confine`, `order`, `flock_orders`, `noise`, `speed`, `variation`, `peer_weight`; constants `NEIGHBOURHOOD`, `ETA_BASE`, `K_CV`, `K_PSI`, `PSI_KNEE`, `V_MIN`, `V_SPAN`, `SPEED_KNEE`, `FLEE`, `H`. Tests: `vicsek_step_does_not_depend_on_update_order`, `order_parameter_is_the_magnitude_of_the_summed_velocity`, `flocks_order_at_low_noise_and_dissolve_past_critical_noise`, `chickens_align_only_with_their_own_flock`, `flock_follows_the_same_path_at_ten_and_sixty_frames_per_second`.
* `src/simulation.rs`: `SpatialHash`, `Rng`, `FixedStep`. `src/reef.rs`: `Reef::school`.
* Figure scripts: `docs/figures/flock_sim_w6.py` (both simulations), `flocking-vicsek-order.py`, `flocking-snapshots.py`.

## Variations and open questions

1. **Locate the crossover.** Sweep $N$ and $v_0 h/r$ for the yard rule and extract a Binder cumulant. At isotop's sizes is there any finite-size scaling to find?
2. **Topological Coop.** Replace the radius by the 7 nearest same-flock chickens. Does the crossover move?
3. **Vectorial noise.** Add the noise to the vector sum and compare the two curves, after Chate et al. (2008).
4. **Reef with a radius.** Give the school a neighbourhood and a noise term, and let the order parameter come out of the same legend line.
5. **Order as a measurement.** Is there a quantity in the system (not the noise inputs) that the heading order could be made to measure? Currently it measures nothing about the processes beyond its inputs.
6. **Rank without fights.** Schjelderup-Ebbe's hens settled their order by early contests and kept it by memory; Coop queues chickens at a feeder by scheduling priority, which is settled by `nice(1)` and kept by the kernel. Is a hierarchy that no one had to fight for still a pecking order?

## References

* T. Vicsek, A. Czirok, E. Ben-Jacob, I. Cohen and O. Shochet, "Novel type of phase transition in a system of self-driven particles", *Physical Review Letters* 75 (1995), 1226-1229.
* G. Gregoire and H. Chate, "Onset of collective and cohesive motion", *Physical Review Letters* 92 (2004), 025702.
* H. Chate, F. Ginelli, G. Gregoire and F. Raynaud, "Collective motion of self-propelled particles interacting without cohesion", *Physical Review E* 77 (2008), 046113.
* F. Ginelli, "The physics of the Vicsek model", *The European Physical Journal Special Topics* 225 (2016), 2099-2117.
* J. Toner and Y. Tu, "Long-range order in a two-dimensional dynamical XY model: how birds fly together", *Physical Review Letters* 75 (1995), 4326-4329; and "Flocks, herds, and schools: a quantitative theory of flocking", *Physical Review E* 58 (1998), 4828-4858.
* C. W. Reynolds, "Flocks, herds and schools: a distributed behavioral model", *Computer Graphics* (SIGGRAPH '87) 21(4) (1987), 25-34.
* I. D. Couzin, J. Krause, R. James, G. D. Ruxton and N. R. Franks, "Collective memory and spatial sorting in animal groups", *Journal of Theoretical Biology* 218 (2002), 1-11.
* M. Ballerini et al., "Interaction ruling animal collective behavior depends on topological rather than metric distance: evidence from a field study", *Proceedings of the National Academy of Sciences* 105 (2008), 1232-1237.
* A. Cavagna et al., "Scale-free correlations in starling flocks", *Proceedings of the National Academy of Sciences* 107 (2010), 11865-11870.
* See also [View Coop](View-Coop), [View Reef](View-Reef), [Murmuration](Planned-Murmuration), [Math Spatial Hashing](Math-Spatial-Hashing), [Theory Simulation Toolkit](Theory-Simulation-Toolkit).
