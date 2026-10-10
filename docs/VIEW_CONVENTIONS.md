# View conventions

These rules apply to every view and building block in `docs/briefs/views-2026-10.md`. Read `AGENTS.md` first. Its pipeline, invariants, gates, code conventions and git workflow all apply and are not repeated here.

Precedence:

- Where this document and an issue disagree, the issue wins for that item.
- Where the brief and a mockup disagree on a mapping, the brief wins.
- Where the brief and this document disagree on process (approval, filing issues, platform handling), this document wins.
- The brief's "Prerequisites from the earlier brief" section refers to a brief that is not in this repository. The issues replace it. Where the brief says "as that brief specifies", read the issue.

Work items are referred to by ID: P1 to P5 (prerequisites), S1 to S6 (shared prerequisites), B1 to B12 (building blocks) and one issue per view. [ROADMAP.md](ROADMAP.md) maps every ID to its GitHub issue, and each issue title starts with its ID. A name marked "from <ID>" does not exist until that issue is merged. Check `main` before relying on it. Everything else named here exists on `main` today.

## The realness contract

- Every visual element maps to a measured quantity. Anything that does not map is listed under **Decorative:** in the view's README section.
- `--demo` synthesizes every field the view consumes. `--time T` makes a frame repeatable byte for byte.
- When a source is unreadable (permissions, kernel version, hardware, platform), the view degrades visibly and the status strip names what is missing and what is used instead. Live mode never fills a gap with invented numbers.
- Simulations advance on wall-clock time, never per frame. All randomness is seeded from process identity and `--time`.
- isotop never escalates privilege. Sources that need CAP_NET_ADMIN or root are optional. The README documents `sudo setcap cap_net_admin+ep ./target/release/isotop` once, as the user's choice, and lists what it adds. The status strip names the active optional sources.
- No network access. No new dependencies unless one replaces substantial, error-prone code. If you add one, justify it under `## Decisions` in the PR. Simulations, transforms, fonts and colour tables stay in-tree.
- Existing views do not change behaviour or appearance. Any PR that touches shared code proves this; see "Demo synthesis".

## Adding a view

### Today (before [P1 (#17)](https://github.com/tfpickard/isotop/issues/17))

`AGENTS.md` lists what a view needs:

- a `View` variant and its `ALL`, `backdrop()` and `next()` entries in `render.rs`;
- a `Scene` field and a dispatch arm in `Scene::render`;
- the title and legend arms in `App::text` in `main.rs`;
- any headless warm-up in `main.rs`.

