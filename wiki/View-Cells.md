# View: Cells

Cells turns every cgroup into a living cell in a petri dish. Three dishes hold the machine's system services, your
session and your containers. A cell's area follows the memory the kernel charges to its cgroup, a dashed ring marks
its memory limit, an arc around the membrane shows its CPU use against its quota, the membrane trembles with the
pressure the kernel measures for that group, flashes red when the quota throttles it and bursts when the OOM killer
strikes inside it. The processes of a cell are its organelles; the largest when the cell appears is the nucleus, and it keeps the centre until it exits.

![Cells: cgroups as cells in three petri dishes](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/cells.webp)

## The system being modelled

### Control groups

A Linux **control group** (cgroup) is a node in a tree to which the kernel attaches processes and, through
*controllers*, accounts for and limits what the processes underneath it use. The mechanism started as "process
containers" at Google and was merged in Linux 2.6.24 (2008); the second, *unified-hierarchy* version (cgroup v2,
declared non-experimental in Linux 4.5) has a single tree with all controllers in it, which is what systemd uses to
give every service, every user session and every container its own node. Containers are cgroups plus namespaces:
the cgroup is the accounting boundary, the namespaces are the illusion of a private machine.

The files isotop reads, in each cgroup directory under `/sys/fs/cgroup`:

| File | Contents | Used for |
|---|---|---|
| `memory.current` | Bytes charged to the group: anonymous memory, page cache, kernel memory (slab, stacks) of it and its descendants | Cell size |
| `memory.max` | Hard limit in bytes, or `max` | Limit ring |
| `memory.events` | Counters `low`, `high`, `max`, `oom`, `oom_kill` | Bursts |
| `cpu.stat` | `usage_usec` (cumulative CPU time of the group and descendants), `nr_periods`, `nr_throttled`, `throttled_usec` | CPU arc and throttle flash |
| `cpu.max` | `quota period` in microseconds, or `max period` | CPU quota |
| `pids.current`, `pids.max` | Tasks in the group, and the cap | Inspector |
| `cpu.pressure`, `memory.pressure`, `io.pressure` | PSI, below | Trembling |

**CPU bandwidth control.** With `cpu.max` of `150000 100000` the group may use 150 ms of CPU time (summed over all its
CPUs) in every 100 ms period, i.e. at most 1.5 cores. When the quota is spent the group is *throttled*, its tasks
not scheduled, until the period ends; `nr_throttled` counts the periods in which that happened. A quota is therefore
a duty-cycle limit, not a limit on instantaneous parallelism. isotop expresses it as `100 * quota / period` percent of one core.

**Memory limits and the OOM killer.** When a group reaches `memory.max` the kernel first reclaims (dropping page cache,
swapping if allowed) and only if that fails invokes the out-of-memory killer *inside the group*, which kills a
process and increments `oom_kill`. The limit is enforced on `memory.current`, which includes file-backed pages the
group has touched; it is not the sum of its processes' resident sets.

### Pressure stall information

Utilisation cannot tell a busy, healthy resource from a busy, starved one. **PSI** (pressure stall information,
Johannes Weiner, merged in Linux 4.20) measures lost time directly. Take the tasks of a group sharing one resource
(CPU, memory or I/O). A task is *stalled* when it could run but is waiting for that resource (runnable on a
saturated CPU, in direct reclaim, waiting on block I/O), *productive* when it runs, and *idle* when it sleeps
voluntarily. Over a window $T$:

$$
\text{some} = \frac{1}{T}\int_T \mathbf{1}\bigl[\text{at least one task is stalled}\bigr]\,dt,\qquad
\text{full} = \frac{1}{T}\int_T \mathbf{1}\bigl[\text{at least one task is non-idle and all non-idle tasks are stalled}\bigr]\,dt .
$$

so $\text{full}\le\text{some}$. *some* is the share of wall time in which the workload lost throughput to the
resource; *full* is the share in which nothing at all got done. The kernel exports a cumulative `total` in
microseconds and three running averages. It closes a period every 2 s and folds the period's share $p_k$ into

$$
a_k = a_{k-1}\,\varepsilon + p_k\,(1-\varepsilon),\qquad \varepsilon_{10} = e^{-2/10},\ \varepsilon_{60} = e^{-2/60},\ \varepsilon_{300} = e^{-2/300},
$$

