# isotop

A living picture of your machine inside the terminal, in sixteen views: a process
city, an orbital observatory, a rippling pond, a spacetime weather map, a race
track of CPU cores, petri dishes of cgroups, a ridgeline landscape of CPU
history, a globe of network connections, a coral reef, a chicken yard, a geocentric
cosmos of Fourier epicycles, the system log (the systemd journal, or the unified log on
macOS) as Matrix rain, a minute of process worldlines in a tesseract whose fourth
axis is time, a moonlit cemetery of the processes that exited, and a black hole whose orbits rank the
processes the OOM killer will take next. Written in
Rust for Linux and for macOS on Apple Silicon, with real process data and pixel
graphics through the Kitty graphics protocol. Ghostty is the primary target.

https://github.com/user-attachments/assets/05663db3-6c64-48bc-8696-45f166ce325c

| | | |
|:-:|:-:|:-:|
| ![City](docs/media/city.webp)<br>**1 City** | ![Orbit](docs/media/orbit.webp)<br>**2 Orbit** | ![Ripple](docs/media/ripple.webp)<br>**3 Ripple** |
| ![Flow](docs/media/flow.webp)<br>**4 Flow** | ![Cores](docs/media/cores.webp)<br>**5 Cores** | ![Cells](docs/media/cells.webp)<br>**6 Cells** |
| ![Strata](docs/media/strata.webp)<br>**7 Strata** | ![Globe](docs/media/globe.webp)<br>**8 Globe** | ![Reef](docs/media/reef.webp)<br>**9 Reef** |

The video at the top is a live session on a real machine, touring the views. The
other screenshots and recordings are of Ghostty running `isotop --demo`, the
synthetic workload; the coop image is a crop of a headless `--demo --output` render.

## Run

