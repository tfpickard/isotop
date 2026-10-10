# isotop: agent guide

isotop is a process visualizer for the terminal on Linux and macOS, written in Rust (edition
2024). It samples the operating system (`/proc` on Linux, libproc and Mach on macOS), builds a 3D scene, rasterizes it on the GPU (wgpu) or the CPU, and shows the
frames through the Kitty graphics protocol. The primary terminal is Ghostty; Kitty also works.
The README covers usage. This file covers how to work on the code.

## Pipeline

1. **Sample** (`model.rs`, `platform/`). `Collector::sample` reads the operating system through
   `platform::Sampler` (on Linux, `/proc`) once per `--sample-ms`, smooths CPU use and returns a
   `Snapshot`. Slow sources (sockets in `platform/linux/net.rs`, open files and file locks in
   `platform/linux/files.rs`, cgroup accounting, NVIDIA memory in `platform/linux/nvml.rs`;
   on macOS sockets and open files only, as it has no lock table) run on a background thread
   every 2 s and are merged in, so the frame loop never waits on them.
   `main.rs` keeps a history ring of snapshots for pause and rewind.
2. **Record** (`render.rs`). `Scene::render` turns a snapshot into a display list of
   screen-space `Item`s (triangles, lines, sphere impostors, glows, beams, stars) on a `Frame`.
   The `Scene` persists between frames to hold layout, smoothing, births and deaths, and the
   simulated media.
3. **Rasterize**. `Frame::rasterize` does this on the CPU; `gpu.rs` with `shaders.wgsl` does it
   on wgpu (Vulkan, Metal or DX12). Both produce colour, depth and a pick buffer.
4. **Present** (`terminal.rs`). Frames go to the terminal through POSIX shared memory (`t=s`),
   or inline as zlib and base64. Labels, popups and the status lines are drawn as terminal
   text over the image.

| File | Owns |
|---|---|
| `main.rs` | CLI (`Options`), `Layout`, `App` (input, camera easing, tour, overlay text), main loop, headless PNG and benchmark |
| `model.rs` | `Process`, `Snapshot`, `Collector` (CPU smoothing, per-interval rates and shares from counter deltas, background thread, merge), demo workload, `describe` |
| `platform/mod.rs` | The OS contract (`Sampler`, `RawProcess`, `network`, `account`, `FileScan`, `locks`, `Gpu`, `journal`) and the platform-neutral `Network`, `Files` and `Locks`; selects the implementation by `target_os` |
| `platform/linux/mod.rs` | Linux `Sampler`: `/proc` parsing, CPUs, pressure, core kinds, cgroup v2 accounting |
| `platform/linux/net.rs` | Socket links: `/proc/net/tcp*`, sock_diag netlink (Unix peers, inet TCP with `tcp_info`) |
| `platform/linux/files.rs` | Open regular files and deleted-but-open files per process (`/proc/<pid>/fd`, bounded per scan) and file locks with their waiters (`/proc/locks`) |
| `platform/linux/nvml.rs` | NVIDIA per-process GPU memory via `dlopen`; never wakes a runtime-suspended GPU |
| `platform/linux/journal.rs` | `journalctl` in export format and its parser |
| `journal.rs` | Journal lines for the Matrix view: reader thread, bounded backlog, demo lines |
| `render.rs` | `Camera`, `Sky`, `Item`, `Frame` (CPU rasterizer and picking), `Scene` and every view |
| `coop.rs` | The Coop view: Vicsek flocks per cgroup, henhouses, nests (eggs for open files, brooding and queueing for file locks), feeders, chicks, dust and foxes, stepped at a fixed 20 Hz |
| `simulation.rs` | Shared pure simulation helpers: `SpatialHash` neighbour queries and the seeded `Rng` |
| `medium.rs` | Pure simulation state for the Ripple (wave equation) and Flow (particles) views |
| `gpu.rs`, `shaders.wgsl` | wgpu backend that mirrors the CPU rasterizer |
| `terminal.rs` | Graphics-capability probe, frame transfer, text overlay, terminal restoration |
| `platform/macos/mod.rs` | macOS `Sampler` (libproc, Mach, sysctl, IORegistry, `proc_pid_rusage` V6 with fallbacks), the cluster clocks, the socket scan, the bounded open-file scan (`FileScan`, one `proc_pidfdinfo` per vnode), and what it reports through `missing`, `per_cluster` and `unreadable` |
| `platform/macos/ffi.rs` | Every extern declaration and `#[repr(C)]` struct that `libc` lacks, with size assertions |
| `platform/macos/logic.rs` | Pure macOS decisions with no FFI: kinds, groups, parents, `KERN_PROCARGS2` parsing, tick conversion, cluster clocks from cycle counters, socket pairing, open and deleted-but-open files from `vinfo_stat` |
| `platform/macos/journal.rs` | The unified-log follower (`log show`, then `log stream`) and its std-only JSON line parser |