in fixed point (factors 1677, 1981 and 2034 out of 2048, i.e. 0.8189, 0.9672 and 0.9932). `avg10` is thus an
exponentially weighted average with a 10 s *time constant*, not a box of ten seconds: a stall of 100 % that lasts
exactly ten seconds reads $1 - e^{-1} = 63\,\%$, and it decays by the same factor after the stall ends. (Across
CPUs the kernel measures per CPU and combines the figures weighted by each CPU's non-idle time; the single-resource
picture above is the one-CPU case.) For the cgroup files only `some` is meaningful for CPU in older kernels; memory
and I/O carry both lines. isotop reads the first line, `some`, and its `avg10`.

![Pressure stall information](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/cells-psi.svg)

### The petri dish and the cell

A **petri dish** is a shallow lidded plate: a bounded, observable culture. Julius Petri's modification of Koch's plate
method dates from 1887. A **cell** is a membrane-bound compartment whose interior (cytoplasm) holds a nucleus and
other organelles; a cgroup is a compartment with an accounting boundary and limits enforced at it, holding
processes. The correspondence is the right *shape* of metaphor, and that is all it is. In a real eukaryotic cell the
nucleus holds the genome and directs the rest, the nucleus is of the order of a tenth of the cell's volume, and the
membrane is a lipid bilayer that fluctuates thermally; none of that is modelled. Here the nucleus is simply the
largest member when the cell appears (it then keeps the centre until it exits), the membrane's tremble is a pair of sine modes, and the area scaling is a choice
of 2D drawing, not a biological law. (The standard reference for cell structure is Alberts et al., *Molecular
Biology of the Cell*.)

## History

Cells joins two histories: the culture dish, and the kernel's control groups.

* Robert Koch's laboratory in the early 1880s grew bacteria on solid media, first gelatin plates, so that a single cell could found a visible colony. Gelatin melts at body temperature and is eaten by some bacteria. The substitute, agar, is credited to Fanny Hesse, who used it in her kitchen and whose husband Walther worked in Koch's laboratory in 1881 to 1882; sources report that Koch's 1882 tuberculosis paper mentioned agar without crediting either Hesse.
* Julius Petri, Koch's assistant, described his shallow lidded glass dish in 1887, in a short paper in the *Centralblatt fur Bakteriologie und Parasitenkunde*. The technique was at first called Koch's plate method; since the early twentieth century only Petri's name has stayed in common use.
* Linux control groups began at Google in 2006 as "process containers", led mainly by Paul Menage and Rohit Seth, and were renamed control groups to avoid confusion with the general term container. They were merged in kernel 2.6.24 (released January 2008). Sources differ on the year of the merge (2007 or 2008), so the page above says 2008, the release.
* The first interface, now called cgroup v1, had a separate hierarchy per controller. Tejun Heo found it could not be redesigned without breaking it and added a second, unified interface; cgroup v2 was declared non-experimental in Linux 4.5 (March 2016), after design work that sources date to 2012 or 2014.
* Pressure stall information (Johannes Weiner, Linux 4.20) is newer still, and is what lets the membrane tremble for a measured reason.
* Containers as most people meet them are cgroups plus namespaces. A dish was a compartment with an accounting boundary for most of a century before the kernel had one.

## Theory

### Area, not volume

A cell drawn as a disc of radius $r$ has area $\pi r^2$. Cells makes the *area* proportional to memory once memory
is large, by taking $r \propto \sqrt{M}$. For a cgroup with $M$ MiB charged and seat span $s$ (the highest organelle
seat plus one):

$$
r = \max\!\Bigl(1.3 + 0.16\sqrt{M},\; 0.75\sqrt{s+1} + 1\Bigr).
$$

The second term keeps enough room for the organelles: they sit on a Vogel (golden-angle) spiral, distance
$d_k \propto \sqrt{k + 1}$, so the area they need grows linearly with their number. The limit ring uses the same law:
if the cell has area proportional to the usage $u = M / M_{\max}$ of its limit, a circle of the area for the whole
limit has radius $r/\sqrt{u}$. isotop draws the ring at $r\sqrt{1/u}$ clamped to between $1.04\,r$ and $1.6\,r$
and only once $u \ge 0.25$, so cell and ring areas stand in the ratio of use to limit wherever the clamp does not bite.
The ring and membrane shift from amber to red between $u = 0.6$ and $u = 1$.

### Pressure to tremble

The membrane outline is
$r(\theta, t) = r\,\bigl[1 + w\sin(5\theta + 2.2\,t + \varphi) + \tfrac{1}{2}w\sin(3\theta - 1.7\,t)\bigr]$ with a per-cell phase
$\varphi$ (a hash of the cgroup path) and the amplitude

