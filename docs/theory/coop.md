# Coop: a fenced chicken yard of flocking processes

**View:** `coop` · **Code:** `src/coop.rs`, `simulation::{FixedStep, SpatialHash}` · **Prototype:** none (this view predates the mockups)

## What you are looking at

| On screen | Measured quantity |
|---|---|
| A chicken | one process; its size is its memory |
| A flock and its henhouse | one cgroup (the henhouse labels it); without cgroups, one group per `GROUP_LABEL` |
| Foraging speed | CPU use: faster for busier |
| Roosting on a perch | idle: below 0.3% CPU, until it rises above 0.8% |
| Pecking at a feeder | running, at the feeder of the CPU it last ran on |
| Pecking order at a feeder | scheduling priority: the highest priority takes the front of the queue |
| Eggs in a nest box | open files, `round(log2(1 + files))` up to 16 |
| Rotten (cracked) eggs | files deleted but still open, one each up to 8 |
| Brooding on straw by the nest | the process holds a file lock; a queue at the nest is blocked on one |
| Chicks following a hen | threads, up to 12 |
| Dust puffs | reads faster than 64 KiB/s, one puff per doubling up to 5 |
| A fox entering the yard | an OOM kill, which takes its victim away as a grey ghost |
| Pairs of eyes in the hedge | memory pressure (one pair plus one per 10%, at most 6) |
| Feet up | a zombie, waiting to be reaped |
| Frozen | stopped (`T` state); a chicken in tonic immobility |
| Stuck in the mud | uninterruptible sleep (`D`) |
| Phi in the legend | the Vicsek order parameter of the foraging flocks |

## History

**Chickens are the most numerous bird on Earth,** and every one descends from the red junglefowl (*Gallus gallus*) of South and Southeast Asia. When the domestication happened has been argued over for decades. Early claims put it at 8,000 years ago in northern China, and those have been challenged. A 2022 re-evaluation of the archaeological evidence, led from the University of Exeter, put the domestication of the modern chicken in Southeast Asia about 3,500 years ago, and tied it to the arrival of dry rice farming there: junglefowl came to the fields to eat the grain. Chickens were first treated as exotic and ritual animals, and only centuries later as food.

**The pecking order** is a chicken discovery. The Norwegian zoologist Thorleif Schjelderup-Ebbe, who watched farmyard hens from childhood, described and named it in the early twentieth century: within a flock every hen knows which others she may peck and which may peck her, a stable hierarchy settled by early fights and kept without more of them. His word, *Hackordnung*, became the English idiom for any rank order, which is why a phrase about chickens is now used of offices. It was a founding observation for the study of animal dominance.

**Collective motion** arrived later. In 1986 Craig Reynolds built *boids*, three local rules (separate, align, cohere) that make a computer flock look alive. In 1995 Tamás Vicsek and colleagues stripped the idea to its physics: particles of constant speed that each take the average heading of their neighbours plus noise. They found that the system has a genuine phase transition: at low noise the whole flock moves together; past a critical noise it dissolves into a gas of individuals. The result was surprising because moving flocks defy a theorem. In equilibrium systems with continuous symmetry, long-range order cannot exist in two dimensions (Mermin-Wagner), but these are not in equilibrium, and Toner and Tu (1995) explained why they can.

**The henhouse** is the cgroup. Google engineers Paul Menage and Rohit Seth began "process containers" in 2006, and renamed them control groups to avoid colliding with the word "container". They were merged into Linux 2.6.24, which shipped in January 2008. Cgroups let the kernel account for, and limit, the CPU, memory and I/O of a *set* of processes: a flock. Docker and every container system since rest on them. The fox is the **OOM killer**, a kernel mechanism that when memory runs out picks a process and kills it.

## Background theory

### The Vicsek model

$N$ self-propelled particles move at constant speed $v$. At each step, particle $i$ takes the average heading of all particles within a radius $r$ (including itself) and adds a random angle:

$$\theta_i(t + \Delta t) = \langle\theta\rangle_{r}(t) + \eta\,\xi_i, \qquad \xi_i \sim U\left(-\tfrac12, \tfrac12\right)$$

where $\eta$ is the noise strength, so the heading changes by at most $\eta/2$ radians either way. The **order parameter** is the length of the mean velocity relative to the mean speed,

$$\varphi = \frac{\left|\sum_i \vec v_i\right|}{\sum_i |\vec v_i|}$$

so $\varphi = 1$ is a perfectly aligned flock and $\varphi \approx 0$ is random motion (about $1/\sqrt{N}$ for $N$ particles at random). For fixed density there is a critical noise $\eta_c$ below which $\varphi$ rises sharply from zero: a transition, with an order that grows as the noise falls. The model is a fair caricature of birds, fish, bacteria and, here, hens. Its important property is that the order is *emergent*: no hen knows the flock's heading, and the heading is reached through local averaging.

### Where the noise comes from

Each chicken has its own noise, built from three measured things:

$$\eta = 0.5 + 0.5\,\min(c_v, 2) + 5.5\,\frac{\psi}{\psi + 15}$$

