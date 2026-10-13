# isotop

A living picture of your machine inside the terminal, in nineteen views: a process
city, an orbital observatory, a rippling pond, a spacetime weather map, a race
track of CPU cores, petri dishes of cgroups, a ridgeline landscape of CPU
history, a globe of network connections, a coral reef, the system log (the systemd
journal, or the unified log on macOS) as Matrix rain, a chicken yard, a geocentric
cosmos of Fourier epicycles, a minute of process worldlines in a tesseract whose fourth
axis is time, a moonlit cemetery of the processes that exited, a slice of cortex
whose neurons fire with CPU, a black hole whose orbits rank the processes the OOM
killer will take next, a murmuration of starlings with one bird for each thread,
the process tree as a tree, and an anthill whose ants carry the bytes of read and write
calls between nests and files. Written in
Rust for Linux and for macOS on Apple Silicon, with real process data and pixel
graphics through the Kitty graphics protocol. Ghostty is the primary target.

https://github.com/user-attachments/assets/05663db3-6c64-48bc-8696-45f166ce325c

| | | |
|:-:|:-:|:-:|
| ![City](docs/media/city.webp)<br>**1 City** | ![Orbit](docs/media/orbit.webp)<br>**2 Orbit** | ![Ripple](docs/media/ripple.webp)<br>**3 Ripple** |
| ![Flow](docs/media/flow.webp)<br>**4 Flow** | ![Cores](docs/media/cores.webp)<br>**5 Cores** | ![Cells](docs/media/cells.webp)<br>**6 Cells** |
| ![Strata](docs/media/strata.webp)<br>**7 Strata** | ![Globe](docs/media/globe.webp)<br>**8 Globe** | ![Reef](docs/media/reef.webp)<br>**9 Reef** |
| ![Matrix](docs/media/matrix.webp)<br>**10 Matrix** | ![Coop](docs/media/coop.webp)<br>**11 Coop** | ![Ptolemy](docs/media/ptolemy.webp)<br>**12 Ptolemy** |
| ![Tesseract](docs/media/tesseract.webp)<br>**13 Tesseract** | ![Necropolis](docs/media/necropolis.webp)<br>**14 Necropolis** | ![Cortex](docs/media/cortex.webp)<br>**15 Cortex** |
| ![Horizon](docs/media/horizon.webp)<br>**16 Horizon** | ![Murmuration](docs/media/murmuration.webp)<br>**17 Murmuration** | ![Arbor](docs/media/arbor.webp)<br>**18 Arbor** |
| ![Anthill](docs/media/anthill.webp)<br>**19 Anthill** | | |

The numbers are the views' places in the Tab order (the registry's, which
`isotop --list-views` prints); the keys 1 to 9 and 0 jump to the first ten.

The video at the top is a live session on a real machine, touring the views. The
stills of the first nine views and the other recordings are of Ghostty running
`isotop --demo`, the synthetic workload; the rest are headless
`--demo --output` renders, and the coop image is a crop of one.

The [wiki](https://github.com/tfpickard/isotop/wiki) is the academic companion: the theory of operation, the
mathematics and physics behind every view (and the eleven views still planned), media, and recipes. Its
sources are in [`wiki/`](wiki/) and are published to the wiki on every merge to `main`.

## Run

```sh
cargo build --release
./target/release/isotop --demo
./target/release/isotop
./target/release/isotop --view orbit      # also: city, ripple, flow, cores, cells, strata, globe, reef, matrix, coop, ptolemy, tesseract, necropolis, cortex, horizon, murmuration, arbor, anthill
./target/release/isotop --list-views      # the views this platform supports, one per line
./target/release/isotop --views reef,city # Tab cycles only these, starting on reef
```

Each view knows whether it runs on the platform; every view runs on Linux and macOS
today. Tab, Shift+Tab and the number keys pass over a view that does not. `--view`
with such a view starts on the first view that does instead, except in a headless
`--output` or `--benchmark` run, which exits with status 1 and says why.
`--list-views` prints the supported views in Tab order and exits, and `--verbose`
prints `skipping view <name>: not supported on this platform` to stderr at
startup for each view it passes over. isotop never shows a "not supported" screen.

`--views a,b,c` sets the Tab playlist. An unknown name is a usage error that lists the
valid ones; a name repeated is kept once, at its first place; a name this platform does
not support is skipped (`--verbose` says so), and if none is left isotop prints
`isotop: no available view in --views` and exits with status 1. The run starts on
`--view` when given and otherwise on the first playlist entry, which is also the view a
headless `--output` or `--benchmark` run renders. Without the flag the playlist is every
supported view. The `v` picker adds views to it and removes them while isotop runs.

Run directly in a graphics-capable terminal such as Ghostty or Kitty; tmux does
not pass the graphics protocol through. The app queries graphics support on
startup and fails fast when it is missing. `--force-graphics` bypasses that query.

The scene renders at the window's native resolution up to 1920 pixels wide
(`--width` lowers the cap) and animation targets 10 FPS (`--fps` raises it, at a cost in CPU). Process data is sampled
once per second; sockets, cgroup accounting, open files, file locks and NVIDIA GPU
usage every two seconds on a background thread. If that thread stops or a scan hangs,
the first status line says so (`background sampler stopped` or `stalled`), since what
it measured last then no longer changes. CPU uses a 1.5-second exponential smoothing time constant; 100% means one
fully occupied CPU core. Sizes ease toward each sample over 0.3 seconds;
inspector values remain the sampled measurements.

**What a Linux machine may lack.** isotop says what it cannot read rather than
drawing zeros, as it does on macOS (below). On Linux the gaps depend on the
machine: without `cpufreq` (common in virtual machines) the Cores legend says "no
clock readings"; without a readable `/proc/schedstat` that counts waiting, "no run
queue readings"; without the hybrid `cpu_core`/`cpu_atom` sysfs (any CPU that is not
a hybrid Intel one, ARM big.LITTLE included) its lanes read "no core kinds
reported". Each pressure file under `/proc/pressure` is judged on its own, and the
status line prints `n/a` for one that cannot be read. Without cgroup v2 with a
memory controller, or when no process sits in a cgroup below the root (inside a
container, say), Cells and the coop group processes by service and draw no limits,
quotas, pressure or OOM kills. An unreadable `/proc/locks` means no brooding or
queueing in the coop, and without NVML the Orbit legend leaves out its green NVIDIA
entry.

## Rendering

Every frame is a display list of screen-space primitives (triangles, lines,
shaded sphere impostors, glows, translucent beams, stars) over a sky. Two
rasterizers draw it with matching output:

- **GPU** (default): wgpu on Vulkan (NVIDIA, AMD, Intel) or Metal (Apple GPUs).
  `--gpu-power high` (default) prefers a discrete GPU, `low` an integrated one,
  which keeps a laptop's discrete GPU asleep. Software adapters are refused.
- **CPU**: a software rasterizer with a depth buffer. `--renderer cpu` forces it;
  `--renderer auto` falls back to it when no GPU is available or a GPU error
  occurs, and the status strip says which rasterizer drew the frame.

Frames reach the terminal through POSIX shared memory when the terminal can read
it (Ghostty and Kitty can, locally): no compression, no base64, almost no pty
traffic. Over SSH, or with `--direct`, frames are zlib-compressed inline instead.
The image sits below cells with a background colour, so popups and labels are
ordinary terminal text drawn over the scene.