$$
w = 0.03 + 0.22\,\frac{p}{p + 15},
$$

where $p$ is the largest of the cgroup's three `some avg10` percentages. $w$ is 0.03 at rest, 0.14 at $p = 15$ and
approaches 0.25. The curve is a saturating knee, the same $x/(x+k)$ shape (`bounded`) isotop uses for almost every
unbounded quantity.

![Transfer functions of a cell](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/cells-transfer.svg)

## Mapping

| Visual element | Measured quantity (platform source) | Transform |
|---|---|---|
| Dish | Process kind, from the cgroup path (`classify`: container markers `docker`, `containerd`, `libpod`, `machine.slice`, `kubepods`, `lxc`; `user.slice` or `/user@` is session; the rest system) | Three dishes: system services, your session, containers; kernel threads belong to none and are only counted |
| Cell | One cgroup path (`Process::cgroup`, from `/proc/<pid>/cgroup`, unified `0::` line); without cgroups, the process group (app bundle and user on macOS) | One per distinct path per dish |
| Cell area | `memory.current` of the cgroup (`Unit::memory`); without a unit, the sum of members' memory | $r = \max(1.3 + 0.16\sqrt{\text{MiB}}, 0.75\sqrt{s+1} + 1)$ |
| Dashed ring | `memory.max` | $r\sqrt{1/u}$ clamped to $[1.04r, 1.6r]$ when $u \ge 0.25$; amber to red over $u$ in 0.6 to 1 |
| CPU arc | `cpu.stat usage_usec` rate (`Unit::cpu`, percent of one core) over `cpu.max` quota (100 when none) | Sweep $\min(\text{cpu}/\text{quota}, 1)$ of a turn from 12 o'clock; red above 0.9 and only with a quota |
| Cytoplasm brightness | Same CPU | tint $0.16 + 0.14\,\text{cpu}/(\text{cpu}+50)$ of the dish colour |
| Trembling | Largest of `cpu.pressure`, `memory.pressure`, `io.pressure` `some avg10` | Amplitude $0.03 + 0.22\,p/(p+15)$; two sine modes (5 and 3), phase from a hash of the name |
| Red flash | `nr_throttled` grew since the previous reading | $\tfrac12 + \tfrac12\sin(12\,t)$ mix of the membrane with red while true |
| Burst | `oom_kill` in `memory.events` rose | 24 glows expanding to $r(1 + 1.5\,a)$ and fading over 1.6 s |
| Organelle | A drawn process in the cell | One sphere |
| Organelle size | `Process::memory` (eased) | $0.8\,\texttt{mass\_radius}$, at most $0.42\times$ spacing, times a sprout ease-in for births |
| Organelle glow | `Process::cpu` above 1 % | Radius and strength grow with $\text{cpu}/(\text{cpu}+50)$ and $\text{cpu}/(\text{cpu}+40)$ |
| Nucleus | The cell's first nucleus is its largest member (ties go to the smaller identity); it keeps the centre until it exits, and a cell that has lost it gives it to the largest process that was not in the cell at the previous frame (`Dishes::elect`); if nobody new arrives the cell has none | At the centre, radius `mass_radius` clamped to $[0.45, 0.32\,r]$, with a soft glow |
| Seat | Stable per member (`Seats`) | Golden-angle spiral of spacing $\max\big((0.7\,R - 0.4)\cdot 0.8/\sqrt{n+1},\ 0.75\big)$ where $R$ and $n$ are the radius and seat count of the cell's `Room`, the room it reserved (`SHRINK`, `MARGIN`, `SPREAD`, `SEAT`) |
| Label | Cgroup leaf with `.service`/`.scope` and systemd escapes removed, at most 28 characters | Only the 12 largest cells of a dish are named |
| Inspector | `Unit` fields | `cell memory X (limit Y) \| CPU Z% of a Q% quota \| tasks`, `pressure cpu a% mem b% io c% \| throttled \| n OOM kills` |
| Dish disc, ring lines, cytoplasm shade, spiral stems | none | Decorative |

## How isotop realises it