(capped at a full turn, $2\pi$). Here $c_v$ is the *coefficient of variation* of the process's CPU over its last 30 samples, so a bursty process is jittery, and $\psi$ is the CPU pressure (percentage of time tasks wait for the CPU), saturating with a knee at 15%. A quiet machine therefore has $\eta = 0.5$ rad and its flocks align; a machine under heavy CPU pressure approaches $\eta \approx 6$ rad, and the flocks dissolve. The constants are calibrated by the test `flocks_order_at_low_noise_and_dissolve_past_critical_noise`: a flock of 60 idle foragers must settle at $\varphi > 0.8$ at $\psi = 0$ (noise 0.5 rad) and fall below $\varphi < 0.4$ at $\psi = 40\%$ (noise 4.5 rad). So $\varphi$ in the legend is a measurement of how contended the machine is, read off as flocking.

### Speed

A forager moves at

$$v = 0.6 + 3.4\,\frac{\text{cpu}}{\text{cpu} + 25} \text{ world units per second}$$

so an idle chicken drifts at 0.6, one at 25% CPU walks at 2.3, one flat out at 100% goes at 3.3, and nothing exceeds 4.0. The saturating form means one runaway process cannot outrun a camera.

### Hysteresis

A chicken roosts when its CPU falls below 0.3% and leaves the perch only above 0.8%. A threshold with a single value would flip a process that sits on the line every sample; two thresholds (a Schmitt trigger) hold the state between them. The same device is used by every thermostat.

## How isotop implements it

1. **The yard** advances at a fixed step of 0.05 s (`H`) fed from wall-clock time through `FixedStep`, with at most 10 catch-up steps per frame and any longer gap dropped, so the same trajectory happens at every frame rate and a paused or stalled view does not jump.
2. **Neighbours** are found with a `SpatialHash` at radius 4 world units. A forager aligns with flock mates only, so flocks do not merge; socket peers contribute weighted pairs.
3. **Roles** come from the sampled process state: `Z` is a zombie, `T` frozen, `D` stuck, CPU below the roost threshold is roosting, running with a recorded CPU goes to that feeder, a lock holder not running broods, a lock waiter queues. Roosting, feeding, brooding and waiting chickens *walk* to their target at 3 units/s and arrive.
4. **The feeder** is the CPU the process last ran on. Chickens at one feeder queue by scheduling priority, so "rank 1 of 3 by priority -51 (real-time)" appears in the inspector. If the platform cannot report the last CPU, running chickens share one trough and the legend says so.
5. **Nests** are the open-file view: a clutch of eggs per file count, cracked eggs for deleted-but-open files, in a 4 by 6 grid. Reads raise dust. Lock holders sit on straw pads beside the nest, and blocked processes queue behind them in a line, so contention on a lock is a *queue you can count*.
6. **Foxes** are OOM kills. The kernel's kill counter comes from a background thread, so a fox can arrive after the victim has left: the departed stay suspects for 4 s, each kill claims a suspect and the first three kills in a window each send a fox. A fox runs in through the hedge to its victim's last place and leaves with a grey ghost in its mouth. When memory pressure is positive, pairs of eyes appear in the hedge, and nearby foragers turn away from them.
7. **Layout** is stable: seats, perches and houses are placed once and the yard grows when a flock outgrows its reserved space, so a process does not move because another one did.

## Faithful, stylized, invented

- **Faithful:** the Vicsek update and the order parameter; the noise being an increasing function of measured CPU variance and pressure; one flock per cgroup; ranking at feeders by real scheduling priority; counted eggs and queues.
- **Stylized:** the particular noise constants, the speed curve, the logarithmic egg counts, one chick per thread up to 12, and eggs, dust and chicks being decoration of counts.
- **Invented:** that a process is a hen. The pecking order is a priority queue dressed as farm life; "tonic immobility" for a stopped process is a joke with a real name (chickens do freeze when inverted).
- **Not modelled:** real flocks do not need a speed limit, and real Vicsek order depends on density as well as noise. Here density is whatever the process counts give, so $\varphi$ is comparable over time on one machine and not across machines.

## Reading the picture

- A flock walking as a body is a cgroup under low CPU pressure with steady members. A flock milling about is noise: pressure or bursty members.
- A very low $\varphi$ in the legend with many foragers is a contended machine; $\varphi$ near 1 is a calm one.
- A queue of chickens at a nest is lock contention.
- Eyes in the hedge mean memory pressure is rising. A fox after eyes have been gathering is the end of that story.
- Chickens stuck in the mud are in `D` state: waiting on I/O that is not coming back quickly.

## Further reading

- Vicsek, T., Czirók, A., Ben-Jacob, E., Cohen, I. & Shochet, O. (1995), "Novel type of phase transition in a system of self-driven particles", *Phys. Rev. Lett.* 75, 1226.
- Toner, J. & Tu, Y. (1995), "Long-range order in a two-dimensional dynamical XY model: how birds fly together", *Phys. Rev. Lett.* 75, 4326.
- Reynolds, C. (1987), "Flocks, herds and schools: a distributed behavioral model", *SIGGRAPH '87*.
- Chaté, H., Ginelli, F., Grégoire, G. & Raynaud, F. (2008), "Collective motion of self-propelled particles interacting without cohesion", *Phys. Rev. E* 77, 046113: the first-order character of the Vicsek transition.
- Peters, J. et al. (2022), "Questioning new answers regarding Holocene chicken domestication in China", and the 2022 Exeter-led work cited by it, for the dating debate.
- Linux kernel documentation, `Documentation/admin-guide/cgroup-v2.rst`, for how cgroup accounting and the OOM counters work.