No new view is built this way. Every view issue depends on [P1 (#17)](https://github.com/tfpickard/isotop/issues/17).

### After [P1 (#17)](https://github.com/tfpickard/isotop/issues/17)

A view is one module plus one registry row. Do not add arms to `Scene::render`, `App::text`, the headless warm-ups or `App::command`. [P1 (#17)](https://github.com/tfpickard/isotop/issues/17) also rewrites the paragraph in `AGENTS.md`.

1. Create `src/<name>.rs` with a `#[derive(Default)]` state struct. Keep pure simulation state free of drawing, either in that file or in a pure helper module, as `medium.rs` does. Add a row for each new module to the file table in `AGENTS.md`.
2. Add one registry row (from [P1 (#17)](https://github.com/tfpickard/isotop/issues/17)). Set only the fields you need; the defaults reproduce existing behaviour. The row carries:
   - name (lowercase CLI name) and title (status-line text, e.g. `COOP`)
   - backdrop
   - `available()`
   - legend
   - per-sample record hook
   - headless warm-up (see "Demo synthesis")
   - links rule: all, quiet, or drawn by the view
   - generic birth and death flash, on or off
   - pressure haze, on or off
   - default pitch
   - tour participation
   - header-count override
   - view-local key hook
   - card hook (from [B9 (#36)](https://github.com/tfpickard/isotop/issues/36))
   - `sources` (from [S6 (#27)](https://github.com/tfpickard/isotop/issues/27))
   - `demo` options (from [S4 (#25)](https://github.com/tfpickard/isotop/issues/25))
3. Draw with the signature the module views already use (`cells.rs`, `cores.rs`, `strata.rs`, `reef.rs`, `coop.rs`):
   `draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize`.
   The return value is the number of processes shown; the rest count as collapsed.
4. Fill `stage.positions` for every drawn process. Picking anchors, the tour and the generic exit flash depend on it. If the content moves on its own, fill `stage.bounds` with a fixed envelope so the camera fit does not follow the simulation.
5. Add a smoke-test line, a README section and the key help. See "Tests", "README section" and "Keys".

`isotop --list-views` (from [P1 (#17)](https://github.com/tfpickard/isotop/issues/17)) prints the views available on the running platform. [S1 (#22)](https://github.com/tfpickard/isotop/issues/22) and the smoke test use it.

## Module shape

- Keep state that spans samples in the view's struct, keyed by `Identity` (pid plus start), never by pid alone. Prune identities that are no longer alive.
- History that must already exist when the view is first opened is recorded on every sample in every view, through the record hook. Use [B1 (#28)](https://github.com/tfpickard/isotop/issues/28)'s `History` for per-process CPU and memory rather than a ring of your own.
- Deduplicate records on `snapshot.elapsed > last`. Draw the window that ends at `snapshot.elapsed`, so pause and rewind (`[` `]`) work.
- Express time windows in seconds of `snapshot.elapsed`, not as sample counts. `--sample-ms` ranges from 100 to 10000.
- Recompute expensive derived data (fits, routes, ownership grids, shadows) once per new sample, or spread it over several frames. Nothing O(n²) in processes runs per frame.
- Put large new per-process detail in `Snapshot` behind `Arc`. The history ring (`--history`, 120 samples by default) clones snapshots.
- Anything slower than a `/proc/<pid>/stat` read runs off the render thread. By default it runs on the Collector's background thread (`isotop-sampler` in `model.rs`), which calls into `platform/`. An issue may place it on an additional named thread instead (for example `isotop-events` in [B3 (#30)](https://github.com/tfpickard/isotop/issues/30)); that thread's OS code lives in `platform/<os>/` and hands results to the Collector. A view never reads the OS.

## Simulation time

- Use `simulation::FixedStep` (from [S2 (#23)](https://github.com/tfpickard/isotop/issues/23)). It has a fixed step H, a cap on steps per frame, a maximum gap, and returns an interpolation alpha. Coop uses `FixedStep::new(0.05, 10, 0.5)`, matching its `H` and `MAX_STEPS` today. Choose H per view and state it in the README.
- When time does not advance (pause, rewind), the simulation does not step.
- Compute closed-form animation (trails, chains, orbits, swings) from state and time, on a time grid aligned to fixed multiples. Do not accumulate it per frame.
- Give speeds, delays and radii in world units, never screen pixels, so zoom does not change the dynamics.
- Simulation cost must not grow without bound with `--processes`. Cap populations (agents, particles, bubbles, bees), state the caps in the README, and count anything hidden by a cap in the legend.

## Seeded randomness

- Per-process randomness comes from `simulation::Rng::for_identity(id, SALT)`, with one `SALT` constant per view.
- Demo-wide randomness is a pure function of `--time` and indices.
- Stateless per-frame variation (flicker, jitter) is a hash of a stable id and an event index, never a per-frame RNG advance.
- Gaussian draws use `Rng::normal` (from [P4 (#20)](https://github.com/tfpickard/isotop/issues/20)).

## Demo synthesis

- `model::demo(time, count)` must stay byte-identical. A view that needs births and exits, or an exact CPU-time integral, sets `DemoOptions { churn, exact_cpu_time }` on its registry row (from [S4 (#25)](https://github.com/tfpickard/isotop/issues/25)). Use `demo_with`, `demo_life` and `demo_cpu_seconds`, all from [S4 (#25)](https://github.com/tfpickard/isotop/issues/25). Other views' demo frames do not change.
- Demo data for a new field goes into that new field. Never alter a field that existing views draw.
- A new field on `Process`, `Snapshot` or `RawProcess` follows the rule in `AGENTS.md`: update `parse_stat` (`platform/linux/mod.rs`), `Sampler::process` (`platform/macos/mod.rs`), `model::demo` and the `render.rs` test helper; a `RawProcess` field also passes through `measure` in `model.rs`. **Decision:** in `model::demo` the field takes its empty value (`Default`, an empty collection or `None`), so existing frames stay byte-identical. Synthesized values for it come only through `demo_with` (from [S4 (#25)](https://github.com/tfpickard/isotop/issues/25)) when a registry row asks for them. A building block that needs this adds a `DemoOptions` field that defaults to off.
- Calendar-dependent visuals use `SystemTime::now()` in live mode. In demo mode they use the fixed epoch 2026-10-10T04:12:00Z plus `--time`.
- Headless warm-ups:
  - **Existing views.** The warm-ups in `main.rs` today (strata, coop, ripple, flow, reef, cores, matrix) differ in kind: some rasterize frames, some only record samples, and some run only under `--demo` with `--output`. [P1 (#17)](https://github.com/tfpickard/isotop/issues/17) moves each one to its registry row unchanged, with the same phases, step counts, rasterizing and `--demo`/`--output`/`--benchmark` conditions. Their PNGs and benchmark behaviour stay as they are.
  - **New views.** A view that needs a settled frame declares a warm-up on its registry row. It runs before both `--output` and `--benchmark`, in live and demo runs, so the benchmark measures a settled scene. It replays N seconds of demo samples through the record hook, steps the simulation without rasterizing, or both. It rasterizes frames only if the view builds state inside `draw`, and the PR says why under `## Decisions`.
  - Two runs with the same `--time` produce byte-identical PNGs.
- The default reference time is `--time 12`. A view whose history window is not full at 12 s states a later time in its issue (for example `--time 300`) and uses it for parity, the mockup comparison and the README image.
- Prove that existing views are unchanged after any change to shared code. Render every existing view with `--demo --time 12 --renderer cpu --width 960 --output` before and after, compare the files with `cmp`, and paste the result.

## Degradation and the status strip

- A source the platform lacks adds a short lowercase noun to `Snapshot.missing`, for example "socket traffic", "interrupts", "oom score", "wait channel" or "temperature". A process that refuses a read is `Measured::Unreadable`. A read not reached yet is `Measured::Pending`. Never substitute zero.
- Active optional sources are listed in `Snapshot.sources` (from [S6 (#27)](https://github.com/tfpickard/isotop/issues/27)), for example "proc connector" or "sample diff".
- Optional background sources run only while a shown view declares them in `sources` (from [S6 (#27)](https://github.com/tfpickard/isotop/issues/27)). Budgeted per-process selections use `top_n` (from [S6 (#27)](https://github.com/tfpickard/isotop/issues/27)).
- The legend line is the third status line. It starts with a space, like the existing legends, and is built in this order:
  1. Dynamic parts first (counts, names), so they survive 80 columns.
  2. Degradation notes: ` | no <source>: <what is used instead>`, from `status::missing_note` (from [S6 (#27)](https://github.com/tfpickard/isotop/issues/27)).
  3. ` | sources: a, b` where relevant, from `status::sources_note` (from [S6 (#27)](https://github.com/tfpickard/isotop/issues/27)).
  4. The static mappings in `X = Y` form, separated by ` | `.

  Example: ` 12 retrograde | no socket traffic: corridors from socket counts | brightness = memory | speed = CPU`.
- Platform work a view needs that no building block covers (for example a `wchan` read) lives in `platform/<os>/`, runs off the render thread as described in "Module shape", and is listed in the view's PR as platform work.

## Platform availability

- Aim for reasonable parity on macOS. Identical visuals are not required. Prefer visual quality and fidelity to the mockup on each platform.
- If a view has no feasible data source on a platform without prohibitively heavy work, its `available()` returns false there. The view still compiles on that platform.
- An unavailable view is skipped by Tab, Shift+Tab, the `v` picker and `--views`. A headless `--view <name>` for an unavailable view exits 1 with `isotop: view <name> is not supported on this platform`. An interactive run starts on the first available view instead.
- These rules apply to every view, existing ones included. [P1 (#17)](https://github.com/tfpickard/isotop/issues/17) implements them once, in the registry. **Decision:** every existing view's `available()` returns true on Linux and macOS, so no existing run changes.
- With `--verbose` (from [P1 (#17)](https://github.com/tfpickard/isotop/issues/17)), isotop prints `skipping view <name>: not supported on this platform` to stderr once per unavailable view at startup, before it enters the alternate screen. Without the flag it prints nothing.
- Never draw a "not supported" screen.
- If a view is available but some of its sources are missing, it degrades as described in "Degradation and the status strip".
- Every PR runs `cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings`, with its own `CARGO_TARGET_DIR`.
- Mark claims about macOS APIs that the tree does not use yet as "to verify" in the PR. They are checked on the `macos-15` CI runner. If a claim fails, the feature degrades through `Snapshot.missing` and does not block the PR.
- Unavailable on macOS today: switchboard. die is available on macOS: it shows relative heat, and [B12 (#39)](https://github.com/tfpickard/isotop/issues/39) reports "temperature", "throttle counts" and "package power" as missing.

## README section

Add a `### <Name>` section under `## Worlds`, modelled on `### Coop`:

1. An image: a CPU render of `--demo --time <T>` in `docs/media/<name>.webp`, like the existing sections. Use `.png` if no WebP encoder is available.
2. One plain sentence saying what the view shows. Describe what it shows. Do not characterise it.
3. The mapping table:

   ```
   | Visual element | Measured source | Transform |
   | --- | --- | --- |
   | Star brightness | Memory (RSS; the physical footprint on macOS) | m = -2.5 log10(memory / 1 GiB), floored at 1 MiB |
   ```

   The transform states the log, sqrt, smoothing constant, quantum, clamp or cap.
4. A **Decorative:** paragraph listing every element that is not a measurement.
5. Bold-lead paragraphs for the model: the simulation and its step H, the population caps, **What cannot be measured.**, and one sentence per platform difference.
6. A row in `### What macOS does not report and how each view shows it` for each source that is degraded on macOS. If the view is unavailable on macOS, add one row saying so.
7. View-local keys go in the section and in `## Controls`. Per-view flags follow `--<view>-<option>` and are listed in `## Run`.

## Picking

- Process items carry an index into `frame.identities`. `NONE` means not pickable. Only triangles and spheres write picks; lines clear picking where they draw. Decorative strokes near bodies are therefore `Beam`s, or sit behind the body in depth.
- Non-process things (IRQ lines, files, graves, core blocks) use `Thing { kind, key }` and `Frame::thing_pick` (from [B9 (#36)](https://github.com/tfpickard/isotop/issues/36)). The key stays the same across frames, and the card is built on demand through the card hook. Never mint a synthetic `Identity`.
- Translucent or splatted content stays pickable through [B9 (#36)](https://github.com/tfpickard/isotop/issues/36)'s pick-only item.
- Grid content picks through the [P3 (#19)](https://github.com/tfpickard/isotop/issues/19) owner grid. An absent owner grid gives `NONE`.
- On the GPU, picks are read back only in a 160 px window around the pointer (`PICK_WINDOW` in `gpu.rs`). Any test that picks away from the pointer rasterizes on the CPU.

## Untrusted text

- Every string from the system is stored raw in the model: process names, paths, IRQ and device names, thread names, taskstats `ac_comm`, journal fields, cgroup names.
- That text reaches the terminal only through `terminal::clean_text`. Scene labels also go through `render::label_name`.
- Text drawn into the frame (`Item::Glyph`, and [B10 (#37)](https://github.com/tfpickard/isotop/issues/37) stroke text) passes through `clean_text` before layout.
- Kernel strings that are only compared (such as `wchan`) are mapped to enums and never displayed raw.
- Do not widen `clean_text`; `process_text_cannot_inject_terminal_controls` guards it. Generated non-ASCII text, such as braille sparklines, uses a separate function that accepts only its own generated code points.

## Rendering helpers

- Prefer baking shading into per-face or per-cell colours on the CPU before recording, over adding rasterizer behaviour.
- Thick, dashed or tubular strokes use `Frame::ribbon` and `Frame::tube` (from [S3 (#24)](https://github.com/tfpickard/isotop/issues/24)). They emit only triangles and share the `LIGHT` constant with [P3 (#19)](https://github.com/tfpickard/isotop/issues/19) relief.
- Grid simulations draw through the field item (from [P3 (#19)](https://github.com/tfpickard/isotop/issues/19)). One triangle pair per cell is not allowed.
- Screen-space items keep their depth inside the scene's depth range, because the GPU normalizes depth per frame (`normalized` in `shaders.wgsl`).
- If you change how anything is rasterized, change `render.rs` and `shaders.wgsl` together.

## CPU/GPU parity

Render the same frame twice and compare the two files:

```sh
./target/release/isotop --demo --view <name> --time <T> --width 960 --renderer cpu --output cpu.png
./target/release/isotop --demo --view <name> --time <T> --width 960 --renderer gpu --output gpu.png
python3 scripts/compare_png.py cpu.png gpu.png      # from [S1 (#22)](https://github.com/tfpickard/isotop/issues/22)
```

**Tolerance.** [S1 (#22)](https://github.com/tfpickard/isotop/issues/22) records the tolerance in this section. Two limits apply: the per-channel mean absolute difference, and the share of pixels with any channel more than 2 levels off.

| Limit | Mean | Share | Views |
|---|---|---|---|
| Default | ≤ 0.25 | ≤ 0.25 % | all |
| Line-dominant (only if [S1 (#22)](https://github.com/tfpickard/isotop/issues/22) records it) | ≤ 0.5 | ≤ 1.0 % | named by [S1 (#22)](https://github.com/tfpickard/isotop/issues/22) |

These values are provisional until [S1 (#22)](https://github.com/tfpickard/isotop/issues/22) replaces them with measured numbers. A view PR never loosens them.

- `Gpu::new` refuses software adapters, and CI has no hardware GPU on Linux. Without a GPU, take both PNGs from the `parity-macOS` CI artifact (from [S1 (#22)](https://github.com/tfpickard/isotop/issues/22)) and say so in the PR.
- Attach both PNGs and the script output to the PR.

## Performance budget

- Each view holds at least 10 fps on the CPU rasterizer at 1920x1080 with the demo's 192 processes, and at least 5 fps at `--processes 1000`. Simulation work counts against the budget. Headless output is 16:9, so `--width 1920` is 1080p.
- Measure both runs:

  ```sh
  ./target/release/isotop --demo --view <name> --renderer cpu --width 1920 --benchmark 120
  ./target/release/isotop --demo --view <name> --renderer cpu --width 1920 --processes 1000 --benchmark 120
  ```

  Report the results as `| Run | ms/frame | fps | visible | target |`. Add GPU rows where a GPU is available.
- Known CPU costs:
  - A triangle scans its whole bounding box, so split long thin geometry. [S3 (#24)](https://github.com/tfpickard/isotop/issues/24) does this.
  - Glow cost grows with the square of the radius, so cap glow radii and counts.
  - The sky is painted per pixel.
- Building blocks report the cost of their background pass, in milliseconds, on a live machine.

## Tests

- Tests live in a `tests` module at the bottom of the file. Name them as sentences about behaviour, e.g. `worldlines_keep_their_seat_when_resources_change`. Rendering tests use `model::demo` or the `process(pid, parent)` helper in `render.rs`.
- Every view has a layout-stability test: a process keeps its place when CPU, memory or the population changes. State exactly what is frozen, e.g. "closed rows are frozen; the open row may re-pack".
- Every simulation view has a test that 10 fps and 60 fps produce the same state at the same wall-clock time. Coop's `flock_follows_the_same_path_at_ten_and_sixty_frames_per_second` is the model.
- Every view passes `every_view_renders_the_demo_and_an_empty_machine` in `render.rs`. It must show something, have positions, and have something to click. After [P1 (#17)](https://github.com/tfpickard/isotop/issues/17) the test iterates the registry.
- Parsers load fixtures from `src/platform/<os>/fixtures/` with `include_str!` or byte arrays. Each fixture's header states where it came from.
- Readers of `/proc` and `/sys` take an injectable root path. Tests that build trees use a unique directory under `std::env::temp_dir()` and remove it afterwards.
- The smoke test, `scripts/smoke_terminal.py`, gets one line per new view in its list of views (from [P2 (#18)](https://github.com/tfpickard/isotop/issues/18)). The view is visited through the `v` picker, and its `/ NAME /` header is asserted. A view that `--list-views` does not report on the runner is skipped. Every existing assertion stays.
- Never weaken or delete a test to make a PR pass.

## Keys

**Global keys**, reserved in every view:

- q, Ctrl-C, Tab, Shift+Tab
- arrows, w a s d
- + = -, e, Q, PgUp, PgDn
- t, l, h, 0-9, g, c, Home, r
- ?, /, n, f, Esc, Space, [, ]
- v (the picker, from [P2 (#18)](https://github.com/tfpickard/isotop/issues/18))
- every capital letter, kept for future global use

**View-local keys** may use only b i j k m o p u x y z , . ; and only when all of these hold:

- the view is shown;
- no search or picker is open;
- the key arrives through the registry row's key hook, never through a new arm in `App::command`.

Rules:

- `m` is the conventional key for a view's alternate mode.
- Home, r or Esc may gain an extra view effect only if the global meaning still happens, at most one press later.
- No new view gets a digit. Digits stay bound to the first ten registry rows; never reorder those rows. New views are reached by Tab within the playlist, the `v` picker, `--view` or `--views`.
- List each view-local key in the view's README section, in `## Controls`, and on the `?` help line while the view is shown.
- Per-view options that are not keys are flags named `--<view>-<option>`.

Assignments:

| View | Keys | Notes |
|---|---|---|
| chamber | `p` | Print/negative toggle. Global `t` is unchanged. |
| hr | `m` | Sky/diagram toggle. The mockup's `d` is pan-right, so it is not used. |
| tesseract | `,` `.` `;` `b` | While boosted, the first Esc returns to the lab frame and the next Esc has its global meaning. Home and `r` also reset the 4D orientation. |
| ouija | quit effect on `q` | Interactive runs only, never under `--duration`, `--output` or `--benchmark`. The effect lasts about 1 s at most. A second `q` quits at once, and Ctrl-C always quits at once. This is not a rebinding. |
| horizon | none | PgUp/PgDn, e/Q, right-drag and zoom set inclination, azimuth and distance as a pure function of the global camera. |
| cortex | none | Global `c` cycles axons. |
| all other new views | none | |

## Shared helpers

| Need | Use |
|---|---|
| Registry row, availability, `--verbose`, `--list-views` | [P1 (#17)](https://github.com/tfpickard/isotop/issues/17) |
| Picker, playlist, `--views` | [P2 (#18)](https://github.com/tfpickard/isotop/issues/18) |
| Colour grid or lit heightfield with owner grid, ground-aligned or four-corner planar | [P3 (#19)](https://github.com/tfpickard/isotop/issues/19) |
| FFT and DFT, 3D k-nearest, Gaussian RNG | [P4 (#20)](https://github.com/tfpickard/isotop/issues/20) |
| User/system split, scheduling policy, syscall I/O split, major faults, start time | [P5 (#21)](https://github.com/tfpickard/isotop/issues/21) |
| CPU/GPU comparison and tolerance | [S1 (#22)](https://github.com/tfpickard/isotop/issues/22) |
| Fixed-step clock | [S2 (#23)](https://github.com/tfpickard/isotop/issues/23) |
| Thick, dashed or tubular strokes | [S3 (#24)](https://github.com/tfpickard/isotop/issues/24) |
| Demo births, exits and exact CPU seconds | [S4 (#25)](https://github.com/tfpickard/isotop/issues/25) |
| Magnitude scale, point-spread stars, nova and supernova flashes | [S5 (#26)](https://github.com/tfpickard/isotop/issues/26) |
| Source demand, `top_n`, legend notes, `Snapshot.sources` | [S6 (#27)](https://github.com/tfpickard/isotop/issues/27) |
| Per-process CPU and memory history | [B1 (#28)](https://github.com/tfpickard/isotop/issues/28) |
| Unix socket queues, pipes, loopback TCP rates | [B2 (#29)](https://github.com/tfpickard/isotop/issues/29) |
| Fork, exec and exit events | [B3 (#30)](https://github.com/tfpickard/isotop/issues/30) |
| Interrupts, softirqs, IRQ affinity, `oom_kill` | [B4 (#31)](https://github.com/tfpickard/isotop/issues/31) |
| Blackbody colour | [B5 (#32)](https://github.com/tfpickard/isotop/issues/32) |
| OOM score and `smaps_rollup` | [B6 (#33)](https://github.com/tfpickard/isotop/issues/33) |
| Per-thread CPU and names | [B7 (#34)](https://github.com/tfpickard/isotop/issues/34) |
| Per-file I/O attribution | [B8 (#35)](https://github.com/tfpickard/isotop/issues/35) |
| Non-process pickables and cards | [B9 (#36)](https://github.com/tfpickard/isotop/issues/36) |
| Hershey stroke text | [B10 (#37)](https://github.com/tfpickard/isotop/issues/37) |
| Persistent view state | [B11 (#38)](https://github.com/tfpickard/isotop/issues/38) |
| Thermal sensors, topology, throttle counts, package power | [B12 (#39)](https://github.com/tfpickard/isotop/issues/39) |
| Stable seats and group discs | `pack::Seats`, `pack::Discs` (today) |
| Sunflower placement | `render::vacant`, `render::GOLDEN_ANGLE` (today) |
| 2D neighbour queries, seeded RNG | `simulation::SpatialHash`, `simulation::Rng` (today) |
| Grid placement in world space | `medium::Grid` (today) |
| Birth growth | `Stage::growth` (today) |
| World-space triangles and quads | `Frame::facet`, `Frame::quad`, `Frame::line` (today) |
| Memory label per platform | `platform::MEMORY_LABEL` (today) |

## Persistent state

- Persistent state goes only through [B11 (#38)](https://github.com/tfpickard/isotop/issues/38): one file per view, a version number per file, and saves made by its `Saver` thread with atomic replace.
- Never write in `--demo`, `--output` or `--benchmark` runs, or under sudo. The status strip says when saving is off.
- A corrupt or old-version file means a fresh start plus a status note, never a crash.
- The smoke test sets `XDG_DATA_HOME` to a temporary directory.

## Working without an approval loop

The brief tells implementers to wait for approval and to file issues. That does not apply here. An implementer works from its issue alone:

1. Read `AGENTS.md`, this document, the issue, and the brief section it links. For views, also open the mockup `docs/mockups/<view>.jpg` (hr also has `docs/mockups/hr-diagram.jpg`). Read `docs/mockups/code/<view>.py` for numbers, technique and motion. Do not run it: it contains absolute paths from another machine. There are no GIF mockups in the repository; where the brief mentions one, use the `.py` prototype instead.
2. Open the PR description with `## Plan`: one short paragraph each on data, simulation, rendering, layout and degradation.
3. Follow it with `## Decisions`, recording every choice the issue leaves open and why. See "When an issue leaves something open".
4. Implement, run the gates, open the PR, and stop. One item per PR. Do not start another item.

## When an issue leaves something open

- Decide it yourself. Use, in order: the brief section, the mockup, the prototype in `docs/mockups/code/`, and the nearest precedent in the existing code.
- Record the choice and its reason in one or two lines under `## Decisions` in the PR, then continue. Do not wait for an answer.
- An issue item marked "implementer decides and records under ## Decisions" is such a choice.
- Stop only when a decision would change another issue's API: a registry row field, a type, function or field that another issue defines or consumes, or a `Snapshot` field another issue names. In that case implement nothing that depends on the decision, write the question and the options you see under `## Decisions`, and open the PR as a draft.

## PR checklist

- [ ] Branch `feat/view-<name>` or `feat/<slug>`, cut from an up-to-date `main`. The PR body says `Closes #<issue>`.
- [ ] Commits are attributed to the human author only, with no AI co-author trailers.
- [ ] `## Plan` and `## Decisions` open the PR description.
- [ ] The gates pass in order:
  - `cargo fmt && cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test`
  - `cargo build --release`
  - `python3 scripts/smoke_terminal.py`
- [ ] `cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings` is clean.
- [ ] The issue's named tests pass, including layout stability and, for simulations, the 10-versus-60 fps test. No test was weakened or deleted.
- [ ] The smoke test visits the view.
- [ ] CPU and GPU PNGs of the same `--demo --time` frame are attached, with `compare_png.py` output within tolerance. The GPU PNG comes from a GPU machine or from the `parity-macOS` artifact.
- [ ] A CPU render is placed next to the mockup.
- [ ] A benchmark table is included for 192 processes (at least 10 fps on the CPU at 1080p) and for `--processes 1000` (at least 5 fps).
- [ ] The README section opens with the mapping table, and everything visible is either in the table or marked decorative.
- [ ] The legend line names the main mappings and follows "Degradation and the status strip".
- [ ] The demo synthesizes every consumed field.
- [ ] Platform behaviour follows "Platform availability".
- [ ] A new `Process`, `Snapshot` or `RawProcess` field follows "Demo synthesis", and a new module has a row in the `AGENTS.md` file table.
- [ ] For any change to shared code, existing views' demo PNGs are byte-identical (`cmp` output in the PR).