**Data flow.** The cgroup files are read by `platform::account` on the background thread every 2 s
(`model::BACKGROUND_PERIOD`) for the cgroup paths of the sampled processes, plus the previous cycle's paths so that a
group whose only process was just OOM-killed is read once more. Rates are changes of the cumulative counters since the previous reading; a counter that
went backwards (a cgroup recreated under the same path), or no previous reading, gives `None`, never zero. The first
sample of `Unit::cpu` and `Unit::throttled` is therefore unknown, and a cell then draws no arc. Without a unified
hierarchy mounted at `/sys/fs/cgroup` with a `memory` controller, `Source::Cgroups` is missing: cells are formed by
group name, sized by the members' own memory, and the legend says "no limits, quotas or pressure".

**Placement.** Cells is a `ViewModule` in [`src/cells.rs`](https://github.com/tfpickard/isotop/blob/main/src/cells.rs): the state struct `Dishes` is built by its registry row in `views::REGISTRY` with a closure, and `Scene` holds none of it. `Dishes` keeps `Seats` (a seat per process within its cell, for life), a
`Discs` packing of cells within each dish and of the dishes themselves, a `Room` per cell, and the nucleus of each cell. A disc keeps its centre until it needs more
than it reserved (reservation is 1.15 times the need), then only it moves; a dish's drawn radius only grows
(`radii`, with 5 % headroom). Seats, cell sizes and places are reserved for every *sampled* process, drawn or not, so
focusing a subtree or lowering `--limit` frees nothing and clearing it puts everything back; each process is measured as the scene eases it (`Stage::eased`) so hiding one never changes the figures that size and rank the cells.

**Rooms.** A cell's seats are not laid out for the membrane it has this frame but for a `Room` it reserved: the membrane radius and seat count at the time, with the seats spread over $0.8$ (`SPREAD`) of the smallest membrane the cell is expected to keep, $0.7$ (`SHRINK`) of the room's radius, less a margin of 0.4 (`MARGIN`). Memory can thus move the membrane between about half and double its memory without moving a seat. `Room::serves` keeps the room while the outermost seat stands inside the membrane and the membrane has not grown past `SPARSE` = 1.5 times the room's radius; otherwise `Dishes::reserve` lays it out again for the cell as it is.

**Drawing.** Each cell is 64 cytoplasm facets (all pickable as the nucleus, so clicking the cell selects its largest
process, unless the nucleus is hidden), two membrane lines (inset 0 and 0.07), the dashed ring (72 segments, every other
one drawn), the CPU arc (64 steps) and, on a kill, the burst. Organelles are drawn after, each with a thin stem to the
centre. Dish labels read `title: N cells, M processes`; a line counts the kernel threads outside every cell.

**Time.** There is no simulation: the tremble, flash and burst are functions of `stage.time`. A kill is remembered per
cell in `kills` (count and when it last rose) for every sampled cell, shown or not, so a kill that happens while a cell is
hidden does not burst when it comes back, and one recorded while another view is shown is not replayed.

**Stable layout.** The row waives no property. Seats, rooms and discs are stable, and the nucleus keeps the centre until it exits, so a change in memory neither swaps it with a seated member nor moves an organelle (tests `the_nucleus_keeps_the_centre_until_it_exits_and_the_next_arrival_takes_it` and `hiding_a_process_does_not_change_which_process_is_the_nucleus_while_memory_eases`). A cell whose nucleus exits and that gains nobody has none, and no seated organelle moves to the centre.

**CPU and GPU.** Cells draws facets, lines, glows and sphere impostors with no new primitive, so the backends agree. Its row
uses the `Dusk` backdrop and no warm-up.

## Reading the picture

* **Cell size is the kernel's charge, not the sum of its organelles.** `memory.current` includes page cache charged
  to the group and kernel memory; organelle sizes come from each process's resident set. A large cell with small
  organelles is a group with a lot of cache, which is normal for a service that reads files.
* **The CPU arc is the whole group, hierarchical.** `usage_usec` counts descendants too. The organelles' glow is per
  process. The arc and the glows can disagree when a group's work is done by processes isotop does not show (hidden
  by `--limit`) or that exited between samples.
* **Trembling is the worst of three resources, delayed.** The amplitude uses a 10 s exponential average that the kernel
  updates every 2 s and isotop reads every 2 s. The membrane keeps trembling for seconds after the cause ends, and a
  burst of stalls shorter than a few seconds barely registers. The view does not say which resource; the inspector
  does.
* **A red flash means "a period was throttled since the last reading", not "is throttled now".** The bit comes from
  `nr_throttled` growing over a 2 s window.
* **Bursts are late by design.** The kill count is read every 2 s; the membrane bursts when it rises, which can be a
  sample or two after the victim vanished.
