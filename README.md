# isotop

A living picture of your machine inside the terminal: a process city, an orbital
observatory, a rippling sea, and a spacetime weather map. Linux-first, written in
Rust, with real process data and pixel graphics through the Kitty graphics
protocol. Ghostty is the primary target.

## Run

```sh
cargo build --release
./target/release/isotop --demo
./target/release/isotop
./target/release/isotop --view orbit      # also: city, ripple, flow
```

Run directly in a graphics-capable terminal such as Ghostty or Kitty; tmux does
not pass the graphics protocol through. The app queries graphics support on
startup and fails fast when it is missing. `--force-graphics` bypasses that query.

The scene renders at the window's native resolution up to 1920 pixels wide
(`--width` lowers the cap) and animation targets 20 FPS. Process data is sampled
once per second; sockets and NVIDIA GPU usage every two seconds on a background
thread. CPU uses a 1.5-second exponential smoothing time constant; 100% means one
fully occupied CPU core. Sizes ease toward each sample over 0.3 seconds;
inspector values remain the sampled measurements.

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

isotop only runs on Linux today: process data comes from `/proc`. The renderer
itself is portable through wgpu's Metal backend, but a macOS port also needs a
process collector.

## Worlds

The scene is real 3D geometry seen through an orthographic camera: isometric by
default, free to rotate and tilt between a low angle and top-down. Tab cycles
city, orbit, ripple and flow. Orbit, ripple and flow share one layout, so a
process sits in the same place in each.

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
- Systems are packed around the centre, largest first, and keep their place.

Four levels are drawn per system; deeper processes are counted as collapsed.

### Ripple

The machine as a body of water: a damped 2D wave equation (nine-point stencil,
absorbing border) simulated on a grid under the layout and drawn as a lit
low-poly surface.

- Busy processes drive ripples at their own pitch, louder with more CPU; where
  neighbours are busy, their waves interfere.
- Buoys float on the surface, sized by memory as in orbit.
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

### Links and weather

Socket links come from the kernel: loopback TCP pairs from `/proc/net/tcp{,6}`
and connected Unix-socket peers from the sock_diag netlink interface, matched to
processes through `/proc/<pid>/fd`. Only processes whose descriptors isotop may
read appear. In orbit, ripple and city they are cyan arcs with travelling pulses
(`c` cycles all / highlighted / off); TCP connections to other machines rise as
pink beams. Links of the selected, hovered or toured process glow.

The sky is tinted by pressure stall information from `/proc/pressure`: amber for
CPU, crimson for memory, blue for I/O.

NVIDIA GPU memory comes from NVML, loaded at run time. A runtime-suspended GPU has
no processes, so isotop does not query it and never wakes a laptop's discrete GPU
for monitoring.

### Life

New processes fly out of their parent (orbit) or rise from their plot (city).
The processes already running when isotop starts bloom in the same way. Exiting
processes leave a brief flash and an expanding ring.

### Labels, hover, and the tour

System names (`init`, `systemd --user`, `kthreadd`, ...) and the busiest
processes are labelled; `l` toggles labels. Hovering a body or building shows its
name and a short description of what it is. After 20 seconds without input
(`--tour`, `0` disables), a guided tour eases the camera between notable
processes: system stars, the busiest, and the largest. Each gets a callout with
a pointer. `g` starts the tour at any time, even with the idle tour disabled.
Any key or mouse movement ends the tour.

## Controls

| Key / mouse | Action |
| --- | --- |
| Tab | Next view: city, orbit, ripple, flow |
| Two-finger scroll / wheel | Pan (vertical and horizontal) |
| Ctrl + scroll | Zoom towards the pointer |
| Alt + scroll | Rotate |
| Right or middle drag | Rotate (horizontal) and tilt (vertical) |
| Arrows or WASD | Pan |
| `+` / `-` | Zoom |
| `Q` / `e` | Rotate a quarter turn counterclockwise / clockwise |
| PgUp / PgDn | Tilt up / down |
| `t` | Toggle top-down and isometric |
| Home | Fit scene |
| `r` | Reset camera and fit |
| Hover | Name and description of the process under the pointer |
| Left click | Select the nearest process and open its inspector popup |
| Left click on empty space | Close the popup |
| Left drag | Pan |
| `l` | Toggle labels |
| `g` | Start the guided tour now; `g` again (or any input) ends it |
| `c` | Socket links: all, highlighted only, off |
| `/` | Search name, command, or exact PID |
| Tab while searching / `n` afterward | Next match |
| Enter | Accept search |
| `f` | Focus selected process and descendants |
| Esc | Leave search, close the popup, or clear subtree focus |
| Space | Pause / resume |
| `[` / `]` | Previous / next retained snapshot |
| `?` | Toggle help |
| `q` / Ctrl-C | Quit |

Terminals deliver touchpad scrolling as wheel events but do not forward pinch or
rotate gestures, so Ctrl-scroll and Alt-scroll stand in for them. Camera moves
from keys, fitting, focusing, search, and the tour ease into place; direct mouse
gestures apply immediately.

Pause freezes collection and animation; up to 120 snapshots are retained by
default. When the terminal supports SGR-Pixels mouse reporting (Ghostty and
Kitty do), hover and clicks are pixel-precise; otherwise clicks pick the nearest
object within about a cell. The popup inspector follows the selected process and
reports exact values, threads, NVIDIA GPU memory, the memory of the system a
body anchors, socket counts, command line, parent, and cgroup. Process identity
combines PID and start time to distinguish PID reuse.

## Headless rendering and performance

```sh
./target/release/isotop --demo --output city.png
./target/release/isotop --demo --view flow --output flow.png
./target/release/isotop --demo --processes 1000 --benchmark 120
./target/release/isotop --demo --renderer cpu --benchmark 120
```

`--time` fixes the synthetic workload time for repeatable screenshots. PNG output
uses a 16:9 viewport; ripple and flow simulate four seconds first so the media
have developed. Live headless output takes two samples to measure CPU.
Benchmarks measure scene recording and rasterization, excluding terminal
transport. `--duration 5` runs an interactive session for five seconds and
reports presented frame throughput after restoring the terminal.

The scene admits at most 1024 processes by default (`--limit` changes this).
Selection can bring a process beyond the cap into the scene.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
python3 scripts/smoke_terminal.py
```

The smoke test emulates a graphics-capable terminal through a pseudo-terminal,
reads frames both inline and through shared memory, and exercises interactive
controls and restoration. Visual quality, flicker, and display latency should
also be checked in Ghostty.

## Next milestones

- A macOS process collector, so the Metal path can run on Apple GPUs.
- Zoom-dependent aggregation for very dense systems.
- Optional GUI presentation using the same monitoring and scene model.