## Invariants

- **The CPU and GPU backends must draw the same frame.** If you change how anything is
  rasterized (sky, sphere shading, glow falloff, depth mapping), change `render.rs` and
  `shaders.wgsl` together. To check, render the same view with `--renderer cpu` and
  `--renderer gpu` and compare the PNGs.
- **Layouts are stable.** A process keeps its place when CPU, memory or the population changes.
  Systems and districts are only re-placed when their structure outgrows the reservation, and
  tests enforce this. Don't introduce a layout that reshuffles every sample.
- **Picking.** A pickable item carries an index into `frame.identities`. `NONE` means "not
  pickable", and lines clear picking where they draw. Mouse input is cell-coarse, so
  `pick_near` searches a radius.
- **Process and system sampling lives in `platform/`.** Nothing outside it reads `/proc` or
  `/sys` or spawns OS tools; terminal I/O (`terminal.rs`) and local lookups (`geo.rs`) are the
  exceptions. Both platforms provide the same names; a macOS build must compile without
  warnings (`cargo clippy --target aarch64-apple-darwin`).
- **Sampling stays off the frame loop.** Anything slower than a `/proc/<pid>/stat` read belongs
  on the background thread.
- **Process text is untrusted.** Names and command lines are sanitized before they reach the
  terminal (`process_text_cannot_inject_terminal_controls`). Keep it that way for any new text.
- **No network access.** isotop never sends anything off the machine. Lookups such as GeoIP use
  local databases only.
- **Unsafe code** is limited to FFI (libc, netlink, shared memory, NVML) and the SSE4.1
  readback copy in `gpu.rs`. Each `unsafe` block is preceded by a comment that says why it is
  sound, normally a `// SAFETY:` comment.

## Gates

Run these in order before calling work done:

```sh
cargo fmt && cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
python3 scripts/smoke_terminal.py
```

macOS code cannot run on Linux. Type-check it locally with
`cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings` (give it its own
`CARGO_TARGET_DIR`); CI runs the gates and the live renders on `macos-15`. The pure logic in
`platform/macos/logic.rs` and `platform/macos/journal.rs` is also compiled into Linux test
builds, so `cargo test` covers it here. Keep FFI out of those two files so that stays true.

The smoke test drives the release binary through a pseudo-terminal. It runs once in demo mode
over shared memory and once in live mode inline, and exercises view switching, search, focus,
pause, quit and terminal restoration.

## Visual checks

Unit tests don't show whether a view reads well, so render it and look:

```sh
./target/release/isotop --demo --view orbit --width 1600 --output /tmp/orbit.png
./target/release/isotop --view city --output /tmp/live.png          # live, two samples
./target/release/isotop --demo --view flow --width 1600 --renderer gpu --benchmark 120
```

`--time` fixes the demo workload, so screenshots are repeatable. To check a real Ghostty
session on Wayland without disturbing the desktop, run a headless sway and capture with
`grim`:

```sh
env -u WAYLAND_DISPLAY -u DISPLAY -u SWAYSOCK WLR_BACKENDS=headless \
  WLR_LIBINPUT_NO_DEVICES=1 setsid -f sway --unsupported-gpu -c <config>
WAYLAND_DISPLAY=<socket> ghostty --config-default-files=false -e ./target/release/isotop --demo
WAYLAND_DISPLAY=<socket> grim /tmp/ghostty.png
```

Kitty crashes when headless. Stop sway by PID, not with `pkill -f` and a pattern that can
match your own shell.

## Code conventions

- Comments explain math, protocol values or rules that aren't obvious, never what the next
  line does. Doc comments state what a type or function means.
- Use full-word names (`processes`, `snapshot`, `radius`), not abbreviations.
- Tests live in a `tests` module at the bottom of each file. Name them as sentences about
  behaviour (`city_does_not_relocate_when_resources_or_population_change`). Rendering tests use
  `model::demo` or the `process(pid, parent)` helper in `render.rs`.
- When you add a field to `Process` or `Snapshot`, update `parse_stat` (in
  `platform/linux/mod.rs`), `Sampler::process` (in `platform/macos/mod.rs`), `model::demo`
  and the `render.rs` test helper.
- A new view needs: a `View` variant, a `next()` entry, the status-line name and legend in
  `App::text`, and a draw function called from `Scene::render`.
- Prefer the standard library. Add a dependency only when it replaces substantial,
  error-prone code.

## Shell notes

- The interactive shell may be zsh, which does not word-split unquoted variables. Use
  `bash -c` for loops that rely on splitting.

## Git workflow

- `main` only changes through pull requests. Do each piece of work on a branch named
  `feat/...`, `fix/...` or `docs/...` cut from an up-to-date `main`, and open a PR with `gh`.
- Run the gates before pushing a branch. The PR description says what changed and how it was
  checked, including screenshots for visual changes.
- Commits are attributed to the human author only, with no AI `Co-authored-by` trailers.