isotop runs on Linux and on macOS on Apple Silicon. Process data comes from `/proc`
on Linux and from libproc and Mach on macOS. The renderer is the same on both. On
macOS some readings do not exist, and each view says what it lacks; see
[macOS](#macos).

## macOS

Build and run it the same way as on Linux:

```sh
cargo build --release
./target/release/isotop --demo
./target/release/isotop
```

Use Ghostty. Kitty works too. The GPU rasterizer draws through Metal. Frames go
through POSIX shared memory when the terminal can read it, which it can when it
runs on the same machine, and the startup probe falls back to inline frames when
it cannot (over SSH, for example).

**Privileges.** Without root, isotop can measure only your own processes. It still
lists the others, but macOS refuses to give their CPU or memory, and isotop never
guesses them; they are left out of the scene and counted beside the process count on the
first status line, for example `212 processes (+57 unreadable: run with sudo)`.
Run `sudo ./target/release/isotop` to see the whole machine. Under sudo, the
processes of the user who ran sudo (`SUDO_UID`) still count as your session. Sockets
follow the same rule: only processes isotop may read contribute links. Anyone may
read another process's pid, parent and name (`PROC_PIDT_SHORTBSDINFO`), so in Orbit
and Flow the processes directly under a parent isotop cannot read, such as launchd
without sudo, circle one hollow star named after it, labelled `launchd (unreadable)`
with their count, instead of each standing alone.

**Measured differently.** Memory is the physical footprint, the figure Activity
Monitor shows. Available memory is free plus inactive pages. CPU time comes from
Mach absolute time converted to nanoseconds. I/O is the disk bytes each process has
read and written. Priority is `51 - Mach priority`, so an ordinary application (31)
reads as 20 and a lower number runs first, as on Linux. The scheduling policy is
`pti_policy` (timeshare, round robin or FIFO), the kernel-mode share of CPU time comes
from the system and user times of `proc_pid_rusage` (`PROC_PIDTASKINFO` when that is
unreadable), and the start time from `pbi_start_tvsec` and `pbi_start_tvusec`. Process state is R when a
thread is running, S otherwise, T for stopped and Z for zombies. The performance and
efficiency cores come from the IORegistry (`cluster-type`); if that cannot be read,
the lanes have no kind and are not guessed.

**Open files.** The coop's eggs are measured on macOS too. Each process's
descriptors come from `proc_pidinfo` (`PROC_PIDLISTFDS`), and each vnode among them
from `proc_pidfdinfo` (`PROC_PIDFDVNODEINFO`), which stats the file through the
descriptor. A regular file counts once by device and inode, and one whose link count
is 0 was deleted while still open: a rotten egg, with its size. The scan has the
same bounds as on Linux (4096 vnodes examined and 16 384 descriptors listed per
process, 65 536 per scan, resuming where it stopped), but only vnodes are examined,
since the listing already says which descriptors are sockets, pipes or kqueues, and
past 4096 the open count is estimated in proportion among the vnodes alone. A vnode
that is still open but cannot be read (a revoked device, a network server that fails
the stat) is estimated the same way and marks the count partial. A zombie holds no
files. Other users' tables need root, as their CPU and memory do, and
`kernel_task` is read like any process when isotop runs as root. macOS cannot ask a
filesystem for cached attributes only, as Linux does, so a network mount that stops
answering can hold up the background thread (sockets, files) while its attribute
cache is stale.

### What macOS does not report and how each view shows it

| Missing on macOS | How it shows |
|---|---|
| CPU and I/O pressure | The status line prints `n/a` for each: `pressure cpu n/a mem 0% io n/a`. The coop's heading noise comes from CPU variation only, and its legend says so |
| Memory pressure as a stall percentage | The kernel's pressure level stands in for it: normal reads 0, warning 25, critical 75. The sky's memory tint and the coop's fox eyes show that level, not a stall percentage |
| Last CPU of each process | macOS says only how each process's CPU time divided between the performance and the efficiency cores (`proc_pid_rusage`, `RUSAGE_INFO_V6`). Cores draws every running process as a marble on one of two bands, like an electron on an energy level, once it has a share since the last sample (a process seen for the first time, or one whose counters macOS will not give out, has no marble): the middle of the gold lanes or the middle of the teal lanes, starting on the gold band when at least half of its CPU time was on performance cores. It hops to the gold band when a sample's share reaches 60% and back to the teal band when it falls to 40% or below, along the same arc as a migration between lanes; in between it stays where it is, so an even split does not make it hop every sample, and a process that used no CPU time keeps its band. The inspector gives the share and counts the cluster hops, and the lane labels leave out the running and idle counts, which belong to a core. Where the split is not reported (a kernel without V6, or cores with no kind), Cores draws the lanes with their load and no marbles, and its legend says so. In the coop, running chickens share one trough instead of walking to the feeder of the core they ran on; the feeders are still drawn, brightened by how busy each core is |
| CPU clock | There is no clock per core. Each cluster's average clock is the cycles its cores ran divided by the CPU time they ran them in, summed over every process isotop can read between two samples, and every lane of that kind shows it, so a cluster's chevrons move together and the legend says "cluster clock". A cluster with less than 0.05 s of CPU time in the interval has no reading, and its chevrons pause. Virtual machines count no cycles; there the legend says "no clock readings" and the chevrons stay still. Without sudo the clocks come from your own processes only |
| Run queue | There is no run queue per CPU, so no red marbles at the start line. Each process's waiting threads come instead from its runnable time, which counts running as well, minus its CPU time: red beads trail its marble, one per thread waiting on average, with a faint red glow when any wait. The kernel brings runnable time up to date only when a thread is switched onto a core or blocks, so isotop counts only growth beyond the highest value it has seen; waiting can show a sample late. A process first seen while a thread had run for a long time without a switch can show that run as waiting once, so each reading is held to the process's thread count. After exec, whose new task keeps only one thread's counters, waiting starts over |
| cgroups | Cells groups processes by app bundle and user, with no limits, quotas, throttling, OOM events or pressure. The coop forms its flocks the same way, one per group, and no fox comes for an OOM kill; its legend says so |
| Per-connection socket traffic and RTT | Globe arcs are drawn without rates or round-trip times, and its legend says so. Loopback links have no byte counters, so in the coop peers pull by co-activity only, the smaller of the two CPUs and only when both are busy. In the cortex a synapse weighs the sockets of its link instead of the traffic, min(0.25 ln(1 + sockets), 2), and the legend says "no socket traffic: weight = socket count" |
| Uninterruptible sleep (D) | Never shown, so no red buildings, and no mud puddles in the coop |
| File locks | There is no table of who holds or waits for a lock: `fcntl(F_GETLK)` only tests a range of a file the caller has open itself. No hen broods or queues at a nest in the coop, and its legend says "no file lock table: no brooding or queueing". Open files and files deleted while open are measured; see above |
| GPU memory | Not read. Apple GPUs share memory with the CPU, and there are no green GPU beacons or halos; the Orbit legend leaves out "green = NVIDIA GPU" |
| Syscall I/O | The bytes and calls of read and write syscalls (`rchar`, `wchar`, `syscr` and `syscw` in Linux's `/proc/<pid>/io`) have no macOS counterpart, so `Process::syscall_io` is always empty and the platform reports "syscall io" as missing. Storage bytes (`proc_pid_rusage`) are measured as on Linux |
| Syscall counts in the anthill | Anthill is never skipped. Without the read and write counts an ant is K KiB moved instead of K calls, the direction of an ant is the open flags', and a crumb has one size, unsized; its legend says "no syscall io: ant = K KiB moved, crumbs unsized". With no remainder to tell (the next row) tunnels show no rate and no ants go to a blanket, which the legend says as "no unattributed io: tunnels show no rate, nothing goes to a blanket" |
| Unattributed I/O (per-file I/O detail) | Without `rchar` and `wchar` there is nothing to hold a file's claim to and nothing to take it from, so the remainder is never told (`IoDetail::unattributed` is None, and the platform reports "unattributed io" as missing) and a claim is held only to the size of its file: an `lseek` can make a descriptor claim bytes nothing read, and a descriptor inherited across `fork` shows the position either process moved. The 24 processes are ranked by disk I/O per second rather than by syscalls, so I/O that the page cache absorbed ranks low. Other users' processes are unreadable without root, and a vnode whose path the kernel refuses to build (the `fsgetpath` check of a security policy) counts as another kind of descriptor and marks the reading partial. No view draws it yet |
| OOM score | macOS has no OOM killer; jetsam ranks processes by priority band instead, and only root may list the bands (`memorystatus_control`). Run with `sudo` and `oom score` reads the negated band, so a lower band ranks as killed sooner and `oom_score_adj` is unknown; without root, or if the call fails, it is missing and each process's ranking is unreadable, never zero. A view that ranks by it says so |
| Memory detail | There is no `smaps_rollup`, so a process's memory is never split into private, shared, anonymous, file and swap parts: `memory detail` is always missing and every process's rollup is unreadable. The footprint stands in for memory everywhere else |
| Kernel counters | There is no interrupt table, no softirq count, no IRQ affinity and no readable count of OOM kills (it lives in the private IOReport framework), so `Snapshot::interrupts`, `softirqs` and `oom_kills` are never filled and the platform reports "interrupts", "softirqs", "irq affinity" and "oom kills" as missing. Murmuration reads the OOM kills for its falcon: with none, the falcon comes from memory pressure only (the level stands in, as above) and its legend says "no oom kills: falcon from memory pressure only". No view draws the others yet |
| Major page faults | Not a gap, but not the same count: macOS has no per-process major fault counter, so the pages the process paged in (`pti_pageins` from `PROC_PIDTASKINFO`) stand in for it. That count includes pages brought in ahead of any fault, so it can run higher than Linux's `majflt` |
| The systemd journal | Matrix follows the unified log instead; see below |
| Exit statuses (necropolis) | Every epitaph reads "died between samples", stone heights and widths are the CPU time and memory last sampled, lifetimes run from the start time, there are no unmarked markers, and a mausoleum stands for an app group (there is no `.service`) whose processes are all gone |
| OOM scores and kills (horizon) | Horizon is never skipped. Without an `oom_score` (macOS only lists jetsam bands, and only as root, with no `oom_score_adj`) it ranks by memory footprint, so rank 1 on the ISCO is the largest process; there is no unkillable band, because there is no `oom_score_adj` -1000; and the OOM kill count lives in a private framework, so no exit is classified and every exit fades in place instead of plunging. Its legend says "no oom score: ranked by memory footprint" and "no oom kills: exits not classified". Lensing, the disk and the memory-pressure brightness work as on Linux |
| CPU pressure, kernel threads and unreadable parents (arbor) | Arbor is never skipped. With no CPU pressure the air is still and no branch sways, and its legend says "no cpu pressure: still air". macOS reports `kernel_task` and no thread of the kernel, so the roots are one stump or none, and the legend says "no kernel threads reported: no roots" when there is none. Threads are read as on Linux, but another user's processes are unreadable without sudo: their leaves are shaded by the process's CPU, and the legend counts them (`per-thread CPU unreadable for N: leaves use process CPU`). Without sudo launchd is a parent isotop cannot read, so the trunk is a pale branch of no memory of its own that carries every process it started, and the legend adds `pale branch = a parent isotop can't read (run with sudo)` |
| Fork events and exit records | There is no proc connector or taskstats. kqueue gives the exact time of each exec and exit of your own processes, without the exit status, and births come from comparing samples, at the time of the sample that saw them; see [Fork, exec and exit events](#fork-exec-and-exit-events). The status strip says "no fork events" and "no exit records" |

**Kinds.** Each process is one of four kinds, which set its colour. `kernel_task`
(pid 0) is the kernel. A process is a container if its executable is a
virtualization helper: Virtualization.framework (`com.apple.Virtualization.VirtualMachine`),
OrbStack, Docker, Lima (`limactl`), UTM and other QEMU (`qemu-system-*`) processes,
`vfkit` and `krunkit`. Otherwise it is a system process if its uid is below 500 or its
executable is under `/System`, `/usr` (except `/usr/local`), `/bin`, `/sbin` or
`/Library/Apple`. Otherwise it belongs to your session if it runs under your uid, and
to the system if it does not.

**Groups and parents.** A process's group is the outermost `.app` bundle in its
path, so the helpers inside `Safari.app` all group with Safari; without a bundle it
is the executable's name. Cells and the coop use the group, with the kind, in place
of a cgroup. A process's parent is the app responsible for it, as macOS reports it
through a private function isotop looks up at run time, so helpers that launchd
started sit under their app rather than under launchd. A process whose real parent
already descends from that app keeps its real parent, so a command in a terminal stays
under its shell. If the function is missing or would create a loop, the real parent
is used.

**The unified log.** Matrix runs `log show --last 2m` and then `log stream` (both as
JSON) on a background thread, so the rain starts full as it does on Linux. Lines read
`process[pid]: message`. Severity comes from the message type: Fault is critical and
Error is an error (both red), Default is a notice and Info is information (both
green), Debug is debug (teal), and anything else counts as a notice. The unified log
has no warning level, so nothing is amber.

## Worlds

The scene is real 3D geometry seen through an orthographic camera: isometric by
default, free to rotate and tilt between a low angle and top-down. The view registry
(`REGISTRY` in `src/views.rs`, which `isotop --list-views` prints) is the source of
truth for the list of views, their Tab order and their digits: Tab cycles
the nineteen views in that order, the number keys 1 to 9 jump straight to the first nine
and 0 to the matrix. The coop, ptolemy, the tesseract, the necropolis, the cortex, the horizon, murmuration, arbor and anthill are reached with Tab, Shift+Tab,
the `v` picker or `--view`.
Orbit and flow lay the systems out the same way, but each keeps its own galaxy:
orbit turns its systems about their barycenter and flow holds its wells still, so
switching from one to the other does not carry the places orbit has turned to over to
flow. Views
that grow as data arrives (cores, cells, strata, globe, reef, coop) keep the camera
framed until you move it; Home, Tab or a number key frames them again.

**Reading the mapping tables.** Every view's section opens with a table of what it
draws, the measured quantity behind it and the transform, and a **Decorative:**
paragraph for everything it draws that is not a measurement. In them, *CPU* is the
share of one core a process used, smoothed with a 1.5 s time constant when it is
sampled and eased towards each sample over 0.3 s when it is drawn (so 100 means one
busy core), and *memory* is the resident set (the physical footprint on macOS),
eased the same way. `mass_radius` is the radius several views give a body so that
its volume follows memory: 0.24 × ∛MiB world units, clamped to 0.25 to 4.5 (about
1.1 MiB and below share the smallest size, about 6.4 GiB and above the largest, and
a 1 GiB process is about five times as wide as a 6 MiB one). *Growth* is the ease
with which a process that appears arrives: 1 − (1 − t)³ over 0.9 s. A threshold or
a saturating curve such as `cpu / (cpu + 50)` is the transform the code uses, with
its constants.

### City

![City: districts of buildings, tall where a process is busy](docs/media/city.webp)

A town of districts with one building for each process: its height is set by CPU and its footprint by memory.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Building | Process | One per drawn process, on a plot it keeps until it leaves the sample; picking and the inspector use the process |
| Height | CPU | (1 + 44 × cpu / (cpu + 100)) × growth, growth at least 0.05: 1 unit when idle, 23 at one busy core, 36 at four, towards 45, against plots 4 units across |
| Footprint | Memory (RSS; the physical footprint on macOS) | Half the side is (0.42 + 1.05 × √(MiB / (MiB + 220))) × (0.35 + 0.65 × growth): a side from 0.84 units for the smallest process to 2.94 for the largest, 2.3 at 220 MiB |
| District and plot | `Process::group`: the service or scope the cgroup path ends in (`ungrouped` if none; the app bundle or executable name on macOS) | A district is a 4 by 4 block of plots. A new process takes the first vacant plot of its group's districts, or opens a district; districts spiral out in square shells 23 units apart, and a freed plot goes to the next arrival of its group ([#155](https://github.com/tfpickard/isotop/issues/155)) |
| Wall and roof colour | Process state, otherwise the district | Zombie pink (213, 97, 126), stopped or traced orange (225, 161, 75), uninterruptible sleep red (222, 94, 73); any other state takes its district's colour, one of eight repeating by district number. The kind of the process is not shown |
| Window rows | CPU, through the height | One row of three windows on each wall for every 0.85 units of height; drawn only when zoomed in (zoom above 3) |
| Window brightness | CPU | A lit window is (241, 206, 136) × (0.55 + 0.35 × cpu / (cpu + 50)); an unlit one is (28, 40, 53) |
| Rooftop lamp | CPU above 1 % | An amber sphere on the roof; its brightness pulse is decorative |
| GPU beacon | NVIDIA GPU memory: the process has an entry (`Process::gpu_memory`), whatever the amount | A green glow and sphere at a corner of the roof |
| I/O pulse | Storage I/O (`io_rate`, bytes read plus written) above 1 KiB/s | A cyan dot that runs along the foot of the east wall once every 1.4 s; the rate beyond the threshold does not change it |
| Socket links | `Snapshot::links` | Every link, lifted 1.5 units over the plots (`link_lift`); see [Links and weather](#links-and-weather) |

**Decorative:** each district's ground slab, its 4 by 4 grid and its coloured border; each building's shadow, the shading of its walls (0.40, 0.66, 0.55 and 0.30 of its colour) and roof; the colour of a district; which windows are lit (a fixed pattern from the pid: a window is dark when pid + 7 × row + 11 × column is a multiple of 5); the pulse of the rooftop lamp and the run of the I/O dot; the outline of the selected building.

Plots stay fixed as resources change. Exited processes release their plots. Districts grow along square shells, so a newly allocated district does not relocate existing ones. The camera fit leaves room for the tallest building (`Stage::headroom`). **What cannot be measured.** Without NVML there are no beacons; the I/O counters of other users' processes are unreadable without privileges, so those buildings have no pulse; macOS has no uninterruptible sleep, so no red buildings.

### Orbit

![Orbit: the process tree as nested systems of bodies on orbits](docs/media/orbit.webp)

The process tree as nested systems: every top-level process is a star, its children orbit it on Keplerian ellipses with the parent at one focus, theirs orbit them, down to four levels.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Body | Process | One sphere per drawn process; four levels (0 to 3) are drawn in each system, and deeper processes are counted as collapsed |
| Body size | Memory (RSS; the physical footprint on macOS) of the whole subtree the body anchors, including the processes below the four levels drawn (a leaf's own memory) | `mass_radius` × growth: a star carries the mass of its system, and a 1 GiB subtree has a radius of 2.4 units against 0.44 for a 6 MiB one |
| Body colour | Kind and state | Kernel slate (104, 122, 156), system teal (72, 183, 199), session amber (236, 178, 92), container violet (178, 136, 232), each × (0.86 + 0.24 × j) with j from a hash of the pid; zombie pink, stopped orange and uninterruptible sleep red, without the jitter |
| Thread rings | Thread count (`Process::threads`) | None below 4 threads, then one more ring at 4, 16, 64 and 256 (at most four); ring k stands (1.45 + 0.28 × k) body radii out, and the systems leave room for the rings |
| CPU glow | CPU above 1 % | A warm (255, 190, 110) glow of radius core × (2.2 + 2 × cpu / (cpu + 50)) and strength 0.3 + 0.6 × cpu / (cpu + 40), core being the body's radius on screen, at least 3.5 pixels |
| Comet trail | CPU above 3 %, of a body on an orbit (a star has no orbit) | 2 + ⌊14 × cpu / (cpu + 40)⌋ segments back along the orbit, each min(0.6 / a, 0.25) radians of mean anomaly, fading from full to 0.2 |
| NVIDIA glow | NVIDIA GPU memory | A green (118, 214, 60) glow of radius core × (2.6 + 0.5 × ln(1 + MiB / 64)) and strength 0.6; a process whose amount the driver does not report counts as 0 MiB |
| NVIDIA halo | Any NVIDIA GPU memory entry | A green ring of 1.9 body radii |
| System | The process tree (`Process::parent`) | A process with no readable parent in the sample, and a branch with children that hangs off a top-level process (init, kthreadd), is a star; the leaves of a top-level process stay in its orbit, so kernel threads form one ringed system around kthreadd |
| Shell and orbit size | None: packing. The extents of the siblings (their size, rings and what they anchor) set how far apart the shells are | Children, those with children of their own first and then by identity, fill confocal shells around their parent, which share one eccentricity and orientation; a shell's semi-major axis a grows with the extent of the bodies it must hold, and siblings on a shell are spaced evenly in mean anomaly |
| Orbital period | Memory of the parent's subtree, and the orbit | T = clamp(30 s × (a / 3)^1.5 / √(M / 512 MiB), 8 s, 600 s), M the parent's subtree memory, at least 1 MiB; Kepler's equation is solved every frame, so a body runs faster near periapsis |
| Galaxy turn | Memory of each system, and its distance from the pivot | The systems whose distances from the memory-weighted barycentre overlap, give or take their room, turn together as a band with T = clamp(240 s × (r / 30)^1.5 / √(M / 4 GiB), 120 s, 900 s), M the memory inside the band; no band outside another turns faster, and the galaxy turns only across consecutive frames |
| System title | The star's name, and the number of processes the system draws | `init` for systemd as pid 1, `systemd --user` for another systemd, otherwise the name cut to 15 cells; ` (N)` follows when N is more than 1. The tour visits stars with the most members first, and a body with children says in its inspector how much memory its system holds |
| Hollow star | A parent isotop can see but not read (`Snapshot::shadows`, on macOS without sudo) that has readable leaves | An unpickable outline (168, 182, 210) with no figures, titled `name (unreadable) (N)`, that its leaves orbit |
| Socket links | `Snapshot::links`, `Snapshot::outside` | Every link and every connection leaving the machine; see [Links and weather](#links-and-weather) |

**Decorative:** the orbit lines (slate blue, fading as 1 / (1 + 0.3 × depth), gold for the selected body's) and the spokes to bodies that anchor systems; the eccentricity (0.06 to 0.22) and the direction of the periapsis of each parent's shells, from a hash of its pid; where on its shell a body starts and which shell it takes; where a system stands (a golden-angle spiral, largest first); the direction the rings of a body lean in; the per-pid brightness of a body; the stars of the sky.

**The systems.** Large sibling sets fill concentric shells that share one eccentricity and orientation, so they are scaled copies of each other and never cross; siblings on a shell share one ellipse and keep their order. Systems are packed around the centre, largest first, each keeping 30 % more room than it needs, and the whole galaxy revolves about its memory-weighted barycenter like satellite galaxies: systems whose rings overlap turn together, and detached outer systems turn at their own slower Kepler rate, so they never collide. A system only moves out of its place on the wheel when it outgrows the room reserved for it. Four levels are drawn per system; deeper processes are counted as collapsed.

Known gap: where a body stands inside its system is not yet stable when memory and the population change; tracked in [#142](https://github.com/tfpickard/isotop/issues/142).

### Ripple

![Ripple: pebble clusters driving interfering ripples](docs/media/ripple-motion.webp)

The machine as a pond: processes are pebbles in clusters, and each busy one drives a damped wave on the water.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Pebble | Process | One per drawn process, floating on the water at its height plus 0.35 of its radius |
| Pebble size | Memory (RSS; the physical footprint on macOS) | `mass_radius` × growth |
| Pebble colour | Kind and state | The same colours as Orbit |
| Cluster and slot | `Process::group`: the service or scope the cgroup path ends in | One cluster per group. Slot k of its sunflower spiral lies √(k + ½) spacings out and k golden angles round; a new process takes the lowest free slot of its group and keeps it for life, drawn or not ([#155](https://github.com/tfpickard/isotop/issues/155)) |
| Spacing of a cluster | Memory of its largest pebble | (2 × largest radius + 1) ÷ 1.4 × 1.15, set again only when the largest would touch a neighbour, so pebbles grow in place into the room left round them |
| Place of a cluster | Its extent (spacing × √(highest slot + 1) + the largest radius) | 1.1 × (extent + 3) units of open water reserved on a golden-angle spiral that clears every other cluster; a cluster moves only when it outgrows its water |
| Ripple loudness | CPU, of a process above 0.3 % | Its pebble's cell of the water is driven by 900 × cpu / (cpu + 40) × sin(ω t + φ) per second, spread over the 3 × 3 cells round it with weights 1, ½ and ¼ |
| Drip | A process that appeared in this sample | An upward kick of 30 at its pebble |
| Splash | A process that left | A downward kick of 90 where it was last drawn |
| Swell | CPU and I/O pressure (some, 10 s average; memory pressure does not enter) | Amplitude 0.01 + 0.12 × p / (p + 20) world units of height, p the CPU pressure plus the I/O pressure in percent |
| Water tint | The nearest process, by kind | Every grid node belongs to its nearest pebble (a breadth-first flood, the larger pebble taking a shared node first); 8 % of the owner's colour is mixed into the deep water (10, 30, 46), and hovering the water names its owner |
| Foam | The wave's own vertical speed | speed × 0.05, at most 1, makes up a quarter of the highlight that blends the water towards moonlight (never beyond 0.6), so water at rest has none |
| CPU and NVIDIA glow | CPU above 1 %, NVIDIA GPU memory | As in Orbit, round the pebble |
| Cluster title | The drawn pebble of the lowest slot, and the number of drawn pebbles (two or more) | That pebble's name as in an Orbit title, then ` (N)`: the name of one process, not of the group. The tour visits the largest clusters first, and the pebble's inspector says how much memory the cluster holds ([#154](https://github.com/tfpickard/isotop/issues/154)) |
| Socket links | `Snapshot::links`, `Snapshot::outside` | Every link and every connection leaving the machine; see [Links and weather](#links-and-weather) |

**Decorative:** the pitch of a ripple (0.35 to 1.25 Hz) and its phase, both from a hash of the pid; the lighting of the water (a light at the side of the viewer, specular glints, a cool crest tint) and its palette, whose crest and moonlight tints are added to the deep water; the 10 % wobble of a pebble off its spiral slot, from the pid, and the turn of each cluster's spiral, from its name; the three travelling sines of the swell, whose directions and speeds are fixed.

**The water.** A damped wave equation, `h_tt = c² ∇²h − 0.35 h_t − 0.5 h`, on a grid of about 150 cells along its longer side (`medium::Wave`): c is 14 cells a second, the Laplacian has nine points so rings stay round, and an absorbing border 10 cells thick drinks up what reaches it. It steps in substeps of at most half a cell's travel time and catches up at most 0.25 s a frame. The surface is drawn as lit facets of 0.09 times the wave height. New processes drip; exiting processes splash.

### Flow

![Flow: wells, whirlpools and rivers on a sheet of spacetime](docs/media/flow.webp)

Spacetime weather: particles traced through a velocity field over a sheet that memory bends.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Body | Process | One sphere per drawn process, at its place in its system (held still) and sunk into the sheet (z = the sheet's depth + 0.6 of its radius); a hollow outline for a hub |
| Body size | Memory (RSS; the physical footprint on macOS) of the whole subtree the body anchors | 0.7 × `mass_radius` × growth, as in Orbit |
| Body colour | Kind and state | The same colours as Orbit |
| CPU and NVIDIA glow | CPU above 1 %, NVIDIA GPU memory | As in Orbit, round the body |
| System well | Memory of the whole system (the subtree of its star) | A Gaussian bowl 2 + 14 × m / (m + 2048) units deep, m in MiB, falling off as exp(−d² ÷ w²) with w = max(extent ÷ 2, 4), the extent being how far the system reaches; the sheet is flat between systems |
| Process dimple | Memory of a process's subtree above about 53 MiB (a body radius above 0.9) | A bowl 0.4 × r deep and 1 + 2 × r wide that also draws particles in (inflow 0.6 × r³), r the body's radius |
| Fabric colour | Depth of the sheet | Each net line runs from (28, 42, 72) at the surface to (98, 82, 192) at the deepest point, in proportion to depth ÷ deepest |
| Whirlpool | CPU above 0.5 % | A vortex of strength 6 + 60 × cpu / (cpu + 40) with a core of (1 + 2 × r)² |
| Busy particles | CPU above 0.5 % | 1 + 30 × cpu / (cpu + 40) bright particles a second in the process's colour, born within 1.5 × r + 0.5 of the body and living 4 to 7 s |
| River | Socket links (`Snapshot::links`): the number n of sockets joining two processes, and the CPU of both | Two lanes along the line between the bodies, one each way, each 2.5 + 0.8 × ln(1 + n) wide, with a flow of (8 + 3 × ln(1 + n)) × (0.6 + 1.4 × b), b = (cpu₁ + cpu₂) ÷ (cpu₁ + cpu₂ + 30). Each end sends (0.4 + 0.3 × n) × (0.5 + 2 × b) particles a second, in its own colour, into the lane leading away from it. Bytes are not measured here: both lanes carry the same amount ([#148](https://github.com/tfpickard/isotop/issues/148)) |
| Spark | Connections leaving the machine (`Snapshot::outside`: n, per process) | 2 × n particles a second in pink (255, 110, 200), rising 2.5 units a second for 2.5 s |
| Turbulence | CPU, memory and I/O pressure (some, 10 s average) | Amplitude 0.4 + 6 × p / (p + 25) units a second, p the three percentages added together, in a fixed sine field |
| Exit burst | A process that left | 40 particles flung out at 9 units a second from where it stood, living 1.6 s |
| Birth puff | A process that appeared | 12 particles at 3 units a second, living 1.2 s |

**Decorative:** the background particles (about 400 a second in dim blue anywhere on the sheet, living 5 to 8 s, which only show the field); the sense of a whirlpool, which alternates with the parity of the pid; the sine field the turbulence blows through; the trail of a particle (20 positions), its fade and the glow of a bright one; drawing every third line of the net; the lane offsets and the lit sphere's shading.

**The sheet.** A `medium::Flow` of about 140 cells along its longer side, kept while the footprint of the layout stays about the same, whose velocity field and depths are worked out again every quarter second (speeds capped at 20, depths at −20). Particles are advected by a midpoint step of at most 0.1 s, at most 6,000 at a time. The systems are laid out as in Orbit but with the bodies held still.

Known gap: where a body stands inside its system is not yet stable when memory and the population change; tracked in [#142](https://github.com/tfpickard/isotop/issues/142).

### Cores

![Cores: marbles lapping a lane per CPU](docs/media/cores-motion.webp)

A race track with one lane per CPU, banked like a velodrome: performance cores inside in gold, efficiency cores outside in teal, and a marble for every running process.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Lane | A CPU (`Snapshot::cpus`) | One lane each, ordered by core kind and then number, from a radius of 9 in steps of 1.5, with 1.2 more between kinds; one lane of unknown kind per core when the platform lists none |
| Lane colour | Core kind (`/sys/devices/cpu_core` and `cpu_atom` on hybrid Intel CPUs) | Performance gold (236, 178, 92) inside, efficiency teal (72, 183, 199) outside, unknown (140, 160, 196) |
| Lane brightness | How busy the CPU was in the last interval (`/proc/stat`) | The lane's colour × (0.10 + 0.45 × busy); before a first reading it draws as idle |
| Chevron speed | The CPU's clock (`cpufreq`) | The pattern of 12 chevrons turns 0.1 radians a second for every GHz; without a reading it stands still |
| Chevron brightness | How busy the CPU was | The lane's colour × (0.55 + 0.8 × busy) |
| Red queue at the start line | Tasks waiting for the CPU (run-queue delay in `/proc/schedstat`, as an average over the interval) | w rounded to that many red beads, at most 6, and a red glow of strength 0.25 + 0.5 × w ÷ (w + 1), once w is above 0.05 |
| Marble | A running process: CPU of 1 % or more, or state R | One per such process, on the lane of the CPU it last ran on (`Process::core`) ([#152](https://github.com/tfpickard/isotop/issues/152)) |
| Marble position | CPU | Starts at 2π × cpu_time ÷ 10 s and advances 2π ÷ 10 s × cpu ÷ 100 every second, so a full core laps in 10 s |
| Marble size | Memory (RSS; the physical footprint on macOS) | min(0.2 + 0.3 × `mass_radius`, 0.6): it tells processes apart between about 1 and 171 MiB |
| Marble colour | Kind and state | The same colours as Orbit |
| Marble trail | CPU | Eight beams over (0.05 + 0.5 × cpu / (cpu + 60)) × 6 ÷ r radians behind it, fading |
| Marble glow | CPU above 20 % | A warm glow of size × zoom × (2 + 2 × cpu / (cpu + 100)) and strength 0.25 + 0.5 × cpu / (cpu + 100) |
| Hop | A change of the process's last CPU between samples | The marble crosses to the new lane in 0.6 s on a smoothstep arc that rises up to 0.8 + 0.3 × the distance, leaving a light trail; the inspector counts the hops |
| Lane label | CPU number, kind, clock, busy share, process counts | `cpuN`, `P` or `E`, the clock in GHz, the busy percentage, and `N running, M idle`: the processes by last CPU, running being those on the track (the counts are left out where the platform reports no last CPU) |

**Decorative:** the shape of the track (inner radius 9, lanes 1.5 wide), its banking (outer lanes stand higher, 0.16 per unit of radius), the infield disc, the rim and the start line, the number of chevrons (12) and the starting phase of each lane's pattern (the CPU number × 1.9), the arc and the light trail of a hop.

Idle processes stay off the track and only count towards their lane's label. A process with no last CPU the track can place is put on the first lane, and counted there. **On macOS**, which reports no last CPU, marbles ride the middle of the performance or the efficiency lanes, whichever they mostly ran on, and hop between the two when that changes; chevrons run at each cluster's clock, and red beads trail a marble for its waiting threads; see [macOS](#macos).

### Cells

![Cells: cgroups as cells in three petri dishes](docs/media/cells.webp)

Every cgroup is a cell in a petri dish: one dish each for system services, your session and containers. Accounting comes from the cgroup v2 files (`memory.current`, `memory.max`, `cpu.stat`, `cpu.max`, `memory.events`, `pids.current`, `*.pressure`).

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Dish | Process kind | Three discs: system services (teal), your session (amber) and containers (violet), labelled with their cell and process counts. Kernel threads belong to no cell and are counted on a label |
| Cell | The cgroup (`Process::cgroup`, its full v2 path; the group, an app bundle on macOS, where there is none) | One per cgroup with processes of the dish's kind; the 12 largest of each dish are named, by the last path component without `.service` or `.scope` |
| Cell size | Memory of the cgroup (`memory.current`; the sum of its processes' memory without accounting), and how many seats it has used | Radius max(1.3 + 0.16 × √MiB, 0.75 × √(seats + 1) + 1) ([#156](https://github.com/tfpickard/isotop/issues/156)) |
| Cytoplasm | CPU use of the cgroup (`cpu.stat`) | The dish colour × (0.16 + 0.14 × cpu / (cpu + 50)) |
| Membrane tremble | Pressure of the cgroup: the highest of `cpu.pressure`, `memory.pressure` and `io.pressure` (some, 10 s average) | Amplitude (0.03 + 0.22 × p / (p + 15)) of the radius |
| Red flash | CPU quota throttling (`cpu.stat`, since the previous reading) | The membrane blends towards (255, 96, 80) by 0.5 + 0.5 × sin(12 t) while the cgroup is throttled |
| Dashed ring | Memory limit (`memory.max`) against memory in use | Drawn once a quarter of the limit is in use, at r ÷ √(use ÷ limit) clamped to 1.04 to 1.6 × r (where the membrane would stand at the limit if it grew with √memory), amber (236, 190, 110) turning red as use goes from 60 % to 100 % ([#156](https://github.com/tfpickard/isotop/issues/156)) |
| CPU arc | CPU use of the cgroup against its quota (`cpu.max`), or against one core with no quota | Sweeps 360° × min(use ÷ quota, 1) round the rim from the top once the share is above 1 %, red (255, 96, 80) when a quota exists and more than 90 % of it is used, warm otherwise |
| Burst | The cgroup's OOM kills (`memory.events`) rising | 24 red glows spreading from the membrane to 2.5 radii over 1.6 s |
| Organelle | Process | One sphere per drawn process, at a seat of its cell |
| Organelle size | Memory (RSS; the physical footprint on macOS) | min(0.8 × `mass_radius`, 0.42 × the spacing of the seats) × (0.3 + 0.7 × growth) |
| Organelle colour | Kind and state | The same colours as Orbit |
| Organelle glow | CPU above 1 % | A warm glow of size × zoom × (2 + 2 × cpu / (cpu + 50)) and strength 0.3 + 0.5 × cpu / (cpu + 40) |
| Nucleus | The largest process (by memory) of a new cell | At the centre, `mass_radius` clamped to 0.45 to 0.32 of the cell's radius; it keeps the centre until it exits, then the largest process to arrive after that takes it, and the centre stays bare until one does |

**Decorative:** the discs of the dishes and their rims; the shape of the membrane's tremble (a five-lobed and a three-lobed wave turning at fixed speeds, with a phase from the cell's name); the cytoplasm's base tint; the halo of the nucleus and the spokes from it to each organelle; where a seat lies (a golden-angle spiral numbered by arrival); where the cells lie in a dish; the spread of the burst.

**Seats.** Organelles keep their seats while memory moves the membrane and processes come and go. The seats are spread for the room the cell reserved, and are laid out again only when the membrane would no longer hold the outermost one or has grown half again past the room. Processes hidden by focus (`f`) or by `--limit` keep their seats, and their cells keep their place and size, so clearing the focus puts every cell back. A cell draws only the processes that are shown, and its nucleus stays the same process even while that one is hidden. The cgroup figures (memory, CPU, pressure, kills) are the last reading of the background thread, taken every 2 s and not eased. **What cannot be measured.** Without cgroup v2 the cells are the services (or app bundles) the processes belong to, and there are no limits, quotas, pressure or OOM bursts; the legend says so.

### Strata

![Strata: a minute of CPU use as ridges, kernel threads at the back](docs/media/strata.webp)

The last minute of CPU use as a ridgeline landscape: each busy process is a ridge whose height traces its CPU, with now at the front edge.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Ridge | A process whose CPU reached 0.5 % at some sample of the last 60 s | At most 40, each in its own row for as long as it stays busy; the processes drawn come first when rows are scarce |
| Height | CPU at each sample of the window (the shared history, which keeps the sampled values without the 0.3 s easing) | 0.15 + 16 × cpu / (cpu + 60) units: 10.1 at 100 %, 14.0 at 400 %; a sample the process was not in counts as 0 ([#153](https://github.com/tfpickard/isotop/issues/153)) |
| Position along the ridge | Time of the sample (`Snapshot::elapsed`) | 1 unit for every second before the shown sample, 60 units in all; while the newest sample is shown the landscape scrolls on between samples by up to one sampling interval |
| Row | Process kind | A quarter of the 40 rows for each kind, kernel threads at the back, then system services, your session and containers at the front; a newcomer takes the free row nearest the middle of its quarter and spills into the next ones when it is full |
| Ridge colour | Kind and state | The same colours as Orbit; the face is the colour × (0.10 + 0.12 × f) below and × (0.20 + 0.45 × f) above, and the crest line × (0.75 + 0.6 × f), f being the mean height of the segment ÷ 16, so a higher ridge is brighter |
| Label | Name and current CPU | `name 23%` at the front of the ridge |
| Inspector note | Highest CPU in the window | `peak N% CPU over the last S s` |
| Time marks | Seconds before the shown sample | `now`, `-15 s`, `-30 s`, `-45 s` and `-60 s` |

**Decorative:** the floor and its tick lines, the split of a ridge's face into a darker base and a brighter upper part at 45 % of its height, and the 2.4 units between rows. Memory is not drawn.

A process keeps its ridge, in the same row, while it stays busy. History is recorded whichever view is shown, so the landscape is already a minute deep when you switch to it.

### Globe

![Globe: arcs from home to the places the machine's TCP connections reach](docs/media/globe.webp)

Where the machine's TCP connections go: arcs rise from home to every remote place over a turning earth.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Home | The system time zone's city (`/etc/localtime` and `zone1970.tab`), or `--home LAT,LON` | A glowing point on the earth with its name; when home is unknown it is drawn at 0°N 0°E and labelled `home unknown: pass --home LAT,LON` ([#150](https://github.com/tfpickard/isotop/issues/150)) |
| Endpoint | Remote addresses of established TCP connections (`Snapshot::remotes`), placed by the local GeoIP database | One per place, connections landing on the same whole degree of latitude and longitude joining, or per address when it has no place; a glow and a sphere at the end of its arc, labelled with the place and ` xN` for N connections (the 12 with most traffic, when facing you) |
| Arc | The endpoint's connections | A great circle from home, 40 segments, ending at 1.01 R for a place and at 1.25 R for an address with no place, R being the globe's radius; its lift off the surface, R × (0.04 + 0.22 × angle ÷ π) at the middle, angle being the great-circle distance from home, only keeps long arcs clear of the sphere and is decorative |
| Arc brightness | Traffic: bytes a second sent plus received over the endpoint's connections (`tcp_info` through sock_diag) | The colour × (0.35 + 0.65 × t ÷ (t + 50 000)); a rate not measured yet counts as 0. The end glow is 0.3 + 0.5 × t ÷ (t + 50 000) |
| Arc colour | Direction of the traffic | Cyan (110, 220, 255) when the endpoint received at least as much as it sent (also with no traffic), pink (255, 140, 210) when it sent more; neutral (170, 190, 215) on a platform with no per-connection rates ([#151](https://github.com/tfpickard/isotop/issues/151)) |
| Pulses | Traffic above 64 bytes a second | Three glows on the arc, moving at 0.15 + 0.5 × t ÷ (t + 100 000) arcs a second, towards home when download is at least upload and away otherwise |
| Ring round the north pole | Addresses with no place | On a ring 24° from the pole at 1.25 R, at an angle from a hash of the address; one label counts them: `no GeoIP database: N addresses (see --geoip)` and `locating N addresses`; an address the database does not list is labelled with itself, a private one with `(private)` |
| Process above home | A process with at least one remote connection | A sphere of min(0.25 + 0.3 × `mass_radius`, 0.6) × growth in its kind's colour, 2.5 above the surface, on a line to home |
| Inspector lines | The process's connections | Up to four of `address:port place`, the smoothed round-trip time in ms, and the up and down rates (`n/a` until a second scan), then `and N more connections` |

**Decorative:** the earth (ocean, Natural Earth 110 m coastlines, a graticule every 30°), its halo and its turn, once every five minutes at a constant speed that starts with home facing the isometric camera; the phase of a pulse; the seat of a process in the cloud above home (a golden-angle spiral numbered by arrival, so it does not reshuffle); the angle of an address round the pole.

Locations come from a local GeoIP database, read the first time the globe is shown and looked up only while it is shown; nothing is sent anywhere.

- A city-level MaxMind-format database gives cities: pass `--geoip PATH`, or put
  one in `~/.local/share/isotop/` (any `*.mmdb`). DB-IP's free
  [IP to City Lite](https://db-ip.com/db/download/ip-to-city-lite) works and is
  credited on screen as "IP Geolocation by DB-IP", as its CC BY 4.0 license
  requires; GeoLite2 City in `/usr/share/GeoIP` or `/var/lib/GeoIP` is found
  automatically.
- Otherwise the legacy country database many distributions ship
  (`/usr/share/GeoIP/GeoIP.dat`) places connections at country label points. If
  only the IPv4 or only the IPv6 file is installed, the legend says so and
  connections of the other family are counted as having no database.
- macOS ships no GeoIP database, and isotop never looks addresses up online, so
  install one to see places. For example, DB-IP's City Lite:

  ```sh
  mkdir -p ~/.local/share/isotop
  curl -L "https://download.db-ip.com/free/dbip-city-lite-$(date +%Y-%m).mmdb.gz" | gunzip > ~/.local/share/isotop/dbip-city-lite.mmdb
  ```

- Connections with no place circle the north pole, and the reason is shown. With
  no database, one label there counts the addresses (`no GeoIP database: N
  addresses`), and while addresses are still being looked up another says
  `locating N addresses`. An address the database does not list is labelled with
  the address itself, and a private, loopback or carrier-grade NAT address, which
  has no geography, with `(private)`. The count label sits over the north pole, so
  it is out of sight when the globe is turned to show the south; the legend still
  says "no GeoIP database".

Coastlines and country label points are from [Natural Earth](https://www.naturalearthdata.com/)
(public domain).

### Reef

![Reef: coral, schools of fish and crabs](docs/media/reef-motion.webp)

The machine as a coral reef under a sea sky: system services are coral, your session's apps are fish, containers are crabs and kernel threads are plankton.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Coral colony | The cgroup of system processes (the kind and the cgroup path; the group where there is none) | One per cgroup; its polyps stand on a golden-angle spiral numbered by arrival, a stalk rising from the colony's base, taller near the middle |
| Polyp | Process | A sphere on the stalk, which stands 1.5 + 5 × (1 − distance ÷ room) high |
| Polyp size | Memory (RSS; the physical footprint on macOS) | min(0.18 + 0.32 × `mass_radius`, 0.75) × (0.3 + 0.7 × growth), breathing by ±8 % at 3 radians a second while CPU is above 1 % |
| Polyp glow | CPU above 1 % | A glow of size × zoom × (2.2 + 2 × cpu / (cpu + 50)) and strength 0.3 + 0.5 × cpu / (cpu + 40) |
| School | The cgroup of session processes | A boid school of the group's fish around a goal that drifts round the group's home, with separation (within 2.4 units), alignment and cohesion |
| Fish | Process of your session | A body of length (1 + 2.2 × m ÷ (m + 300)) × (0.3 + 0.7 × growth), m in MiB, and girth 0.26 of it |
| Fish speed | CPU | A speed limit of 0.8 + 5 × cpu / (cpu + 40) units a second; a fish never swims slower than 0.3 of it |
| Fish glow | CPU above 5 % | A glow of length × zoom × 1.4 and strength 0.2 + 0.4 × cpu / (cpu + 50) |
| Crab | Container process | A violet (178, 136, 232) body of size (0.35 + 0.5 × m ÷ (m + 200)) × (0.3 + 0.7 × growth), m in MiB, scuttling by up to 0.9 units about its seat on the sand round its container group's home |
| Crab pace | CPU | The scuttle and the legs' stepping advance at 0.4 + 2 × cpu / (cpu + 40) radians a second (the legs six times as fast) |
| Plankton | Kernel thread | Idle (CPU below 0.5 %): a faint glint that cannot be picked. Busy: a pickable mote of radius 0.05 of brightness 0.4 + 0.6 × cpu / (cpu + 5); its size is not memory |
| Zombie | State Z | Grey (150, 150, 150). A fish floats belly-up and rises towards the surface at 0.3 units a second; a coral or a crab only turns grey |
| Bubbles | Storage I/O (`io_rate`) above 4 KiB/s | 1 + ⌊3 × rate ÷ (rate + 1 MiB/s)⌋ bubbles rise from the creature to the surface every 4 s |

**Decorative:** the colours of the coral and the fish, taken from a hash of the group's name (they tell groups apart and nothing else); the sand and its relief, the caustics, 28 strands of kelp and six light shafts; where a plankton drifts (from a hash of its pid) and its track; the goal a school follows; the sway of a polyp and the wag of a tail; the steps of a crab's legs. Stopped and uninterruptible-sleep processes have no look of their own here.

Processes hidden by focus (`f`) or by `--limit` keep their seats and their cluster's place, and a hidden fish waits where it swam, so clearing the focus puts everything back. The reef keeps room for the clusters of hidden processes, so a limit that hides many leaves empty water, and focusing a subtree does not change the framing.

### Coop

![Coop: flocks foraging by their henhouses, roosting on perch ladders and queueing at the feeders](docs/media/coop.webp)

How it works, and the science and history behind it: [docs/theory/coop.md](docs/theory/coop.md).

A fenced chicken yard in which every process is a chicken in a Vicsek flock: each one steers by the average heading of its flock mates, plus noise.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Chicken | Process | One per drawn process; picking and the inspector use the process |
| Body radius | Memory (RSS; the physical footprint on macOS) | (0.12 × ∛MiB) clamped to 0.3 to 1.2 (volume follows memory; about 16 MiB and below share the smallest size (0.3 / 0.12 = 2.5, and 2.5³ = 15.6), 1000 MiB and above the largest), scaled by 0.3 + 0.7 × growth while it hatches |
| Plumage | Kind and state | The same colours as the other views; zombies pink, stopped orange, uninterruptible sleep red |
| Flock and henhouse | cgroup (the process group if there is none; kernel threads share one "kernel" flock) | One flock and one henhouse per group, labelled with the group name |
| Perch seat | Seat number in the flock | Fixed ladder grid around the henhouse, nearest seats first |
| Roosting on a perch | CPU below 0.3 % | Roosts under 0.3 %, leaves above 0.8 %, a newcomer roosts under 0.5 % |
| Foraging speed | CPU | 0.6 + 3.4 × cpu / (cpu + 25) units per second |
| Heading noise | CPU variation over 30 samples, CPU pressure | 0.5 + 0.5 × min(CV, 2) + 5.5 × psi / (psi + 15) radians, at most a full turn; without pressure readings the pressure term is dropped and the CPU-variation term still applies |
| Pull towards a socket peer, measured | Loopback TCP bytes per second | w = min(log2(1 + B/s ÷ 1024) / 10, 1.5); an idle connection gives 0; a connection first seen since the previous 2 s scan counts all the bytes it has received over that interval, and on the very first scan nothing is measured yet |
| Pull towards a socket peer, Unix or unmeasured | CPU of both ends | w = 0.8 × min(cpu_i, cpu_k) / (min + 25); a pair joined by a Unix socket and a measured TCP connection both is pulled by the larger of the two weights, and the inspector says which one ("over TCP" or "both busy") |
| Feeders | Per-core CPU list (the core count if there is none) | Sorted by kind and id; performance cores get 3 slots and a gold trough, efficiency cores 2 and teal, unknown kinds 2 and neutral |
| Grain brightness | Core busy fraction | Tint 0.25 + 0.85 × busy |
| Feeding at a feeder | Running state, last core | A queue per feeder ordered by priority, then identity; an unknown core goes to the first feeder. A long queue wraps into at most three columns in the feeder's own lane (straight back, then right, then left) and packs tighter if it still does not fit, so it stays inside the fence and apart from the next feeder's queue |
| Pecking rank | Priority and nice | Shown in the inspector |
| Frozen, crouched | Stopped or traced state | Held in place without noise |
| Mud puddle | Uninterruptible sleep | Does not move |
| Feet up | Zombie | Does not move |
| Clutch of eggs in the nest box | Open regular files of the flock's members (`/proc/<pid>/fd`; `proc_pidfdinfo` on macOS) | round(log2(1 + Σ open files)) eggs, at most 16: 1 file lays 1 egg, 7 lay 3, 140 lay 7, and 46 340 or more fill the clutch. Each member counts a file once however many of its descriptors refer to it. A state, not events: the clutch shrinks when files close. Members whose descriptor tables are unreadable or not read yet add nothing, and the nest's inspector line says how many |
| Rotten eggs, cracked and olive | Files deleted while still open (link count 0) | One per deleted file (by device and inode, so a rotated log that several members hold open is one egg), at most 8 shown in front of the clutch; the inspector gives the count and the bytes held |
| Brooding by the nest | Holding a file lock or lease (`/proc/locks`; macOS has no lock table, so none brood there) | A holder that is not running sits on a straw pad beside her own house's nest box, clear of its eggs |
| Queueing at a nest | Blocked on a file lock (a `->` line in `/proc/locks`) | Walks to the nest of the house of the process holding the lock, whatever its own flock, and queues beside it after the brooders, with a faint line to the holder |
| Dust puffs | Read rate | min(5, 1 + ⌊log2(rate / 64 KiB)⌋) puffs above 64 KiB/s |
| Chicks | Threads | min(threads − 1, 12), following the hen along her trail |
| Fox | Rise in a unit's OOM kill count | One to three foxes run for 3 s at the largest member that vanished in the last 4 s (or two and a half sampling intervals, if longer) |
| Fox eyes | Memory pressure | min(6, 1 + ⌊psi / 10⌋) pairs above 0.5 %, 3 × (1 − psi / (psi + 20)) + 0.4 units outside the fence |
| phi in the legend | Foragers | \|Σv\| / Σ\|v\| |
| Inspector notes | Open and deleted files, locks, bytes written, write and read rates, nice | Exact counts behind the capped clutch and rotten eggs, the lock holder a waiter waits for, and the dust and feeding lines |

Decorative: the grass grid and the dirt band under the feeders, the hedge, the fence posts and rails, the henhouse roof and door, straw in the nests and the straw pads beside them, the cracks on rotten eggs, the brooding pose, ladder stringers, ground shadows, the walking stride, the pecking head bob, the chick hop, the swirl of the dust puffs, the blink of the fox's eyes, and the fox's gallop and shape.

**The Vicsek rule.** The update is synchronous: every forager's new heading is computed from the old state of its flock mates within 4 units, and then all are applied together. Only chickens of the same flock count as neighbours, found through a spatial hash. Noise comes from the process's CPU variation over its last 30 samples and from system CPU pressure, so a steady machine holds its flocks together and a stalled one scatters them. The order parameter phi = |Σv| / Σ|v| is 1 when every forager heads the same way and near 0 when headings are random. It is shown in the legend for the whole yard and in the inspector per flock. The fence is a reflecting boundary, and a henhouse pulls foragers back when they stray more than its range. The fence hugs the henhouses' reserved discs but only ever moves outward during a session: a flock at the edge arriving or leaving does not move the fence, the feeders along it or the fox eyes around it. Processes hidden by focus (`f`) or by `--limit` keep their perch seats, their flock's place and their chickens where they stood, so clearing the focus puts every house and chicken back; the fence also takes in the places of hidden flocks, so a limit that hides many of them leaves room for houses that are not drawn.

**Peers.** A chicken is pulled towards the processes it holds sockets with. Loopback TCP has per-link byte counters, so those links pull by measured bytes per second. The kernel keeps no per-link byte counters for Unix sockets, so they pull by co-activity instead: the smaller of the two CPUs, and only when both ends are busy. Two processes can hold both kinds of socket between them; then the larger of the two pulls, so an idle TCP connection does not cancel a busy Unix one.

**Feeders and the pecking order.** Each CPU core is a feeder. A running chicken walks to the feeder of the core it last ran on and pecks while it is in state R, and leaves after two consecutive samples in which it was not running. When a feeder is full, the rest queue, and a lower priority number goes first, so real-time tasks eat before nice ones.

**Roles.** Each chicken takes the first role that applies: zombie, frozen (stopped or traced), stuck (uninterruptible sleep), feeding (running), queueing for a lock, feeding (the two-sample hold), brooding (holding a lock), roosting, foraging. So a running lock holder still feeds, and an idle one broods instead of roosting. A request blocked on a lock sleeps, so a running process that the lock table (read up to 2 s earlier) still lists as blocked has been granted the lock, and feeds.

**Seats by the nest.** Brooders, then the hens queueing there, take seats in a grid east of the nest box, a column at a time from the box's north end, with the first column against the box. The seats stay inside the flock's reserved disc and off its ladders, so a long queue never reaches another flock or the feeders: when more hens come than fit, the grid closes up and they crowd together.

**Eggs and locks.** The nest box of each henhouse holds a clutch for the regular files its flock holds open, and a rotten egg for each of those files that was deleted while still open: its disk space is not returned until the file is closed, which is how a rotated log can keep gigabytes. Both are read every 2 s on the background thread. A descriptor counts as an open file when its link names a path (memfds excluded) and a stat through it finds a regular file; it is deleted when the link ends in ` (deleted)` and the stat finds no links left. Files are told apart by device and inode, so descriptors on one file count once. The stat takes the attributes the kernel already holds (`AT_STATX_DONT_SYNC`), so a hung network or FUSE mount cannot stall the background thread. The scan examines at most 4096 descriptors and lists at most 16 384 per process, and lists at most 65 536 per scan: past that a process's open files are estimated in the proportion found among the examined ones, and its deleted files are counted only among the examined ones, so they are never invented. The inspector then says "about" the open files and "at least" the deleted ones. A process is only scanned while a whole per-process share of the budget remains; otherwise the scan stops and the next one resumes at that process, and processes not reached keep their previous counts or show as not read yet. File locks come from one read of `/proc/locks`: POSIX, flock and lease lines count as held by their pid, and each blocked request (`->`, nested deeper for a request queued behind another) is mapped to the holder of the lock line above it. A request blocked on a lock no process can be named for queues at its own nest.

**The fox.** When a unit's `oom_kills` count rises, a fox runs to the largest member (by memory) that vanished in the last 4 seconds, or two and a half sampling intervals when `--sample-ms` is longer, and takes it. The window exists because cgroup counters are read every 2 s on a background thread, so the count can rise a sample or two after the process disappears from the list; each victim is claimed by one kill only. If no member vanished, the fox leaves empty-mouthed. Kills recorded while another view is shown are dropped once they are older than a fox's run, so switching to the coop does not replay them. Memory pressure puts fox eyes in the hedge: more pairs and nearer the fence as pressure grows, and foragers near them flee.

**What cannot be measured.** The legend says so. Without CPU pressure (kernels without PSI) the pressure term of the noise is fixed; the CPU-variation term still applies. I/O counters of other users' processes are unreadable without privileges, so those chickens raise no dust, and the legend counts them ("I/O unreadable for N"). The same goes for their descriptor tables: their open files add nothing to the clutch, and the legend says "no files for N (permissions)". A process the file scan has not reached yet (just started, or past a scan's budget) is not counted there; its inspector says "open files not read yet". Kernel threads hold no descriptors and count as having no files. Open file description (OFD) locks belong to an open file, not a process, and `/proc/locks` reports them with pid -1: no hen broods for them, a request blocked on one has no holder to walk to and queues at its own nest, and the legend counts them. macOS has no lock table at all, so there no hen broods or queues, and the legend says so in place of the brooding and queueing entries.

**Fixed time step.** The yard advances in whole 20 Hz steps of wall-clock time, so the frame rate does not change how a flock moves; a gap longer than half a second, such as a pause, is capped at half a second.

The view is a port of [tfpickard/chicken](https://github.com/tfpickard/chicken) with its four bugs fixed: it updated headings in place instead of synchronously, its alignment readout summed speed magnitudes so it was always 1, it stepped once per frame instead of by time, and it searched neighbours in O(N²).

### Ptolemy

![Ptolemy: Earth at the centre, planets on deferents with epicycle chains and looping trails, fixed stars on the outer ring](docs/media/ptolemy.webp)

How it works, and the science and history behind it: [docs/theory/ptolemy.md](docs/theory/ptolemy.md).

A geocentric sky seen from above: isotop itself is Earth at the centre, the seven processes whose CPU and memory varied most over the last minute are planets on deferents and epicycles, and every other process is a fixed star.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Earth | isotop's own process | At the centre, labelled `isotop (pid N)` and `X cores \| Y MiB` (CPU / 100, memory in MiB); clickable when isotop is among the drawn processes |
| Planet (a wanderer) | The seven drawn processes with the highest score over their last 60 samples of the shared history | Score = variance of CPU in cores (cpu / 100) + variance of memory in doublings (log2 of MiB, floored at 1 MiB); a planet needs at least 30 samples and a score above 0. An incumbent keeps its sphere while it is alive and ranks tenth or better; an empty sphere takes the best of the rest |
| Sphere | Mean CPU over the window | When the membership changes, the seven are sorted busiest innermost: Moon, Mercury, Venus, Sun, Mars, Jupiter, Saturn, at 0.245, 0.33, 0.415, 0.5, 0.585, 0.67 and 0.755 of the sky radius (40 units). A planet that changes sphere eases there over 1.5 s. Labelled `Sphere of Mars: name` |
| Deferent and its circle | Sphere radius | Period 1200 s × (r / r_Saturn)^1.5 (Kepler's third law); the starting angle comes from the identity hash |
| Epicycle chain | CPU in cores + i × memory in doublings, minus their means, over the last 60 samples | A direct 60-point DFT; the 12 largest terms without the mean, drawn largest first from the point on the deferent, whose tip is the planet. Samples before a process's first are held at its first reading. Coefficients ease to each new sample with a smoothstep over the measured sample interval, clamped to 0.1 to 10 s. Epicycles under 1.5 pixels across are not drawn |
| Chain length | The sum of a chain's coefficient magnitudes | One global gain: gain = K L tanh(T / L) / T with T = K Σ\|c\|, K = 0.085 × 40 = 3.4 units per core or doubling and L = 0.19 × 40 = 7.6 units, so no chain is longer than 7.6 units however wild the process |
| Planet size | Memory | 0.34 + 0.105 × log2(max(memory / 64 MiB, 1)) units, at most 1.2 |
| Planet and trail colour | Kind and state | The same colours as the other views |
| Trail | The planet's path | Positions from the current coefficients and the deferent over the last min(240 s, 0.8 × deferent period), on a 0.25 s grid aligned to multiples of 0.25 s; the oldest 45 % fades out |
| Red trail and red glow | Retrograde: the apparent longitude seen from Earth falling | d lambda / dt = Im(conj(p) p') / \|p\|² < 0, from the position p and its analytic derivative |
| Dashed red sight line and marker | A planet in retrograde now | From Earth out to the ecliptic scale, the marker pointing back |
| Fixed star | Every other drawn process | Angle and radius (0.935 + 0.06 u) × 40 units from the identity hash, on a ring that turns once a minute |
| Star brightness | Memory (RSS; the physical footprint on macOS) | m = -2.5 log10(memory / 1 GiB), floored at 1 MiB; core and halo from the shared point-spread |
| Nova | A process that appeared since the sample before | Brightens by 4 magnitudes over 0.3 s, then decays with an e-folding time of 2 s, for 6 s |
| Fade | A process that exited | Dims out over 1.5 s at its place on the ring |
| Legend lead | Samples in the window, planets in retrograde | `the heavens are still forming: N of 30 samples` until 30 samples exist, then `compiler-90 is in retrograde` (or `A and B are in retrograde`) |

**Decorative:** the armillary rings (the outer sphere, the two ecliptic hoops and the two tilted hoops), the ecliptic degree scale and its labels, the faint spokes from Earth to each deferent point, the dot on the deferent, the colours of the epicycle circles and the chain arm, and Earth's continents.

**The replay.** Each chain replays its process's last minute once a minute, at any `--sample-ms`. The chain at time t is Σ c'_k e^{2πi f_k t / 60}, and each coefficient c'_k = c_k e^{-2πi f_k s0 / 60} is turned back by its phase at the absolute number s0 of the oldest sample, so the coefficients barely change when the window slides on by a sample. A process that swings about a steady mean draws a closed loop on top of its steady revolution, as Ptolemy's planets did.

**Retrograde.** A planet runs backwards against the stars when the process's swings outrun its steady revolution: when its epicycles carry it across the line of sight faster than its deferent carries it forward. The stretch of trail where that happened turns red, and the status line names the planets doing it now.

**The trail is deterministic.** It is computed every frame from the current coefficients and the deferent, not recorded, so it needs no warm-up and is the same at any frame rate: it shows where the planet would have gone with the epicycles it has now.

**Births and exits** come from the shared history, which compares each sample with the one before, so a new process flares as a nova and an exited one fades even in a headless frame. A process that was a planet when it exited does not fade on the ring.

**Caps.** Seven planets, 12 epicycles each (a few more while terms trade places after a sample), and trails of at most 961 points; every other drawn process is a star, up to `--limit`. The selection and the DFTs run once per sample; scoring is 60 readings per process.

**What cannot be measured.** Ptolemy reads only per-process CPU and memory and isotop's own row, so it has no optional sources to lose. If isotop's own process was not read, Earth has no figures and is labelled `isotop`. The demo synthesizes Earth outside the processes (pid 999), so there it cannot be clicked. The pressure haze is off: trail strokes are opaque and blended against the plain sky, which a haze would show through. On macOS memory is the physical footprint; the picture is otherwise the same.

### Tesseract

![Tesseract: worldlines of the last minute in a Schlegel-projected tesseract, superluminal processes leaving through the +x cell in Cherenkov blue](docs/media/tesseract.webp)

How it works, and the science and history behind it: [docs/theory/tesseract.md](docs/theory/tesseract.md).

Each process is a worldline through four dimensions over the last 60 seconds, drawn inside a wireframe tesseract in Schlegel projection: the outer cube is now, the inner cube a minute ago.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| w (outer cube to inner cube) | Wall-clock time of each sample (`elapsed`) | w = 1 + (t − now) / 30 s: +1 now, −1 sixty seconds ago; between samples the window scrolls on by up to one sampling interval |
| x | CPU time (`cpu_time`, user plus system) | x = −1 + (CPU-seconds since the window opened) / 30, the start interpolated at the window's left edge (or the first sample of a later process); a falling counter counts as no growth. x and w share a scale, so the slope dx/dw is the CPU use in cores |
| y (up) | Memory (RSS; the physical footprint on macOS) | y = −1 + 2 √(MiB / (MiB + 220)) |
| z | Seat | Four bands, kernel, system, session, containers, share −0.9 to 0.9 in proportion to their capacities; a newcomer takes the lowest free seat in its band (newcomers of one sample by cgroup, then start); a seat is held until the worldline has left the window; a band holds a power of two of at least 16 seats (the first sample reserves twice what it fills) and doubles when full, never shrinking |
| Head sphere | A process alive in the drawn sample | At w = +1, its last segment extended to now; radius ∝ the perspective factor and 0.6 + 0.3 (y + 1); colour by kind, darker the further back in w it sits; a newborn grows in |
| Worldline | The process's samples in the window | At most one sample per 1 s bin; runs whose slopes differ by less than 0.02 (with memory within 0.002) merge into one segment, so an idle process is one line |
| Blue segment | CPU above one core | Slope > 1.05: only a multithreaded process can, since one thread's CPU time cannot outrun wall time |
| Cherenkov glow and cone | A superluminal head | β = slope over the last 5 s; 12 hairlines on a cone of half-angle acos(1/β) trailing the head |
| Dashed diagonal, "c = one core" | Exactly one core | The line x = w on the x-w face at y = z = +1 |
| Fork vertex and curve | A process first seen after the first recorded sample whose parent was a recorded live process | The child starts at its parent's (x, y, z) and smoothsteps y and z to its own over 2 s; a small glow marks the vertex |
| Exited worldline | A process no longer listed | Kept, without a head, until it slides out of the window 60 s later; drawn only without subtree focus, newest exits first, within `--limit` |
| Cyan marks on a worldline | Disk I/O (`io_rate` > 0) at that sample | A 2.4-pixel-wide stretch of the worldline 0.4 s either side of the sample, one per sample with I/O, at most 2000 a frame, newest first |
| Links | Sockets between processes | The shared link code between heads; `c` cycles them |

**Decorative:** the wireframe tesseract itself (a frame of reference: the now cell brighter, the past cell dimmer), the drift and the 4D orientation (camera only), and the dim tilted cube drawn while boosted (the lab frame's now cell seen from the boosted frame).

**Frames of reference.** The 4D rotation is camera only: it composes rotations in the XW, YW and ZW planes from the canonical angles (0.12, 0.10, −0.08), eased with a 0.2 s time constant, and drifts a full turn in XW every 120 s (a function of the clock, so `--time` repeats it). Points are compressed beyond ±1 in x and w to 1 + 0.6 tanh((|v| − 1) / 0.6), then divided along w from an eye at w = 3.5. A quarter turn in XW makes time the horizontal axis and CPU the depth along w: the only orientation in which the view reads like a chart is a quarter turn away from the default, and there CPU is the distance from you along a direction you cannot see. A boost is physics, not camera: `b` applies a Lorentz boost in the (w, x) plane about the selected head, with β the chord slope of its visible worldline and the rapidity eased with a 0.4 s time constant, so its worldline stands vertical and the others tilt. A process at or above β = 0.999 has no rest frame, and the legend says so. The guided tour boosts into each process it visits; one without a rest frame stays in the lab frame and its callout says "no rest frame".

**History.** The view keeps its own recorder (cumulative CPU seconds need another cadence than the shared history): at most one sample every 0.25 s per process, kept for 180 s so a rewound frame still has its minute, recorded whichever view is shown. Pause and rewind draw the minute that ends at the shown sample. Headless demo runs replay 60 s of samples at `--sample-ms` first; a live headless run starts with its two samples, so its window is short, and the legend says `window Ns` until it holds a minute.

**What cannot be measured.** A process whose I/O counters are unreadable (other users' processes without privileges) has no I/O marks, and the legend counts them (`I/O unreadable for N`). A rewound frame older than the seat release can show an exited worldline on a seat that has since been given to a newcomer.

On macOS, forks branch from the responsible app rather than the Unix parent, because `parent` is the responsible process there.

Keys while the tesseract is shown: `,` and `.` turn a quarter turn back and forward in the current plane, `;` cycles the plane (XW, YW, ZW), `b` boosts into the selected process's rest frame and `b` again or Esc returns to the lab frame (the next Esc has its usual meaning), and Home and `r` also restore the canonical orientation, the lab frame and the drift. Any of `,` `.` `;` stops the drift.

### Necropolis

![Necropolis: rows of headstones by minute of death fading into fog, zombies in open pits at the front, a mausoleum and the waning moon](docs/media/necropolis.webp)

How it works, and the science and history behind it: [docs/theory/necropolis.md](docs/theory/necropolis.md).

Exited processes as headstones in a cemetery at night, one row per minute of death with the newest in front.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Row | The UTC minute of the death (`Snapshot::wall` at the exit's time) | One row per minute with deaths, 3.1 apart, newest in front; when a new minute opens a row the others ease back one spacing over 1.2 s. A row is two lines of 26 units, the second 1.5 behind, filled from the left; graves beyond both are counted on the label (`+N`). Closed rows are frozen; a late death joins the end of its minute's row |
| Row label | The minute | `HH:MM UTC` for the newest row, `HH:MM` for older ones |
| Stone height | Lifetime CPU time: taskstats utime + stime, else the last sampled `cpu_time` | 0.55 + 0.36 log10(max(ms, 3)), at most 3.2 |
| Stone width | Peak memory: taskstats `hiwater_rss`, else the most any sample saw | 0.62 + 0.15 log2(max(MiB, 1)), at most 2.4 |
| Stone tint | Process kind | Slate (kernel), green granite (system), sandstone (session), lilac marble (containers) |
| Place in the row | Parent and child | In the open row, a death whose parent or child is already buried there takes the slot after that family, and only the graves after it move |
| Epitaph (hover, click) | Name, pid, lifetime, cause, kind, cgroup, parent | `name-pid`, `lived <lifetime>   <cause>`, `kind \| cgroup \| parent name-pid`. Lifetime from taskstats `etime`, else wall clock minus start; the cause is the first rule that matches: SIGKILL while the cgroup's `oom_kills` rises in the same or the next sample, "taken by the OOM killer" (each rise claims the largest victim); a core dump, "dumped core"; another signal, "killed by SIGTERM" (Linux names, else "killed by signal N"); AFORK with exit 0, "forked, never exec'd"; exit N > 0, "exited N"; exit 0, "exited peacefully"; no cause, "died between samples" |
| Mass grave | More than 8 deaths from one parent within 1 s | One low slab 0.42 high and 2.4 + 0.1 N long (at most 3.6), labelled `comm x N mass grave (parent-pid)`; it counts as one grave |
| Zombie in an open pit | A live process in state Z | A pink figure in a pit in front of the newest row, pickable as the process; a pit is kept for life. Reaped, it sinks over 0.5 s and is buried |
| Mausoleum | A `.service` cgroup with no live process and at least 3 standing graves | Its graves leave their rows (the gaps stay, nothing else moves); it stands in the lot at the left end of the row of its last death, two rows deep, labelled `unit (unit stopped, N graves)`; later graves of the unit join it |
| Unmarked marker | An exit of a process no sample listed | A marker 0.28 high in the corner field, the newest 120 drawn, all counted: `N died between samples` |
| Ghost | A new process whose pid is that of a standing grave, with another start time | Five stacked glows over the grave rising for 4 s and fading for 2 s, labelled `pid N reused` and `name-pid rests below`; not pickable |
| Fog | Memory pressure (stall percentage, 10 s average) | Each face blended towards the fog colour by its distance from the front of the plot as a share of its depth times 0.4 + 1.2 p / (p + 10), at most 0.6; 4 to 16 wisps (4 + 12 p / (p + 10)), Gaussian patches on the ground up to 12% of the frame wide, add up to 0.3 |
| Sinking rows | The number of standing graves | Above 600 the oldest whole rows are dropped; from 450 the oldest row sinks by up to 1.5 and is fully fogged at 600, the second oldest half as much |
| Moon | The date (`Snapshot::wall`) | age = (days since 2000-01-06 18:14 UTC) mod 29.530588853; lit = (1 − cos(2π age / 29.530588853)) / 2, the lit side on the right while waxing |

**Decorative:** the fence and its gate, the gravel paths, the grass, the soil beds in front of the stones, the two bare oaks, the stars, the earthshine on the moon's dark side, the shapes of the stones (round, slab, gothic, cross, and an obelisk for some of the tallest, drawn from the identity), the lines engraved on them, a zombie's sway, and the fog's base density of 0.4.

**Burial.** Deaths are taken in from the fork, exec and exit events (see [Fork, exec and exit events](#fork-exec-and-exit-events)) on every sample, whichever view is shown, while the events are read. A death is buried 1 s after it happened and at least one sample later, so a mass grave and an OOM kill are known by then; a process still listed (a zombie) waits until it is gone. Everything a grave shows is fixed when it is dug, and its stone is built once.

**Caps.** 600 graves (whole rows go), 120 markers drawn, four ghosts at once, 16 wisps. A grave costs at most about 40 triangles, culled when they face away.

**What cannot be measured.** The events are read only while the necropolis is shown, so deaths while another view is shown are not seen. Without taskstats (it needs CAP_NET_ADMIN), causes come from the proc connector's exit status, heights and widths from what the samples saw and lifetimes from the start time, and the legend says "no exit records"; where only sample diffing works (an ordinary container), every cause is "died between samples" and there are no unmarked markers, and the legend says "no fork events". Live mode never invents a cause.

On macOS only sample diffing and kqueue see deaths: see [What macOS does not report](#what-macos-does-not-report-and-how-each-view-shows-it).

### Cortex

![Cortex: a dark slice of cortex in layers, somas flashing as they fire, dendrites branching from each, axons arcing between linked processes with spikes travelling along them, and a raster of the last ten seconds along the bottom](docs/media/cortex.webp)

How it works, and the science and history behind it: [docs/theory/cortex.md](docs/theory/cortex.md).

Every process is an Izhikevich neuron in a layered slice of cortex, with a raster plot of the last ten seconds of spikes along the bottom.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Soma | A process | One shaded sphere per process, radius 0.45 + 0.35 min(sqrt(GiB of memory), 2) world units (footprint on macOS); it grows in with the scene's birth easing |
| Layer | Process kind | Session II/III, containers IV, system V, kernel VI, each a flat band of the slice; the shares of the height are I 0.10 (empty), II/III 0.25, IV 0.18, V 0.25, VI 0.20 and white matter 0.07, over their sum 1.05 |
| Place in the layer | Process identity | A seat in the layer for the life of the process (`layout::Seats`), at the seat's point of the R2 sequence, (frac(0.7549 (s + 1)), frac(0.5698 (s + 1))), in the band, plus a seeded jitter. A layer with more seats than its band holds at 2.2 by 2.2 units a neuron widens the whole slice by a factor of 1.25 (repeatedly, if need be), and the slice never narrows |
| Firing pattern | Process kind | Kernel fast spiking (a, b, c, d = 0.1, 0.2, -65, 2), system regular spiking (0.02, 0.2, -65, 8), session chattering (0.02, 0.2, -50, 2), containers intrinsically bursting (0.02, 0.2, -55, 4) |
| Input current I | CPU (smoothed) | Piecewise linear in CPU / 100 over 0, 0.005, 0.05, 1, 3 to 0, 0.62 rh, 0.98 rh, I40, I40 + 2 (I40 - 0.98 rh), flat beyond 3 cores. rh is the smallest current that fires 1 Hz and I40 the one that fires 40 Hz, measured for each pattern without noise at first use. While I > 0 the neuron gets noise of standard deviation 1.2 per step: idle is silent, about 5% fires occasionally, one core fires near 40 Hz |
| Dendrites | Thread count | One primary dendrite per thread, 1 to 12, each forking twice; a dendrite's direction is fixed by its index, so a new thread never moves the old ones |
| Axon | Socket links and pipes | An arc below the two somas for each linked pair. Socket links carry spikes both ways, a pipe or FIFO only from its writer to its reader (both ways when both ends are open for reading and writing) |
| Synaptic weight | Loopback TCP rate (`Snapshot::link_traffic`) | min(0.25 ln(1 + bytes per second / 1000), 2). A link whose rate is not measured (a Unix socket, a pair not seen twice yet) weighs min(0.25 ln(1 + sockets), 2); a pipe weighs 1, since neither platform measures its throughput. Synaptic current decays with a time constant of 5 ms |
| Conduction delay | Axon length | Its length in world units over 125 units a second, which crosses the unwidened slice in 0.8 s, so it does not change with zoom |
| Pulse | A spike in flight | The events of the queue the network will deliver, drawn where they are along their axon; at most 3000 are drawn, every n-th above that |
| Flash | A spike | A brightening and a halo that decay with a time constant of 60 ms; a soma also brightens with its rate over the last second, up to 40 Hz |
| Chi (legend) | Membrane potentials of the talking neurons | Golomb's synchrony: the square root of the variance over time of the mean potential over the mean of each neuron's variance over time, from averages with a time constant of 2 s, over the neurons with a link and at least 5% CPU. The inspector gives it for the selected neuron's connected component too |
| Raster | Spikes of the last 10 s | A row for each neuron that fired, by layer and then seat, a dot in the pixel column of each spike (about 5 ms at 1920 wide); above half the plot height in pixels the rows merge into bins, and a brighter cell means more spikes of the bin fell in it |
| Inspector | The selected neuron | Its pattern and a, b, c, d, I and rate over 2 s, its layer and the threads drawn of its total, the synapses in with their least and greatest delay, its component's chi, and two lines of braille: the membrane potential over the last 320 ms, -80 to +30 mV, 40 cells |
| Exit | A process that is gone | The soma shrinks over 1.2 s and leaves 12 specks, besides the generic exit flash |

**Decorative:** the apical trunk (the long dendrite from every soma but the kernel's up to the molecular layer, with its tuft and side branches), the stain (the colour, dimming and depth shading of dendrites and axons), the dust of an exit, the shapes of branches (their bends, lengths and forks, drawn from the identity), the sag of an axon, the fibres of the white matter and the purples of the bands.

**Simulation.** Izhikevich 2003, `v' = 0.04 v^2 + 5 v + 140 - u + I` and `u' = a (b v - u)`, in steps H of 0.5 ms; a neuron that reaches 30 mV fires, `v` goes to c and d is added to u. A frame catches up at most 100 ms of steps and drops a longer gap, so pausing freezes the neurons, and every frame rate takes the same steps and fires the same spikes. At most 262,144 spikes are in flight. Above 400 processes, or when a soma is smaller than 3 px, only the primary dendrites are drawn.

**What cannot be measured.** Pipes carry no rate, so they weigh 1. The links come from the sockets and pipes the scans found: a pipe the scan has not reached yet has no axon, and the demo's links are synthesized.

**macOS.** The view runs, but loopback links have no byte counters, so every synapse weighs the sockets of its link, min(0.25 ln(1 + sockets), 2), and the legend adds "no socket traffic: weight = socket count"; see [What macOS does not report](#what-macos-does-not-report-and-how-each-view-shows-it).
### Horizon

![Horizon: a lensed accretion disk bent over and under a black shadow with a thin photon ring, processes riding orbits ranked by oom_score, the next victim on the innermost stable circular orbit](docs/media/horizon.webp)

How it works, and the science and history behind it: [docs/theory/horizon.md](docs/theory/horizon.md).

The OOM killer as a Schwarzschild black hole: a lensed accretion disk around a black shadow, processes on circular orbits ranked by how soon the kernel would kill them, and a plunge that freezes short of the horizon when it does.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Orbit radius | `oom_score` (`/proc/<pid>/oom_score`), as a rank | Rank among processes that are not kernel threads, not zombies and not `oom_score_adj` -1000, highest score first, ties by identity; two neighbours swap only if their scores differ by 5 or more. Rank r (1 is the next victim) of the top 28 rides at 6 + 17.5 ((r - 1) / 27)^0.8 M, so the next victim is on the ISCO at 6 M; the rest are counted, not drawn, except the selected process, at the same formula up to 25 M. A change of rank eases the radius with a time constant of 1.5 s, at most 2 M per second, and the azimuth stays continuous |
| Unkillable band | `oom_score_adj` -1000 | At most 12 processes at 25.7 to 27.3 M (radius by identity), starting a golden angle apart, labelled `unkillable`; dashed guide at 26.5 M |
| Speed | The orbit radius | Omega = sqrt(M / r^3) in coordinate time, **identical to Kepler's**: 10 M of time per second of the animation clock, so the ISCO's period of 92 M is 9.2 s |
| Body size and colour | Memory (RSS; the physical footprint on macOS), process kind | Sphere of radius clamp(0.12 cbrt(MiB), 0.18, 0.9) M, in the kind's colour; its secondary image (60% as bright, 0.6 as big) picks the same process |
| Disk brightness | Memory pressure (some, 10 s average) | gain = 0.45 + 0.9 p / 100, followed with a time constant of 0.6 s: I = gain F g^4 tex bb(T), shown as 255 srgb(1 - exp(-0.36 I^1.3)) per channel |
| Disk colour | The orbit and the observer | Page-Thorne flux F of the thin disk from 6 M to 30 M, g = 1 / (1 + z) with 1 + z = (1 - 3/r)^(-1/2) (1 + sqrt(1/r^3) sin i x) for the image at screen x (gravitational and Doppler), T = 7200 K (F^(1/4) g)^1.55 coloured by the blackbody table: the approaching side (left) is brighter and bluer |
| Inclination i | Camera pitch | Piecewise linear: 86 degrees at the lowest pitch, 80 degrees at the default, 0 at top-down; PgUp, PgDn, `e` and `Q` tilt it; camera rotation turns the disk's azimuth; zoom sets pixels per M |
| Kill | The kernel's OOM kill count: `oom_kill` in `/proc/vmstat`, else the sum of the cgroups' `memory.events` | A body that vanishes is held for 2.5 s plus the time the counter takes to arrive (the background thread's 2 s and a sample interval). If the counter rose within that of it, before or after, the best-ranked ghost plunges, from the sample that showed the rise if that is later than the exit: it eases to the ISCO in 0.8 s, then follows the geodesic of a body that leaves it with 0.95 of the ISCO's angular momentum and a small inward kick, as a radial streak whose brightness 1.5 (1 - 2/r) falls by e every 4 M of a distant observer's time (0.4 s) once it is near the horizon, turning red, and freezes at r > 2.0006 M, never reaching the horizon; the track is played on that observer's clock, so the plunge takes about 6 s. Every other exit fades where it stood over 1.2 s. The legend says which counter it used and how many kills it has seen |
| Next victim | The top of the ranking | A red ring on its sphere, the first label and the first stop of the tour |
| Inspector | `oom_score`, `oom_score_adj`, rank, radius | `oom_score N \| oom_score_adj N`, `rank R of K`, `orbit r = X.XX M \| period P M (S s)` |

**Decorative:** the disk's turbulence (value noise in the frame that goes round with the disk, streaks long in azimuth), the glare (up to 24 glows along the inner edge, where the disk is brightest), the 1500 background stars on a shell 40 M behind the hole and their glows (a fixed seed, coloured by temperature), the dashed guides at the ISCO and the band, the photon ring, the next victim's red ring and the haze behind the sky. **The photon ring is drawn, not traced:** a ring 0.1 M wide (at least 1.5 px) at 5.27 M, outside the shadow of radius 3 sqrt(3) M = 5.196 M, tinted by the Doppler shift of matter at 7 M; no ray is cast for it. **The texture, the glare and the stars carry no data.**

**Lensing.** Units are G = c = M = 1, so one world unit is one M. Every vertex of the scene is carried to its image on the screen and ordinary triangles are drawn between the images, so the CPU and the GPU draw the same frame and picking works. A table of photon rays traced back from an observer at infinity (fourth-order Runge-Kutta on d2u/dphi2 = 3u^2 - u with u = 1/r, a step of 0.002 rad, 1800 impact parameters) gives, for 360 log-spaced radii from 2.0005 to 150 M and 721 sweep angles, the screen distance of the primary image (the ray that sweeps the angle) and of the secondary image (the ray that sweeps 2 pi minus it, round the other side of the hole, never inside the shadow). The table is built on the first frame, in a fifth of a second. The disk is cut into cells of about 4 px, at most 120,000 triangles in both images (the secondary has half the density), lensed again only when the inclination or the scale changes, and recoloured every frame. The flux is the Page-Thorne integral in closed form, which falls quadratically to zero at the ISCO.

**What cannot be measured.** The kernel keeps the count of kills, not their victims: a plunge is the best-ranked process that vanished within the window of a rise, which a different exit at the same moment can fool. With no counter at all every exit fades, and the legend says "no oom kills: exits not classified"; where only the cgroups count, it says "kills counted from cgroups". A process whose score cannot be read is left out and counted (`N unranked`). Rewinding with `[` shows the older sample's processes, not the older orbits.

**Platform.** On Linux everything above. On macOS the ranking is by memory footprint, there is no unkillable band and no plunge: see [What macOS does not report](#what-macos-does-not-report-and-how-each-view-shows-it). Horizon has no keys of its own; `l` hides the labels.
### Murmuration

![Murmuration: a flock of starlings, one dark bird for each thread, over a twilight sky and a treeline](docs/media/murmuration.webp)

Every thread is a starling in one flock over a roost at dusk, and every process is a sub-flock inside it.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Bird | One thread of a process (`Process::thread_list`, read while murmuration is shown) | One bird per thread, keyed by process and thread id. Above 8,000 threads in all, each process flies max(1, round(threads × 8000 / total)) of them, those with the lowest hash of tid, so the selection holds from sample to sample; the legend then says `1 bird per K threads` |
| Flock | Process | A bird aligns with the weighted mean velocity of its seven nearest neighbours, those of its own process counting double, so each process holds together without balling up. A process whose threads are not read yet flies one bird for each of `Process::threads` |
| Speed | The thread's CPU, percent of one core over the last background interval | 1 + 0.3 × clamp(CPU / 100, 0, 1) world units a step, so busy threads surge to the front. A thread with no CPU reading yet, or a process with no thread list, uses the process's CPU divided by its threads; the legend counts them: `speed from process mean for N` |
| Darkness | The birds | Each bird is a soft splat of radius 0.85 of a world unit in pixels (1.2 to 6 px) with peak alpha 1 − exp(−1.1 w), w = 1; splats stack, so dense regions go black and thin ones grey. The nearest bird is picked |
| Dark wave | The falcon | An agitated bird weighs w = 1 + 1.6 × its agitation, up to 2.6. A bird within 14 units of the falcon is fully agitated and flees; agitation passes to the neighbours as 0.93 of the largest among a bird's seven while it is above 0.35, fades by 0.86 a step and adds 0.12 of noise |
| Falcon | Memory pressure, the 10 s stall average, at or above 10 %; or a rise in the OOM kill count (`oom_kill` in `/proc/vmstat`, else the sum of the cgroups' `oom_kills`) | A red chevron that dives through the flock at 2.6 units a step for 6.5 s, labelled `memory pressure` or `OOM`. Pressure releases one falcon at most every 8 s while it stays at or above 10 %; a kill releases one when none is flying |
| Rising bird | A thread that appeared since the last sample | Climbs from the roost at the origin to 35 units, where it joins the flock |
| Falling bird | A thread that exited | Falls to the roost over 1.5 s and is removed |
| Sky tint | CPU, memory and I/O pressure | The pressure haze every view has, over the sky |
| Lit birds | The process that is selected, under the pointer or visited by the tour | Its birds are drawn again in its kind colour, labelled `name  N threads`; the inspector adds the thread count and the three busiest thread names with their CPU |

**Decorative:** the twilight sky, a gradient from deep blue (28, 34, 70) through violet (120, 92, 138) at 60 % of the height to orange (238, 150, 104) at the horizon; the treeline along the bottom of the frame, which is a silhouette and **carries no data**; the clumps the flock starts as and the noise of each bird's flight.

**Flight.** The flock steps at a fixed 0.05 s (20 steps a second), whatever the frame rate, and holds still while paused. Each step a bird steers by 0.30 towards the weighted mean velocity of its 7 nearest neighbours (alignment), 0.02 towards the centroid of its 21 nearest (cohesion; with only 7, closed groups of eight split off as islands) and 0.4 times the sum of away / d² from those closer than 1.5 (separation); 0.0006 towards the whole flock's centroid (a pull on each process's own centroid of 0.0012 or more splits the flock into separate balls), 0.004 per unit beyond 80 units from the roost's axis, and 0.01 per unit outside the altitude band of 35 to 110; and it damps its vertical velocity by 0.10 (0.22 flattens the flock to a pancake), so it flies level and turns by banking, which flattens the flock into sheets that show as dark bands edge-on. Noise adds 0.02. Its speed is then set to the thread's. The camera is fitted to the roost cylinder (radius 90, height 110), not to the flock, and starts at the lowest tilt, about 20 degrees.

**Caps.** 8,000 birds. Above it every process still flies at least one, so the flock can hold as many more birds as there are processes in view (`--limit`). The 21 nearest neighbours of each bird come from a spatial hash with 2-unit cells.

**What cannot be measured.** Another user's threads are unreadable without privileges, so those birds fly at their process's mean speed and the legend counts them. If the threads cannot be read at all the legend says `no threads: speed from process mean`, and without an OOM kill count (an old kernel with no `oom_kill`, and no cgroup counting them) it says `no oom kills: falcon from memory pressure only`; until the first pass has read the threads (up to two seconds after the view opens) every bird flies at its process's mean speed. A view entered after a gap of more than two sample intervals releases nothing for the kills and pressure of the time it was away.

On macOS the threads and their CPU are read as on Linux, but there is no count of OOM kills: the falcon comes from memory pressure only, where the kernel's level (warning reads 25, critical 75) stands in for the stall percentage. See [What macOS does not report](#what-macos-does-not-report-and-how-each-view-shows-it).

### Arbor

![Arbor: the process tree as a great tree at dusk, a thick trunk branching in golden-angle slots, a canopy of green and red leaves with pink blossoms, fallen leaves on a cut-away slab of soil and blue roots under it](docs/media/arbor.webp)

How it works, and the science and history behind it: [docs/theory/arbor.md](docs/theory/arbor.md).

The process tree drawn as a tree: PID 1 is the trunk, its descendants are the branches, the threads are the leaves and the kernel threads are the roots under the soil.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Branch thickness | Memory (RSS; the physical footprint on macOS), summed over the process and every process below it | Leonardo's rule: radius r = 1.6 sqrt(A / A_trunk) world units, A the subtree memory, never under 0.05. Along a branch the area before each junction is the node's own memory plus every child attached further out, so at a fork the area of the branch equals the areas of what leaves it |
| Branch | A process, and `Process::parent` | The trunk is PID 1, or the root of the subtree with the most processes where there is no PID 1. A process whose parent is not drawn hangs on the trunk |
| Where a branch leaves its parent | The order in which the parent was first seen to have it, by start time | Child number k of a parent (a counter that only rises, so a branch that exits frees nothing) leaves at azimuth k x 137.5 degrees round the parent, at 0.35 + 0.6 (1 - 0.8^(k + 1)) of its length, 66 degrees off the parent's axis for the trunk's children and 52 degrees deeper. A reparented process takes a new slot on its new parent |
| Branch length | Depth in the tree | 14 x 0.84^depth world units |
| Leaf | One thread (`Process::thread_list`, read while arbor is shown) | At most 32 leaves to a twig: above that the threads are bucketed by tid, and a bucket's leaf shows the bucket's mean. At most 6,000 leaves in all, the cap per twig falling for every twig alike. A process whose threads are not read draws one leaf per `Process::threads`, all shaded by its CPU divided by its threads |
| Leaf colour | The thread's CPU, percent of one core | Green (74, 150, 72) when idle, (104, 170, 70) at 8%, yellow (214, 196, 60) at 30%, orange (236, 120, 44) at 60% and red (220, 52, 44) at a whole core |
| Blossom | A process the scene first saw after its first frame | Six blossoms at the twig for 5 s: open over 0.7 s, closed again over the last 0.8 s. Processes that were running when isotop started never blossom |
| Falling leaf | A thread that is gone | Falls for 4 s, drifting downwind and fluttering, rests on the soil for 30 s and goes. A process that exits drops all its leaves (at most 32), a thread of a live process drops one (at most 8 a sample). At most 400 leaves fall or rest at once |
| Zombie | A process in state Z | A bare grey twig (128, 128, 128) with no leaves |
| Root | A kernel process that is top-level or a child of the trunk, with its subtree | Grows downwards under the soil. **Root thickness is the number of kernel threads**: kernel threads have no memory to speak of, so r = 0.88 sqrt(n / N) with n the kernel threads of the root and N those of all roots |
| Root tip glow | The kernel thread's CPU | Strength 0.2 + 0.5 x p / (p + 6), p percent of a core |
| Wind | CPU pressure (some, 10 s average) | w = p / (p + 20), followed with a time constant of 1.5 s; the legend names `wind = CPU pressure N%`. With no pressure the air is still and nothing sways |
| Sway | Each branch's length and thickness, and the wind | A damped oscillator in the ground plane, stiffer for thick branches and softer for long ones: f = clamp(0.6 sqrt(r / 0.18) / sqrt(L / 6), 0.15, 3) Hz, damping ratio 0.12, pulled towards a gust of w x 0.09 L (a steady push along the wind and two gusts of unrelated periods). The gust's phase is seeded by the branch's identity; a child leaves its parent where the parent has swayed to |
| Pale branch | A parent isotop can see but not measure (`Snapshot::shadows`, another user's process) | Drawn with no memory of its own, so its thickness is what it carries, pale and not pickable |
| Bark and leaf shading | The one light every lit surface shares (`render::shared::LIGHT`) | Each face of a branch is 0.35 + 0.65 max(n . LIGHT, 0) bright; a leaf takes the outward direction from the crown as its normal |

**Decorative:** the dusk sky, the soil slab, its strata and the checkered lawn on it, the colours of bark, roots, soil and blossoms, the number of sides of a branch (8, 6 or 4 by its thickness on the screen), the upward bend and wobble of each branch (seeded by its identity), the shape, size (0.8 to 1.2), tilt and brightness of each leaf and where it hangs on its twig (seeded by process and thread id), the heading of the wind and the steady push in it, and the flutter of falling leaves.

**Layout.** A branch's place depends on the tree and nothing else: the slot, the parent's own place and the branch's seed. CPU, memory and population never move a branch, and a process that stays keeps its slot while others come and go.

**Simulation.** The wind steps at a fixed 0.05 s (at most 10 steps a frame, a gap of more than 0.5 s counted as 0.5 s) whatever the frame rate, and holds still while paused. Falling leaves and blossoms are closed forms of the time since they began, so they follow the same path at any frame rate. The tree is rebuilt once per sample, and each frame runs only the wind and the projection.

**Caps.** At most 32 leaves to a twig, 6,000 leaves, 400 fallen leaves, and one step of radius for every 2% of a branch where children leave it (hundreds of children are a few tube pieces).

**What cannot be measured.** Another user's threads are unreadable without privileges, and until the first pass has read the threads (up to two seconds after the view opens) leaves are shaded by their process's CPU; the legend counts them or says `no threads: leaves use process CPU`. Without CPU pressure the legend says `no cpu pressure: still air`, without kernel threads `no kernel threads reported: no roots`.

**Platform.** On Linux everything above, and pale branches appear when isotop does not run as root. On macOS the tree, the areas, the leaves, the blossoms and the falling leaves are as on Linux; without sudo launchd is the pale trunk; there is no CPU pressure and no thread of the kernel (see [What macOS does not report](#what-macos-does-not-report-and-how-each-view-shows-it)).

Leonardo da Vinci noted that the branches of a tree keep their total cross-section from fork to fork; C. Eloy, "Leonardo's rule, self-similarity, and wind-induced stresses in trees", *Physical Review Letters* 107, 258101 (2011), shows that it follows from trees withstanding wind stress, which is why the wind here is the real CPU pressure. Arbor has no keys of its own.

### Anthill

![Anthill: a sandy slab at dusk with a heap of dark mounds in a clearing, piles of pebbles in the regions round it and streams of ants carrying pale and coral crumbs between them, over glowing trails](docs/media/anthill.webp)

Syscalls as foraging ants: every process is a nest, every open regular file a pile of pebbles on a map of the filesystem, and one ant walks for every K read and write calls, carrying a crumb as big as the bytes in one call.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Mound (nest) | A process, and its memory (RSS; the physical footprint on macOS) | Radius 0.9 + 0.38 log2(max(MiB, 8) / 8) world units; a paraboloid 0.95 of its radius high, of 24 segments (12 under 8 px of radius) |
| Where a nest stands | The first sample that had the process | The first place along a golden-angle spiral from the middle where the room it reserved fits: 1.15 times the radius it would have at twice its memory. It keeps the place until it outgrows that room, whatever its CPU, its memory below that, or the population do |
| Mound colour, crater and entrances | The process's kind (kernel, system, session, container) and whether its descriptors were read | The kind's colour lights the crater and up to four entrances, one for each distinct destination (a file, a blanket, the nest at the other end of a tunnel), busiest first, each on the slope that faces it. A nest not read this sample sleeps: darker, with no crater glow and no entrances |
| Pile | An open regular file (`IoDetail::files`, one pile for a device and inode however many processes hold it) | Radius 0.45 + 0.12 log2(size / 4096), from 0.45 to 3 world units, of 6 to 24 pebbles by its radius. At most 256 piles, the ones with the most attributed bytes a second first. A file deleted while it is open has cooler, greyer stone. A file that opens grows in over 1.5 s and one that closes fades over 1.5 s |
| Where a pile stands | The file's path | A fixed pinwheel round the clearing: `/usr` (and `/lib`, `/bin`, `/sbin`, macOS `/System`, `/Library`, `/Applications`), `/var`, `/tmp` (and `/dev/shm`), `/home` (and `/Users`, `/root`) and everything else. Each second-level directory (`usr/lib`, `home/tom`) takes one 8-unit cell of its region, the first free one from where its name hashes to, and keeps it; the file stands at a spot its whole path hashes to (FNV-1a) inside the cell |
| Ant | K read and write calls (`syscr` and `syscw`) | K is the smallest of 1, 2, 5, 10, 20, 50, ... for which the sum over streams of calls a second times seconds a round trip takes, over K, is at most 3,000. The first sample with anything to carry sets K; after that K changes only when three samples in a row ask for the same one. At most 3,750 ants, and the legend counts the ants the cap refused |
| Ants sent to a file | The calls the process made, and the bytes its positions credit to each file | A read-only file gets the share of the process's read calls that its bytes are of all the bytes it read (`rchar`, which is the files' bytes plus what no file explains); a write-only file the same of its write calls; what is left of the calls is shared by the read-write files and what nothing explains, in proportion to their bytes. The shares add up to the calls the process made and never to more |
| Ant direction and crumb colour | The descriptor's open flags | Pale (255, 238, 196): the ant comes home carrying what it read. Coral (255, 118, 92): it carries a write out. The colour between the two, carried both ways: a read-write file, whose direction is never split |
| Crumb size | Bytes per call: `read_chars / reads`, `write_chars / writes`, or for a read-write file the mean over all of the process's calls | Radius 0.55 + 0.27 log2(bytes / 64) px, from 0.42 to 3.6, at a frame 1600 wide with the slab fitted to it, scaled with the zoom, and never under 1 px |
| Tunnel | Pipes and FIFOs (`Snapshot::pipes`), Unix sockets and loopback TCP (`Snapshot::links`) between two drawn nests | An arch of dashes between the nests; the `c` key shows all of them, those of the highlighted processes or none. A pipe is one lane from its writer to its reader, sockets two lanes, one each way. The dashes slide at 1.2 log10(1 + calls a second) world units a second, scaled with the slab, where the calls are those of the bytes no file explains, shared over the process's tunnels by their measured traffic when every one has some and equally otherwise. A tunnel whose ends were not read is dotted, thin and still: unknown, not idle |
| Blanket | A process that holds a terminal, has no tunnel at all and has unattributed I/O | A gingham blanket 5.4 units across in a corner of the clearing (four at most, to the processes with the most unattributed bytes, kept while they live). Its ants carry the read and write calls no file explains; their direction is the counters' when no read-write file took any bytes, otherwise unknown |
| Labels | The paths of the piles, and the crumbs | The names of the regions, the file names of the 12 piles with the most attributed bytes a second, and the biggest and the smallest crumb among the streams (`128 KiB/read`, `64 B/write`) |
| Pheromone glow | The ants' own deposits | Three channels, the colour of each nest's kind, four cells to the world unit and at most 256 along a side. An ant lays 1.0 a second while it carries a crumb and 0.45 while it does not; the grid evaporates as exp(-dt / 2.2) and then spreads with a three-tap blur; the glow saturates as 1 - exp(-2.2 x amount) |

**Decorative:** the sand (its dunes, ripples, specks and the darkening of the rim), the tint and string outline of each region, the cut sides of the slab and their strata, the dusk sky, the facets, soil colour and crater of a mound, which pebble lies where in a pile and its shade, the bend of each route and the arch of each tunnel, the side a mound's ants leave by, the speed jitter (13 %), wander and heading of each ant and how it follows the pheromone, the ant's own shape, and the gingham. The routes are the colony's, the endpoints and the rates are the machine's: two runs with the same data show the same ants going to the same places by different paths.

**Layout.** A square slab, 64 world units across to begin with: a clearing at the middle for the nests, with the five regions round it as a pinwheel (each of `/usr`, `/var` and `/tmp` an arm that reaches a corner, `/home` the arm between two corners of the clearing and `other` the square in the one corner of the slab the pinwheel leaves), the arms 16 units deep. Every sampled process has a place, drawn or not, so `--limit` and a focus move nothing. Only a clearing too small for its nests grows, in steps of 1.25 (the arms, the piles and the trails move then), and `Stage::bounds` is the slab's four corners.

**Simulation.** The colony steps at a fixed 0.05 s (at most 10 steps a frame, a gap of more than 0.5 s counted as 0.5 s), whatever the frame rate, and holds still while paused; ants move 12 world units a second, between steps interpolated. An ant steers toward a carrot on a quadratic Bezier with a hashed bend for its stream, leans toward the side of its heading (plus or minus 0.5 rad) that smells stronger, and adds noise of order the square root of dt. Each ant is seeded from its nest, its goal and how many its stream had sent. A sample is taken in once, not each frame; demo anthill plays the ten seconds before `--time` as a live run would have drawn them, so the trails are laid.

**Caps.** 256 piles, 24 pebbles to a pile, 3,750 ants, 4 entrances to a nest, 4 blankets and 512 tunnels; a thousand nests are placed in one walk along the spiral by a spatial index.

**What cannot be measured.** A file's bytes are the growth of its descriptors' positions, so they are a lower bound: `pread`, `pwrite`, memory-mapped I/O, `io_uring`, and a file opened and closed between two background passes (about 2 s) are not seen, and a file opened for append says nothing of bytes (it is a pile with no ants). A file with no earlier reading of its descriptor is a pile with no ants too: unknown is not zero, and its card says so. What no file explains stays unattributed: it goes to the tunnels or the blanket or is counted in the legend, and is never given to a file. Which terminal a descriptor is cannot be told, only that a process holds one, so a blanket is a process's, not a terminal's. Only the 24 busiest processes (by syscalls a second) have their descriptors read: the rest sleep, and a process whose descriptors the platform refuses (another user's) sleeps too, counted as `fdinfo unreadable for N`. A process that has descriptors but no read and write counts on a platform that has them sends no ants and is counted as `syscall counts unreadable for N`. Until the first pass has read the descriptors (up to two seconds after the view opens) the legend says `reading open files`.

**Platform.** On Linux everything above. On macOS the read and write calls are not counted (`Source::SyscallIo` is missing): an ant is K KiB moved, the direction is the open flags', the crumbs have one size and are unsized, nothing says what a process's files leave of its bytes (`Source::UnattributedIo` is missing), so tunnels show no rate and nothing goes to a blanket, and the processes read are the ones with the most disk I/O. The legend says `no syscall io: ant = K KiB moved, crumbs unsized` and `no unattributed io: tunnels show no rate, nothing goes to a blanket`. See [What macOS does not report](#what-macos-does-not-report-and-how-each-view-shows-it). Anthill has no keys of its own.

### Matrix

![Matrix: journal lines decoding out of digital rain](docs/media/matrix.webp)

How it works, and the science and history behind it: [docs/theory/matrix.md](docs/theory/matrix.md).

The systemd journal decoded out of digital rain (on macOS, the unified log; see [macOS](#macos)): a log of `source[pid]: message` lines, newest at the bottom, in which each character resolves only when a falling stream of rain passes over it.

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Row of text | One journal entry (`journalctl --follow --lines=200 --output=export`: `SYSLOG_IDENTIFIER`, else `_COMM`, then `_PID` and `MESSAGE`) | `source[pid]: message`, with line breaks and tabs turned into spaces and each field cut at 4,096 bytes; wrapped after the last space that fits, continuation rows indented by two; the newest at the bottom, the rest scrolling up |
| Line colour | Syslog severity (`PRIORITY`) | 0 to 3 red (255, 72, 60), 4 amber (255, 190, 60), 5 and 6 green (80, 255, 95), 7 teal (70, 160, 150), at 0.85 of full strength once decoded; an entry with no readable priority is read as 6 |
| Pace of the log | Entries waiting in the queue | The next entry is written 0.5 s ÷ (1 + queued ÷ 3) after the last, so a queue empties faster the longer it is; the entries' own times are not drawn and do not set the pace |
| Skipped count | Entries beyond two screens of rows (at least 32) waiting to be written, or beyond the follower's backlog of 2,048 | The oldest are dropped and counted in the legend: `N queued, M skipped` |
| Shower and decode | The arrival of an entry | Each non-blank character gets a stream of its own that starts 1 to 11 rows above its row, falls at 14 to 26 rows a second and is 2 to 6 rows long, stopping at the character's row. The character flashes white where it is struck and settles into its colour with a time constant of 0.35 s; one never struck appears 3 s after its entry |
| Newest entry | The most recent entry | Shown on the second status line in place of the inspector |

**Decorative:** the ambient rain between the lines, all of it (about half a stream for every column, falling at 8 to 16 rows a second, 8 to 23 glyphs long, with a bright head and a glow, and glyphs that change six times a second out of step with their neighbours); the black sky; the 7x14 font. **The greeting** is not from the journal: the first time the view is shown it writes `Wake up, <your login>...`, `The Matrix has you...` and `Follow the white rabbit.` as three entries of severity 6, in the same green as a real one, ahead of the journal's own; ([#149](https://github.com/tfpickard/isotop/issues/149)) the demo's journal is also synthetic. The view draws no processes, so there is no pressure haze and the status line counts none as visible or collapsed.

`journalctl -f` runs on a background thread from the moment the view is first shown, starting with the last 200 entries, and stops 30 seconds after you leave the view; coming back starts it again with the last 200 entries. Lines are written one at a time, faster when they queue up; a flood that outruns the log is trimmed, and the status strip counts what was skipped. The camera and process controls do nothing here. Glyphs are the public-domain X11 misc-fixed 7x14 font, and `--demo` writes synthetic entries every 0.35 s. **What cannot be measured.** Without the `adm` or `systemd-journal` group, journalctl shows only your own user's journal; if the follower cannot run, one red line says so.

### Links and weather

Socket links come from the kernel: loopback TCP pairs from `/proc/net/tcp{,6}`
and connected Unix-socket peers from the sock_diag netlink interface, matched to
processes through `/proc/<pid>/fd`. Only processes whose descriptors isotop may
read appear. In orbit, ripple and city they are cyan arcs with travelling pulses
(`c` cycles all / highlighted / off); TCP connections to other machines rise as
pink beams. Links of the selected, hovered or toured process glow. The tesseract
draws every link too; flow, cores, cells, strata, globe, reef, the coop, ptolemy
and arbor draw only the glowing ones; the necropolis, cortex, horizon and murmuration
leave links to the view, and so does the anthill, whose tunnels follow `c` too.

The scene draws these over the views, and the table gives the transforms:

| Visual element | Measured source | Transform |
| --- | --- | --- |
| Link arc | Pairs of processes joined by sockets (`Snapshot::links`, with the number n of sockets) | An arc between the two bodies that rises to 0.175 × their distance over the line (plus the registry row's `link_lift`), in 20 beams of weight min(0.1 + 0.05 × ln n, 0.3) in (80, 175, 235); (150, 232, 255) and weight 0.8 when the selected, hovered or toured process is an end |
| Beam out of the machine | TCP connections whose far end is not a local socket (`Snapshot::outside`, n for the process) | A pink (255, 110, 200) beam rising 5 + 3 × ln(1 + n) units over the body, fading with height, at strength 0.45 (0.9 when the process is highlighted) |
| Sky haze | CPU, memory and I/O pressure (`/proc/pressure`, some, 10 s average) | With c = cpu / (cpu + 20), m = memory / (memory + 10) and i = io / (io + 20) in percent, the sky gains (110 c + 140 m + 30 i, 70 c + 20 m + 50 i, 10 c + 60 m + 150 i) where a drifting field is dense: amber for CPU, crimson for memory, blue for I/O |

**Decorative:** the pulse that travels each link arc (one glow once every 2.9 s, with a phase from the two pids, at strength 0.35, or 0.9 on a highlighted link) and the drifting pattern of the haze.

Two more measurements ride on those links, for the views that draw them: the bytes waiting
unread on each connected Unix socket (for a datagram socket only that a packet waits, since
the kernel gives the size of the first packet and not a total), and the pipes and FIFOs between
processes with their direction, from `/proc/<pid>/fd` and `fdinfo` (macOS: `proc_pidfdinfo`,
which also counts the bytes waiting in a pipe). Linux gives no such count for a pipe another
process holds. Only the pipes are read on demand; the existing views draw neither.

Another, for the views that draw it, is where the busiest processes' I/O goes (`Process::io_detail`,
read only while such a view is shown). For the 24 busiest processes (on Linux by syscalls per
second; macOS counts none and ranks by the bytes that reached storage per second,
`ri_diskio_bytesread` plus `ri_diskio_byteswritten`) isotop reads the regular files their
descriptors hold, with each descriptor's position, from `/proc/<pid>/fd` and `fdinfo` (macOS:
`proc_pidfdinfo`, `PROC_PIDFDVNODEPATHINFO`), at most 256 files per process and 4096 descriptors
per pass, and counts their pipes, sockets, terminals and anonymous descriptors. A file is
credited with how far a descriptor's position moved between two passes and with nothing else:
by access mode, so a read-write descriptor's bytes are never split between reads and writes;
nothing for a descriptor that is new, reopened, rewound or changed in its flags (no rate, which is
not zero), or one opened to append, whose position is always the end of the file. On Linux the
claims of a process are held to the growth of its `rchar` and `wchar` over the same interval,
read just before and after the walk. The **unattributed** remainder is what those counters grew by
and no file position explains: `pread` and `pwrite`, which name their own offset, memory-mapped
I/O, pipes, sockets, terminals and files opened to append. A file's rate is therefore a lower
bound on what the process did with it.

The sky is tinted by pressure stall information from `/proc/pressure`: amber for
CPU, crimson for memory, blue for I/O (the matrix has no haze, and a view's row can
turn it off).

NVIDIA GPU memory comes from NVML, loaded at run time. A runtime-suspended GPU has
no processes, so isotop does not query it and never wakes a laptop's discrete GPU
for monitoring.

### Life

New processes fly out of their parent (orbit) or rise from their plot (city).
The processes already running when isotop starts bloom in the same way. Exiting
processes leave a brief flash and an expanding ring. A process that appears grows in
with a cubic ease-out over 0.9 s; an exit leaves a ring of radius 0.6 + 3.4 × age
(age running from 0 to 1 over 1.2 s) fading from the colour (255, 196, 140), and a glow
for the first 35 % of that time, where the process was last drawn. A view's registry
row can turn the flash off.

### Fork, exec and exit events

A view that shows births and deaths as they happen asks for life events, and isotop
then takes each fork, exec and exit from the best source the machine allows. The
status strip names the sources in use, and what is missing:

| Source | What it adds | Needs |
|---|---|---|
| proc connector | Every fork, exec and exit with its exact time, its parent and its exit status or signal, including processes that live and die between two samples | Linux 6.6 or later (CAP_NET_ADMIN before), and the initial PID and user namespaces |
| taskstats | A record of every exit: command, start, run time, CPU time of all its threads, peak memory, storage I/O, context switches, and whether it forked without ever calling exec | CAP_NET_ADMIN, and the initial namespaces |
| kqueue (macOS) | The exact time of each exec and exit of your own processes, without the exit status | Nothing |
| sample diff | Births, exits and execs found by comparing two samples, always on; it misses a process that lives between them | Nothing |

isotop never raises its own privileges. Taskstats records, and the connector on a
kernel older than 6.6, need CAP_NET_ADMIN, which you may choose to grant the binary
yourself:

```sh
sudo setcap cap_net_admin+ep ./target/release/isotop
```

Inside a container with its own PID namespace only sample diffing works, and the
status strip says "no fork events" and "no exit records". Without the connector,
Linux has no exec counter, so sample diffing counts a process whose name changed as
an exec; a process that renames itself (`prctl(PR_SET_NAME)`) counts too. The events
reach a sample through a queue of 4096; what does not fit is dropped and counted.

### Labels, hover, and the tour

System names (`init`, `systemd --user`, `kthreadd`, ...) and the busiest
processes are labelled; `l` toggles labels. Labels are at most 15 cells wide, so
a longer name (macOS allows 31 characters) is cut after 13 characters and ends in
`..`, and Apple's own `com.apple.` prefix is dropped. Two labels on the same row
always have a blank column between them; one that would touch another is left
out. Hovering a body or building shows its full name and a short description of
what it is. After 20 seconds without input
(`--tour`, `0` disables), a guided tour eases the camera between notable
processes: system stars, the busiest, and the largest, then every other visible
process. Each gets a callout with a pointer. On the globe, the tour and search also turn the camera so the process
faces you, because the world's spin carries home to the far side for half of
every turn. `g` starts the tour at any time, even with the idle tour disabled.
Any key or mouse movement ends the tour, except Space, Shift+Space and `*` (and the press of a modifier or lock key on its own), which
visit the next or a random process in the tour's own order and let it go on from there.

## Controls

`isotop --keys` prints the tables below, and `isotop --help` ends with the same list. The
in-app help (`?`) is made from the same table, so they agree.

| Key / mouse | Action |
| --- | --- |
| **Camera** | |
| Arrows or WASD | Pan |
| `+` / `-` | Zoom |
| `Q` / `e` | Rotate a quarter turn counterclockwise / clockwise |
| PgUp / PgDn | Tilt up / down |
| `t` | Toggle top-down and isometric |
| Home | Fit scene (in the tesseract, also the canonical 4D orientation, the lab frame and the drift) |
| **Views** | |
| Tab | Next view in the playlist (every view unless `--views` or the picker narrowed it; `--list-views` names them) |
| Shift + Tab | Previous view in the playlist |
| `v` | Open the view picker, which has its own keys |
| `1`-`9`, `0` | Jump to a view in that order; the playlist does not apply |
| **Tour and visits** | |
| `g` | Start the guided tour now, from the process last visited; `g` again (or any other input) ends it; on the globe it also lets a turned earth spin again |
| Space | Visit the next process: the camera zooms to it and shows its callout, as a tour step does. A running tour goes on from there; with no tour the visit just holds. The order is the tour's notable processes, then every other visible process by identity. A view that opts out of the tour ignores it |
| Shift + Space, or `*` | Visit a random other process; a running tour goes on from it. Shift + Space needs a terminal with the Kitty keyboard protocol (Ghostty and Kitty have it); without it, Shift + Space arrives as Space, so use `*`, which the help line then names. To tell Shift + Space apart isotop asks such a terminal to report every key as an escape code, so a character composed with AltGr, Option or a dead key reaches the search box as its base key (known limitation) |
| **Pointer** | |
| Two-finger scroll / wheel | Pan (vertical and horizontal); on the globe, turn the earth (horizontal) and tilt it (vertical) |
| Ctrl + scroll | Zoom towards the pointer |
| Alt + scroll | Rotate |
| Right or middle drag | Rotate (horizontal) and tilt (vertical) |
| Hover | Name and description of the process under the pointer, or the title of a thing a view draws |
| Left click | Select the nearest process, or thing, and open its inspector popup |
| Left click on empty space | Close the popup |
| Left drag | Pan (on the globe too) |
| **Search, selection and focus** | |
| `/` | Search name, command, or exact PID |
| `n` | Next match of the search |
| `f` | Focus selected process and descendants |
| Esc | Close the popup, then the selection, then clear subtree focus |
| **Display** | |
| `l` | Toggle labels |
| `c` | Socket links: all, highlighted only, off |
| `r` | Reset camera and fit; on the globe, also lets a turned earth spin again; in the tesseract, as Home |
| **History** | |
| `P` (Shift + p) | Pause / resume |
| `[` / `]` | Previous / next retained snapshot |
| **View keys** | |
| `,` / `.` | Tesseract: a quarter turn back / forward in the current 4D plane (stops the drift) |
| `;` | Tesseract: cycle the 4D plane, XW, YW, ZW (stops the drift) |
| `b` | Tesseract: boost into the selected process's rest frame; `b` again or Esc returns to the lab frame |
| **Panels and quit** | |
| `h` | Hide or show the status panel, giving the scene the whole terminal |
| `?` | Toggle help; its first line leads with the shown view's own keys |
| `q` | Quit; the shown view may play a short farewell first, and `q` again ends it |
| Ctrl-C | Quit at once, whatever is open |

On the globe, scrolling follows your fingers the way panning moves the other
views: scrolling right carries the surface to the left, and scrolling down
carries it up, so any latitude, the poles included, can be brought to face you.
Scrolling at all stops the earth's own turn, and a turned globe holds still until
`r` or `g`; the idle tour waits while it is held. Search keeps the hold and
stands the earth upright on the match.

**Search.** `/` opens the search line. It takes every key until it closes, except Ctrl-C,
which quits:

| Key | Action |
| --- | --- |
| Any character | Add it to the search; the first match is selected as you type |
| Backspace | Delete the last character |
| Enter | Accept the search and close the line |
| Esc | Leave the search, keeping the selection |
| Tab | Next match |

**View picker.** `v` lists the views this platform supports, one row each, such as
`> * 11 coop    COOP`: `>` marks the highlighted row, `*` a view in the Tab playlist and
the number the view's place in the list. The picker takes every key until it closes, except
Ctrl-C, which quits:

| Key | Action |
| --- | --- |
| Letters and digits | Filter by name or title, ignoring case; `q` filters rather than quits |
| Backspace | Delete a filter character |
| Up / Down | Move the highlight among the matches |
| Enter | Show the highlighted view, fitted, and close the picker (nothing happens when no view matches) |
| Space | Add the highlighted view to the Tab playlist or remove it; the last view cannot be removed |
| Esc | Close without switching |

A view picked with `v` that is not in the playlist is not part of the cycle: Tab goes to
the first playlist view after it in the list, Shift+Tab to the last one before it, and
both wrap round. The mouse is ignored while the picker is open. The playlist lasts for
the run; `--views` sets it at the start.

Terminals deliver touchpad scrolling as wheel events but do not forward pinch or
rotate gestures, so Ctrl-scroll and Alt-scroll stand in for them. Camera moves
from keys, fitting, focusing, search, and the tour ease into place; direct mouse
gestures apply immediately. Focus and `--limit` only hide processes: every view keeps
the places of the hidden ones, so clearing the focus puts everything back where it was.

Pause freezes collection and animation; up to 120 snapshots are retained by
default. When the terminal supports SGR-Pixels mouse reporting (Ghostty and
Kitty do), hover and clicks are pixel-precise; otherwise clicks pick the nearest
object within about a cell. The popup inspector follows the selected process and
reports exact values, threads, NVIDIA GPU memory, the memory of the system a
body anchors, socket counts, command line, parent, and cgroup. Process identity
combines PID and start time to distinguish PID reuse. Some views also draw things
that are not processes (interrupt lines, files, graves); clicking one opens the
card the view writes for it, and the card says so when the view no longer shows
the thing. Esc closes a thing's card first; switching views closes it too.
Search, focus, links and the tour work on processes only.

## Headless rendering and performance

```sh
./target/release/isotop --demo --output city.png
./target/release/isotop --demo --view flow --output flow.png
./target/release/isotop --demo --processes 1000 --benchmark 120
./target/release/isotop --demo --renderer cpu --benchmark 120
```

`--time` fixes the synthetic workload time for repeatable screenshots. PNG output
uses a 16:9 viewport; ripple, flow, cores, reef and coop simulate four seconds first so
the media and creatures have settled, demo murmuration plays 15 seconds of its flight (300 steps) from its clumps as a live run would have drawn them, so at `--time 12` the demo's memory pressure has sent its falcon through the flock (a live headless run has no past to play and shows the clumps), demo strata replays a minute of
history (ptolemy does too, but its planets need 30 samples, so its reference renders use
`--time 300`), demo coop replays 30 seconds of samples for CPU variation, demo tesseract
replays a minute of samples at `--sample-ms` (before `--benchmark` too), and demo necropolis
replays ten minutes of samples, from before time 0, so `--time 12` shows ten rows of graves. Demo cortex plays the twelve seconds before `--time` as a live run would have drawn them (a frame every 0.1 s, none of them rasterized), so its raster is full, and demo horizon replays eight seconds of samples, so a kill that happened just before `--time` is seen as a plunge (`--time 20.5` is half a second after one). Demo arbor plays the eight seconds before `--time` as a live run would have drawn them (a frame every 0.05 s, none of them rasterized), so its blossoms, falling leaves and sway are settled. Demo anthill plays the ten seconds before `--time` the same way (a frame every 0.05 s), so its trails are laid and its ants are on their way. Live headless output takes two samples to measure CPU, so live strata
shows only its newest slice.
Benchmarks measure scene recording and rasterization, excluding terminal
transport. `--duration 5` runs an interactive session for five seconds and
reports presented frame throughput after restoring the terminal.

The scene admits at most 1024 processes by default (`--limit` changes this).
Selection can bring a process beyond the cap into the scene.

<!-- metrics:start -->
## By the numbers

71,109 lines of Rust (41,127 production, 29,982 test) in 106 files, 986 tests, 18 views, 10 direct and 168 total crate dependencies. Generated; see [docs/METRICS.md](docs/METRICS.md) for the breakdown and the debt counters.
<!-- metrics:end -->

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
python3 scripts/smoke_terminal.py
python3 scripts/code_report.py --check
```

`python3 scripts/code_report.py` regenerates `docs/METRICS.md` and the block above; it needs
[tokei](https://github.com/XAMPPRocky/tokei) (`brew install tokei`, `paru -S tokei` or
`cargo install tokei --locked`).

The smoke test emulates a graphics-capable terminal through a pseudo-terminal,
reads frames both inline and through shared memory, and exercises interactive
controls and restoration. Visual quality, flicker, and display latency should
also be checked in Ghostty.

The Rust toolchain is pinned in `rust-toolchain.toml` (and `rust-version` in
`Cargo.toml`). To bump it, change both in one pull request and fix any new lints.

CI builds and tests on Linux and on macOS (`macos-15`, Apple Silicon). The macOS
code can be type-checked from Linux with
`cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings`.

## Credits

Stroke font: A. V. Hershey, Naval Weapons Laboratory, NWL Report 2101 (1967). The Usenet distribution's notice is reproduced verbatim in `src/hershey/NOTICE` as its terms require.

## Next milestones

- Zoom-dependent aggregation for very dense systems.
- Optional GUI presentation using the same monitoring and scene model.
