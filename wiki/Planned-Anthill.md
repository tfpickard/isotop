> **Planned, not yet implemented.** Specified in [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md) and tracked in issue [#47](https://github.com/tfpickard/isotop/issues/47).

# Planned: Anthill

The `anthill` view is specified as sandy terrain on which every process is an ant nest, a mound with entrances sized by its memory. Open files are food piles, laid out on a stable map of the filesystem with regions for `/usr`, `/home`, `/var` and `/tmp`. Streams of ants run between nests and food: a crumb carried home is a read, a crumb carried out is a write; one ant walks for every K system calls, and the size of the crumb is the number of bytes per call. Pheromone glows along well-used routes and fades when they are abandoned, pipes and sockets are dashed tunnels between nests, and a picnic blanket stands for the terminal. This page is the biology of foraging and stigmergy, the optimisation algorithm that grew out of it, the exact meaning of the `/proc/<pid>/io` counters, and above all the line between what the view *measures* and what it *infers*, because the per-file attribution of I/O is the part of this view most easily over-read. Nothing here is implemented: `isotop --list-views` does not list `anthill`.

![Reference mockup of the anthill view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/anthill.jpg)

*The reference mockup (`docs/mockups/anthill.jpg`), rendered in Python by `docs/mockups/code/anthill.py` from synthetic data. It is the visual target, not an isotop render.*

## The system being modelled

An ant colony forages without a plan. A scout that finds food returns to the nest laying a chemical trail (a **pheromone**), and nest-mates that meet the trail follow it, and reinforce it on their own way home if they found food. Pheromone **evaporates**, so a trail that is no longer reinforced disappears. No ant knows the route, the distance or the colony's needs; the structure is stored in the environment, in the trail, and read by whoever walks past.

The French entomologist Pierre-Paul Grassé named this kind of coordination **stigmergy** (from the Greek *stigma*, a mark, and *ergon*, work) in 1959, studying how termites rebuild a damaged nest: the work done so far is itself the stimulus that directs the next worker's action, with no direct communication needed. Two decades later the Brussels group of Jean-Louis Deneubourg showed experimentally with Argentine ants that a colony offered two routes of unequal length to a food source converges, in most trials, on the shorter one (the *double bridge*; Goss et al. 1989, Deneubourg et al. 1990), a result that follows from nothing more than positive feedback and evaporation. Marco Dorigo turned the mechanism into an algorithm in the early 1990s (**ant colony optimisation**, ACO), and the family of methods is still used for routing and scheduling problems.

Why it is a good metaphor for system calls. A process that reads and writes files makes a stream of requests to the kernel from a fixed home (the process) to destinations (files, pipes, sockets) that persist across requests, in numbers that can be counted but not individually followed: the machine tells us *how many* calls and *how many bytes*, not which call went where at what moment. A colony is the same kind of object: individually invisible workers, a countable flow, persistent destinations. The view makes each ant stand for K calls, the destinations the files, and lets routes emerge. The routes themselves are decorative; the endpoints and the rates are data.

## Theory

### Stigmergy and the double bridge

A minimal model of the double-bridge experiment (Deneubourg and colleagues): an ant arriving at the fork picks branch A with probability

$$
P_A = \frac{(k + a)^{n}}{(k + a)^{n} + (k + b)^{n}},
$$

where $a$ and $b$ are the amounts of pheromone laid so far on the two branches, $k$ is the strength of the attraction of an unmarked branch (the ants' choice is not deterministic when no trail is present) and $n$ the nonlinearity of the response; values near $k \approx 20$ and $n \approx 2$ fitted the Argentine ant data (as I recall; unverified). Positive feedback appears because an ant that chooses A makes A more attractive to the next ant. Two things follow:

* With two **equal** branches the colony nevertheless picks one: the early choices are random and then amplified, a **symmetry-breaking** bifurcation. Different colonies pick different branches (in a symmetric experiment about half choose each).
* With branches of **unequal** length the short branch accumulates pheromone first, because ants complete the round trip and deposit sooner, and the colony converges on it, in most but not in all colonies.

I simulated exactly this model (one ant per time unit, round trips 10 and 10, or 10 and 20, a branch marked only when an ant has finished the trip) with a fixed seed. Of 30 colonies on equal branches 16 ended mostly on branch A and 14 on B; with unequal branches 27 of 30 ended mostly on the short one (top panels below). Evaporation, left out of this minimal model, makes the long branch's marks fade and sharpens the convergence, and keeps the colony able to *change its mind* when the world changes, which is why isotop's pheromone decays.

![Double-bridge simulation, and the view's ant scale K and crumb size](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/anthill-stigmergy-crumbs.svg)

*Top: the double-bridge model; thin lines are individual colonies. Bottom left: the specified ant scale K against the syscall rate. Bottom right: crumb radius against bytes per syscall. Script: `docs/figures/anthill-stigmergy-crumbs.py`.*

### Ant colony optimisation

In ACO (Dorigo, Maniezzo and Colorni 1996) artificial ants build tours on a graph; the probability that ant $k$ at node $i$ chooses edge $ij$ combines the pheromone $\tau_{ij}$ with a heuristic desirability $\eta_{ij}$ (for the travelling salesman problem, the inverse of the distance):

$$
p_{ij}^{k} = \frac{\tau_{ij}^{\alpha}\,\eta_{ij}^{\beta}}{\sum_{l \in \mathcal{N}_i^k} \tau_{il}^{\alpha}\,\eta_{il}^{\beta}},
$$

and after every ant has finished its tour the pheromone is updated by evaporation and deposit,

$$
\tau_{ij} \leftarrow (1 - \rho)\,\tau_{ij} + \sum_{k} \Delta\tau_{ij}^{k}, \qquad \Delta\tau_{ij}^{k} = \frac{Q}{L_k}\ \text{if ant } k \text{ used edge } ij,
$$

with $L_k$ the length of its tour: shorter tours deposit more per edge. $\rho$ is the evaporation rate, which keeps early random choices from locking in. The view does not solve an optimisation problem: it uses the *rule* (probabilistic steps weighted by pheromone and by the heading toward the target) to produce plausible routes between endpoints that the data fixes.

### What the counters in `/proc/<pid>/io` mean

Linux keeps seven I/O counters per process. Four of them are the view's raw material, and they are *syscall-level*, not device-level:

| Field | Meaning (after `proc(5)`) |
|---|---|
| `rchar` | bytes the process has passed to `read(2)` and similar calls, whether or not the data came from a disk (the page cache, a pipe, a socket and a terminal all count) |
| `wchar` | the same for `write(2)` and similar calls |
| `syscr` | the number of read-like system calls (`read`, `pread`, ...) |
| `syscw` | the number of write-like system calls |
| `read_bytes`, `write_bytes`, `cancelled_write_bytes` | bytes the process caused to be fetched from or sent to the *storage layer*; the existing `Process.read_rate` and `write_rate` come from these |

From two successive readings, a rate for each follows, and the ratio $\mathrm{rchar}/\mathrm{syscr}$ is the **mean bytes per read call** over the interval: it is a mean, and says nothing about the spread of individual call sizes. The model already carries these as `Process.syscall_io: Option<SyscallIo>` (`reads`, `writes` in calls per second; `read_chars`, `write_chars` in bytes per second), which is `None` until a second reading exists, after an exec or a counter that went backwards, when `/proc/<pid>/io` is unreadable, and always on macOS. P5 (issue #21) added it and it is on `main`.

### Which file? Attribution from descriptor positions

The kernel does not say which file each `read` went to. What it exposes is, for each open file descriptor, a **position**: `/proc/<pid>/fdinfo/<fd>` has a `pos:` line with the current file offset and a `flags:` line with the open flags in octal. Building block B8 (issue #35) turns this into an estimate, with these binding rules:

1. Key a descriptor by (`Identity`, fd, device, inode, access mode). If any part of the key changes between two samples, the descriptor is new and gets no delta.
2. A positive `pos` advance goes to `read` if the file is open `O_RDONLY`, to `written` if `O_WRONLY`, and to `either` if `O_RDWR`: the split is **never guessed**. A negative advance (a rewind) attributes nothing.
3. If the attributed totals exceed the interval's `rchar` and `wchar` deltas (an `lseek` jump can cause that), they are scaled down proportionally.
4. The **unattributed** remainder is the `rchar` and `wchar` delta minus the attributed bytes, floored at 0. It contains everything that does not move an offset: `pread` and `pwrite`, memory-mapped I/O, asynchronous and `io_uring` I/O, and I/O to descriptors that were opened and closed between two samples; it also contains pipe, socket and terminal traffic, which have no meaningful offset.

Only the 24 busiest processes by syscall rate are scanned (`IO_PROCESSES = 24`), with caps of 256 regular files per process and 4,096 descriptors per pass; a pass is marked partial when a cap is hit. On macOS `fi_offset` plays the role of `pos`, there is no read/write syscall count, and the remainder is `None`.

### Measured versus inferred

| Quantity | Status | Basis |
|---|---|---|
| Calls per second, bytes per second, per process | **measured** (Linux) | `/proc/<pid>/io` counter deltas: `syscr`, `syscw`, `rchar`, `wchar` |
| Which regular files are open, their size, whether deleted | **measured** | the descriptor table and `statx`; unreadable for other users' processes without privilege |
| Bytes advanced in a file's offset over the interval | **measured** | `pos` at two samples |
| Bytes read from or written to a given file | **inferred** | the `pos` advance, valid only for sequential I/O on a descriptor that stayed open across both samples; `pread`, `mmap` and `io_uring` are invisible to it |
| Direction of an `O_RDWR` file | **not known** | kept as `either` and never split |
| Number of calls that went to a given file | **inferred** | the view splits the process's call rate in proportion to attributed bytes, which assumes the call size is similar across its files |
| Bytes per call, per crumb | **inferred (a mean)** | $\mathrm{rchar}/\mathrm{syscr}$ over the interval; one crumb size per flow |
| Where unattributed bytes went | **not known** | drawn through the process's pipes and sockets if it has any (weighted by their traffic), to the terminal's blanket if it has a tty descriptor, otherwise counted and not drawn |
| The shape of each ant's route, pheromone strength, ant motion | **decorative** | the ant-colony rule; endpoints are data, routes are not |
| A file opened and closed between two samples | **invisible** | its key never appears in two successive samples; a background sample is taken about every 2 s |

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Nest (mound) | a process | radius $0.9 + 0.38\log_2(\max(\mathrm{MiB}, 8)/8)$ world units from `Process.memory`; a paraboloid mesh of 24 segments (12 below 8 px radius); up to 4 entrances, one per distinct destination; placed by `layout::vacant` (the issue writes `render::vacant`) with a 1.15 times reservation, so a nest moves only when it outgrows it |
| Food pile | an open regular file in B8's `FileFlow` list | one per file, capped at 256 by attributed bytes; radius $0.45 + 0.12\log_2(\mathrm{size}/4096)$, clamped to 0.45 to 3.0; at most 24 pebbles placed by a path hash; pickable through B9 `thing_pick`, kind `"file"`, key from device and inode |
| Position of a pile | the file's path | a stable treemap of the directory tree: a square slab with a fixed pinwheel, a central clearing for the nests surrounded by `/usr`, `/home`, `/var`, `/tmp` and `other`; second-level directories take `layout::Seats` cells; each file at an FNV-hashed spot inside its cell |
| One ant | K system calls | the smallest K in the 1-2-5 sequence with $\sum \mathrm{rate}\cdot\mathrm{mean\_trip}/K \le 3000$; K changes only after 3 consecutive agreeing samples; the pool is hard-capped at 3,750 ants and refused spawns are counted |
| Direction of an ant | read versus write | a read ant goes nest to file and comes back with a crumb; a write ant carries a crumb out; `either` files send ants that carry a crumb both ways in the midpoint colour |
| Crumb size | `read_chars / reads` or `write_chars / writes` | the `crumb_px` curve ($0.55 + 0.27\log_2(\mathrm{bytes}/64)$ px, clamped 0.42 to 3.6 at 1600 px in the prototype), scaled by zoom, at least 1 px; read crumbs pale `[255,238,196]`, write crumbs coral `[255,118,92]` |
| Ants per destination | attributed bytes | the process's `reads` and `writes` per second split across destinations in proportion to attributed bytes |
| Tunnel (dashed ribbon) | pipe, Unix and loopback TCP links (B2) | `Frame::ribbon` with a dash moving with the flow at a speed proportional to $\log_{10}(\mathrm{syscalls/s})$; no ants travel along tunnels |
| Blanket | a tty descriptor | up to 4, at fixed slots on the clearing's edge; reads from a tty are ants bringing back what you typed |
| Pheromone glow | the ants' own deposits | 3 channels (the nest's kind colour), 4 cells per world unit, at most 256 per side; deposit 1.0 while carrying and 0.45 otherwise; evaporation $e^{-dt/2.2}$, then a 3-tap blur |
| Ant speed | none | 12 world units per second (the routes and speeds are decoration) |
| Sand ripples, pebble placement, blanket checks, slab sides | none | **decorative** |

## Design

Everything below is the specification of issue #47. None of it exists.

**Module and registry.** `src/anthill.rs` with `Anthill: Default`, registered through the P1 registry with `Backdrop::Dusk`, drawing through `Stage` (`draw(&mut Stage, &[&Process], &Snapshot) -> usize`). It declares B8's source in the row's `sources` and warms up 10 s before a headless `--time` frame. No view-local keys. There is no plugin system.

**Dependencies.** Status at commit `192299b`:

| Item | Provides | Status |
|---|---|---|
| P1 (#17), P2 (#18), S1 (#22), S2 (#23), S6 (#27) | registry, picker, parity check, `FixedStep`, source demand | merged |
| P5 (#21) | `Process.syscall_io` | merged |
| B9 (#36) | `Thing` picks and `Card`s | merged |
| B2 (#29) | pipe, Unix and loopback TCP links with traffic, for tunnels | implemented in open PR [#134](https://github.com/tfpickard/isotop/pull/134), not merged |
| B8 (#35) | `IoDetail` / `FileFlow` per-file attribution and tty counts | not implemented in any open pull request |
| P3 (#19) | field primitive for the ground and the pheromone glow | not implemented in any open pull request |
| S3 (#24) | `Frame::ribbon` with `Dash` for the tunnels | not implemented in any open pull request |

**Simulation.** `FixedStep::new(0.05, 10, 0.5)` (20 Hz) with an interpolation alpha. The ants follow the prototype's `Colony.step`: steer toward a quadratic Bezier carrot with a hashed bend per flow, sense pheromone at plus and minus 0.5 rad, add noise of order $\sqrt{dt}$, deposit 1.0 while carrying and 0.45 otherwise. The pheromone grid is 3 channels by at most 256 cells a side. Each ant is seeded from (identity, destination hash, spawn sequence). Rates come from the sampled counters, not from the simulation, so the colony cannot invent traffic.

**Layout (stable).** A square slab with a fixed pinwheel; nests move only when they outgrow their 1.15 times reservation; the slab grows in 1.25 times steps only when the clearing overflows. `stage.bounds` is the slab's corners. Nests and piles keep their places when CPU, memory or the population change (a named test).

**Rendering.** The ground is one P3 field: static sand, region tint and the additive pheromone glow. Ants are two-triangle quads and are not pickable; nests are pickable by process index and piles and blankets by `thing_pick`. `places` labels the regions, the top 12 piles by traffic, and the top 2 crumb sizes (for example "128 KiB/read"). All file paths are untrusted text and pass through `terminal::clean_text` before display; the card of a pile shows the sanitized path, size, readers, writers and bytes per second.

**Legend** (specified): `mound = process (size ~ memory) | pile = open file | ant = K syscalls (K = 50) | crumb = bytes per syscall | pale = read, coral = write | glow = pheromone | dashes = pipes, sockets`, with `fdinfo unreadable for N` appended when processes cannot be read.

**Demo.** `model::demo` must provide every consumed field: consistent syscall counts and byte counts, about 24 files across the regions with `pos` deltas, one that opens and one that closes, a pipe, a Unix link, a loopback TCP link and one tty reader. The existing views' demo PNGs stay byte-identical.

**Degradation.**

* Linux: full. A process whose fdinfo is unreadable sends no ants and is counted in the legend.
* macOS: degraded but available. B8 attributes `fi_offset` advances, but `syscall_io` is `None`: an ant means K KiB moved, direction comes from the open flags, crumbs have a fixed size; the legend says `macOS reports no read/write syscall counts: ant = K KiB moved, crumbs unsized`.
* Without B8 data at all: no ants are drawn, and `missing_note` says so. Never invented numbers.

**Tests named by the issue** include `read_ants_go_to_the_files_whose_position_advanced`, `unattributed_bytes_travel_tunnels_or_the_blanket`, `k_keeps_ants_on_screen_near_three_thousand`, `k_steps_through_one_two_five_without_flicker`, `crumb_size_follows_bytes_per_syscall`, `file_paths_are_sanitized_before_display`, `pheromone_evaporates_with_its_time_constant`, `colony_follows_the_same_path_at_ten_and_sixty_frames_per_second`, `piles_and_nests_are_pickable_and_ants_are_not` and `missing_syscall_counts_switch_ants_to_bytes_and_say_so`.

## Reading the picture

* **Rates are the machine's; routes are the colony's.** How many ants leave a nest and how big their crumbs are follow `syscr`, `syscw`, `rchar` and `wchar`. How an ant walks does not. Two runs with the same data show the same stream of ants but different paths.
* **A pile with ants is a statement about file offsets.** It says that a descriptor of that file advanced, and by about how many bytes, while its process was among the 24 busiest by syscall rate. It does not say the file was read by `read(2)` rather than `pread(2)` unless the offset moved, and a database doing positioned I/O on a large file shows *no* ants to that file: its bytes are in the unattributed remainder.
* **Ants are K calls, not events.** One ant is K system calls of a flow; the legend gives K. A small K and many tiny crumbs is the signature of many small reads, a real and useful thing to see; but crumb size is a mean over the interval.
* **Unattributed bytes are on purpose visible as such.** Whatever the offsets cannot account for goes to tunnels or the blanket or is counted in the legend; it is never assigned to a file.
* **Open and close are not seen if they are quick.** A file opened and closed within one background period never appears. Short-lived processes are not seen at all by this view; see [Theory-Life-Events](Theory-Life-Events) for how exits are observed elsewhere.
* **Layout encodes the path, not importance.** A pile's position is a stable hash within its directory's cell; being close to a nest means nothing.
* **Resolution limits.** At most 256 piles, 4 entrances per nest, 3,750 ants. Processes outside the busiest 24 send no ants at all, however much they read.

## Try it

None of these work yet: `anthill` is not registered.

```sh
# Once the view lands (issue #47).
./target/release/isotop --demo --view anthill --time 12 --width 1600 --output /tmp/anthill.png
./target/release/isotop --demo --view anthill --renderer cpu --width 1920 --benchmark 120
```

The quantities the view will draw can be read by hand on a Linux machine: `cat /proc/<pid>/io`, `ls -l /proc/<pid>/fd` and `cat /proc/<pid>/fdinfo/<fd>`. The figure regenerates from the repository root with `python3 docs/figures/anthill-stigmergy-crumbs.py`.

## Where in the code

* Existing: `src/model.rs` (`Process.syscall_io`, `SyscallIo`, `Process.files`, `Snapshot.link_traffic`, `Snapshot.links`), `src/platform/linux/files.rs` (`classify`, `Bounds`, `FileScan`: the open-file scan that B8 extends), `src/platform/mod.rs` (`Source::SyscallIo`, `Source::OpenFiles`), `src/views.rs`, `src/simulation.rs` (`FixedStep`, `Rng::for_identity`), `src/layout.rs` (`Seats`, `vacant`), `src/render.rs` (`Thing`, `Card`, `Frame::thing_pick`, `label_name`), `src/terminal.rs` (`clean_text`).
* Planned: `src/anthill.rs`; B8 (`IoDetail`, `FileFlow`), P3, S3; B2 is in the open PR #134.
* Prototype, reference only: `docs/mockups/code/anthill.py` (`Colony.step`, `crumb_px`, `flow_rate`, the treemap `REGIONS`).

## Variations and open questions

* **Ant colony optimisation as a layout.** Let the ants minimise something real: for instance the total length of tunnels between nests that share sockets, and compare the emergent routes with the shortest paths. Does the colony find them (Goss et al. say yes for two routes)?
* **Memory-mapped I/O.** Page faults of file-backed mappings (`major_faults` in `Process`, already measured) are the missing channel: a process reading through `mmap` moves no offset. Major faults per second, attributed to a mapping through `/proc/<pid>/maps` and `smaps`, could be a third kind of ant.
* **Pheromone as a measurement.** The decay constant of 2.2 s is a design choice. A pheromone whose evaporation matched the sampling period would make the glow read directly as "bytes in the last N seconds".
* **Latency.** Ants could slow down when the process's wait channel (`wchan`) says it is blocked in I/O, a thing the plumbing view considers for pipes.
* **Alarm pheromone.** Real ants lay a different chemical when disturbed; a failed read (`EIO`, a `read` on a deleted file) has no counter in `/proc/<pid>/io`, so this needs another source.
* **A truth check.** On a quiet machine, run a known program (`dd`, `cat` a file) and compare the view's attributed bytes with the file size and with `rchar`: the quickest test of the attribution rules.

## References

* P.-P. Grassé, "La reconstruction du nid et les coordinations interindividuelles chez *Bellicositermes natalensis* et *Cubitermes* sp. La théorie de la stigmergie: essai d'interprétation du comportement des termites constructeurs", *Insectes Sociaux* 6, 41-80 (1959).
* S. Goss, S. Aron, J. L. Deneubourg, J. M. Pasteels, "Self-organized shortcuts in the Argentine ant", *Naturwissenschaften* 76, 579-581 (1989).
* J. L. Deneubourg, S. Aron, S. Goss, J. M. Pasteels, "The self-organizing exploratory pattern of the Argentine ant", *Journal of Insect Behavior* 3, 159-168 (1990).
* M. Dorigo, V. Maniezzo, A. Colorni, "Ant system: optimization by a colony of cooperating agents", *IEEE Transactions on Systems, Man, and Cybernetics, Part B* 26(1), 29-41 (1996).
* M. Dorigo, L. M. Gambardella, "Ant colony system: a cooperative learning approach to the traveling salesman problem", *IEEE Transactions on Evolutionary Computation* 1(1), 53-66 (1997).
* M. Dorigo, T. Stützle, *Ant Colony Optimization*, MIT Press (2004).
* The Linux manual page `proc(5)`: `/proc/<pid>/io` and `/proc/<pid>/fdinfo`.
