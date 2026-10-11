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
   every `model::BACKGROUND_PERIOD` (2 s) and are merged in by `Identity`, against the pids the
   cycle scanned, so the frame loop never waits on them and a reused pid inherits nothing.
   `Snapshot.sampler` reports whether that thread is alive and how long its scans took.
   Optional sources, the ones only some views draw, are read only while the shown view declares
   them: `main.rs` hands `Collector::demand` the `sources` of the shown view's registry row at
   startup and on every view change, the thread takes a copy of that `Demand` at the start of
   each cycle, and reads an optional source through `Demand::read_if` (or `Reads::read`, which
   also records it as active or, when it cannot be read, missing). Remote locations
   (`Source::Locations`, the GeoIP lookups the globe draws) go through it, and the journal
   follower in `main.rs` runs while the demand contains `Source::Journal` and for
   `JOURNAL_LINGER` (30 s) after; no view is special-cased. `Snapshot.sources` lists the
   optional sources read for the sample, and `Snapshot.missing` those demanded and unreadable;
   one nobody demanded is in neither. `--demo` starts no thread, so `sources` is empty there. On Linux
   the sampler caches each process's cmdline and cgroup per `Identity` and re-reads them after an
   exec or every few samples, so the frame loop reads only `stat` and `io` per process.
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
| `main.rs` | CLI (`Options`), `Layout`, `App` (input, camera easing, tour, overlay text, when the journal follower runs), `preflight` (`--verbose`, `--list-views` and views this platform lacks, before the terminal is touched), main loop, `demo_snapshot` (every demo sample goes through it with the shown view's `DemoOptions`), headless PNG and benchmark, and `warm_up`, which runs the shown view's warm-up phases before a headless frame (the golden render test calls it too) |
| `model.rs` | `Process`, `Snapshot`, `Collector` (CPU smoothing, per-interval rates and shares from counter deltas, background thread, merge), `Demand` and `Reads` (which optional sources the background thread reads), `top_n` (the budgeted selection: largest key first, ties by `Identity`, NaN last), demo workload (`demo_with` with the `DemoOptions` a view's row asks for: `churn` for deterministic births and exits, which `demo_life` lists, and `exact_cpu_time`; `demo` is the unoptioned test shorthand), `describe` |
| `platform/mod.rs` | The OS contract (`Sampler`, `RawProcess`, `network`, `account`, `FileScan`, `locks`, `Gpu`, `journal`), the `Source` registry and `Sources` bit set that `Snapshot::missing` holds, and the platform-neutral `Network`, `Files` and `Locks`; selects the implementation by `target_os` |
| `platform/linux/mod.rs` | Linux `Sampler`: `/proc` parsing, CPUs, pressure, core kinds, cgroup v2 accounting, and the gaps it reports through `missing` (cpufreq, schedstat, core kinds, cgroup v2) |
| `platform/linux/net.rs` | Socket links: `/proc/net/tcp*`, sock_diag netlink (Unix peers, inet TCP with `tcp_info`) |
| `platform/linux/files.rs` | Open regular files and deleted-but-open files per process (`/proc/<pid>/fd`, bounded per scan) and file locks with their waiters (`/proc/locks`) |
| `platform/linux/nvml.rs` | NVIDIA per-process GPU memory via `dlopen`; never wakes a runtime-suspended GPU |
| `platform/linux/journal.rs` | `journalctl` in export format and its parser |
| `journal.rs` | Journal lines for the Matrix view: reader thread, bounded backlog, demo lines |
| `views.rs` | The view registry: `REGISTRY`, one `ViewInfo` row per view in Tab order; `View`, an index into it; `Hooks`, `BuiltIn`, the `ViewModule` trait, `ViewKey`, `Warmup`, `LinkRule` and the harness `Contract` (`Property`, `Unstable`); the built-in views' legends and record hooks; and `testing`, module views for tests |
| `harness.rs` | Test only: the shared view harness. `Session` runs a view headless through `warm_up` and `App::render`; `rescaled` and `churned` perturb a demo sample; `check` tests every `Property` of a row against its `Contract`, and `every_view_satisfies_the_view_contract` runs it for every registered view |
| `render.rs` | `Camera`, `Sky`, `Item`, `Frame` (CPU rasterizer and picking), `RenderInput`, `Stage` with its `dt` policy and `TourTarget`, and `Scene`, which draws its registry rows: module views through `ViewModule`, and the built-in views in its own arms (City, Orbit, Ripple and Flow as methods, Matrix by hand, and the views below through `Stage`) |
| `render/shared.rs` | Constants the CPU rasterizer and `shaders.wgsl` must agree on, declared once; `prelude` writes them into the WGSL source |
| `cores.rs`, `cells.rs`, `strata.rs`, `globe.rs`, `reef.rs` | Module views, each a state struct with `draw(&mut Stage, processes, snapshot) -> usize`: the CPU core track, cgroup petri dishes, a minute of CPU history as ridges, TCP connections on a turning earth, and the coral reef |
| `matrix.rs`, `glyphs.rs` | The Matrix view (`Rain`, drawn from journal lines, not processes) and its 7x14 bitmap font |
| `geo.rs`, `places.rs`, `coastline.bin` | Local GeoIP lookup and home from the time zone (`geo.rs`), country label points and legacy GeoIP country order (`places.rs`), and the packed coastlines the globe draws |
| `coop.rs` | The Coop module view: Vicsek flocks per cgroup, henhouses, nests (eggs for open files, brooding and queueing for file locks), feeders, chicks, dust and foxes, stepped at a fixed 20 Hz |
| `simulation.rs` | Shared pure simulation helpers: `SpatialHash` (2D) and `SpatialHash3` (3D, with exact k-nearest) neighbour queries, the seeded `Rng` (uniform and `normal`), `Complex` with `dft`, `fft` and `inverse_fft`, the `FixedStep` clock, hashes, and the `growth`, `sprout` and `smoothing` easing |
| `status.rs` | The legend wording for optional sources, so every view says it alike: `missing_note` (` \| no <source>: <consequence>`) and `sources_note` (` \| sources: a, b`) |
| `stellar.rs` | Stellar photometry for views that draw processes as stars: `magnitude` (from memory, `-2.5 log10(memory / 1 GiB)`, floored at 1 MiB), `spread` (core and halo size and strength from a magnitude and the frame height), `Frame::star` (one pickable pixel-sized sphere and at most one glow, existing items only), and the nova, fade and supernova timelines (`Flash`, `exit_flash`, `flash`), which depend only on age |
| `palette.rs` | Every process-kind, process-state and core-kind colour, `mix`, `tint`, `kind_color`, `core_color` and `core_name` |
| `geometry.rs` | `dot`, `normalize`, `normalize_or_zero` and the shaded `cuboid` |
| `picker.rs` | The view picker opened by `v` and the Tab playlist: `Picker` (filter, highlight, popup lines), the name and title filter, `playlist` (the `--views` rules) and the `cycle` and `toggle` rules; pure, so the tests drive it without a terminal |
| `layout.rs` | Stable placement: `Seats`, `Discs`, the golden-angle `spiral`, `vacant` and `spin` |
| `medium.rs` | Pure simulation state for the Ripple (wave equation) and Flow (particles) views |
| `gpu.rs`, `shaders.wgsl` | wgpu backend that mirrors the CPU rasterizer |
| `terminal.rs` | Graphics-capability probe, frame transfer, text overlay, terminal restoration |
| `platform/macos/mod.rs` | macOS `Sampler` (libproc, Mach, sysctl, IORegistry, `proc_pid_rusage` V6 with fallbacks), the cluster clocks, the socket scan, the bounded open-file scan (`FileScan`, one `proc_pidfdinfo` per vnode), and what it reports through `missing`, `per_cluster` and `unreadable` |
| `platform/macos/ffi.rs` | Every extern declaration and `#[repr(C)]` struct that `libc` lacks, with size assertions |
| `platform/macos/logic.rs` | Pure macOS decisions with no FFI: kinds, groups, parents, the Mach scheduling policy, `KERN_PROCARGS2` parsing, tick conversion, cluster clocks from cycle counters, socket pairing, open and deleted-but-open files from `vinfo_stat` |
| `platform/macos/journal.rs` | The unified-log follower (`log show`, then `log stream`) and its std-only JSON line parser |

`agents_file_table_lists_every_source_file` fails when a `.rs` file under `src/` has no row. Name
files by path relative to `src/` in the first column; a row may list several.

## Architecture

Dependencies mostly run one way: `platform` and `model` (the data), then the shared helpers, then
`render` with `views` and the view modules, then `gpu` and `terminal`, then `main`. Check with
`grep -n 'use crate::' src/*.rs` before adding an import. Nothing imports `main.rs`, `model` and
`platform` never import `render`, a view or `terminal`, and `gpu` and `terminal` import only
`render`. The exceptions, so don't assume otherwise:

- `model` and `platform` import each other: the platform builds `model` types and `model` calls the
  `Sampler`. `journal` and `platform/*/journal.rs` do the same.
- `render` imports each module view, and each view imports `render` for `Frame`, `Stage` and
  `Color`. `render` and `views` import each other: `Scene` draws the rows, and the rows name
  `Scene`, `Stage` and `Backdrop` and call the built-in views' legends. `palette`, `geometry` and `medium` import `render` too, for `Color`, `Point` and
  `Frame`. `simulation` and `layout` import only `model::Identity`.
- The test-only `harness` imports `main` for `App`, `Options`, `demo_snapshot` and `warm_up`, so
  it runs a view exactly as a headless `--output` run does.
- `render` and the views read `platform` types (`Shadow`, `Files`, `DeletedFile`, and `Source`
  and `Sources` from `Snapshot::missing`), and Cells and the coop its `GROUP_LABEL` and
  `WITHOUT_CGROUPS`.

**The view contract.** A view is one module plus one row of `views::REGISTRY`. The module owns
a state struct and implements `views::ViewModule`:
`draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize` and
`legend`, and where it needs them `record` (every sample, whichever view is shown), `key` and
`inspector_line`. A view reads only `Stage`, the process slice and `&Snapshot`, and talks back
only through `Stage` outputs. `Stage` carries in the frame, camera, selection, time, `dt`, births
and growth; what the platform could not measure is `snapshot.missing`, which `Stage` does not
copy. `stage.dt` is the one step policy for new code, computed once per frame by `Scene`: 0 on
the first frame, while paused and when time goes back (a rewind), otherwise the time since the
last frame that moved forward, at most `render::MAX_DT` (0.5 s). Step a simulation with it or
with a fixed-step clock fed from `stage.time`; the older views keep their own clamps. A view
writes back `stage.positions` (picking anchors, labels, the tour and the exit flash all read it),
`stage.bounds` (a fixed envelope for the camera fit when the content moves by itself),
`stage.places`, `stage.notes` (popup lines after the sockets), `stage.tour` (`TourTarget { id,
position, weight }`, which the tour visits heaviest first) and `stage.popup` (popup lines right
after the process's own figures), and returns how many processes it showed; the rest count as
collapsed. `Scene::render` takes one `RenderInput` (snapshot, view, camera, size, selection,
time, limit, focus and whether the app is paused). Orbit, Ripple and Flow still report through
`scene.stars`, `scene.mass` and `scene.hub_names`, which `main.rs` reads besides the outputs
until they move. The row (`ViewInfo`) holds everything else that sets the view apart, and `Scene`,
`App` and the headless warm-up read it instead of naming the view. The eleven older views are
built in (`Hooks::BuiltIn`): `Scene` keeps their state in typed fields and draws them in its own
arms, City, Orbit, Ripple and Flow as `Scene` methods and Matrix by hand. They are being moved
onto `ViewModule`, so write new views the module way and never add an arm.

**Shared helpers live in one place each.** Don't write a second copy:
- `simulation`: `SpatialHash` and `SpatialHash3`, `Rng` (seeded per identity, `normal` for a
  Gaussian), `XorShift32`, `Complex` with `dft`, `fft` and `inverse_fft` (forward sign
  `exp(-2*pi*i*k*n/N)`), `FixedStep` (a fixed-step clock: `advance(time, tick)` runs whole steps
  and returns the interpolation fraction; step simulations with it, never copy `Coop::advance`),
  `simulation::hash` (`fnv1a_64_bytes` and the rest), `hash_identity`, and
  the `growth`, `sprout` and `smoothing` easing.
- `palette`: every colour table, `mix`, `tint`, `kind_color`. `geometry`: vector maths and
  `cuboid`. `layout`: `Seats`, `Discs`, the golden-angle `spiral`.
- `status`: `missing_note` and `sources_note`, the legend wording for missing and active optional
  sources. `model::top_n` picks the processes a budgeted view draws.
- `render::shared`: constants the CPU rasterizer and the WGSL shader share.
- `model::testing::process` builds a `Process` in tests.

## Invariants

- **The CPU and GPU backends must draw the same frame.** If you change how anything is
  rasterized (sky, sphere shading, glow falloff, depth mapping), change `render.rs` and
  `shaders.wgsl` together. Shared constants live in `render::shared`; never write them in WGSL
  directly. To check, render the same view with `--renderer cpu` and
  `--renderer gpu` and compare the PNGs with `scripts/compare_png.py` against the tolerance in
  `docs/VIEW_CONVENTIONS.md` (`CPU/GPU parity`).
- **Layouts are stable.** A process keeps its place when CPU, memory or the population changes.
  Systems and districts are only re-placed when their structure outgrows the reservation, and
  tests enforce this: the view harness checks every registered view (`harness.rs`). Don't
  introduce a layout that reshuffles every sample.
- **Picking.** A pickable item carries an index into `frame.identities`. `NONE` means "not
  pickable", and lines clear picking where they draw. Mouse input is cell-coarse, so
  `pick_near` searches a radius.
- **Process and system sampling lives in `platform/`.** Nothing outside it reads `/proc` or
  `/sys` or spawns OS tools; terminal I/O (`terminal.rs`) and local lookups (`geo.rs`) are the
  exceptions. Both platforms provide the same names; a macOS build must compile without
  warnings (`cargo clippy --target aarch64-apple-darwin`).
- **Unknown is not zero.** A reading that was not taken (the first sample of a rate, a counter
  that went backwards, an unreadable file) is `None` in the model, not 0. A view converts it at
  its own boundary (`unwrap_or(0.0)`) only to draw what it drew before.
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
python3 scripts/code_report.py --check
```

The toolchain is pinned in `rust-toolchain.toml`; to bump it, change that file and `rust-version` in `Cargo.toml` together in a dedicated PR.

macOS code cannot run on Linux. Type-check it locally with
`cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings` (give it its own
`CARGO_TARGET_DIR`); CI runs the gates and the live renders on `macos-15`. The pure logic in
`platform/macos/logic.rs` and `platform/macos/journal.rs` is also compiled into Linux test
builds, so `cargo test` covers it here. Keep FFI out of those two files so that stays true.

`every_view_matches_its_golden_hash` checks every view's CPU render against `tests/golden.txt`
on Linux x86_64; `ISOTOP_BLESS=1 cargo test every_view_matches` rewrites the file. A PR that
changes a golden hash says why in its description and carries the renders. On other platforms,
bless a local baseline before a change with `ISOTOP_GOLDEN_FILE=<path> ISOTOP_BLESS=1 cargo test
every_view_matches`, then run it again without `ISOTOP_BLESS` after the change to compare.

The smoke test drives the release binary through a pseudo-terminal. It runs once in demo mode
over shared memory and once in live mode inline, and exercises view switching, search, focus,
pause, quit and terminal restoration.

## Metrics

`docs/METRICS.md` and the "By the numbers" block in the README (between `<!-- metrics:start -->`
and `<!-- metrics:end -->`) are generated by `scripts/code_report.py` (python3 standard library
plus the `tokei` tool, never a crate dependency; install with `brew install tokei`, `paru -S tokei`
or `cargo install tokei --locked`). The report holds tokei's per-language table, Rust code lines
per file split into production and test, the ten largest files (marked over 1,200 lines), debt
counters (`#[expect(clippy::too_many_lines`/`too_many_arguments`, `unsafe` blocks, `unwrap_or(0`
and `unwrap_or(0.0` sites, `known_unstable` entries) and health counters (tests, views, README
mapping tables, dependencies). The output is deterministic and records the tokei version, which CI
pins.

A PR that changes code runs `python3 scripts/code_report.py` and commits the result; on a conflict,
regenerate. `--check` is the last gate and fails CI when either file is stale; `--self-test` runs
the script's fixture tests without tokei. Debt counters should only go down: a PR that raises one
says why.

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

## Planned work

`docs/ROADMAP.md` lists the planned views and the prerequisites and building blocks they rely on. Each item is one GitHub issue. `docs/VIEW_CONVENTIONS.md` holds the rules every new view and building block follows. Read both before taking an item.

## Code conventions

- Comments explain math, protocol values or rules that aren't obvious, never what the next
  line does. Doc comments state what a type or function means.
- Use full-word names (`processes`, `snapshot`, `radius`), not abbreviations.
- Tests live in a `tests` module at the bottom of each file. Name them as sentences about
  behaviour (`city_does_not_relocate_when_resources_or_population_change`). Rendering tests use
  `model::demo` or the shared `process(pid, parent)` helper in `model::testing`.
- When you add a field to `Process`, decide it in the four exhaustive literals (no
  `..Default::default()`, so the compiler insists): `parse_stat` in `platform/linux/mod.rs`,
  `Sampler::process` in `platform/macos/mod.rs`, `model::demo_with` and `Transient::process`
  (the demo's churn, `model.rs`). Also set it in `Default for
  Process` in `model.rs`, which `Process::placeholder`, `hub()` and the `model::testing::process`
  test helper build on. Update `RawProcess` in `platform/mod.rs` if the platform supplies it. A
  rate also needs `Counters` and `measure()`; a field filled on the background thread needs
  `Extras`, `background()` and the merge in `Collector::sample` (all in `model.rs`). `Snapshot`
  derives `Default`; set a new `Snapshot` field in `Collector::sample` and `model::demo_with`.
- A new view is one module plus one registry row (`docs/VIEW_CONVENTIONS.md` has the rules):
  - `src/<name>.rs` with a `#[derive(Default)]` state struct that implements `ViewModule`, a
    `mod` line in `main.rs` and a row in the file table above.
  - One `ViewInfo` at the end of `REGISTRY` in `views.rs`, built with
    `ViewInfo::new(name, title, backdrop, Hooks::Module(...))`, setting only the fields that
    differ from these defaults: `available` (everywhere; a runtime probe lives in `platform/`),
    `warmup` (none; a new view's phases run under `When::Headless`), `links` (`Quiet`; or `All`,
    or `DrawnByView`), `flash` and `haze` (on), `pitch` (None: leave the camera's), `tour` (on),
    `header` (None: the scene's visible and collapsed counts), `face` (None), `turn` (None: the
    wheel pans), `sources` (none), `demo` (`DemoOptions::default()`) and `contract`
    (`Contract::DEFAULT`: it draws processes, which may move at most 1 world unit over the
    ground, checked where they are drawn, and it is known to fail nothing). Tab, Shift+Tab, the
    `v` picker, `--view`, `--views`, `--list-views` and `--verbose` follow from the table. Never
    reorder the first ten rows: the digits pick them.
  - View-local keys reach `ViewModule::key` as a `ViewKey`: `Local` for `b i j k m o p u x y z ,
    . ;` only, `Reset` for Home and `r` (the global reset happens as well) and `Escape`
    (returning true consumes that press). Every other key is global, and no key reaches the view
    while a search is open.
  - The view harness (`harness.rs`) covers every row: `every_view_satisfies_the_view_contract`
    checks that two runs from the same inputs draw the same pixels and picks, that the demo
    shows processes with positions and something to click, that every pick names a process,
    that no surviving process moves when CPU and memory change (CPU scaled 0.25 to 4, memory 0.5
    to 2) or when a tenth of the processes exit and as many start, and that an empty machine
    draws. A new view needs tests only for what is its own beyond that. Set its tolerance on
    the row's `contract`; a view whose creatures wander declares `anchors` (Coop's seats). Never
    add a `known_unstable` entry for a new view: those record debt of the views that predate the
    harness, each with its issue, and the harness reports one that starts to pass.
  - Test the module through `Scene::with_rows` and a table of its own, as `views::testing` does.
  - Outside `src/`: the README section, key table and view lists, and a line in
    `tests/golden.txt` (bless it and say so in the PR). CI renders every view `--list-views`
    prints, and the golden test renders every row.
- Prefer the standard library. Add a dependency only when it replaces substantial,
  error-prone code.
- Suppress lints with `#[expect(lint, reason = "...")]` only, never `#[allow]`.

## Shell notes

- The interactive shell may be zsh, which does not word-split unquoted variables. Use
  `bash -c` for loops that rely on splitting.

## Git workflow

- `main` only changes through pull requests. Do each piece of work on a branch cut from an
  up-to-date `main` and open a PR with `gh`. The prefix says what the work is: `feat/...` for new
  behaviour, a view or tooling; `fix/...` for a bug; `refactor/...` for restructuring that changes
  no behaviour (the golden hashes stay as they are); `docs/...` for documentation and templates
  only.
- A PR stacked on another feature branch does not close its issues: GitHub honours `Closes #N`
  only when the PR merges into the default branch. List them as `Part of #N` in the body and
  close them when the stack reaches `main`, or retarget the PR to `main` before merging and use
  `Closes #N` then.
- Run the gates before pushing a branch. The PR description says what changed and how it was
  checked, including screenshots for visual changes.
- Commits are attributed to the human author only, with no AI `Co-authored-by` trailers.