```sh
cargo build --release
./target/release/isotop --demo
./target/release/isotop
./target/release/isotop --view orbit      # also: city, ripple, flow, cores, cells, strata, globe, reef, coop, matrix, ptolemy, tesseract, necropolis, cortex, horizon
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
| OOM score | macOS has no OOM killer; jetsam ranks processes by priority band instead, and only root may list the bands (`memorystatus_control`). Run with `sudo` and `oom score` reads the negated band, so a lower band ranks as killed sooner and `oom_score_adj` is unknown; without root, or if the call fails, it is missing and each process's ranking is unreadable, never zero. A view that ranks by it says so |
| Memory detail | There is no `smaps_rollup`, so a process's memory is never split into private, shared, anonymous, file and swap parts: `memory detail` is always missing and every process's rollup is unreadable. The footprint stands in for memory everywhere else |
| Kernel counters | There is no interrupt table, no softirq count, no IRQ affinity and no readable count of OOM kills (it lives in the private IOReport framework), so `Snapshot::interrupts`, `softirqs` and `oom_kills` are never filled and the platform reports "interrupts", "softirqs", "irq affinity" and "oom kills" as missing. No view draws them yet |
| Major page faults | Not a gap, but not the same count: macOS has no per-process major fault counter, so the pages the process paged in (`pti_pageins` from `PROC_PIDTASKINFO`) stand in for it. That count includes pages brought in ahead of any fault, so it can run higher than Linux's `majflt` |
| The systemd journal | Matrix follows the unified log instead; see below |
| Exit statuses (necropolis) | Every epitaph reads "died between samples", stone heights and widths are the CPU time and memory last sampled, lifetimes run from the start time, there are no unmarked markers, and a mausoleum stands for an app group (there is no `.service`) whose processes are all gone |
| OOM scores and kills (horizon) | Horizon is never skipped. Without an `oom_score` (macOS only lists jetsam bands, and only as root, with no `oom_score_adj`) it ranks by memory footprint, so rank 1 on the ISCO is the largest process; there is no unkillable band, because there is no `oom_score_adj` -1000; and the OOM kill count lives in a private framework, so no exit is classified and every exit fades in place instead of plunging. Its legend says "no oom score: ranked by memory footprint" and "no oom kills: exits not classified". Lensing, the disk and the memory-pressure brightness work as on Linux |
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
default, free to rotate and tilt between a low angle and top-down. Tab cycles
the sixteen views in order, the number keys 1 to 9 jump straight to the first nine
and 0 to the matrix. The coop, ptolemy, the tesseract, the necropolis, the cortex and the horizon are reached with Tab, Shift+Tab,
the `v` picker or `--view`.
Orbit and flow share one layout, so a process sits in the same place in each. Views
that grow as data arrives (cores, cells, strata, globe, reef, coop) keep the camera
framed until you move it; Home, Tab or a number key frames them again.

### City

- Height: smoothed process CPU usage, `1 + 44 * cpu / (cpu + 100)` world units
  against 4-unit plots: idle processes are low blocks, one busy core is a tower
  about six plots tall, and multi-core work climbs towards eleven.
- Footprint: bounded square-root memory scale (resident set).
- District: service/cgroup, split into fixed 16-building blocks as needed.
- Windows and rooftop lights: decorative CPU activity cues; green beacons:
  processes using the NVIDIA GPU.
- Cyan ground pulses: attributed disk I/O activity when readable.
- Pink: zombie; orange: stopped; red: uninterruptible sleep.

Plots stay fixed as resources change. Exited processes release their plots.
Blocks grow along square shells so newly allocated districts do not relocate
existing ones.

### Orbit

- Colour: slate kernel threads, teal system services, amber processes in your
  session, violet containers (from kernel thread flags and the cgroup path).
- Size: volume proportional to memory (radius is the cube root), so a 1 GiB
  process is about five times as wide as a 6 MiB one. A body with children is
  sized by the memory of its whole subtree, so stars carry their systems' mass.
- Rings: Saturn rings for threads, one more ring per fourfold thread count.
- CPU: a warm glow and a comet trail along the orbit. Green glow and halo: NVIDIA
  GPU memory.
- Children orbit their parents on Keplerian ellipses with the parent at one
  focus. Motion follows Kepler's equation, so bodies speed up near periapsis, and
  periods follow T ~ sqrt(a^3 / M): wider orbits are slower and heavier parents
  pull their children round faster (clamped to 8 s to 10 min).
- Branches hanging off a top-level process (init, kthreadd) become separate solar
  systems; leaf processes stay in orbit, so kernel threads form one ringed system
  around kthreadd.
- Large sibling sets fill concentric shells that share one eccentricity and
  orientation, so they are scaled copies of each other and never cross; siblings
  on a shell share one ellipse and keep their order.
- Systems are packed around the centre, largest first, and the whole galaxy
  revolves about its memory-weighted barycenter like satellite galaxies:
  systems whose rings overlap turn together, and detached outer systems turn at
  their own slower Kepler rate (once every 2 to 15 minutes), so they never
  collide. A system only moves out of its place on the wheel when it outgrows
  the room reserved for it.

Four levels are drawn per system; deeper processes are counted as collapsed.

### Ripple

![Ripple: pebble clusters driving interfering ripples](docs/media/ripple-motion.webp)

The machine as a pond: a damped 2D wave equation (nine-point stencil, absorbing
border) simulated on a grid and drawn as a smoothly lit surface.

- Processes are pebbles, clustered by cgroup like handfuls thrown into the
  water. A pebble keeps its place for life; a cluster moves only when it
  outgrows the open water around it.
- Busy pebbles drive ripples at their own pitch, louder with more CPU, so a
  cluster's rings interfere and spread out to its neighbours.
- Pebbles float on the surface, sized by memory.
- Every patch of water belongs to its nearest process and is tinted by it;
  hovering the water names its owner.
- New processes drip; exiting processes splash. Pressure raises a swell.

### Flow

Spacetime weather: particles traced through a velocity field over a sheet that
memory bends.

- Each solar system bends a gravity well as wide as the system and deeper for
  more memory; heavy processes add their own dimples. The fabric brightens and
  turns violet with depth.
- Busy processes spin whirlpools and emit particles in their colour.
- Socket links become two-lane rivers between the processes that talk, one lane
  each way, carrying traffic in each end's colour.
- Connections that leave the machine send sparks rising off the sheet; exits
  burst outward; pressure adds turbulence.

### Cores

![Cores: marbles lapping a lane per CPU](docs/media/cores-motion.webp)

A race track with one lane per CPU: performance cores inside in gold, efficiency
cores outside in teal (from `/sys/devices/cpu_core` and `cpu_atom` on hybrid
Intel CPUs), banked like a velodrome.

- Lane brightness: how busy that CPU was (`/proc/stat`). Chevrons run at its
  clock speed (`cpufreq`).
- Red marbles queued behind the start line: tasks waiting for that CPU, from
  run-queue delay in `/proc/schedstat`.
- Marbles: running processes, sized by memory. A marble covers one lap per 10
  seconds of CPU time, so a process using a full core laps every 10 seconds. When
  the scheduler moves a process to another CPU, its marble hops across lanes;
  the inspector counts the hops.
- Idle processes stay off the track and only count towards their lane's label.
- On macOS, which reports no last CPU, marbles ride the middle of the performance
  or the efficiency lanes, whichever they mostly ran on, and hop between the two when
  that changes; chevrons run at each cluster's clock, and red beads trail a marble
  for its waiting threads; see [macOS](#macos).

### Cells

Every cgroup is a cell in a petri dish: one dish each for system services, your
session and containers. Accounting comes from the cgroup v2 files
(`memory.current`, `memory.max`, `cpu.stat`, `cpu.max`, `memory.events`,
`pids.current`, `*.pressure`).

- Cell size: the cgroup's memory. A dashed ring marks its memory limit, reddening
  as usage nears it; an arc around the membrane shows CPU use against its quota.
- The membrane trembles with the cgroup's pressure, flashes red when its CPU
  quota throttles it, and bursts when the OOM killer strikes inside it.
- Organelles: its processes, with the largest as the nucleus. Kernel threads
  belong to no cell and are only counted.
- Processes hidden by focus (`f`) or by `--limit` keep their seats, and their
  cells keep their place and size, so clearing the focus puts every cell back.
  A cell draws only the processes that are shown, and its nucleus stays its
  largest process even while that one is hidden.

### Strata

The last minute of CPU use as a ridgeline landscape. Each of up to 40 busy
processes is a ridge whose height traces its CPU over time, with now at the
front edge; rows run from kernel threads at the back through system services and
your session to containers at the front. A process keeps its ridge, in the same
row, while it stays busy. History is recorded whichever view is shown, so the
landscape is already a minute deep when you switch to it.

### Globe

Where the machine's TCP connections go. The world turns once every five minutes;
arcs rise from home to every remote place, brighter with more traffic, cyan when
mostly downloading and pink when mostly uploading, with pulses travelling the way
the bytes flow. Processes with connections hover above home, and their inspector
lists each connection with its round-trip time and rates (from the kernel's
`tcp_info` through sock_diag).

Locations come from a local GeoIP database, read the first time the globe is
shown and looked up only while it is shown; nothing is sent anywhere.

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
- Home is the system time zone's city (`/etc/localtime` and `zone1970.tab`), or
  `--home LAT,LON`.

Coastlines and country label points are from [Natural Earth](https://www.naturalearthdata.com/)
(public domain).

### Reef

![Reef: coral, schools of fish and crabs](docs/media/reef-motion.webp)

The machine as a coral reef under a sea sky. System services grow as coral
colonies, one per cgroup, with a polyp per process that glows with CPU; your
session's apps swim in schools whose speed follows their CPU; containers are
crabs scuttling on the sand; busy kernel threads drift as plankton. Size follows
memory everywhere, I/O rises as bubbles, and zombies float belly-up. Processes
hidden by focus (`f`) or by `--limit` keep their seats and their cluster's place,
and a hidden fish waits where it swam, so clearing the focus puts everything back. The reef
keeps room for the clusters of hidden processes, so a limit that hides many leaves empty water,
and focusing a subtree does not change the framing.

### Coop

![Coop: flocks foraging by their henhouses, roosting on perch ladders and queueing at the feeders](docs/media/coop.webp)

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
| Kill | The kernel's OOM kill count: `oom_kill` in `/proc/vmstat`, else the sum of the cgroups' `memory.events` | A body that vanishes stays a ghost for 2.5 s. If the counter rose within 2.5 s of it, before or after, the best-ranked ghost plunges: it eases to the ISCO in 0.8 s, then follows the geodesic of a body that leaves it with 0.95 of the ISCO's angular momentum and a small inward kick, as a radial streak whose brightness 1.5 (1 - 2/r) falls (the light, its square root, by e every 4 M, 0.4 s), turning red, and freezes at r > 2.0006 M, never reaching the horizon. Every other exit fades where it stood over 1.2 s. The legend says which counter it used and how many kills it has seen |
| Next victim | The top of the ranking | A red ring on its sphere, the first label and the first stop of the tour |
| Inspector | `oom_score`, `oom_score_adj`, rank, radius | `oom_score N \| oom_score_adj N`, `rank R of K`, `orbit r = X.XX M \| period P M (S s)` |

**Decorative:** the disk's turbulence (value noise in the frame that goes round with the disk, streaks long in azimuth), the glare (up to 24 glows along the inner edge, where the disk is brightest), the 1500 background stars on a shell 40 M behind the hole and their glows (a fixed seed, coloured by temperature), the dashed guides at the ISCO and the band, the photon ring, the next victim's red ring and the haze behind the sky. **The photon ring is drawn, not traced:** a ring 0.1 M wide (at least 1.5 px) at 5.27 M, outside the shadow of radius 3 sqrt(3) M = 5.196 M, tinted by the Doppler shift of matter at 7 M; no ray is cast for it. **The texture, the glare and the stars carry no data.**

**Lensing.** Units are G = c = M = 1, so one world unit is one M. Every vertex of the scene is carried to its image on the screen and ordinary triangles are drawn between the images, so the CPU and the GPU draw the same frame and picking works. A table of photon rays traced back from an observer at infinity (fourth-order Runge-Kutta on d2u/dphi2 = 3u^2 - u with u = 1/r, a step of 0.002 rad, 1800 impact parameters) gives, for 360 log-spaced radii from 2.0005 to 150 M and 721 sweep angles, the screen distance of the primary image (the ray that sweeps the angle) and of the secondary image (the ray that sweeps 2 pi minus it, round the other side of the hole, never inside the shadow). The table is built on the first frame, in a fifth of a second. The disk is cut into cells of about 4 px, at most 120,000 triangles in both images (the secondary has half the density), lensed again only when the inclination or the scale changes, and recoloured every frame. The flux is the Page-Thorne integral in closed form, which falls quadratically to zero at the ISCO.

**What cannot be measured.** The kernel keeps the count of kills, not their victims: a plunge is the best-ranked process that vanished within 2.5 s of a rise, which a different exit at the same moment can fool. With no counter at all every exit fades, and the legend says "no oom kills: exits not classified"; where only the cgroups count, it says "kills counted from cgroups". A process whose score cannot be read is left out and counted (`N unranked`). Rewinding with `[` shows the older sample's processes, not the older orbits.

**Platform.** On Linux everything above. On macOS the ranking is by memory footprint, there is no unkillable band and no plunge: see [What macOS does not report](#what-macos-does-not-report-and-how-each-view-shows-it). Horizon has no keys of its own; `l` hides the labels.

### Matrix

![Matrix: journal lines decoding out of digital rain](docs/media/matrix.webp)

The systemd journal decoded out of digital rain (on macOS, the unified log; see
[macOS](#macos)). `journalctl -f` runs on a
background thread from the moment the view is first shown, starting with the
last 200 entries, and stops 30 seconds after you leave the view; coming back
starts it again with the last 200 entries. The screen is a log of `source[pid]: message` lines written
horizontally, newest at the bottom, coloured by severity: red for errors and
worse, amber for warnings, green for notices and information, teal for debug.
Each character resolves only when a falling stream of rain passes over it,
flashing white before it settles; every new line brings a shower down onto its
own characters, and anything the rain misses appears after three seconds.
Between the lines falls cmatrix-style rain of flickering green glyphs. Lines
are written one at a time, faster when they queue up; a flood that outruns the
log is trimmed, and the status strip counts what was skipped. The camera and
process controls do nothing here. Glyphs are the public-domain X11 misc-fixed
7x14 font, and `--demo` writes synthetic entries. Without the `adm` or
`systemd-journal` group, journalctl shows only your own user's journal.

### Links and weather

Socket links come from the kernel: loopback TCP pairs from `/proc/net/tcp{,6}`
and connected Unix-socket peers from the sock_diag netlink interface, matched to
processes through `/proc/<pid>/fd`. Only processes whose descriptors isotop may
read appear. In orbit, ripple and city they are cyan arcs with travelling pulses
(`c` cycles all / highlighted / off); TCP connections to other machines rise as
pink beams. Links of the selected, hovered or toured process glow.

Two more measurements ride on those links, for the views that draw them: the bytes waiting
unread on each connected Unix socket (for a datagram socket only that a packet waits, since
the kernel gives the size of the first packet and not a total), and the pipes and FIFOs between
processes with their direction, from `/proc/<pid>/fd` and `fdinfo` (macOS: `proc_pidfdinfo`,
which also counts the bytes waiting in a pipe). Linux gives no such count for a pipe another
process holds. Only the pipes are read on demand; the existing views draw neither.

The sky is tinted by pressure stall information from `/proc/pressure`: amber for
CPU, crimson for memory, blue for I/O.

NVIDIA GPU memory comes from NVML, loaded at run time. A runtime-suspended GPU has
no processes, so isotop does not query it and never wakes a laptop's discrete GPU
for monitoring.

### Life

New processes fly out of their parent (orbit) or rise from their plot (city).
The processes already running when isotop starts bloom in the same way. Exiting
processes leave a brief flash and an expanding ring.

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
Any key or mouse movement ends the tour, except Space, Shift+Space and `*`, which
visit the next or a random process in the tour's own order and let it go on from there.

## Controls

| Key / mouse | Action |
| --- | --- |
| Tab | Next view in the playlist (every view unless `--views` or the picker narrowed it): city, orbit, ripple, flow, cores, cells, strata, globe, reef, matrix, coop, ptolemy, tesseract, necropolis, cortex, horizon |
| Shift + Tab | Previous view in the playlist |
| `v` | Open the view picker (below) |
| `1`-`9`, `0` | Jump to a view in that order; the playlist does not apply |
| Two-finger scroll / wheel | Pan (vertical and horizontal); on the globe, turn the earth (horizontal) and tilt it (vertical) |
| Ctrl + scroll | Zoom towards the pointer |
| Alt + scroll | Rotate |
| Right or middle drag | Rotate (horizontal) and tilt (vertical) |
| Arrows or WASD | Pan |
| `+` / `-` | Zoom |
| `Q` / `e` | Rotate a quarter turn counterclockwise / clockwise |
| PgUp / PgDn | Tilt up / down |
| `t` | Toggle top-down and isometric |
| Home | Fit scene (in the tesseract, also the canonical 4D orientation, the lab frame and the drift) |
| `r` | Reset camera and fit; on the globe, also lets a turned earth spin again; in the tesseract, as Home |
| Hover | Name and description of the process under the pointer, or the title of a thing a view draws |
| Left click | Select the nearest process, or thing, and open its inspector popup |
| Left click on empty space | Close the popup |
| Left drag | Pan (on the globe too) |
| `l` | Toggle labels |
| `h` | Hide or show the status panel, giving the scene the whole terminal |
| `g` | Start the guided tour now, from the process last visited; `g` again (or any other input) ends it; on the globe it also lets a turned earth spin again |
| Space | Visit the next process: the camera zooms to it and shows its callout, as a tour step does. A running tour goes on from there; with no tour the visit just holds. The order is the tour's notable processes, then every other visible process by identity. A view that opts out of the tour ignores it |
| Shift + Space, or `*` | Visit a random other process; a running tour goes on from it. Shift + Space needs a terminal with the Kitty keyboard protocol (Ghostty and Kitty have it); without it, Shift + Space arrives as Space, so use `*`, which the help line then names |
| `c` | Socket links: all, highlighted only, off |
| `/` | Search name, command, or exact PID |
| Tab while searching / `n` afterward | Next match |
| Enter | Accept search |
| `f` | Focus selected process and descendants |
| Esc | Leave search, close the popup, or clear subtree focus |
| `P` (Shift + p) | Pause / resume |
| `[` / `]` | Previous / next retained snapshot |
| `,` / `.` | Tesseract: a quarter turn back / forward in the current 4D plane (stops the drift) |
| `;` | Tesseract: cycle the 4D plane, XW, YW, ZW (stops the drift) |
| `b` | Tesseract: boost into the selected process's rest frame; `b` again or Esc returns to the lab frame |
| `?` | Toggle help; the second line leads with the shown view's own keys |
| `q` / Ctrl-C | Quit |

On the globe, scrolling follows your fingers the way panning moves the other
views: scrolling right carries the surface to the left, and scrolling down
carries it up, so any latitude, the poles included, can be brought to face you.
Scrolling at all stops the earth's own turn, and a turned globe holds still until
`r` or `g`; the idle tour waits while it is held. Search keeps the hold and
stands the earth upright on the match.

**View picker.** `v` lists the views this platform supports, one row each, such as
`> * 11 coop    COOP`: `>` marks the highlighted row, `*` a view in the Tab playlist and
the number the view's place in the list. The picker takes every key until it closes:

| Key | Action |
| --- | --- |
| Letters and digits | Filter by name or title, ignoring case; `q` filters rather than quits |
| Backspace | Delete a filter character |
| Up / Down | Move the highlight among the matches |
| Enter | Show the highlighted view, fitted, and close the picker (nothing happens when no view matches) |
| Space | Add the highlighted view to the Tab playlist or remove it; the last view cannot be removed |
| Esc | Close without switching |
| Ctrl-C | Quit |

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
the media and creatures have settled, demo strata replays a minute of
history (ptolemy does too, but its planets need 30 samples, so its reference renders use
`--time 300`), demo coop replays 30 seconds of samples for CPU variation, demo tesseract
replays a minute of samples at `--sample-ms` (before `--benchmark` too), and demo necropolis
replays ten minutes of samples, from before time 0, so `--time 12` shows ten rows of graves. Demo cortex plays the twelve seconds before `--time` as a live run would have drawn them (a frame every 0.1 s, none of them rasterized), so its raster is full, and demo horizon replays eight seconds of samples, so a kill that happened just before `--time` is seen as a plunge (`--time 20.5` is half a second after one). Live headless output takes two samples to measure CPU, so live strata
shows only its newest slice.
Benchmarks measure scene recording and rasterization, excluding terminal
transport. `--duration 5` runs an interactive session for five seconds and
reports presented frame throughput after restoring the terminal.

The scene admits at most 1024 processes by default (`--limit` changes this).
Selection can bring a process beyond the cap into the scene.

<!-- metrics:start -->
## By the numbers

55,206 lines of Rust (32,748 production, 22,458 test) in 57 files, 753 tests, 16 views, 10 direct and 168 total crate dependencies. Generated; see [docs/METRICS.md](docs/METRICS.md) for the breakdown and the debt counters.
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

## Next milestones

- Zoom-dependent aggregation for very dense systems.
- Optional GUI presentation using the same monitoring and scene model.