* **Nothing is shown for a cgroup without processes isotop sampled.** Cells exist only for the cgroups of sampled
  processes.
* **The metaphor's biology is decorative.** Nucleus and organelles carry no functional meaning: the nucleus is the
  largest process when the cell appeared (or the largest newcomer after the first nucleus exited), which is often, but
  not necessarily, the main process of the service, and it need not be the largest now.
* **macOS has no cgroups.** There Cells groups by app bundle and user and draws no limits, quotas, pressure or kills.

## Try it

```sh
./target/release/isotop --demo --view cells --width 1600 --output cells.png --renderer cpu
./target/release/isotop --demo --view cells --time 60 --output cells60.png
./target/release/isotop --view cells        # live; press 6 in a session
```

The demo makes every third family limit its memory to 1.5 times its use, every fourth quota 120 %, a pressure wave per
cell, and an OOM kill roughly every 23 s in one family, so rings, throttle flashes, trembling and bursts all occur.
Click a cell to select its nucleus and read the cell's totals in the inspector. `f` focuses the selected subtree and
leaves the cell where it was.

## Where in the code

* [`src/cells.rs`](https://github.com/tfpickard/isotop/blob/main/src/cells.rs): `Dishes`, `Dishes::draw`, `Dishes::draw_cell`, `Dishes::elect`, `Dishes::reserve`, `Dishes::watch_kills`, `Dishes::legend`, `Room`, `describe`, `cell_name`, `short`; constants `DISHES`, `SEAT`, `SPREAD`, `SHRINK`, `MARGIN`, `SPARSE`, `LIMIT`, `NAMED`, `DISH_GAP`, `LYSIS_SECONDS`.
* [`src/platform/linux/mod.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/mod.rs): `account`, `account_under`, `parse_pressure`, `classify`, `cgroup_path`, `cgroup_name`.
* [`src/model.rs`](https://github.com/tfpickard/isotop/blob/main/src/model.rs): `Unit`, the `background` thread, `accounted`, `merge`.
* [`src/layout.rs`](https://github.com/tfpickard/isotop/blob/main/src/layout.rs): `Seats`, `Discs`, `spiral`, `spin`, `GOLDEN_ANGLE`.
* [`src/views.rs`](https://github.com/tfpickard/isotop/blob/main/src/views.rs): the `cells` row (no waived property, no warm-up).
* Figures: `docs/figures/cells-psi.py`, `docs/figures/cells-transfer.py`.

## Variations and open questions

* **Nested cells.** The cgroup tree is a hierarchy; a service with child cgroups could be drawn as a cell holding
  cells (endosymbiosis is the obvious joke and the obvious geometry).
* **Split the cytoplasm.** `memory.stat` separates `anon` from `file`; anonymous memory could be the cytoplasm and page
  cache a halo, which would show directly why cell area exceeds the sum of organelles.
* **A second ring for `memory.high`**, the soft limit at which the kernel starts throttling allocation.
* **PSI `full` as its own signal.** `full` means no useful work at all; it could flash where `some` only trembles.
* **PSI triggers.** The kernel can wake a poller when stall time within a window exceeds a threshold, an event
  source with exact times, in place of reading `avg10`.
* **Open:** is an area law the right size encoding for memory when the limit ring is meant to be read as a ratio of areas?
  People read areas poorly; a bar around the membrane would be more accurate and less pretty.

## References

* Linux kernel documentation: `Documentation/admin-guide/cgroup-v2.rst` (control group v2: `memory.*`, `cpu.max`,
  `cpu.stat`, `pids.*`) and `Documentation/accounting/psi.rst` (pressure stall information, by Johannes Weiner).
* J. R. Petri, "Eine kleine Modification des Koch'schen Plattenverfahrens", Centralblatt für Bakteriologie und
  Parasitenkunde 1, 1887, pp. 279-280.
* B. Alberts et al., *Molecular Biology of the Cell* (Garland Science; any edition), for the structure of the cell.
* H. Vogel, "A better way to construct the sunflower head", Mathematical Biosciences 44, 1979, pp. 179-189 (the
  spiral the organelles sit on).
* Related pages: [Theory: Background sampler and demand](Theory-Background-Sampler-and-Demand),
  [Theory: Sampling on Linux](Theory-Sampling-Linux), [Theory: Stable layout](Theory-Stable-Layout),
  [View: Coop](View-Coop) and [View: Reef](View-Reef), which group processes by the same cgroups.
