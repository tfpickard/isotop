# View Coop

The machine as a fenced chicken yard. Every process is a chicken, every cgroup is a flock with its own henhouse, and
the flock moves by the rule Tamas Vicsek proposed in 1995 for self-driven particles: steer by the average heading of your
flock mates, plus noise. Idle processes roost on perch ladders, running ones queue at the feeder of the CPU they last ran on in
order of priority, and the hens that hold file locks brood by the nest box where the eggs, one clutch per flock, count its open files. Foxes
come when the kernel's OOM killer takes something. The noise that scatters a flock is the machine's own instability: the
variation of a process's CPU and the system's CPU pressure.

![Coop: flocks foraging by their henhouses, roosting on perch ladders, queueing at the feeders](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/coop.webp)

![The README image of the same view, cropped to show nest boxes and feeders](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/coop.webp)

## The system being modelled

Three real things are being borrowed.

**Collective motion.** Starling flocks, fish schools and locust marching bands share a rule of thumb: align with neighbours.
Vicsek and colleagues made the minimal version, constant-speed particles that adopt the mean heading of their neighbours plus a random
kick, and found an order-disorder transition as the noise grows; see [Math Flocking](Math-Flocking) for the model, its
history and what it does at the sizes isotop uses. It is a good metaphor for a *cgroup*: a set of processes that belong together, that
are driven by the same service and that stay coherent while the machine is calm.

**A henhouse.** Hens roost on perches when idle, feed when hungry, queue at the trough in a pecking order (the term is Schjelderup-Ebbe's, 1922: a stable dominance hierarchy in which each bird pecks those below it), lay eggs in nest boxes, brood on a clutch, dust-bathe, and freeze motionless
(tonic immobility) when seized. Each behaviour maps to a process state: idle (S) roosts, running (R) feeds, zombie (Z) lies feet up, stopped (T) freezes, uninterruptible sleep (D) is stuck in the mud.

**A yard with predators.** The Linux OOM killer removes a victim chosen by memory use when a cgroup exhausts its limit; a fox that carries off the largest hen is the same event at a scale a viewer can read. Memory pressure is the fox's eyes in the hedge.

## Theory

The dynamics, in the notation of [Math Flocking](Math-Flocking): $N_f$ foragers in a flock, positions $x_i$, headings $\theta_i$, speed $v_i$, heading noise $\eta_i$.
At each step $h = 0.05$ s

$$
\theta_i' = \operatorname{atan2}\Bigl(\sum_{j\in\text{flock}(i),\,|x_j-x_i|\le 4}(\cos\theta_j,\sin\theta_j) + \text{(peer, home, fox terms)}\Bigr) + \eta_i\,\xi_i,\qquad \xi_i\sim U[-\tfrac12,\tfrac12),
$$

$$
x_i' = x_i + v_i h\,(\cos\theta_i',\ \sin\theta_i'),\qquad v_i = 0.6 + 3.4\,\frac{c_i}{c_i + 25},\qquad
\eta_i = \min\Bigl(0.5 + 0.5\min(\mathrm{CV}_i, 2) + 5.5\,\frac{\psi}{\psi+15},\ 2\pi\Bigr).
$$

The order parameter reported in the legend is $\varphi = |\sum_i\mathbf v_i|/\sum_i|\mathbf v_i|$. In the ordered regime it is near 1; for $N$ independent headings it falls to $0.886/\sqrt N$. At isotop's flock sizes the change is a smooth crossover centred near
$\eta\approx 4$ radians ($\psi\approx 20$ to $40\%$ with a steady CPU), not a sharp transition; the simulations and the figure are in [Math Flocking](Math-Flocking).

Two further pieces of theory are small. The **clutch** $\mathrm{round}(\log_2(1+n))$ is the bit length of the count $n$ of open files, so a flock with 7 files lays 3 eggs, one with 140 lays 7, and the clutch is full at $2^{15.5} - 1 = 46\,340$ files: eggs measure the *order of magnitude* of a count. The **feeder** is a priority queue: each CPU is a trough with 2 or 3 slots, the chickens that last ran on it queue behind
the slots ordered by the kernel's scheduling priority (real-time tasks, whose priority number is negative, eat first), and a hen leaves only after two consecutive samples in which she was not running (a hysteresis against the R/S flicker of a one-second sample).

## Mapping

| Visual element | Measured quantity (platform source) | Transform |
|---|---|---|
| Chicken | A process | One per drawn process; picking and the inspector use the process |
| Body radius | Memory (RSS; physical footprint on macOS) | $0.12\sqrt[3]{\mathrm{MiB}}$ clamped to 0.3 to 1.2 (volume follows memory; at or below about 16 MiB all share the smallest size), scaled in while hatching |
| Plumage | Kind and state | `palette::kind_color`; zombie pink, stopped orange, uninterruptible sleep red |
| Flock and henhouse | cgroup path (process group where there are no cgroups; kernel threads share a `kernel` flock) | One flock and one henhouse per group, labelled with the group |
| Perch seat | Seat number within the flock | A fixed ladder grid around the henhouse: bars of 6 seats, 4 bars to a ladder, each bar 0.6 higher, nearest ladders first (`layout::Seats`) |
| Roosting | CPU below 0.3 % (leave above 0.8 %; a newcomer roosts below 0.5 %) | Walks to its seat and sits on its bar |
| Foraging speed | CPU | $0.6 + 3.4\,c/(c+25)$ units per second |
| Heading noise | CPU variation over 30 samples; system CPU pressure | $\eta$ above; without pressure readings the pressure term is dropped |
| Flocking neighbourhood | cgroup | 4 units, same flock only |
| Pull towards a socket peer | Loopback TCP bytes per second; else CPU of both ends | $w = \min(\log_2(1 + B/1024)/10,\ 1.5)$ for measured TCP; $w = 0.8\,m/(m+25)$ with $m$ the smaller CPU for Unix or unmeasured pairs (the larger of the two for a pair that has both), added to the sum as $w(\cos\theta_k + \tfrac12\hat u)$ |
| Feeders | Per-core list, sorted by kind and id | Performance cores 3 slots and a gold trough, efficiency cores 2 and teal, unknown kinds 2 and neutral; grain brightness $0.25 + 0.85\times$ busy |
| Feeding | State R and the CPU it last ran on | A queue per feeder ordered by (priority, identity); an unknown core goes to the first feeder; on macOS, which reports no last CPU, all running chickens share one extra trough |
| Pecking rank | Priority and nice | Inspector line |
| Frozen | State T (stopped or traced) | Held in place, no noise |
| Mud puddle | State D | Does not move (not shown on macOS: no D state) |
| Feet up | State Z | Does not move |
| Clutch of eggs | Open regular files of the flock's members (`/proc/<pid>/fd`; `proc_pidfdinfo` on macOS) | $\min(16,\mathrm{round}(\log_2(1+\Sigma\text{files})))$, a file counted once however many descriptors refer to it; a state, not events |
| Rotten egg | Files deleted while still open | One per distinct file (device and inode), at most 8 shown; the inspector gives the count and the bytes held |
| Brooding | Holding a file lock or lease and not running (`/proc/locks`; none on macOS) | Sits on a straw pad beside her own nest box |
| Queueing at a nest | Blocked on a file lock | Walks to the nest of the *holder's* flock and queues after the brooders, with a faint line to the holder |
| Dust puffs | Read rate | $\min(5,\ 1 + \lfloor\log_2(\text{rate}/64\,\mathrm{KiB/s})\rfloor)$ above 64 KiB/s |
| Chicks | Threads | $\min(\text{threads} - 1,\ 12)$, following the hen along her last 12 s of trail |
| Fox | Rise in a cgroup's OOM-kill count | One to three foxes, 3 s each, running at the largest member that vanished in the last 4 s (or 2.5 sampling intervals if longer) |
| Fox eyes in the hedge | Memory pressure (stall percentage) | $\min(6, 1 + \lfloor\psi/10\rfloor)$ pairs above 0.5 %, at $3(1 - \psi/(\psi+20)) + 0.4$ units outside the fence; foragers within 6 units flee |
| phi in the legend | Foragers | $\lvert\Sigma\mathbf v\rvert/\Sigma\lvert\mathbf v\rvert$ |

**Decorative:** the grass grid and dirt band, the hedge, fence posts and rails, the henhouse roof and door, straw, ladder stringers, crack lines on rotten eggs, shadows, the stride and pecking head-bob, the chick hop, the swirl of dust puffs, the blink and shape of the fox, and the position a forager has at any moment (it is simulation output).

![The coop's transfer functions: speed, clutch, dust, peer pull, fox eyes](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/coop-mappings.svg)

*Every transfer function saturates, so no single process can dominate the yard. Script: `docs/figures/coop-mappings.py`.*

## How isotop realises it

**Roles.** `Coop::decide` gives each chicken the first matching role: zombie, frozen (`T`/`t`), stuck (`D`), feeding (running), waiting (blocked on a lock), feeding again (the two-sample hold),
brooding (holds a lock), roosting, foraging. So a running lock holder still feeds, and an idle holder broods instead of roosting. A request blocked on a lock sleeps, so a process that the lock table (read up to 2 s earlier) still lists as blocked but is running has been granted it, and feeds. Decisions are taken once per new sample (or when the drawn set changes), not per frame.

**Layout is stable.** Seats are numbers within a flock (`Seats::assign`, a member keeps its seat for life) and each flock's house and ladders occupy a disc (`Discs::arrange`) that keeps its centre until the flock outgrows its reservation. Seats and discs are reserved for *every sampled process*, drawn or not,
so focusing a subtree or `--limit` hiding processes frees nothing; the fence hugs the reserved discs and only ever moves outward, so a flock at the edge arriving or leaving does not move it, the feeders along it or the fox eyes around it. The harness checks the seats (`Coop::anchors`), because chickens walk.

**A fixed 20 Hz clock.** `Yard` advances in whole steps of `H = 0.05` s of wall-clock time through `simulation::FixedStep::new(H, MAX_STEPS = 10, MAX_GAP = 0.5)`; a frame draws at an interpolation fraction between the last two states, and a gap longer than 0.5 s (a pause, a stall) is dropped. The test `flock_follows_the_same_path_at_ten_and_sixty_frames_per_second` compares full states at equal step counts.

**Determinism.** Every chicken has its own generator, `Rng::for_identity(id, 0xc0_0b)`, and draws once per step whatever it does, so draws never depend on update order; neighbours are summed in identity order, so `vicsek_step_does_not_depend_on_update_order` holds bit for bit. Newcomers hatch at a gathering spot beyond the perches at a direction fixed by the flock name.

**The Vicsek step.** `Yard::step`: build a [spatial hash](Math-Spatial-Hashing) of foragers; for each, sum the unit headings of same-flock foragers within 4 units, add the peer, home and flee terms, take `atan2`, add `noise * draw`; apply all headings together; move; `separate` (a Jacobi pass: overlaps are measured before any push, a forager against a walker takes the whole overlap so walkers always arrive); `confine` (house footprints and the reflecting fence).

**Eggs, locks, foxes.** Open files and deleted-but-open files and lock state come from the 2 s background scans ([Theory Background Sampler and Demand](Theory-Background-Sampler-and-Demand)); the clutch is rebuilt from them, so it shrinks when files close. Brooders and then queueing hens take seats in a grid east of the nest box, a column at a time from its north end,
inside the flock's reserved disc and off its ladders; when more come than fit, the grid closes up to a minimum of 0.15 between seats and they crowd. For foxes, `record` (run on every sample whichever view is shown) compares each cgroup's OOM-kill count with the last, and each rise claims the largest member that left the cgroup in the last `DEPARTED_SECONDS = 4` s (at least 2.5 sampling intervals), one victim per kill; the first three kills send a fox. Calls older than a fox's run are dropped, so switching to the coop does not replay old kills.

**Degradation (stated in the legend).** No CPU pressure (kernels without PSI): the noise has its CV term only. No cgroups (macOS): flocks are process groups and no fox comes. No last CPU (macOS): running chickens share one trough. No lock table (macOS): no brooding or queueing. Unreadable I/O (other users' processes): no dust. Unreadable descriptor tables: no eggs for them (`no files for N (permissions)`). OFD locks (reported with pid -1) cannot be attributed: no hen broods, a request blocked on one queues at its own nest, and the legend counts them.

## Reading the picture

What I see in the demo render: an isometric yard inside a double hedge, about ten red-roofed henhouses each with a pale nest box of white eggs on the east side and railing-like perch ladders, flocks in the kind colours (teal, gold, purple and slate) clustered near their houses with trails of yellow chicks behind some of them, a dirt band along one edge where chickens stand at wooden troughs in
queues, a few pink and red chickens (zombies, a stopped process), and a dense knot of slate-coloured foragers in the middle.

* **Measured:** which flock a chicken belongs to, its size, state, whether it roosts, feeds or broods, the feeder (last CPU), its pecking rank, the clutch and rotten eggs (counts), chicks (threads), dust (read rate), foxes (OOM events), eyes (memory pressure).
* **Derived:** the *heading* and position of a forager: the output of a stochastic simulation driven by CPU, CPU variation and pressure. Two foragers walking together means the model says so (socket links or co-activity), not that they are doing the same work.
* **Coherence is an input in disguise.** A tight, aligned flock means low noise: steady CPU and an unstalled machine. It says nothing about what the processes do. The legend's phi is for the whole yard, so it is low when many flocks point in different directions even if each is ordered; the inspector gives the per-flock value.
* **Invisible:** a process that is idle between samples (roosting hides bursts shorter than a second), a file opened and closed within 2 s, a lock held for less than the 2 s lock scan, and all processes beyond `--limit` (they keep seats and places but are not drawn).
* **Sampling artefacts:** the feeder is the CPU the process was on *at the instant of the sample*; a migrating process hops feeders (the two-sample hold stops it shuttling between trough and perch).
* **Log scales hide differences:** 140 open files and 40 000 open files differ by 7 eggs against 15.

## Try it

```sh
./target/release/isotop --demo --view coop --width 1600 --output /tmp/coop.png --renderer cpu
./target/release/isotop --demo --view coop --time 60 --width 1600 --output /tmp/coop60.png   # fixed demo time
./target/release/isotop --view coop     # live; select a chicken: inspector lines give role, noise, eggs, locks, peers, dust
```

Keys: the global ones (Tab, `v`, `f` focus a subtree, `p` pause and rewind, `c` link modes). The yard replays 30 s of samples before a headless demo frame so CPU variation exists, then runs four seconds of simulation.

## Where in the code

* `src/coop.rs`: `Coop` (state; `record`, `draw`, `decide`, `paint`, `legend`, `anchors`), `Yard` (`step`, `separate`, `confine`, `order`, `flock_orders`), `Chicken`, `Role`, `Flock`, `Feeder`, `Nest`, `Fox`, `nest_seats`, `clutch`, `place_feeders`, `ladder`, `seat_offset`, `body_radius`, `noise`, `speed`, `variation`, `peer_weight`, `puffs`.
* `src/simulation.rs`: `FixedStep`, `SpatialHash`, `Rng::for_identity`; `src/layout.rs`: `Seats`, `Discs`, `spin`; `src/model.rs`: `bounded`; `src/platform/linux/files.rs`, `net.rs`, `mod.rs` (cgroup OOM counts, pressure); `src/palette.rs`: `core_color`, `kind_color`.
* `src/views.rs`: the `coop` row (`Backdrop::Dusk`, warm-up `Phase::Record { seconds: 30 }` for demo output plus `SETTLE`, harness `anchors`). **Note:** the row is `Hooks::BuiltIn { arm: BuiltIn::Coop, .. }` with its state in `Scene::coop`, while `AGENTS.md` lists `coop.rs` among the module views; the code is authoritative.
* Tests: `flocks_order_at_low_noise_and_dissolve_past_critical_noise`, `a_blocked_waiter_walks_to_the_holders_nest_in_another_flock`, `an_oom_kill_sends_a_fox_for_the_departed_member`, `kills_recorded_while_another_view_is_shown_do_not_replay_on_switching_to_coop`, `houses_and_perches_stay_put_when_resources_or_population_change`.
* Design lineage: the view is a port of [tfpickard/chicken](https://github.com/tfpickard/chicken) with four of its bugs fixed (in-place heading updates, an alignment readout that was always 1, one step per frame, O(N^2) neighbour search). No plugin system exists; a new view is a compile-time module plus a registry row.

## Variations and open questions

1. **A transition you can see.** Per-flock $N$ is small. Run a synthetic flock of 500 (`--processes 500` with one cgroup) and watch whether bands appear near the crossover (they are the signature of the first-order transition in the large-$N$ Vicsek model).
2. **Topological alignment.** Use seven nearest same-flock neighbours (Ballerini et al.) instead of a 4-unit disc; compare crossovers.
3. **Noise from I/O pressure.** Use the PSI I/O line as a second noise source for hens that dust-bathe.
4. **Peers by measured Unix traffic.** The kernel gives no byte counters for Unix sockets; a queue-length proxy from `sock_diag` could replace the co-activity fallback.
5. **A clutch with units.** Colour eggs by the file kind (regular, deleted, socket-backed) to restore what the logarithm hides.
6. **Per-flock phi as a sky tint.** Open: is there a quantity worth reading from the order, or should phi be removed in favour of the noise inputs themselves?

## References

* T. Vicsek, A. Czirok, E. Ben-Jacob, I. Cohen and O. Shochet, "Novel type of phase transition in a system of self-driven particles", *Physical Review Letters* 75 (1995), 1226-1229.
* G. Gregoire and H. Chate, "Onset of collective and cohesive motion", *Physical Review Letters* 92 (2004), 025702.
* T. Schjelderup-Ebbe, "Beitrage zur Sozialpsychologie des Haushuhns", *Zeitschrift fur Psychologie* 88 (1922), 225-252.
* The Linux kernel documentation on cgroup v2 (`memory.events`, `oom_kill`) and pressure stall information (`/proc/pressure`).
* [Math Flocking](Math-Flocking), [Theory Stable Layout](Theory-Stable-Layout), [Theory Determinism](Theory-Determinism), [Theory Simulation Toolkit](Theory-Simulation-Toolkit), [Theory Life Events](Theory-Life-Events).
